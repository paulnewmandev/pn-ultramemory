// SPDX-License-Identifier: Apache-2.0
//! A compact map of the repository within a token budget.
//!
//! [`Engine::repo_map`] answers "what is in this repository?" in as many tokens as the caller can
//! spare. It is the first thing an agent asks for in an unfamiliar tree, so it must never blow the
//! context window and must spend what it has on the files that matter.
//!
//! # The algorithm
//! 1. **Score.** A file is worth `1` plus the in-degrees of its symbols, taken from
//!    [`pn_ultramemory_core::Storage::central_symbols`]. A file nothing points at still scores one,
//!    so it can be listed; a hub scores far more, so it earns detail.
//! 2. **Three offers per file.** Its path alone; its path with up to [`MAP_TOP_NAMES`] of its most
//!    referenced symbols; its path with those names and up to [`MAP_TOP_SIGNATURES`] signatures. Each
//!    offer is priced with [`pn_ultramemory_codec::estimate_tokens`] of **exactly the text that
//!    will be printed**, so the packer is never lied to.
//! 3. **Pack.** [`pn_ultramemory_codec::pack`] solves the resulting multiple-choice knapsack: it
//!    buys the cheapest useful detail first, so every file tends to appear before any file gets
//!    signatures.
//! 4. **Guarantee.** The map is then rendered as it will actually be printed and measured. While
//!    the measure exceeds the budget, the offer that gives up the least value per token saved is
//!    lowered, or the file is dropped, and the map is measured again. [`RepoMap::used`] is that
//!    final measure, and it is at most [`RepoMap::budget`].
//! 5. **Refill.** Pricing a row on its own costs slightly more than the same row inside the printed
//!    document, so the first packing leaves tokens unspent. The slack is handed back to the packer a
//!    few times, and a wider selection is kept only while it still measures inside the budget.
//!
//! Files are reported in score order, best first, and `omitted_files` counts every file of the
//! repository (after the path filter) that did not make it in.
//!
//! # Determinism
//! Scores come from ordered storage queries, ties break by path, and the packer is deterministic,
//! so the same index and budget always give the same map.

use std::collections::BTreeMap;

use pn_ultramemory_codec::{Candidate, Format, LevelOption, RenderOptions, estimate_tokens, pack};
use pn_ultramemory_core::{Detail, SymbolId, SymbolKind, SymbolRecord, Visibility};
use pn_ultramemory_toon::Delimiter;
use serde_json::{Map, Value, json};

use crate::engine::Engine;
use crate::error::EngineError;
use crate::render::render_value;

/// The most symbol in-degrees read to score the files.
const CENTRAL_LIMIT: usize = 1000;

/// The most symbol names one file offers beside its path.
pub const MAP_TOP_NAMES: usize = 3;

/// The most signatures one file offers.
pub const MAP_TOP_SIGNATURES: usize = 8;

/// The tokens a row of either table costs beyond its cells: the delimiters and the line break.
const ROW_OVERHEAD_TOKENS: u32 = 4;

/// The tokens the header line of the file table costs.
const FILES_HEADER_TOKENS: u32 = 16;

/// The tokens the header line of the signature table costs.
const SIGNATURES_HEADER_TOKENS: u32 = 12;

/// How much a bare path is worth.
const UTILITY_PATH: f64 = 1.0;

/// How much a path with its top symbol names is worth.
const UTILITY_NAMES: f64 = 2.2;

/// How much a path with names and signatures is worth.
const UTILITY_SIGNATURES: f64 = 3.6;

/// The most files whose symbols are read. Beyond it a file could not fit in any budget this
/// operation accepts, and reading its symbols would only cost time.
const MAX_CONSIDERED: usize = 400;

/// How many budget tokens are assumed to be enough for one more bare path, when deciding how many
/// files to consider. A path costs more than this, so the bound is generous.
const BUDGET_PER_FILE: u32 = 3;

/// How many files are considered on top of what the budget suggests.
const EXTRA_CONSIDERED: usize = 16;

/// How many times the budget left unspent by the first packing is handed back to the packer.
const REFILL_ROUNDS: usize = 4;

/// Slack smaller than this is not worth another packing round.
const REFILL_FLOOR: u32 = 8;

