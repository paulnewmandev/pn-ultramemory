// SPDX-License-Identifier: Apache-2.0
//! Turning ranked candidates into a capsule that fits its budget.
//!
//! The steps, in order:
//!
//! 1. [`options_for`] lists how each candidate can be shown (levels L0 to L4) with what each costs.
//! 2. [`assemble`] reserves room for what surrounds the symbols (the measured frame of the capsule
//!    and the memories), asks the packer for a level per symbol, and corrects the reserve for the
//!    files and relations of the actual selection.
//! 3. It composes the capsule and **measures it as it will be printed**. While it is over the
//!    budget it lowers or drops the selection with the least value per token saved, then the
//!    memories, and measures again, so the result never exceeds the budget.

use std::collections::BTreeMap;

use pn_ultramemory_codec::{
    Candidate, Capsule, CapsuleMemory, CapsuleRelation, CapsuleSymbol, LevelOption, RenderOptions,
    SymbolView, estimate_tokens, measure, pack, symbol_cost, symbol_text,
};
use pn_ultramemory_core::{Detail, SymbolId, SymbolRecord, first_sentence};

use super::candidates::SeenEdge;
use super::ranking::clip_chars;
use super::tuning;

/// One way of showing a symbol.
#[derive(Debug, Clone)]
pub(super) struct Choice {
    /// The level of detail.
    pub(super) detail: Detail,
    /// What it costs in the printed capsule, in estimated tokens.
    pub(super) tokens: u32,
    /// How much it is worth compared with the other levels.
    pub(super) utility: f64,
    /// The text shown at this level.
    pub(super) text: String,
}

/// A candidate ready to be packed.
#[derive(Debug, Clone)]
pub(super) struct Prepared {
    /// The symbol.
    pub(super) record: SymbolRecord,
    /// How relevant it is.
    pub(super) relevance: f64,
    /// Why it is a candidate.
    pub(super) why: String,
    /// The ways of showing it, cheapest first.
    pub(super) options: Vec<Choice>,
    /// Whether its file changed since it was indexed, so that its source is not offered.
    pub(super) stale_source: bool,
}

/// What the capsule around the symbols is made of.
#[derive(Debug, Clone)]
pub(super) struct Settings<'a> {
    /// The query as printed back.
    pub(super) query: String,
    /// The token budget.
    pub(super) budget: u32,
    /// Whether every symbol carries its reason.
    pub(super) explain: bool,
    /// The edges seen while expanding the seeds.
    pub(super) edges: &'a [SeenEdge],
    /// Remarks that are always printed, such as "the index is empty".
    pub(super) notes: Vec<String>,
}

/// The finished capsule and what was shown, for the learning hooks.
#[derive(Debug, Clone)]
pub(super) struct Assembled {
    /// The capsule, measured to fit its budget.
    pub(super) capsule: Capsule,
    /// The symbols shown and the level each was shown at.
    pub(super) shown: Vec<(SymbolId, Detail)>,
}

