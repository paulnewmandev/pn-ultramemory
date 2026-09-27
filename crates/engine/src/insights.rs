// SPDX-License-Identifier: Apache-2.0
//! The figures a report is built from: one pass over the index that answers "what is in this
//! repository, and what should I look at first?".
//!
//! # Role in the architecture
//! Application layer. [`Engine::insights`] gathers the aggregates that
//! [`pn_ultramemory_core::Storage`] already computes (module sizes and coupling, documentation
//! coverage, the most referenced symbols, the files per language, the memories) and returns them as
//! one plain value. The reporting crate renders it; nothing here formats anything but numbers, and
//! nothing is written.
//!
//! # Invariants
//! * **Empty data is not an error.** A repository that has never been indexed produces an
//!   [`Insights`] whose every list is empty, so a report can say "nothing indexed yet" instead of
//!   failing.
//! * **Deterministic ordering everywhere.** Every list is sorted with an explicit tie-break, so the
//!   same index always yields the same document: languages by files then symbols then name, hotspots
//!   by in-degree then path, line and name, memories stale-first then newest then by identity, and
//!   the rest in the order storage documents.
//! * **Bounded.** Every list has a limit, and the outgoing edges of a hotspot are counted up to
//!   [`EDGE_SCAN_LIMIT`], so one call costs the same on a large repository as on a small one.

use std::collections::BTreeMap;

use pn_ultramemory_core::{
    Confidence, Direction, DocCoverageRow, MemoryFilter, MemoryRecord, ModuleEdge, ModuleStats,
    SymbolKind,
};
use serde_json::{Value, json};

use crate::docs::{DocGap, DocGapQuery, gap_rows};
use crate::engine::Engine;
use crate::error::EngineError;
use crate::stats::StatsReport;

/// How many leading directories of a path name a module by default.
const DEFAULT_MODULE_DEPTH: usize = 2;

/// How many of the most referenced symbols are reported by default.
const DEFAULT_HOTSPOTS: usize = 25;

/// How many memories are reported by default.
const DEFAULT_MEMORIES: usize = 25;

/// How many undocumented symbols are shown as examples by default.
const DEFAULT_UNDOCUMENTED: usize = 10;

/// How many pairs of modules the coupling table holds.
const MAX_MODULE_EDGES: usize = 50;

/// How far the outgoing edges of one hotspot are counted.
const EDGE_SCAN_LIMIT: usize = 512;

/// How much of an [`Insights`] to gather.
///
/// # Examples
/// ```
/// use pn_ultramemory_engine::InsightOptions;
///
/// let options = InsightOptions::default();
/// assert_eq!(options.module_depth, 2);
/// assert_eq!(options.max_hotspots, 25);
/// assert_eq!(options.max_undocumented, 10);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InsightOptions {
    /// How many leading directories of a path name its module. Zero is read as one.
    pub module_depth: usize,
    /// How many of the most referenced symbols to report.
    pub max_hotspots: usize,
    /// How many memories to report.
    pub max_memories: usize,
    /// How many undocumented symbols to show as examples.
    pub max_undocumented: usize,
}

impl Default for InsightOptions {
    fn default() -> Self {
        Self {
            module_depth: DEFAULT_MODULE_DEPTH,
            max_hotspots: DEFAULT_HOTSPOTS,
            max_memories: DEFAULT_MEMORIES,
            max_undocumented: DEFAULT_UNDOCUMENTED,
        }
    }
}

/// A symbol many others depend on: the place a change is most likely to be felt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hotspot {
    /// The name qualified by its enclosing symbols.
    pub name: String,
    /// What kind of element it is.
    pub kind: SymbolKind,
    /// Path of the file that declares it.
    pub path: String,
    /// The 1-based line the declaration starts on.
    pub line: u32,
    /// How many symbols reach it, counting edges at least as sure as
    /// [`Confidence::Heuristic`].
    pub callers: u32,
    /// How many symbols it reaches, counted the same way and up to a fixed scan limit of 512.
    pub callees: u32,
}

