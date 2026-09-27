// SPDX-License-Identifier: Apache-2.0
//! Finding the symbols that may answer a query: seeds from names and text, their neighbors in the
//! code graph, the symbols that were used together with them in the past, and the learned bias.
//!
//! Every step reads the storage and adds to a [`CandidateSet`]; none of them writes anything.

use std::collections::BTreeMap;

use pn_ultramemory_core::{
    Confidence, Direction, EdgeKind, SearchHit, SearchQuery, SymbolId, SymbolRecord, Target,
};

use super::candidates::{CandidateSet, EdgeBook, SeenEdge};
use super::ranking::{
    coaccess_relevance, content_terms, neighbor_relevance, partial_relevance, split_words,
    text_relevance,
};
use super::tuning;
use crate::engine::Engine;
use crate::error::EngineError;

/// What gathering produced.
pub(super) struct Gathered {
    /// The candidates, with learned multipliers applied.
    pub(super) set: CandidateSet,
    /// The edges seen while expanding, for the relations of the capsule.
    pub(super) edges: Vec<SeenEdge>,
}

/// Whether a path is inside the optional prefix.
fn under_prefix(path: &str, prefix: Option<&str>) -> bool {
    prefix.is_none_or(|prefix| path.starts_with(prefix))
}

/// Gathers the candidates for a query.
///
/// # Errors
/// Returns the storage error if any read fails.
pub(super) fn gather(
    engine: &Engine,
    text: &str,
    prefix: Option<&str>,
    now: i64,
) -> Result<Gathered, EngineError> {
    let mut set = CandidateSet::default();
    exact_seeds(engine, text, prefix, &mut set)?;
    text_seeds(engine, text, prefix, &mut set)?;
    let seeds = set.top(tuning::EXPAND_SEEDS);
    let mut book = EdgeBook::default();
    expand(engine, &seeds, prefix, &mut set, &mut book)?;
    coaccess(engine, &seeds, prefix, now, &mut set)?;
    apply_learning(engine, &mut set);
    Ok(Gathered {
        set,
        edges: book.into_vec(),
    })
}

/// Adds the symbols whose name is the query, when the query is a single word.
fn exact_seeds(
    engine: &Engine,
    text: &str,
    prefix: Option<&str>,
    set: &mut CandidateSet,
) -> Result<(), EngineError> {
    if text.chars().any(char::is_whitespace) {
        return Ok(());
    }
    let ask = if prefix.is_some() {
        tuning::EXACT_LIMIT * 5
    } else {
        tuning::EXACT_LIMIT
    };
    let found = engine.storage().find_symbols(text, ask)?;
    for symbol in found
        .iter()
        .filter(|symbol| under_prefix(&symbol.path, prefix))
        .take(tuning::EXACT_LIMIT)
    {
        set.offer(symbol, tuning::EXACT_RELEVANCE, "exact name match");
    }
    Ok(())
}

/// The best score among search hits, or negative infinity when there are none.
fn best_score(hits: &[SearchHit]) -> f64 {
    hits.iter()
        .map(|hit| hit.score)
        .fold(f64::NEG_INFINITY, f64::max)
}

/// Runs one text search.
fn search(
    engine: &Engine,
    text: &str,
    prefix: Option<&str>,
    limit: usize,
) -> Result<Vec<SearchHit>, EngineError> {
    let query = SearchQuery {
        text: text.to_owned(),
        kinds: Vec::new(),
        path_prefix: prefix.map(str::to_owned),
    };
    Ok(engine.storage().search_symbols(&query, limit)?)
}

/// Adds the symbols that match the words of the query, relaxing the search when it finds little.
fn text_seeds(
    engine: &Engine,
    text: &str,
    prefix: Option<&str>,
    set: &mut CandidateSet,
) -> Result<(), EngineError> {
    let hits = search(engine, text, prefix, tuning::TEXT_LIMIT)?;
    let best = best_score(&hits);
    for hit in &hits {
        set.offer(&hit.symbol, text_relevance(hit.score, best), "text match");
    }
    if hits.len() < tuning::RELAX_BELOW {
        relaxed_seeds(engine, text, prefix, hits.len(), set)?;
    }
    Ok(())
}