/// The ways a symbol can be shown, cheapest first.
///
/// L0 is always there. L1 needs a signature, L2 a first documentation sentence, L3 a non-empty
/// outline, and L4 the source, when it is available and costs at most `max_source_tokens`. A level
/// whose text is identical to the previous one is left out. With explanations on, `why_tokens` are
/// added to every level that is printed as a table row.
pub(super) fn options_for(
    record: &SymbolRecord,
    source: Option<&str>,
    max_source_tokens: u32,
    why_tokens: u32,
) -> Vec<Choice> {
    let summary = first_sentence(record.doc.as_deref().unwrap_or(""));
    let source_text = source.unwrap_or("");
    let view = SymbolView {
        qualified_name: &record.qualified_name,
        signature: &record.signature,
        summary: &summary,
        outline: &record.outline,
        source: source_text,
    };
    let source_fits = !source_text.is_empty() && estimate_tokens(source_text) <= max_source_tokens;
    let ladder = [
        (Detail::Name, tuning::UTILITY_NAME, true),
        (
            Detail::Signature,
            tuning::UTILITY_SIGNATURE,
            !record.signature.is_empty(),
        ),
        (
            Detail::Summary,
            tuning::UTILITY_SUMMARY,
            !summary.is_empty(),
        ),
        (
            Detail::Outline,
            tuning::UTILITY_OUTLINE,
            !record.outline.is_empty(),
        ),
        (Detail::Source, tuning::UTILITY_SOURCE, source_fits),
    ];
    let mut options: Vec<Choice> = Vec::new();
    for (detail, utility, offered) in ladder {
        if !offered {
            continue;
        }
        let text = symbol_text(&view, detail);
        if options.last().is_some_and(|last| last.text == text) {
            continue;
        }
        let mut tokens = symbol_cost(&view, detail);
        if detail != Detail::Name {
            tokens += why_tokens;
        }
        options.push(Choice {
            detail,
            tokens,
            utility,
            text,
        });
    }
    options
}

/// The tokens the explanation of a symbol adds to its row.
pub(super) fn why_cost(why: &str, explain: bool) -> u32 {
    if explain { estimate_tokens(why) + 1 } else { 0 }
}

/// The estimated tokens of a memory: its text and what surrounds it in the table.
pub(super) fn memory_tokens(memory: &CapsuleMemory) -> u32 {
    estimate_tokens(&memory.text) + tuning::MEMORY_ROW_TOKENS
}

/// The level chosen for each candidate, as an index into its options, or `None` for not shown.
type Chosen = Vec<Option<usize>>;

/// Asks the packer for a level per candidate within `available` tokens.
fn run_pack(prepared: &[Prepared], available: u32) -> Chosen {
    let candidates: Vec<Candidate<usize>> = prepared
        .iter()
        .enumerate()
        .map(|(key, p)| Candidate {
            key,
            relevance: p.relevance,
            options: p
                .options
                .iter()
                .map(|o| LevelOption {
                    detail: o.detail,
                    tokens: o.tokens,
                    utility: o.utility,
                })
                .collect(),
        })
        .collect();
    let mut chosen = vec![None; prepared.len()];
    for selection in pack(candidates, available).selections {
        let position = prepared[selection.key]
            .options
            .iter()
            .position(|o| o.detail == selection.detail);
        chosen[selection.key] = position;
    }
    chosen
}

/// The relations between the selected symbols: structural edges only, strongest pairs first, at
/// most [`tuning::MAX_RELATIONS`].
fn relations(
    settings: &Settings<'_>,
    prepared: &[Prepared],
    chosen: &[Option<usize>],
) -> Vec<CapsuleRelation> {
    let selected: BTreeMap<SymbolId, usize> = prepared
        .iter()
        .enumerate()
        .filter(|(index, _)| chosen[*index].is_some())
        .map(|(index, p)| (p.record.id, index))
        .collect();
    let mut found: Vec<(f64, &SeenEdge, usize, usize)> = Vec::new();
    for edge in settings.edges {
        if !edge.confidence.is_structural() || edge.src == edge.dst {
            continue;
        }
        if let (Some(&from), Some(&to)) = (selected.get(&edge.src), selected.get(&edge.dst)) {
            let weight = prepared[from].relevance.min(prepared[to].relevance);
            found.push((weight, edge, from, to));
        }
    }
    found.sort_by(|a, b| {
        b.0.total_cmp(&a.0)
            .then(a.1.src.cmp(&b.1.src))
            .then(a.1.dst.cmp(&b.1.dst))
            .then(a.1.kind.as_str().cmp(b.1.kind.as_str()))
    });
    found
        .into_iter()
        .take(tuning::MAX_RELATIONS)
        .map(|(_, edge, from, to)| CapsuleRelation {
            from: prepared[from].record.qualified_name.clone(),
            to: prepared[to].record.qualified_name.clone(),
            kind: edge.kind,
            confidence: edge.confidence,
        })
        .collect()
}