/// What map to build.
///
/// # Examples
/// ```
/// use pn_ultramemory_engine::MapQuery;
///
/// let query = MapQuery { budget: Some(2000), path_prefix: Some("crates/core".into()) };
/// assert_eq!(MapQuery::default().budget, None);
/// assert_eq!(query.budget, Some(2000));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MapQuery {
    /// The token budget, or `None` for [`crate::EngineConfig::default_budget`].
    pub budget: Option<u32>,
    /// Only include files whose path starts with this prefix.
    pub path_prefix: Option<String>,
}

/// One file of a map, at the level of detail the budget allowed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapFile {
    /// Path relative to the repository root.
    pub path: String,
    /// The language it was parsed as.
    pub language: String,
    /// How many lines it has.
    pub lines: u32,
    /// How many symbols were stored for it.
    pub symbols: u32,
    /// Its most referenced symbols, qualified, most referenced first. Empty when only the path fit.
    pub top: Vec<String>,
    /// Signatures of those symbols, as `(qualified name, declaration)`. Empty unless the budget
    /// paid for them.
    pub signatures: Vec<(String, String)>,
}

/// A map of the repository, measured to fit its budget.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RepoMap {
    /// The token budget it was built for.
    pub budget: u32,
    /// The tokens it uses when printed, never more than `budget`.
    pub used: u32,
    /// The files, best first.
    pub files: Vec<MapFile>,
    /// How many files of the repository are not listed.
    pub omitted_files: u32,
}

impl RepoMap {
    /// The map as a structured value, shaped so that TOON prints two uniform tables.
    ///
    /// `files` carries one row per file with `top` flattened into a space-separated string, and
    /// `signatures` carries one row per signature with `f` pointing at its file's row. The second
    /// table is left out when no file carries a signature, and so is the whole file table when the
    /// map is empty.
    #[must_use]
    pub fn to_value(&self) -> Value {
        let mut head = Map::new();
        head.insert("budget".into(), Value::from(self.budget));
        head.insert("used".into(), Value::from(self.used));
        if self.omitted_files > 0 {
            head.insert("omitted".into(), Value::from(self.omitted_files));
        }
        let mut root = Map::new();
        root.insert("map".into(), Value::Object(head));
        if !self.files.is_empty() {
            let rows: Vec<Value> = self
                .files
                .iter()
                .map(|file| {
                    json!({
                        "path": file.path,
                        "lang": file.language,
                        "lines": file.lines,
                        "symbols": file.symbols,
                        "top": file.top.join(" "),
                    })
                })
                .collect();
            root.insert("files".into(), Value::Array(rows));
        }
        let signatures: Vec<Value> = self
            .files
            .iter()
            .enumerate()
            .flat_map(|(index, file)| {
                file.signatures
                    .iter()
                    .map(move |(name, text)| json!({ "f": index, "name": name, "text": text }))
            })
            .collect();
        if !signatures.is_empty() {
            root.insert("signatures".into(), Value::Array(signatures));
        }
        Value::Object(root)
    }

    /// The map rendered in one of the three output formats. `delimiter` is used by TOON only, and
    /// the output never ends with a newline.
    #[must_use]
    pub fn render(&self, format: Format, delimiter: Delimiter) -> String {
        render_value(&self.to_value(), format, delimiter)
    }
}

/// One way of showing a file, priced as it will be printed.
#[derive(Debug, Clone, Copy)]
struct Offer {
    /// The level of detail, used only to key the packer's answer.
    detail: Detail,
    /// What the offer costs in the printed map.
    tokens: u32,
    /// How much it is worth against the other offers of the same file.
    utility: f64,
    /// How many of the file's top names it prints.
    names: usize,
    /// How many of the file's signatures it prints.
    signatures: usize,
}

/// One candidate file with everything needed to price, pack and print it.
#[derive(Debug, Clone)]
struct Entry {
    /// The file as it will be printed, with every name and signature it could offer.
    file: MapFile,
    /// How much the file matters: one plus the in-degrees of its symbols.
    relevance: f64,
    /// The ways of showing it, cheapest first.
    offers: Vec<Offer>,
}

/// The chosen offer of each entry, as an index into its offers, or `None` for a file left out.
type Chosen = Vec<Option<usize>>;

/// The tokens a row of the file table costs with `names` of the file's top names printed.
fn row_tokens(file: &MapFile, names: usize) -> u32 {
    let top = file
        .top
        .iter()
        .take(names)
        .map(String::as_str)
        .collect::<Vec<&str>>()
        .join(" ");
    let row = format!(
        "{},{},{},{},{}",
        file.path, file.language, file.lines, file.symbols, top
    );
    estimate_tokens(&row) + ROW_OVERHEAD_TOKENS
}

