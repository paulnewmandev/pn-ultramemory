// SPDX-License-Identifier: Apache-2.0
//! Building a capsule for a query: seeds, graph expansion, learning bias, packing and rendering.
//!
//! [`Engine::recall`] answers one question with the code and memories that matter for it, packed
//! into a token budget. The steps, each in its own module:
//!
//! 1. **Seeds** (`seeds`): symbols whose name is the query, and symbols whose names, signatures or
//!    documentation match its words (`relevance` from `1.0` for an exact name, and from `0.25` to
//!    `1.0` by text score).
//! 2. **Graph expansion** (`seeds`): callers and callees of the best seeds, with a share of the
//!    seed's relevance that depends on how sure the indexer is about the edge, and symbols that were
//!    used together with the seeds before.
//! 3. **Learning** (`seeds`): what was learned about each symbol scales its relevance, within bounds.
//! 4. **Candidates** (`candidates`): at most sixty, best first.
//! 5. **Levels** (`assemble`): each candidate can be shown from name only to full source, priced by
//!    the codec, and the packer chooses one level per symbol under the budget left after the
//!    memories and the frame of the capsule.
//! 6. **Guarantee** (`assemble`): the capsule is measured as it will be printed and shrunk until it
//!    fits, so `used` never exceeds the budget.
//!
//! Every number is in [`tuning`]. Nothing is written to the index; only the session record (what
//! was shown) and the local metrics are touched.
//!
//! # Determinism
//! Identical index, learning state and query give an identical capsule: every collection is
//! ordered and every tie is broken by symbol identity.

mod assemble;
mod candidates;
mod glossary;
pub(crate) mod ranking;
mod seeds;
pub(crate) mod tuning;

use std::collections::BTreeMap;
use std::time::Instant;

use pn_ultramemory_codec::{Capsule, CapsuleMemory, estimate_tokens};
use pn_ultramemory_core::{MemoryId, MemoryRecord, SymbolId, Target};

use crate::engine::Engine;
use crate::error::EngineError;
use crate::metrics::Event;
use crate::sources::SourceCache;
use assemble::{Prepared, Settings, assemble, options_for, why_cost};
use candidates::Cand;
use ranking::clip_chars;

/// A question for [`Engine::recall`].
///
/// # Examples
/// ```
/// use pn_ultramemory_engine::RecallQuery;
///
/// let query = RecallQuery { text: "parse config".into(), budget: Some(800), ..RecallQuery::default() };
/// assert!(!query.explain);
/// assert_eq!(query.path_prefix, None);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RecallQuery {
    /// What to look for: a symbol name, or words describing what the code does.
    pub text: String,
    /// The token budget of the capsule, or `None` for the engine's default.
    pub budget: Option<u32>,
    /// Whether every symbol says why it was chosen.
    pub explain: bool,
    /// Only consider symbols whose path starts with this prefix.
    pub path_prefix: Option<String>,
}

/// The candidates as the packer will see them, with the source of each one read once.
///
/// Returns them together with the ranked candidates' identities and relevances, which the memory
/// lookup needs.
fn prepare(
    engine: &Engine,
    ranked: Vec<Cand>,
    explain: bool,
) -> Result<Vec<Prepared>, EngineError> {
    let max_tokens = engine.config.max_source_tokens;
    let readable_bytes = u64::from(max_tokens) * u64::from(tuning::SOURCE_BYTES_PER_TOKEN);
    let mut cache = SourceCache::new(engine);
    let mut prepared = Vec::with_capacity(ranked.len());
    for cand in ranked {
        let (source, stale_source) = if u64::from(cand.record.span.len_bytes()) > readable_bytes {
            (None, false)
        } else {
            match cache.slice(&cand.record)? {
                Some(text) => (Some(text), false),
                None => (None, true),
            }
        };
        let why = cand.why();
        let options = options_for(
            &cand.record,
            source.as_deref(),
            max_tokens,
            why_cost(&why, explain),
        );
        prepared.push(Prepared {
            record: cand.record,
            relevance: cand.relevance,
            why,
            options,
            stale_source,
        });
    }
    Ok(prepared)
}