/// The remarks of the capsule: the fixed ones, the symbols whose source could not be shown because
/// the file changed, and the stale memories.
fn notes(
    settings: &Settings<'_>,
    prepared: &[Prepared],
    chosen: &[Option<usize>],
    memories: &[CapsuleMemory],
) -> Vec<String> {
    let mut notes = settings.notes.clone();
    let stale: Vec<&str> = prepared
        .iter()
        .enumerate()
        .filter(|(index, p)| chosen[*index].is_some() && p.stale_source)
        .map(|(_, p)| p.record.qualified_name.as_str())
        .collect();
    for name in stale.iter().take(tuning::MAX_STALE_NOTES) {
        notes.push(format!(
            "source of {name} changed since indexing; run index"
        ));
    }
    if stale.len() > tuning::MAX_STALE_NOTES {
        notes.push(format!(
            "source of {} more symbols changed since indexing; run index",
            stale.len() - tuning::MAX_STALE_NOTES
        ));
    }
    let stale_memories = memories.iter().filter(|m| m.stale).count();
    if stale_memories > 0 {
        notes.push(format!(
            "{stale_memories} of the memories shown are stale: the code they describe changed"
        ));
    }
    notes
}

/// Composes the capsule for a selection. `used` is left at zero.
fn compose(
    settings: &Settings<'_>,
    prepared: &[Prepared],
    chosen: &[Option<usize>],
    memories: &[CapsuleMemory],
) -> Capsule {
    let mut files: Vec<String> = Vec::new();
    let mut symbols = Vec::new();
    for (index, p) in prepared.iter().enumerate() {
        let Some(level) = chosen[index] else {
            continue;
        };
        let option = &p.options[level];
        let file = files
            .iter()
            .position(|path| *path == p.record.path)
            .unwrap_or_else(|| {
                files.push(p.record.path.clone());
                files.len() - 1
            });
        symbols.push(CapsuleSymbol {
            id: p.record.id,
            file,
            start_line: p.record.span.start_line,
            end_line: p.record.span.end_line,
            kind: p.record.kind,
            name: p.record.qualified_name.clone(),
            detail: option.detail,
            text: option.text.clone(),
            why: settings.explain.then(|| p.why.clone()),
        });
    }
    let shown = symbols.len();
    Capsule {
        query: settings.query.clone(),
        budget: settings.budget,
        used: 0,
        omitted: u32::try_from(prepared.len() - shown).unwrap_or(u32::MAX),
        files,
        symbols,
        memories: memories.to_vec(),
        relations: relations(settings, prepared, chosen),
        notes: notes(settings, prepared, chosen, memories),
    }
}

/// The measured cost of everything in the capsule except its symbols and memories, plus the
/// reserve for the headers of their tables.
fn frame_cost(
    settings: &Settings<'_>,
    prepared: &[Prepared],
    chosen: &[Option<usize>],
    memories: &[CapsuleMemory],
) -> u32 {
    let mut frame = compose(settings, prepared, chosen, memories);
    frame.symbols.clear();
    frame.memories.clear();
    let mut cost = measure(&frame, &RenderOptions::default()) + tuning::TABLE_HEADER_TOKENS;
    if !memories.is_empty() {
        cost += tuning::MEMORY_HEADER_TOKENS;
    }
    cost
}