/// How much of the repository one language accounts for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageRow {
    /// The language name, as storage records it.
    pub name: String,
    /// Indexed files written in it.
    pub files: u64,
    /// Symbols declared in those files.
    pub symbols: u64,
}

/// Everything a report about one repository is built from.
#[derive(Debug, Clone, PartialEq)]
pub struct Insights {
    /// The counts of the index, the files, what was learned and how the tool was used.
    pub stats: StatsReport,
    /// Files and symbols per language, largest first.
    pub languages: Vec<LanguageRow>,
    /// The groups of files that share a directory prefix, with how coupled each one is.
    pub modules: Vec<ModuleStats>,
    /// The heaviest edges between pairs of modules.
    pub module_edges: Vec<ModuleEdge>,
    /// The most referenced symbols.
    pub hotspots: Vec<Hotspot>,
    /// Documentation coverage per language.
    pub documentation: Vec<DocCoverageRow>,
    /// A few public symbols that have no documentation, as examples.
    pub undocumented: Vec<DocGap>,
    /// Stored memories, the ones whose code changed first.
    pub memories: Vec<MemoryRecord>,
}

/// A percentage of a total, or zero when the total is zero.
fn percent_of(part: u64, total: u64) -> u64 {
    part.saturating_mul(100).checked_div(total).unwrap_or(0)
}

/// The anchors of a memory as one comma-separated list of qualified names.
fn about_of(memory: &MemoryRecord) -> String {
    let mut out = String::new();
    for anchor in &memory.anchors {
        if !out.is_empty() {
            out.push_str(", ");
        }
        out.push_str(&anchor.qualified_name);
    }
    out
}

impl Insights {
    /// The insights as a structured value: eight uniform tables under the counts, so that TOON
    /// declares each shape once.
    #[must_use]
    pub fn to_value(&self) -> Value {
        let languages: Vec<Value> = self
            .languages
            .iter()
            .map(|row| json!({ "name": row.name, "files": row.files, "symbols": row.symbols }))
            .collect();
        let modules: Vec<Value> = self
            .modules
            .iter()
            .map(|module| {
                json!({
                    "name": module.name,
                    "files": module.files,
                    "symbols": module.symbols,
                    "incoming": module.incoming,
                    "outgoing": module.outgoing,
                })
            })
            .collect();
        let module_edges: Vec<Value> = self
            .module_edges
            .iter()
            .map(|edge| json!({ "from": edge.from, "to": edge.to, "weight": edge.weight }))
            .collect();
        let hotspots: Vec<Value> = self
            .hotspots
            .iter()
            .map(|hotspot| {
                json!({
                    "name": hotspot.name,
                    "kind": hotspot.kind.as_str(),
                    "path": hotspot.path,
                    "line": hotspot.line,
                    "callers": hotspot.callers,
                    "callees": hotspot.callees,
                })
            })
            .collect();
        let documentation: Vec<Value> = self
            .documentation
            .iter()
            .map(|row| {
                json!({
                    "language": row.language,
                    "public_symbols": row.public_symbols,
                    "documented": row.documented,
                    "percent": percent_of(row.documented, row.public_symbols),
                })
            })
            .collect();
        let memories: Vec<Value> = self
            .memories
            .iter()
            .map(|memory| {
                json!({
                    "id": memory.id.0,
                    "kind": memory.kind.as_str(),
                    "provenance": memory.provenance.as_str(),
                    "stale": memory.stale_since.is_some(),
                    "created_at": memory.created_at,
                    "about": about_of(memory),
                    "text": memory.text,
                })
            })
            .collect();
        json!({
            "stats": self.stats.to_value(),
            "languages": languages,
            "modules": modules,
            "module_edges": module_edges,
            "hotspots": hotspots,
            "documentation": documentation,
            "undocumented": gap_rows(&self.undocumented),
            "memories": memories,
        })
    }
}