/// The tokens the signature rows of one file cost, priced at its position in the table.
fn signature_tokens(file: &MapFile, position: usize, count: usize) -> u32 {
    file.signatures
        .iter()
        .take(count)
        .map(|(name, text)| {
            estimate_tokens(&format!("{position},{name},{text}")) + ROW_OVERHEAD_TOKENS
        })
        .fold(0_u32, u32::saturating_add)
}

/// The offers of one file, cheapest first, leaving out an offer that would print nothing new.
fn offers_for(file: &MapFile, position: usize) -> Vec<Offer> {
    let mut offers = vec![Offer {
        detail: Detail::Name,
        tokens: row_tokens(file, 0),
        utility: UTILITY_PATH,
        names: 0,
        signatures: 0,
    }];
    let names = file.top.len().min(MAP_TOP_NAMES);
    if names > 0 {
        offers.push(Offer {
            detail: Detail::Signature,
            tokens: row_tokens(file, names),
            utility: UTILITY_NAMES,
            names,
            signatures: 0,
        });
    }
    let signatures = file.signatures.len().min(MAP_TOP_SIGNATURES);
    if signatures > 0 {
        let rows = signature_tokens(file, position, signatures);
        offers.push(Offer {
            detail: Detail::Outline,
            tokens: row_tokens(file, names).saturating_add(rows),
            utility: UTILITY_SIGNATURES,
            names,
            signatures,
        });
    }
    offers
}

/// The order the symbols of a file are offered in: most referenced first, then public ones, then
/// by position in the file and by name, so the choice never depends on storage order.
fn symbol_key(record: &SymbolRecord, degree: u32) -> (u32, u8, u32, &str) {
    let public = u8::from(record.visibility != Visibility::Public);
    (
        u32::MAX - degree,
        public,
        record.span.start_line,
        &record.qualified_name,
    )
}

/// How many files are worth reading the symbols of for this budget.
fn considered_files(budget: u32) -> usize {
    usize::try_from(budget / BUDGET_PER_FILE)
        .unwrap_or(MAX_CONSIDERED)
        .saturating_add(EXTRA_CONSIDERED)
        .min(MAX_CONSIDERED)
}

/// The map composed from a selection, with `used` left at the budget so that measuring it can
/// never under-count its own field.
fn compose(budget: u32, entries: &[Entry], chosen: &Chosen, total_files: usize) -> RepoMap {
    let mut files = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let Some(offer) = chosen
            .get(index)
            .copied()
            .flatten()
            .and_then(|level| entry.offers.get(level))
        else {
            continue;
        };
        files.push(MapFile {
            path: entry.file.path.clone(),
            language: entry.file.language.clone(),
            lines: entry.file.lines,
            symbols: entry.file.symbols,
            top: entry.file.top.iter().take(offer.names).cloned().collect(),
            signatures: entry
                .file
                .signatures
                .iter()
                .take(offer.signatures)
                .cloned()
                .collect(),
        });
    }
    let omitted = total_files.saturating_sub(files.len());
    RepoMap {
        budget,
        used: budget,
        files,
        omitted_files: u32::try_from(omitted).unwrap_or(u32::MAX),
    }
}

/// The tokens the map costs as the default format prints it.
fn measure_map(map: &RepoMap) -> u32 {
    estimate_tokens(&map.render(
        RenderOptions::default().format,
        RenderOptions::default().delimiter,
    ))
}

/// The step that gives up the least value per token saved: `(entry, tokens saved)`.
///
/// A step lowers one file by one offer, or drops it when it is already at its cheapest. Ties go to
/// the less relevant file, then to the one further down the map.
fn cheapest_step_down(entries: &[Entry], chosen: &Chosen) -> Option<(usize, u32)> {
    let mut best: Option<(f64, f64, usize, u32)> = None;
    for (index, entry) in entries.iter().enumerate() {
        let Some(level) = chosen.get(index).copied().flatten() else {
            continue;
        };
        let Some(here) = entry.offers.get(level) else {
            continue;
        };
        let below = level
            .checked_sub(1)
            .and_then(|lower| entry.offers.get(lower));
        let (lost, saved) = match below {
            Some(lower) => (
                here.utility - lower.utility,
                here.tokens.saturating_sub(lower.tokens),
            ),
            None => (here.utility, here.tokens),
        };
        let ratio = entry.relevance * lost / f64::from(saved.max(1));
        let better = best.is_none_or(|(top_ratio, top_relevance, top_index, _)| {
            ratio
                .total_cmp(&top_ratio)
                .then(entry.relevance.total_cmp(&top_relevance))
                .then(top_index.cmp(&index))
                .is_lt()
        });
        if better {
            best = Some((ratio, entry.relevance, index, saved));
        }
    }
    best.map(|(_, _, index, saved)| (index, saved))
}