/// The step that gives up the least value per token saved: `(index, tokens saved)`.
///
/// A step lowers a symbol by one level, or drops it when it is at its cheapest level. Ties go to
/// the less relevant symbol, then to the later one.
fn cheapest_step_down(prepared: &[Prepared], chosen: &[Option<usize>]) -> Option<(usize, u32)> {
    let mut best: Option<(f64, f64, usize, u32)> = None;
    for (index, p) in prepared.iter().enumerate() {
        let Some(level) = chosen[index] else {
            continue;
        };
        let here = &p.options[level];
        let (lost_utility, saved) = match level.checked_sub(1) {
            Some(below) => {
                let lower = &p.options[below];
                (
                    here.utility - lower.utility,
                    here.tokens.saturating_sub(lower.tokens),
                )
            }
            None => (here.utility, here.tokens),
        };
        let ratio = p.relevance * lost_utility / f64::from(saved.max(1));
        let better = best.is_none_or(|(top_ratio, top_relevance, top_index, _)| {
            ratio
                .total_cmp(&top_ratio)
                .then(p.relevance.total_cmp(&top_relevance))
                .then(top_index.cmp(&index))
                .is_lt()
        });
        if better {
            best = Some((ratio, p.relevance, index, saved));
        }
    }
    best.map(|(_, _, index, saved)| (index, saved))
}

/// Lowers or drops symbols, cheapest loss first, until about `need` tokens are saved. Returns
/// whether anything changed.
fn shrink_symbols(prepared: &[Prepared], chosen: &mut [Option<usize>], mut need: u32) -> bool {
    let mut changed = false;
    while need > 0 {
        let Some((index, saved)) = cheapest_step_down(prepared, chosen) else {
            break;
        };
        chosen[index] = chosen[index].and_then(|level| level.checked_sub(1));
        changed = true;
        need = need.saturating_sub(saved.max(1));
    }
    changed
}

/// The capsule to return when even an empty one does not fit: nothing but the frame and a note.
fn bare_capsule(settings: &Settings<'_>) -> Capsule {
    Capsule {
        query: clip_chars(&settings.query, tuning::QUERY_ECHO_MIN_CHARS),
        budget: settings.budget,
        notes: vec!["the budget is too small for any result".to_owned()],
        ..Capsule::default()
    }
}

/// Chooses the levels, composes the capsule and makes it fit.
///
/// `memories` are already chosen and ordered by relevance; the least relevant are the first to be
/// dropped if the budget is too tight. The returned capsule is measured as printed in the default
/// format and `used` is that measure, which is at most the budget unless the budget is smaller
/// than an empty capsule.
pub(super) fn assemble(
    settings: &Settings<'_>,
    prepared: &[Prepared],
    memories: Vec<CapsuleMemory>,
) -> Assembled {
    let mut memories = memories;
    let memory_cost: u32 = memories.iter().map(memory_tokens).sum();
    let mut chosen: Chosen = vec![None; prepared.len()];
    let mut reserve = frame_cost(settings, prepared, &chosen, &memories);
    let mut available = 0;
    for _ in 0..tuning::RESERVE_ROUNDS {
        available = settings
            .budget
            .saturating_sub(memory_cost)
            .saturating_sub(reserve);
        chosen = run_pack(prepared, available);
        let needed = frame_cost(settings, prepared, &chosen, &memories);
        if needed <= reserve {
            break;
        }
        reserve = needed;
    }

    loop {
        let mut capsule = compose(settings, prepared, &chosen, &memories);
        let Err(over) = settle(&mut capsule, settings.budget) else {
            let (capsule, chosen) =
                refill(settings, prepared, &memories, capsule, chosen, available);
            return finish(capsule, prepared, &chosen);
        };
        if shrink_symbols(prepared, &mut chosen, over) {
            continue;
        }
        if memories.pop().is_some() {
            continue;
        }
        let mut bare = bare_capsule(settings);
        // Nothing is left to remove: the budget is smaller than an empty capsule.
        let _ = settle(&mut bare, settings.budget);
        return Assembled {
            capsule: bare,
            shown: Vec::new(),
        };
    }
}

/// How many times the unspent budget is offered back to the packer.
const REFILL_ROUNDS: usize = 4;

/// The least slack worth another round.
///
/// Below this there is nothing a symbol could be shown at, so another pack would return the same
/// selection and the round would only cost time.
const MIN_REFILL: u32 = 8;