/// The memories that belong in the capsule, best first, within a quarter of the budget.
///
/// A memory anchored to a candidate is as relevant as the best candidate it is anchored to, a memory
/// that matched the query text alone is worth [`tuning::MEMORY_TEXT_RELEVANCE`], the learned
/// multiplier applies, and a stale memory counts for [`tuning::STALE_MEMORY_FACTOR`] of that.
fn choose_memories(
    engine: &Engine,
    text: &str,
    ranked: &[Prepared],
    budget: u32,
) -> Result<Vec<CapsuleMemory>, EngineError> {
    let relevance_of: BTreeMap<SymbolId, f64> =
        ranked.iter().map(|p| (p.record.id, p.relevance)).collect();
    let ids: Vec<SymbolId> = ranked.iter().map(|p| p.record.id).collect();
    let mut found: BTreeMap<MemoryId, (MemoryRecord, f64)> = BTreeMap::new();
    if !ids.is_empty() {
        for memory in engine
            .storage()
            .memories_for_symbols(&ids, tuning::MEMORY_SYMBOL_LIMIT)?
        {
            let best = memory
                .anchors
                .iter()
                .filter_map(|anchor| anchor.symbol)
                .filter_map(|id| relevance_of.get(&id).copied())
                .fold(0.0_f64, f64::max);
            found.insert(memory.id, (memory, best));
        }
    }
    for memory in engine
        .storage()
        .search_memories(text, tuning::MEMORY_TEXT_LIMIT)?
    {
        let entry = found
            .entry(memory.id)
            .or_insert_with(|| (memory, tuning::MEMORY_TEXT_RELEVANCE));
        entry.1 = entry.1.max(tuning::MEMORY_TEXT_RELEVANCE);
    }
    if found.is_empty() {
        return Ok(Vec::new());
    }
    let targets: Vec<Target> = found.keys().map(|id| Target::Memory(*id)).collect();
    let multipliers = engine.multipliers(&targets);
    let mut scored: Vec<(f64, MemoryRecord)> = found
        .into_values()
        .map(|(memory, base)| {
            let multiplier = multipliers
                .get(&Target::Memory(memory.id))
                .copied()
                .filter(|m| m.is_finite() && *m > 0.0)
                .unwrap_or(1.0);
            let stale = if memory.stale_since.is_some() {
                tuning::STALE_MEMORY_FACTOR
            } else {
                1.0
            };
            (base * multiplier * stale, memory)
        })
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.id.cmp(&b.1.id)));
    let allowance = budget / tuning::MEMORY_BUDGET_DIVISOR;
    let mut spent = 0_u32;
    let mut chosen = Vec::new();
    for (_, memory) in scored {
        let cost = estimate_tokens(&memory.text) + tuning::MEMORY_ROW_TOKENS;
        if spent + cost > allowance {
            continue;
        }
        spent += cost;
        chosen.push(CapsuleMemory {
            id: memory.id,
            kind: memory.kind,
            provenance: memory.provenance,
            stale: memory.stale_since.is_some(),
            text: memory.text,
        });
    }
    Ok(chosen)
}

/// The number of characters of the query printed back for a budget: about a quarter of the budget
/// in tokens, but never fewer than [`tuning::QUERY_ECHO_MIN_CHARS`] or more than
/// [`tuning::QUERY_ECHO_CHARS`].
fn echo_chars(budget: u32) -> usize {
    usize::try_from(budget)
        .unwrap_or(usize::MAX)
        .clamp(tuning::QUERY_ECHO_MIN_CHARS, tuning::QUERY_ECHO_CHARS)
}

impl Engine {
    /// Answers a query with a capsule: the symbols and memories that matter for it, each at the
    /// level of detail that pays off best, measured to fit the token budget.
    ///
    /// The capsule is measured as it is printed by the default TOON format, and its `used` never
    /// exceeds its `budget`, unless the budget is smaller than an empty capsule, in which case the
    /// capsule is empty and says so in a note. With `explain` every symbol has a `why`. If the file
    /// of a symbol changed since it was indexed, its source is not offered and a note says so.
    ///
    /// Side effects: the symbols shown are recorded for learning and a metrics event is written;
    /// nothing else is stored.
    ///
    /// # Errors
    /// Returns [`EngineError::Invalid`] for an empty or blank query and a storage error if the index
    /// cannot be read.
    pub fn recall(&self, query: &RecallQuery) -> Result<Capsule, EngineError> {
        let started = Instant::now();
        let text = query.text.trim();
        if text.is_empty() {
            return Err(EngineError::Invalid("the query is empty".into()));
        }
        let budget = self.config.effective_budget(query.budget);
        let prefix = query
            .path_prefix
            .as_deref()
            .map(|p| p.trim().trim_start_matches("./"))
            .filter(|p| !p.is_empty());

        let gathered = seeds::gather(self, text, prefix, self.now())?;
        let edges = gathered.edges;
        let ranked = gathered.set.into_ranked(tuning::MAX_CANDIDATES);
        let mut notes = Vec::new();
        if ranked.is_empty() && self.storage().stats()?.files == 0 {
            notes.push("the index is empty; run the index command first".to_owned());
        }
        let prepared = prepare(self, ranked, query.explain)?;
        let memories = choose_memories(self, text, &prepared, budget)?;

        let settings = Settings {
            query: clip_chars(text, echo_chars(budget)),
            budget,
            explain: query.explain,
            edges: &edges,
            notes,
        };
        let assembled = assemble(&settings, &prepared, memories);
        self.note_shown(&assembled.shown);
        self.record(Event::Recall {
            budget,
            used: assembled.capsule.used,
            symbols: u32::try_from(assembled.capsule.symbols.len()).unwrap_or(u32::MAX),
            elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        });
        Ok(assembled.capsule)
    }
}