/// Searches again with the meaning of the query only, and then word by word.
///
/// The store requires every word of a query to match, which is right for a name and for a phrase
/// copied from documentation but returns nothing for a question. First the stop words are dropped
/// (when that changes the query); if there are still few results, each word is searched alone and a
/// symbol scores by how many of the words it matched.
fn relaxed_seeds(
    engine: &Engine,
    text: &str,
    prefix: Option<&str>,
    already: usize,
    set: &mut CandidateSet,
) -> Result<(), EngineError> {
    let terms = content_terms(text, tuning::RELAX_TERMS);
    if terms.is_empty() {
        return Ok(());
    }
    let mut found = already;
    let mut all_words = split_words(text);
    all_words.sort_unstable();
    all_words.dedup();
    if terms.len() < all_words.len() {
        let hits = search(engine, &terms.join(" "), prefix, tuning::TEXT_LIMIT)?;
        let best = best_score(&hits);
        for hit in &hits {
            let relevance = text_relevance(hit.score, best) * tuning::RELAXED_FACTOR;
            set.offer(&hit.symbol, relevance, "text match");
        }
        found += hits.len();
    }
    if found >= tuning::RELAX_BELOW || terms.len() < 2 {
        return Ok(());
    }
    let mut matched: BTreeMap<SymbolId, (SymbolRecord, f64)> = BTreeMap::new();
    for term in &terms {
        let hits = search(engine, term, prefix, tuning::RELAX_PER_TERM_LIMIT)?;
        let best = best_score(&hits);
        for hit in hits {
            let share = if best > 0.0 {
                (hit.score / best).clamp(0.0, 1.0)
            } else {
                0.5
            };
            let entry = matched
                .entry(hit.symbol.id)
                .or_insert_with(|| (hit.symbol.clone(), 0.0));
            entry.1 += share;
        }
    }
    for (record, sum) in matched.values() {
        set.offer(
            record,
            partial_relevance(*sum, terms.len()),
            "partial text match",
        );
    }
    Ok(())
}

/// The reason a neighbor is a candidate, written from the neighbor's point of view.
///
/// Following an outgoing edge of a seed reaches what the seed uses, so the neighbor is "called by"
/// the seed; following an incoming edge reaches what uses the seed, so the neighbor "calls" it.
fn neighbor_reason(
    direction: Direction,
    kind: EdgeKind,
    seed: &str,
    confidence: Confidence,
) -> String {
    let verb = match (direction, kind) {
        (Direction::Out, EdgeKind::Calls) => "called by",
        (Direction::Out, EdgeKind::Inherits) => "extended by",
        (Direction::Out, EdgeKind::Uses) => "used by",
        (Direction::In, EdgeKind::Calls) => "calls",
        (Direction::In, EdgeKind::Inherits) => "extends",
        (Direction::In, EdgeKind::Uses) => "uses",
    };
    format!("{verb} {seed} ({confidence})")
}

/// Adds the callers and callees of the best seeds, and remembers the edges that led to them.
fn expand(
    engine: &Engine,
    seeds: &[(SymbolId, f64)],
    prefix: Option<&str>,
    set: &mut CandidateSet,
    book: &mut EdgeBook,
) -> Result<(), EngineError> {
    for (seed, seed_relevance) in seeds {
        let Some(name) = set.get(*seed).map(|c| c.record.qualified_name.clone()) else {
            continue;
        };
        for direction in [Direction::In, Direction::Out] {
            let neighbors = engine.storage().neighbors(
                *seed,
                direction,
                Confidence::Heuristic,
                tuning::NEIGHBOR_LIMIT,
            )?;
            for neighbor in neighbors {
                if !under_prefix(&neighbor.symbol.path, prefix) {
                    continue;
                }
                book.add(SeenEdge {
                    src: neighbor.edge.src,
                    dst: neighbor.edge.dst,
                    kind: neighbor.edge.kind,
                    confidence: neighbor.edge.confidence,
                });
                let relevance = neighbor_relevance(*seed_relevance, neighbor.edge.confidence);
                let reason = neighbor_reason(
                    direction,
                    neighbor.edge.kind,
                    &name,
                    neighbor.edge.confidence,
                );
                set.offer(&neighbor.symbol, relevance, &reason);
            }
        }
    }
    Ok(())
}

/// Adds the symbols that were used together with the best seeds in the past.
fn coaccess(
    engine: &Engine,
    seeds: &[(SymbolId, f64)],
    prefix: Option<&str>,
    now: i64,
    set: &mut CandidateSet,
) -> Result<(), EngineError> {
    for (seed, seed_relevance) in seeds.iter().take(tuning::COACCESS_SEEDS) {
        let Some(name) = set.get(*seed).map(|c| c.record.qualified_name.clone()) else {
            continue;
        };
        let pairs = engine
            .storage()
            .coaccess_neighbors(*seed, tuning::COACCESS_LIMIT, now)?;
        for (other, weight) in pairs {
            let relevance = coaccess_relevance(*seed_relevance, weight);
            if relevance <= 0.0 {
                continue;
            }
            let record = match set.get(other) {
                Some(known) => Some(known.record.clone()),
                None => engine.storage().symbol(other)?,
            };
            let Some(record) = record else {
                continue;
            };
            if under_prefix(&record.path, prefix) {
                let reason = format!("used together with {name} (learned)");
                set.offer(&record, relevance, &reason);
            }
        }
    }
    Ok(())
}

/// Scales every candidate by what was learned about it.
fn apply_learning(engine: &Engine, set: &mut CandidateSet) {
    if set.is_empty() {
        return;
    }
    let targets: Vec<Target> = set.ids().into_iter().map(Target::Symbol).collect();
    let multipliers = engine.multipliers(&targets);
    set.apply_multipliers(&|id| multipliers.get(&Target::Symbol(id)).copied());
}