/// Spends the budget the reserve asked for and did not need.
///
/// Everything before this works from an *estimate* of what the frame around the symbols costs, and
/// that estimate is deliberately generous: under-estimating it produces a capsule over the budget,
/// which has to be taken apart again. The cost is that a capsule routinely comes back well under
/// what was asked for — on this repository a budget of 200 returned no symbols at all, and 1 200
/// returned 64% of what it could have. That is worst exactly where it matters most, because a
/// small budget is what a small model is given.
///
/// So once a capsule is known to fit, the gap between what it costs and the budget is handed back
/// to the packer and the whole capsule is built again. A wider result is kept only when it is
/// measured, in full, to still fit.
///
/// A round that does not fit halves the amount offered rather than giving up, because the frame is
/// not a constant: showing a symbol at all brings in the table of file paths it refers to, so the
/// first symbol costs far more than the second. Offering the whole gap can therefore overshoot
/// while half of it fits, and stopping at the first failure is what left a 200-token budget
/// returning nothing at all while 174 of those tokens went unspent.
///
/// The capsule passed in is always a valid answer, so this can only return something at least as
/// good as what it was given.
fn refill(
    settings: &Settings<'_>,
    prepared: &[Prepared],
    memories: &[CapsuleMemory],
    capsule: Capsule,
    chosen: Chosen,
    available: u32,
) -> (Capsule, Chosen) {
    let (mut best, mut best_chosen) = (capsule, chosen);
    let mut step = settings.budget.saturating_sub(best.used);
    let mut floor = available;
    for _ in 0..REFILL_ROUNDS {
        if step < MIN_REFILL {
            break;
        }
        let wider = run_pack(prepared, floor.saturating_add(step));
        if wider == best_chosen {
            // The packer has nothing more to add at this width, so only a wider offer could
            // change anything.
            step = step.saturating_mul(2);
            continue;
        }
        let mut candidate = compose(settings, prepared, &wider, memories);
        if settle(&mut candidate, settings.budget).is_ok() && candidate.used > best.used {
            best = candidate;
            best_chosen = wider;
            floor = floor.saturating_add(step);
            step = settings.budget.saturating_sub(best.used);
        } else {
            step /= 2;
        }
    }
    (best, best_chosen)
}

/// The most rounds spent making `used` agree with the measure of the capsule that carries it.
const SETTLE_ROUNDS: usize = 8;

/// Sets `used` on a capsule to what the capsule costs as printed, and reports by how much it is
/// over the budget, if it is.
///
/// The capsule prints its own `used`, so the number is a fixed point: the capsule is first measured
/// with a stand-in as wide as the budget (which can only cost as much or more than the final
/// number), and then `used` is corrected until it equals the measure of the capsule that shows it.
///
/// # Errors
/// Returns how many tokens the capsule is over the budget.
fn settle(capsule: &mut Capsule, budget: u32) -> Result<(), u32> {
    let options = RenderOptions::default();
    capsule.used = budget;
    let measured = measure(capsule, &options);
    let mut used = measured;
    for _ in 0..SETTLE_ROUNDS {
        capsule.used = used;
        let again = measure(capsule, &options);
        if again == used {
            break;
        }
        used = again;
    }
    capsule.used = used;
    if measured > budget {
        Err(measured - budget)
    } else {
        Ok(())
    }
}

/// Pairs the capsule with the symbols it shows.
fn finish(capsule: Capsule, prepared: &[Prepared], chosen: &[Option<usize>]) -> Assembled {
    let shown = prepared
        .iter()
        .enumerate()
        .filter_map(|(index, p)| chosen[index].map(|level| (p.record.id, p.options[level].detail)))
        .collect();
    Assembled { capsule, shown }
}