/// Lowers or drops files, cheapest loss first, until about `need` tokens are saved. Returns whether
/// anything changed.
fn shrink(entries: &[Entry], chosen: &mut Chosen, mut need: u32) -> bool {
    let mut changed = false;
    while need > 0 {
        let Some((index, saved)) = cheapest_step_down(entries, chosen) else {
            break;
        };
        if let Some(slot) = chosen.get_mut(index) {
            *slot = slot.and_then(|level| level.checked_sub(1));
        }
        changed = true;
        need = need.saturating_sub(saved.max(1));
    }
    changed
}

/// Measures a selection as it will be printed and lowers it until it fits.
///
/// Returns the map and its measure. The measure is at most `budget` unless even an empty map costs
/// more than that, in which case nothing can be given up and the empty map is returned as it is.
fn fit(budget: u32, entries: &[Entry], chosen: &mut Chosen, total_files: usize) -> (RepoMap, u32) {
    let mut map = compose(budget, entries, chosen, total_files);
    let mut used = measure_map(&map);
    while used > budget {
        let need = used - budget;
        if !shrink(entries, chosen, need) {
            break;
        }
        map = compose(budget, entries, chosen, total_files);
        used = measure_map(&map);
    }
    (map, used)
}

/// Asks the packer for an offer per file within `available` tokens.
fn run_pack(entries: &[Entry], available: u32) -> Chosen {
    let candidates: Vec<Candidate<usize>> = entries
        .iter()
        .enumerate()
        .map(|(key, entry)| Candidate {
            key,
            relevance: entry.relevance,
            options: entry
                .offers
                .iter()
                .map(|offer| LevelOption {
                    detail: offer.detail,
                    tokens: offer.tokens,
                    utility: offer.utility,
                })
                .collect(),
        })
        .collect();
    let mut chosen: Chosen = vec![None; entries.len()];
    for selection in pack(candidates, available).selections {
        let level = entries.get(selection.key).and_then(|entry| {
            entry
                .offers
                .iter()
                .position(|offer| offer.detail == selection.detail)
        });
        if let Some(slot) = chosen.get_mut(selection.key) {
            *slot = level;
        }
    }
    chosen
}

/// One indexed file with its score, before its symbols are read.
#[derive(Debug, Clone)]
struct Scored {
    /// One plus the in-degrees of its symbols.
    score: u32,
    /// Path relative to the repository root.
    path: String,
    /// The language it was parsed as.
    language: String,
    /// How many lines it has.
    lines: u32,
    /// How many symbols were stored for it.
    symbols: u32,
}

/// How often each symbol is referenced, and how often each file's symbols are, together.
#[derive(Debug, Default)]
struct Degrees {
    /// In-degree per symbol, for the symbols anything points at.
    per_symbol: BTreeMap<SymbolId, u32>,
    /// The sum of those in-degrees per file path.
    per_file: BTreeMap<String, u32>,
}

impl Engine {
    /// The in-degree of every symbol the index points at, and the total per file.
    fn degrees(&self, prefix: Option<&str>) -> Result<Degrees, EngineError> {
        let mut degrees = Degrees::default();
        for (record, degree) in self.storage().central_symbols(CENTRAL_LIMIT, prefix)? {
            degrees.per_symbol.insert(record.id, degree);
            let total = degrees.per_file.entry(record.path).or_insert(0);
            *total = total.saturating_add(degree);
        }
        Ok(degrees)
    }