impl Engine {
    /// Files and symbols per language, the language with the most files first.
    ///
    /// # Errors
    /// Returns a storage error when the files cannot be listed.
    fn language_rows(&self) -> Result<Vec<LanguageRow>, EngineError> {
        let mut per_language: BTreeMap<&'static str, (u64, u64)> = BTreeMap::new();
        for file in self.storage().list_files()? {
            let entry = per_language.entry(file.language.name()).or_insert((0, 0));
            entry.0 += 1;
            entry.1 += u64::from(file.symbol_count);
        }
        let mut rows: Vec<LanguageRow> = per_language
            .into_iter()
            .map(|(name, (files, symbols))| LanguageRow {
                name: name.to_owned(),
                files,
                symbols,
            })
            .collect();
        rows.sort_by(|left, right| {
            right
                .files
                .cmp(&left.files)
                .then(right.symbols.cmp(&left.symbols))
                .then(left.name.cmp(&right.name))
        });
        Ok(rows)
    }

    /// The most referenced symbols, with how many symbols reach them and how many they reach.
    ///
    /// # Errors
    /// Returns a storage error when the index cannot be read.
    fn hotspot_rows(&self, limit: usize) -> Result<Vec<Hotspot>, EngineError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let central = self.storage().central_symbols(limit, None)?;
        let mut hotspots = Vec::with_capacity(central.len());
        for (record, callers) in central {
            let outgoing = self.storage().neighbors(
                record.id,
                Direction::Out,
                Confidence::Heuristic,
                EDGE_SCAN_LIMIT,
            )?;
            hotspots.push(Hotspot {
                name: record.qualified_name,
                kind: record.kind,
                path: record.path,
                line: record.span.start_line,
                callers,
                callees: u32::try_from(outgoing.len()).unwrap_or(u32::MAX),
            });
        }
        hotspots.sort_by(|left, right| {
            right
                .callers
                .cmp(&left.callers)
                .then(left.path.cmp(&right.path))
                .then(left.line.cmp(&right.line))
                .then(left.name.cmp(&right.name))
        });
        Ok(hotspots)
    }

    /// The stored memories worth showing, the ones whose code changed first.
    ///
    /// # Errors
    /// Returns a storage error when the memories cannot be listed.
    fn memory_rows(&self, limit: usize) -> Result<Vec<MemoryRecord>, EngineError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let filter = MemoryFilter {
            limit,
            ..MemoryFilter::default()
        };
        let mut memories = self.storage().list_memories(&filter)?;
        memories.sort_by(|left, right| {
            left.stale_since
                .is_none()
                .cmp(&right.stale_since.is_none())
                .then(right.created_at.cmp(&left.created_at))
                .then(right.id.cmp(&left.id))
        });
        Ok(memories)
    }

    /// Every figure a report about this repository is built from.
    ///
    /// A repository with no index is not a failure: every list comes back empty and the counts are
    /// zero, so a caller can report "nothing indexed yet" without special-casing anything.
    ///
    /// # Errors
    /// Returns a storage error when any aggregate cannot be read. A missing metrics file is not an
    /// error: [`StatsReport::usage`] is then `None`.
    pub fn insights(&self, options: &InsightOptions) -> Result<Insights, EngineError> {
        let depth = options.module_depth.max(1);
        Ok(Insights {
            stats: self.stats()?,
            languages: self.language_rows()?,
            modules: self.storage().module_stats(depth)?,
            module_edges: self.storage().module_edges(
                depth,
                Confidence::Heuristic,
                MAX_MODULE_EDGES,
            )?,
            hotspots: self.hotspot_rows(options.max_hotspots)?,
            documentation: self.storage().doc_coverage()?,
            undocumented: if options.max_undocumented == 0 {
                Vec::new()
            } else {
                self.doc_gaps(&DocGapQuery {
                    path_prefix: None,
                    limit: options.max_undocumented,
                    with_context: false,
                })?
            },
            memories: self.memory_rows(options.max_memories)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{Hotspot, InsightOptions, Insights, LanguageRow, about_of, percent_of};
    use pn_ultramemory_core::{
        Anchor, FileTotals, IndexStats, LearningStatus, MemoryId, MemoryKind, MemoryRecord,
        Provenance, SymbolKind,
    };

    use crate::stats::StatsReport;

    /// The defaults match the ones the documentation promises.
    #[test]
    fn default_options() {
        let options = InsightOptions::default();
        assert_eq!(options.module_depth, 2);
        assert_eq!(options.max_hotspots, 25);
        assert_eq!(options.max_memories, 25);
        assert_eq!(options.max_undocumented, 10);
    }

    /// A percentage of nothing is zero, not a division by zero.
    #[test]
    fn percentages_tolerate_an_empty_total() {
        assert_eq!(percent_of(3, 4), 75);
        assert_eq!(percent_of(0, 0), 0);
        assert_eq!(percent_of(5, 5), 100);
    }

    /// The anchors of a memory are listed by qualified name, and none gives an empty string.
    #[test]
    fn anchors_are_listed() {
        let mut memory = MemoryRecord {
            id: MemoryId(1),
            kind: MemoryKind::Decision,
            text: "t".to_owned(),
            provenance: Provenance::Tool,
            created_at: 10,
            stale_since: None,
            anchors: Vec::new(),
        };
        assert_eq!(about_of(&memory), "");
        for name in ["a::b", "c"] {
            memory.anchors.push(Anchor {
                symbol: None,
                qualified_name: name.to_owned(),
                path: "x.rs".to_owned(),
                sig_hash: 0,
                body_hash: 0,
            });
        }
        assert_eq!(about_of(&memory), "a::b, c");
    }

    /// Empty insights serialize to eight empty tables and zero counts, not to `null`.
    #[test]
    fn empty_insights_serialize() {
        let insights = Insights {
            stats: StatsReport {
                index: IndexStats::default(),
                totals: FileTotals::default(),
                learning: LearningStatus::default(),
                usage: None,
            },
            languages: Vec::new(),
            modules: Vec::new(),
            module_edges: Vec::new(),
            hotspots: Vec::new(),
            documentation: Vec::new(),
            undocumented: Vec::new(),
            memories: Vec::new(),
        };
        let value = insights.to_value();
        assert_eq!(value["stats"]["index"]["files"], 0);
        for key in [
            "languages",
            "modules",
            "module_edges",
            "hotspots",
            "documentation",
            "undocumented",
            "memories",
        ] {
            assert_eq!(value[key].as_array().map(Vec::len), Some(0), "{key}");
        }
    }

    /// The rows of the two tables this module owns carry every column.
    #[test]
    fn row_shapes() {
        let insights = Insights {
            stats: StatsReport {
                index: IndexStats::default(),
                totals: FileTotals::default(),
                learning: LearningStatus::default(),
                usage: None,
            },
            languages: vec![LanguageRow {
                name: "rust".to_owned(),
                files: 2,
                symbols: 9,
            }],
            modules: Vec::new(),
            module_edges: Vec::new(),
            hotspots: vec![Hotspot {
                name: "f".to_owned(),
                kind: SymbolKind::Function,
                path: "a.rs".to_owned(),
                line: 3,
                callers: 4,
                callees: 1,
            }],
            documentation: Vec::new(),
            undocumented: Vec::new(),
            memories: Vec::new(),
        };
        let value = insights.to_value();
        assert_eq!(value["languages"][0]["symbols"], 9);
        assert_eq!(value["hotspots"][0]["kind"], "function");
        assert_eq!(value["hotspots"][0]["callers"], 4);
    }
}