#[cfg(test)]
mod tests {
    use pn_ultramemory_codec::{CapsuleMemory, RenderOptions, measure};
    use pn_ultramemory_core::{
        Confidence, Detail, EdgeKind, FileId, Language, MemoryId, MemoryKind, Provenance, Span,
        SymbolId, SymbolKind, SymbolRecord, Visibility,
    };

    use super::{Prepared, Settings, assemble, options_for, why_cost};
    use crate::recall::candidates::SeenEdge;

    /// A record with a signature, documentation and an outline.
    fn record(id: i64, name: &str, doc: Option<&str>, outline: &[&str]) -> SymbolRecord {
        SymbolRecord {
            id: SymbolId(id),
            file_id: FileId(id % 7),
            path: format!("src/file_{}.rs", id % 7),
            language: Language::Rust,
            name: name.into(),
            qualified_name: format!("module::{name}"),
            kind: SymbolKind::Function,
            signature: format!("pub fn {name}(input: &str, limit: usize) -> Vec<String>"),
            doc: doc.map(str::to_owned),
            visibility: Visibility::Public,
            span: Span {
                start_line: 10 * u32::try_from(id).unwrap_or(0),
                end_line: 10 * u32::try_from(id).unwrap_or(0) + 8,
                start_byte: 0,
                end_byte: 0,
            },
            parent: None,
            outline: outline.iter().map(|s| (*s).to_owned()).collect(),
            sig_hash: 1,
            body_hash: 2,
        }
    }

    /// A prepared candidate with every level offered.
    fn prepared(id: i64, relevance: f64, with_source: bool) -> Prepared {
        let name = format!("function_number_{id}");
        let source =
            format!("pub fn {name}(input: &str) {{\n    let x = helper(input);\n    x\n}}\n");
        let record = record(
            id,
            &name,
            Some("Does the work of the function. More text follows here."),
            &["helper", "other_call"],
        );
        let options = options_for(&record, with_source.then_some(source.as_str()), 3000, 0);
        Prepared {
            record,
            relevance,
            why: "text match".into(),
            options,
            stale_source: false,
        }
    }

    /// The ladder has every level, costs grow with the level, and levels with nothing new are left out.
    #[test]
    fn options_form_a_ladder() {
        let full = record(1, "load", Some("Loads it. Then more."), &["read"]);
        let body = "let value = read(input);\n".repeat(30);
        let options = options_for(&full, Some(&body), 3000, 0);
        let details: Vec<Detail> = options.iter().map(|o| o.detail).collect();
        assert_eq!(details, Detail::ALL);
        assert!(
            options
                .windows(2)
                .all(|pair| pair[0].tokens <= pair[1].tokens)
        );
        assert!(
            options
                .windows(2)
                .all(|pair| pair[0].utility < pair[1].utility)
        );

        let bare = record(2, "load", None, &[]);
        let details: Vec<Detail> = options_for(&bare, None, 3000, 0)
            .iter()
            .map(|o| o.detail)
            .collect();
        assert_eq!(details, [Detail::Name, Detail::Signature]);

        let mut blank = record(3, "load", None, &[]);
        blank.signature.clear();
        assert_eq!(options_for(&blank, None, 3000, 0).len(), 1);
    }

    /// The source is offered only when it fits the limit, and never when it repeats a poorer level.
    #[test]
    fn source_level_is_limited_and_deduplicated() {
        let r = record(1, "load", None, &[]);
        let long = "let value = compute(input);\n".repeat(400);
        assert!(
            options_for(&r, Some(&long), 100, 0)
                .iter()
                .all(|o| o.detail != Detail::Source)
        );
        assert!(
            options_for(&r, Some(&long), 100_000, 0)
                .iter()
                .any(|o| o.detail == Detail::Source)
        );
        assert!(
            options_for(&r, Some(""), 3000, 0)
                .iter()
                .all(|o| o.detail != Detail::Source)
        );
        let same = r.signature.clone();
        let options = options_for(&r, Some(&same), 3000, 0);
        assert!(
            options.iter().all(|o| o.detail != Detail::Source),
            "{options:?}"
        );
    }

