// SPDX-License-Identifier: Apache-2.0
//! The whole repository as one navigable picture: every symbol, every relationship and every
//! memory, shaped for the interactive brain view.
//!
//! # Why a separate shape
//! [`Engine::graph`] answers *draw the neighbourhood of this* and stops at sixty nodes, because a
//! diagram on a page cannot hold more. The brain view is not a page: it is a scene a person flies
//! through, and its value is that nothing is missing from it. So this module returns the symbols of
//! the repository up to a cap of thousands, the edges among them, and the memories anchored to
//! them, in columns rather than objects so that a large repository still embeds in a few megabytes.
//!
//! # Invariants
//! * **Endpoints are indices.** Every edge and every memory anchor points into the node list. An
//!   endpoint outside the selection is dropped, never clamped.
//! * **The most connected survive the cap.** When a repository has more symbols than the cap, the
//!   symbols other code depends on are kept first, then the public ones, then the rest in path
//!   order, so the structure that matters is what is drawn.
//! * **Identities are strings.** A symbol identity is a 64-bit hash, larger than a JavaScript number
//!   can hold exactly, so it is written as text.

use std::collections::{BTreeMap, BTreeSet};

use pn_ultramemory_core::{
    Confidence, Direction, EdgeKind, MemoryFilter, SymbolId, SymbolKind, SymbolRecord, Visibility,
    first_sentence,
};
use serde_json::{Value, json};

use crate::engine::Engine;
use crate::error::EngineError;

/// How many symbols the brain holds when the caller names no cap.
pub const DEFAULT_BRAIN_NODES: usize = 6000;

/// The most neighbours read from one symbol.
const MAX_FANOUT: usize = 200;

/// How many leading directories name the region a symbol belongs to.
const REGION_DEPTH: usize = 2;

/// The longest signature carried, in characters.
const MAX_SIGNATURE_CHARS: usize = 220;

/// The longest documentation sentence carried, in characters.
const MAX_DOC_CHARS: usize = 280;

/// The symbol kinds, in the order their index is written.
const KINDS: [SymbolKind; 12] = [
    SymbolKind::Module,
    SymbolKind::Class,
    SymbolKind::Struct,
    SymbolKind::Interface,
    SymbolKind::Enum,
    SymbolKind::Function,
    SymbolKind::Method,
    SymbolKind::Constant,
    SymbolKind::Variable,
    SymbolKind::Type,
    SymbolKind::Macro,
    SymbolKind::Other,
];

/// How many numbers describe one edge: source, target, kind and confidence.
const EDGE_STRIDE: usize = 4;

/// The edge kinds, in the order their index is written.
const EDGE_KINDS: [EdgeKind; 3] = [EdgeKind::Calls, EdgeKind::Inherits, EdgeKind::Uses];

/// What brain to build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BrainQuery {
    /// The most symbols to include.
    pub max_nodes: usize,
    /// The weakest edge to draw.
    pub min_confidence: Confidence,
}

impl Default for BrainQuery {
    fn default() -> Self {
        Self {
            max_nodes: DEFAULT_BRAIN_NODES,
            min_confidence: Confidence::Heuristic,
        }
    }
}

/// The repository as the brain view draws it.
#[derive(Debug, Clone, PartialEq)]
pub struct Brain {
    /// How many symbols are drawn.
    pub nodes: usize,
    /// How many edges are drawn.
    pub edges: usize,
    /// How many memories are drawn.
    pub memories: usize,
    /// How many symbols the index holds, drawn or not.
    pub total_symbols: u64,
    /// The columns, as they are embedded in the page.
    value: Value,
}

impl Brain {
    /// The brain as a structured value: tables of strings that the rows point into, then nodes,
    /// edges and memories as arrays, which is several times smaller than one object per row.
    ///
    /// A node is `[id, name, qualified name or "", kind, region, path, line, signature, doc,
    /// public]`, an edge is four numbers in a flat list (source, target, kind, confidence), and a
    /// memory is `[id, kind, text, stale, by, anchor node indices, anchor names, created]`.
    #[must_use]
    pub fn to_value(&self) -> Value {
        self.value.clone()
    }
}

/// The region a path belongs to: its first [`REGION_DEPTH`] directories, or `.` at the root.
fn region_of(path: &str) -> String {
    let mut parts: Vec<&str> = path.split('/').collect();
    parts.pop();
    if parts.is_empty() {
        return ".".to_owned();
    }
    parts.truncate(REGION_DEPTH);
    parts.join("/")
}