    /// The candidate files of a map, in score order, each priced at all three offers.
    ///
    /// Returns the entries and how many files the repository has after the path filter, which is
    /// what `omitted_files` is measured against.
    fn map_entries(
        &self,
        budget: u32,
        prefix: Option<&str>,
    ) -> Result<(Vec<Entry>, usize), EngineError> {
        let degrees = self.degrees(prefix)?;
        let mut scored: Vec<Scored> = Vec::new();
        for record in self.storage().list_files()? {
            if prefix.is_some_and(|prefix| !record.path.starts_with(prefix)) {
                continue;
            }
            scored.push(Scored {
                score: degrees
                    .per_file
                    .get(&record.path)
                    .copied()
                    .unwrap_or(0)
                    .saturating_add(1),
                path: record.path,
                language: record.language.name().to_owned(),
                lines: record.lines,
                symbols: record.symbol_count,
            });
        }
        let total_files = scored.len();
        scored.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.path.cmp(&b.path)));
        scored.truncate(considered_files(budget));

        let mut entries = Vec::with_capacity(scored.len());
        for (position, candidate) in scored.into_iter().enumerate() {
            let mut ranked: Vec<(u32, SymbolRecord)> = self
                .storage()
                .symbols_in_file(&candidate.path)?
                .into_iter()
                .filter(|record| record.kind != SymbolKind::Module)
                .map(|record| {
                    (
                        degrees.per_symbol.get(&record.id).copied().unwrap_or(0),
                        record,
                    )
                })
                .collect();
            ranked.sort_by(|a, b| symbol_key(&a.1, a.0).cmp(&symbol_key(&b.1, b.0)));
            let file = MapFile {
                path: candidate.path,
                language: candidate.language,
                lines: candidate.lines,
                symbols: candidate.symbols,
                top: ranked
                    .iter()
                    .take(MAP_TOP_NAMES)
                    .map(|(_, record)| record.qualified_name.clone())
                    .collect(),
                signatures: ranked
                    .iter()
                    .filter(|(_, record)| !record.signature.is_empty())
                    .take(MAP_TOP_SIGNATURES)
                    .map(|(_, record)| (record.qualified_name.clone(), record.signature.clone()))
                    .collect(),
            };
            let offers = offers_for(&file, position);
            entries.push(Entry {
                file,
                relevance: 1.0 + f64::from(candidate.score).ln(),
                offers,
            });
        }
        Ok((entries, total_files))
    }

    /// Builds a compact map of the repository within a token budget.
    ///
    /// The scoring, the three offers per file and the budget guarantee are set out in the
    /// documentation of this module. [`RepoMap::used`] is measured on the map as the default TOON format prints
    /// it and is at most the effective budget, unless the budget is smaller than the frame of an
    /// empty map (the configured minimum budget is far above that).
    ///
    /// # Errors
    /// Returns a storage error when the index cannot be read.
    ///
    /// # Examples
    /// ```no_run
    /// # fn demo(engine: &pn_ultramemory_engine::Engine) -> Result<(), pn_ultramemory_engine::EngineError> {
    /// use pn_ultramemory_engine::MapQuery;
    ///
    /// let map = engine.repo_map(&MapQuery { budget: Some(800), path_prefix: None })?;
    /// assert!(map.used <= map.budget);
    /// # Ok(())
    /// # }
    /// ```
    pub fn repo_map(&self, q: &MapQuery) -> Result<RepoMap, EngineError> {
        let budget = self.config.effective_budget(q.budget);
        let prefix = q
            .path_prefix
            .as_deref()
            .map(|prefix| prefix.trim().trim_start_matches("./"))
            .filter(|prefix| !prefix.is_empty());
        let (entries, total_files) = self.map_entries(budget, prefix)?;

        let empty: Chosen = vec![None; entries.len()];
        let frame = measure_map(&compose(budget, &entries, &empty, total_files))
            .saturating_add(FILES_HEADER_TOKENS)
            .saturating_add(SIGNATURES_HEADER_TOKENS);

        let mut available = budget.saturating_sub(frame);
        let mut packed = run_pack(&entries, available);
        let (mut map, mut used) = fit(budget, &entries, &mut packed, total_files);
        // Pricing a row on its own costs a little more than the same row inside the printed
        // document, and the frame is a reserve rather than a measurement, so the first packing
        // usually leaves tokens unspent. Hand the slack back and keep the result only while it
        // still measures inside the budget.
        for _ in 0..REFILL_ROUNDS {
            if used.saturating_add(REFILL_FLOOR) >= budget {
                break;
            }
            let stretched = available.saturating_add(budget - used);
            let mut trial = run_pack(&entries, stretched);
            let (trial_map, trial_used) = fit(budget, &entries, &mut trial, total_files);
            if trial_used <= used {
                break;
            }
            available = stretched;
            map = trial_map;
            used = trial_used;
        }
        map.used = used;
        Ok(map)
    }
}

#[cfg(test)]
mod tests;