    /// Explanations make the printed levels cost more, and the name-only level nothing more.
    #[test]
    fn explanations_add_to_the_cost() {
        let r = record(1, "load", Some("Loads it."), &["read"]);
        let plain = options_for(&r, None, 3000, 0);
        let extra = why_cost("exact name match; text match", true);
        assert!(extra > 4);
        assert_eq!(why_cost("anything", false), 0);
        let explained = options_for(&r, None, 3000, extra);
        assert_eq!(plain[0].tokens, explained[0].tokens);
        for (a, b) in plain.iter().zip(&explained).skip(1) {
            assert_eq!(a.tokens + extra, b.tokens);
        }
    }

    /// Settings for a test capsule.
    fn settings(budget: u32, edges: &[SeenEdge]) -> Settings<'_> {
        Settings {
            query: "how are the functions wired".into(),
            budget,
            explain: false,
            edges,
            notes: Vec::new(),
        }
    }

    /// A deterministic set of candidates of decreasing relevance.
    fn many(count: i64) -> Vec<Prepared> {
        (1..=count)
            .map(|id| {
                prepared(
                    id,
                    1.0 - f64::from(u32::try_from(id).unwrap_or(0)) * 0.012,
                    id % 3 == 0,
                )
            })
            .collect()
    }

    /// Whatever the budget, the measured capsule fits, and `used` is that measure.
    #[test]
    fn the_capsule_always_fits_its_budget() {
        let edges = [SeenEdge {
            src: SymbolId(1),
            dst: SymbolId(2),
            kind: EdgeKind::Calls,
            confidence: Confidence::Resolved,
        }];
        let candidates = many(60);
        for budget in (100..=6000).step_by(37) {
            let settings = settings(budget, &edges);
            let assembled = assemble(&settings, &candidates, Vec::new());
            let measured = measure(&assembled.capsule, &RenderOptions::default());
            assert_eq!(assembled.capsule.used, measured);
            assert!(measured <= budget, "budget {budget}: used {measured}");
            assert_eq!(assembled.shown.len(), assembled.capsule.symbols.len());
        }
    }

    /// A budget too small for any detail still answers with names.
    ///
    /// This is the case a weak model is given, and it is the one that used to fail: the reserve
    /// kept for the frame was subtracted before anything was packed, so a small budget packed
    /// nothing, and nothing then handed the unspent part back. A capsule that names what it found
    /// is worth far more than an empty one, so the floor is that something is always named while
    /// there is room for a name at all.
    #[test]
    fn a_small_budget_still_names_what_it_found() {
        let candidates = many(40);
        for budget in [120, 150, 200, 250, 300] {
            let assembled = assemble(&settings(budget, &[]), &candidates, Vec::new());
            assert!(
                !assembled.capsule.symbols.is_empty(),
                "budget {budget}: nothing was named, {} tokens of {budget} used",
                assembled.capsule.used
            );
            assert!(
                assembled.capsule.used <= budget,
                "budget {budget}: used {}",
                assembled.capsule.used
            );
            assert!(
                f64::from(assembled.capsule.used) >= f64::from(budget) * 0.5,
                "budget {budget}: used only {}",
                assembled.capsule.used
            );
        }
    }

    /// A bigger budget never shows fewer symbols, and it uses most of what it is given.
    #[test]
    fn a_bigger_budget_shows_more_and_fills_it() {
        let candidates = many(60);
        let mut previous = 0;
        for budget in [200, 400, 800, 1600, 3200] {
            let assembled = assemble(&settings(budget, &[]), &candidates, Vec::new());
            let bytes: usize = assembled.capsule.symbols.iter().map(|s| s.text.len()).sum();
            assert!(bytes >= previous, "budget {budget}");
            previous = bytes;
            assert!(
                f64::from(assembled.capsule.used) >= f64::from(budget) * 0.6,
                "budget {budget}: used only {}",
                assembled.capsule.used
            );
        }
    }

    /// Memories that do not fit are dropped, least relevant first, before the budget is broken.
    #[test]
    fn memories_are_dropped_before_the_budget_breaks() {
        let memory = |id: i64, words: usize| CapsuleMemory {
            id: MemoryId(id),
            kind: MemoryKind::Lesson,
            provenance: Provenance::Agent,
            stale: id == 2,
            text: "note ".repeat(words),
        };
        let memories = vec![memory(1, 20), memory(2, 20), memory(3, 200)];
        let candidates = many(5);
        let assembled = assemble(&settings(160, &[]), &candidates, memories);
        assert!(assembled.capsule.used <= 160);
        assert!(
            assembled
                .capsule
                .memories
                .iter()
                .all(|m| m.id != MemoryId(3))
        );
    }

    /// A budget smaller than an empty capsule gives the bare capsule and a note.
    #[test]
    fn an_impossible_budget_gives_a_bare_capsule() {
        let candidates = many(5);
        let assembled = assemble(&settings(3, &[]), &candidates, Vec::new());
        assert!(assembled.capsule.symbols.is_empty());
        assert!(assembled.shown.is_empty());
        assert!(assembled.capsule.notes[0].contains("too small"));
    }

    /// Relations connect selected symbols only, structural edges only, and at most twelve.
    #[test]
    fn relations_are_structural_between_selected_symbols() {
        let candidates = many(30);
        let edge = |src: i64, dst: i64, confidence| SeenEdge {
            src: SymbolId(src),
            dst: SymbolId(dst),
            kind: EdgeKind::Calls,
            confidence,
        };
        let mut edges = vec![
            edge(1, 2, Confidence::Resolved),
            edge(2, 3, Confidence::Heuristic),
            edge(1, 999, Confidence::Exact),
            edge(3, 3, Confidence::Exact),
        ];
        for other in 4..=20 {
            edges.push(edge(1, other, Confidence::Exact));
        }
        let assembled = assemble(&settings(6000, &edges), &candidates, Vec::new());
        let relations = &assembled.capsule.relations;
        assert!(!relations.is_empty());
        assert!(relations.len() <= 12);
        assert!(relations.iter().all(|r| r.confidence.is_structural()));
        assert!(relations.iter().all(|r| r.from != r.to));
        assert!(
            relations
                .iter()
                .any(|r| r.from.ends_with("function_number_1")
                    && r.to.ends_with("function_number_2"))
        );
    }

    /// The same inputs always give the same capsule.
    #[test]
    fn assembly_is_deterministic() {
        let candidates = many(40);
        let a = assemble(&settings(900, &[]), &candidates, Vec::new());
        let b = assemble(&settings(900, &[]), &candidates, Vec::new());
        assert_eq!(a.capsule, b.capsule);
        assert_eq!(a.shown, b.shown);
    }

    /// Stale sources are reported for the symbols shown, a few by name and the rest counted.
    #[test]
    fn stale_sources_are_noted() {
        let mut candidates = many(8);
        for candidate in &mut candidates[..5] {
            candidate.stale_source = true;
        }
        let assembled = assemble(&settings(6000, &[]), &candidates, Vec::new());
        let notes = &assembled.capsule.notes;
        assert_eq!(
            notes
                .iter()
                .filter(|n| n.starts_with("source of function"))
                .count(),
            0
        );
        assert_eq!(
            notes
                .iter()
                .filter(|n| n.contains("changed since indexing; run index"))
                .count(),
            4,
            "{notes:?}"
        );
        assert!(
            notes
                .iter()
                .any(|n| n.starts_with("source of module::function_number_1 changed"))
        );
        assert!(
            notes
                .iter()
                .any(|n| n.starts_with("source of 2 more symbols"))
        );
    }
}