/// Text cut to at most `max` characters, on a character boundary, with an ellipsis when cut.
fn clipped(text: &str, max: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        return flat;
    }
    let mut out: String = flat.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// The position of an item in a table of distinct strings, adding it when it is new.
fn intern(table: &mut Vec<String>, positions: &mut BTreeMap<String, usize>, item: &str) -> usize {
    if let Some(&position) = positions.get(item) {
        return position;
    }
    let position = table.len();
    table.push(item.to_owned());
    positions.insert(item.to_owned(), position);
    position
}

/// The index of a kind in [`KINDS`].
fn kind_index(kind: SymbolKind) -> usize {
    KINDS
        .iter()
        .position(|k| *k == kind)
        .unwrap_or(KINDS.len() - 1)
}

/// The index of a confidence in [`Confidence::ALL`], weakest first.
fn confidence_index(confidence: Confidence) -> usize {
    Confidence::ALL
        .iter()
        .position(|c| *c == confidence)
        .unwrap_or(0)
}

/// The index of an edge kind in [`EDGE_KINDS`].
fn edge_kind_index(kind: EdgeKind) -> usize {
    EDGE_KINDS.iter().position(|k| *k == kind).unwrap_or(0)
}

/// One row per drawn symbol, with the table of regions and the table of paths the rows point
/// into, so that a path shared by forty symbols is written once.
fn node_rows(symbols: &[SymbolRecord]) -> (Vec<Value>, Vec<String>, Vec<String>) {
    let mut regions: Vec<String> = Vec::new();
    let mut region_positions = BTreeMap::new();
    let mut paths: Vec<String> = Vec::new();
    let mut path_positions = BTreeMap::new();
    let mut nodes: Vec<Value> = Vec::with_capacity(symbols.len());
    for symbol in symbols {
        let region = intern(
            &mut regions,
            &mut region_positions,
            &region_of(&symbol.path),
        );
        let path = intern(&mut paths, &mut path_positions, &symbol.path);
        let doc = symbol
            .doc
            .as_deref()
            .map(|doc| clipped(&first_sentence(doc), MAX_DOC_CHARS))
            .unwrap_or_default();
        let qualified = if symbol.qualified_name == symbol.name {
            String::new()
        } else {
            symbol.qualified_name.clone()
        };
        nodes.push(json!([
            symbol.id.0.to_string(),
            symbol.name,
            qualified,
            kind_index(symbol.kind),
            region,
            path,
            symbol.span.start_line,
            clipped(&symbol.signature, MAX_SIGNATURE_CHARS),
            doc,
            u8::from(symbol.visibility == Visibility::Public),
        ]));
    }
    (nodes, regions, paths)
}

impl Engine {
    /// The symbols to draw: every one when they fit, otherwise the most depended on first.
    fn brain_symbols(&self, max_nodes: usize) -> Result<(Vec<SymbolRecord>, u64), EngineError> {
        let mut all: Vec<SymbolRecord> = Vec::new();
        for file in self.storage().list_files()? {
            all.extend(
                self.storage()
                    .symbols_in_file(&file.path)?
                    .into_iter()
                    // A module declaration is the file itself, already shown by its symbols.
                    .filter(|symbol| symbol.kind != SymbolKind::Module),
            );
        }
        let total = u64::try_from(all.len()).unwrap_or(u64::MAX);
        if all.len() <= max_nodes {
            return Ok((all, total));
        }
        let central: Vec<SymbolId> = self
            .storage()
            .central_symbols(max_nodes, None)?
            .into_iter()
            .map(|(record, _)| record.id)
            .collect();
        let rank: BTreeMap<SymbolId, usize> = central
            .iter()
            .enumerate()
            .map(|(position, id)| (*id, position))
            .collect();
        // Stable: within one tier the path order of the listing is kept.
        all.sort_by_key(|symbol| match rank.get(&symbol.id) {
            Some(&position) => (0, position),
            None if symbol.visibility == Visibility::Public => (1, 0),
            None => (2, 0),
        });
        all.truncate(max_nodes);
        Ok((all, total))
    }

    /// The edges among the drawn symbols, as [`EDGE_STRIDE`] numbers each, one per source,
    /// target and kind.
    fn brain_edges(
        &self,
        symbols: &[SymbolRecord],
        index: &BTreeMap<SymbolId, usize>,
        min_confidence: Confidence,
    ) -> Result<Vec<usize>, EngineError> {
        let mut seen: BTreeSet<(usize, usize, usize)> = BTreeSet::new();
        let mut edges: Vec<usize> = Vec::new();
        for (from, symbol) in symbols.iter().enumerate() {
            let out =
                self.storage()
                    .neighbors(symbol.id, Direction::Out, min_confidence, MAX_FANOUT)?;
            for neighbor in out {
                let Some(&to) = index.get(&neighbor.symbol.id) else {
                    continue;
                };
                let kind = edge_kind_index(neighbor.edge.kind);
                if from == to || !seen.insert((from, to, kind)) {
                    continue;
                }
                edges.extend([from, to, kind, confidence_index(neighbor.edge.confidence)]);
            }
        }
        Ok(edges)
    }

    /// Every memory, with the drawn symbols it is anchored to as node indices and the names of
    /// all its anchors, drawn or not.
    fn brain_memories(&self, index: &BTreeMap<SymbolId, usize>) -> Result<Vec<Value>, EngineError> {
        let records = self.storage().list_memories(&MemoryFilter {
            limit: usize::MAX,
            ..MemoryFilter::default()
        })?;
        Ok(records
            .iter()
            .map(|memory| {
                let anchors: Vec<usize> = memory
                    .anchors
                    .iter()
                    .filter_map(|anchor| anchor.symbol.and_then(|id| index.get(&id).copied()))
                    .collect();
                let about: Vec<&str> = memory
                    .anchors
                    .iter()
                    .map(|anchor| anchor.qualified_name.as_str())
                    .collect();
                json!([
                    memory.id.0,
                    memory.kind.as_str(),
                    memory.text,
                    u8::from(memory.stale_since.is_some()),
                    memory.provenance.as_str(),
                    anchors,
                    about,
                    memory.created_at,
                ])
            })
            .collect())
    }

    /// Builds the brain: symbols, the edges among them, and the memories anchored to them.
    ///
    /// `name` is what to call the repository, as for [`Engine::brief`].
    ///
    /// # Errors
    /// Returns a storage error when the index cannot be read.
    pub fn brain(&self, name: &str, q: &BrainQuery) -> Result<Brain, EngineError> {
        let (symbols, total_symbols) = self.brain_symbols(q.max_nodes)?;
        let index: BTreeMap<SymbolId, usize> = symbols
            .iter()
            .enumerate()
            .map(|(position, symbol)| (symbol.id, position))
            .collect();
        let (nodes, regions, paths) = node_rows(&symbols);
        let edges = self.brain_edges(&symbols, &index, q.min_confidence)?;
        let memories = self.brain_memories(&index)?;
        let stats = self.storage().stats()?;
        let counts = (symbols.len(), edges.len() / EDGE_STRIDE, memories.len());
        let value = json!({
            "repo": name,
            "files": stats.files,
            "symbols": stats.symbols,
            "drawable": total_symbols,
            "edgesTotal": stats.edges,
            "kinds": KINDS.iter().map(|k| k.as_str()).collect::<Vec<_>>(),
            "edgeKinds": EDGE_KINDS.iter().map(|k| k.as_str()).collect::<Vec<_>>(),
            "confidences": Confidence::ALL.iter().map(|c| c.as_str()).collect::<Vec<_>>(),
            "regions": regions,
            "paths": paths,
            "nodes": nodes,
            "edges": edges,
            "memories": memories,
        });
        Ok(Brain {
            nodes: counts.0,
            edges: counts.1,
            memories: counts.2,
            total_symbols,
            value,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{KINDS, clipped, kind_index, region_of};

    /// A region is the first two directories of a path, and a root file belongs to `.`.
    #[test]
    fn regions_come_from_the_leading_directories() {
        assert_eq!(region_of("crates/core/src/lib.rs"), "crates/core");
        assert_eq!(region_of("app/Models/User.php"), "app/Models");
        assert_eq!(region_of("src/main.rs"), "src");
        assert_eq!(region_of("main.rs"), ".");
    }

    /// Long text is cut on a character boundary and says so; short text is only flattened.
    #[test]
    fn clipping_keeps_characters_whole() {
        assert_eq!(clipped("a  b\n c", 10), "a b c");
        assert_eq!(clipped("ñandú ñandú", 6), "ñandú…");
    }

    /// Every kind has its own index, so the page never shows one kind as another.
    #[test]
    fn kind_indices_are_distinct() {
        let mut seen: Vec<usize> = KINDS.iter().map(|kind| kind_index(*kind)).collect();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), KINDS.len());
    }
}
