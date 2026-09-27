// SPDX-License-Identifier: Apache-2.0
//! Symbol-level and module-level graphs for export and reports.
//!
//! Two shapes of the same idea:
//!
//! * [`Engine::graph`] returns a [`SymbolGraph`]: either the neighbourhood of one symbol, followed
//!   in both directions up to a depth, or, with no centre, the most central symbols of the
//!   repository and the edges among them.
//! * [`Engine::module_graph`] returns a [`ModuleGraph`]: directories as nodes and the edges that
//!   cross between them, which is the picture that fits on one page.
//!
//! Both carry nodes and edges in the shape `pn-ultramemory-report` draws and exports, so a caller
//! can hand the result straight to a renderer.
//!
//! # Invariants
//! * **Endpoints are indices.** [`GraphEdgeInfo::from`] and [`GraphEdgeInfo::to`] index
//!   [`SymbolGraph::nodes`]. An edge with an endpoint outside the graph is dropped rather than
//!   clamped, so no renderer can be pointed at a node that is not there.
//! * **No duplicates.** Edges that share a pair of endpoints and a kind are merged, their weights
//!   summed and the strongest confidence kept: the relationship does exist at that confidence, at
//!   least once.
//! * **No self-loops.** A symbol that calls itself adds nothing to a picture of who depends on
//!   whom, and it would inflate its own degree.
//! * **Deterministic.** Node order comes from the breadth-first walk (or from
//!   [`pn_ultramemory_core::Storage::central_symbols`]), both of which are ordered, and edges are
//!   sorted by their endpoints and kind. Building the same graph twice gives the same value.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use pn_ultramemory_core::{Confidence, Direction, EdgeKind, SymbolId, SymbolRecord};
use serde_json::{Value, json};

use crate::engine::Engine;
use crate::error::EngineError;

/// How many steps from the centre are followed when the caller asks for no depth.
pub const DEFAULT_GRAPH_DEPTH: u32 = 2;

/// How many nodes a graph holds when the caller asks for no cap.
pub const DEFAULT_GRAPH_NODES: usize = 60;

/// The most neighbours read from one symbol in one step, in one direction.
const MAX_FANOUT: usize = 200;

/// How many module edges are read per node of a module graph.
const MODULE_EDGES_PER_NODE: usize = 8;

/// The fewest and the most module edges read, whatever the node cap.
const MODULE_EDGE_BOUNDS: (usize, usize) = (16, 4096);

/// What graph to build.
///
/// # Examples
/// ```
/// use pn_ultramemory_core::Confidence;
/// use pn_ultramemory_engine::GraphQuery;
///
/// let default = GraphQuery::default();
/// assert_eq!(default.center, None);
/// assert_eq!(default.depth, 2);
/// assert_eq!(default.max_nodes, 60);
/// assert_eq!(default.min_confidence, Confidence::Heuristic);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphQuery {
    /// The symbol at the centre, named the way [`Engine::resolve_symbol`] accepts. With no centre
    /// the most central symbols of the repository are used.
    pub center: Option<String>,
    /// How many steps away from the centre to include.
    pub depth: u32,
    /// The most nodes to return.
    pub max_nodes: usize,
    /// The weakest edge to follow and to draw.
    pub min_confidence: Confidence,
}

impl Default for GraphQuery {
    fn default() -> Self {
        Self {
            center: None,
            depth: DEFAULT_GRAPH_DEPTH,
            max_nodes: DEFAULT_GRAPH_NODES,
            min_confidence: Confidence::Heuristic,
        }
    }
}

/// One node of an exported graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphNodeInfo {
    /// Stable identifier: `s` and the symbol id for a symbol, the directory name for a module.
    pub id: String,
    /// The text drawn next to the node.
    pub label: String,
    /// The group the node belongs to, which decides its colour: the directory of its file for a
    /// symbol, the first path component for a module.
    pub group: String,
    /// How many edges of this graph touch the node, at least one.
    pub weight: u32,
}

/// One directed edge of an exported graph, with both endpoints as node indices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GraphEdgeInfo {
    /// Index of the source node.
    pub from: usize,
    /// Index of the target node.
    pub to: usize,
    /// What the relationship means. Module edges aggregate every kind and are reported as
    /// [`EdgeKind::Uses`].
    pub kind: EdgeKind,
    /// The strongest confidence among the edges merged into this one. For a module edge it is the
    /// threshold the underlying edges were counted at, because the aggregate mixes confidences.
    pub confidence: Confidence,
    /// How many underlying edges this one stands for.
    pub weight: u32,
}

/// Declared program elements and the relationships between them, ready to draw or export.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SymbolGraph {
    /// The nodes, centre first when the query had one.
    pub nodes: Vec<GraphNodeInfo>,
    /// The edges, as indices into `nodes`.
    pub edges: Vec<GraphEdgeInfo>,
}

/// Directories and what depends on what between them, ready to draw or export.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ModuleGraph {
    /// The nodes, largest module first.
    pub nodes: Vec<GraphNodeInfo>,
    /// The edges, as indices into `nodes`.
    pub edges: Vec<GraphEdgeInfo>,
}

/// The two uniform tables of a graph, so that TOON writes each set of column names once.
fn graph_value(nodes: &[GraphNodeInfo], edges: &[GraphEdgeInfo]) -> Value {
    let nodes: Vec<Value> = nodes
        .iter()
        .map(|node| {
            json!({
                "id": node.id,
                "label": node.label,
                "group": node.group,
                "weight": node.weight,
            })
        })
        .collect();
    let edges: Vec<Value> = edges
        .iter()
        .map(|edge| {
            json!({
                "from": edge.from,
                "to": edge.to,
                "kind": edge.kind.as_str(),
                "confidence": edge.confidence.as_str(),
                "weight": edge.weight,
            })
        })
        .collect();
    json!({ "nodes": nodes, "edges": edges })
}

impl SymbolGraph {
    /// The graph as a structured value: a `nodes` table and an `edges` table.
    #[must_use]
    pub fn to_value(&self) -> Value {
        graph_value(&self.nodes, &self.edges)
    }
}

impl ModuleGraph {
    /// The graph as a structured value: a `nodes` table and an `edges` table.
    #[must_use]
    pub fn to_value(&self) -> Value {
        graph_value(&self.nodes, &self.edges)
    }
}

/// The directory a path lives in, or `.` for a file at the root of the repository.
fn directory_of(path: &str) -> String {
    path.rsplit_once('/')
        .map_or_else(|| ".".to_owned(), |(directory, _)| directory.to_owned())
}

/// The first path component of a module name, which groups modules that share a root.
fn root_of(module: &str) -> String {
    module
        .split_once('/')
        .map_or_else(|| module.to_owned(), |(head, _)| head.to_owned())
}

/// A stable rank of an edge kind, so that merged edges can be keyed in a sorted map.
const fn kind_rank(kind: EdgeKind) -> u8 {
    match kind {
        EdgeKind::Calls => 0,
        EdgeKind::Inherits => 1,
        EdgeKind::Uses => 2,
    }
}

/// Edges being merged: endpoints and kind rank to kind, strongest confidence and total weight.
type Merged = BTreeMap<(usize, usize, u8), (EdgeKind, Confidence, u32)>;

/// Adds one underlying edge to the merge, dropping self-loops.
fn merge_edge(
    merged: &mut Merged,
    from: usize,
    to: usize,
    kind: EdgeKind,
    confidence: Confidence,
    weight: u32,
) {
    if from == to {
        return;
    }
    let entry = merged
        .entry((from, to, kind_rank(kind)))
        .or_insert((kind, confidence, 0));
    entry.1 = entry.1.max(confidence);
    entry.2 = entry.2.saturating_add(weight);
}

/// The merged edges in order of their endpoints and kind.
fn finish_edges(merged: Merged) -> Vec<GraphEdgeInfo> {
    merged
        .into_iter()
        .map(
            |((from, to, _), (kind, confidence, weight))| GraphEdgeInfo {
                from,
                to,
                kind,
                confidence,
                weight: weight.max(1),
            },
        )
        .collect()
}

/// How many edges of the graph touch each node, by node index.
fn degrees(node_count: usize, edges: &[GraphEdgeInfo]) -> Vec<u32> {
    let mut degrees = vec![0_u32; node_count];
    for edge in edges {
        for end in [edge.from, edge.to] {
            if let Some(degree) = degrees.get_mut(end) {
                *degree = degree.saturating_add(1);
            }
        }
    }
    degrees
}

impl Engine {
    /// The symbols around a centre, breadth first in both directions, at most `max_nodes` of them.
    fn neighbourhood(
        &self,
        center: &str,
        q: &GraphQuery,
    ) -> Result<Vec<SymbolRecord>, EngineError> {
        let root = self.resolve_symbol(center)?;
        let mut seen: BTreeSet<SymbolId> = BTreeSet::from([root.id]);
        let mut order = vec![root.clone()];
        let mut frontier: VecDeque<(SymbolId, u32)> = VecDeque::from([(root.id, 0)]);
        while let Some((id, step)) = frontier.pop_front() {
            if step >= q.depth || order.len() >= q.max_nodes {
                continue;
            }
            for direction in [Direction::Out, Direction::In] {
                let neighbors =
                    self.storage()
                        .neighbors(id, direction, q.min_confidence, MAX_FANOUT)?;
                for neighbor in neighbors {
                    if order.len() >= q.max_nodes {
                        return Ok(order);
                    }
                    let found = neighbor.symbol.id;
                    if seen.insert(found) {
                        order.push(neighbor.symbol);
                        frontier.push_back((found, step + 1));
                    }
                }
            }
        }
        Ok(order)
    }

    /// The edges among a set of symbols, following only outgoing edges so that each is seen once.
    fn edges_among(
        &self,
        nodes: &[SymbolRecord],
        min_confidence: Confidence,
    ) -> Result<Vec<GraphEdgeInfo>, EngineError> {
        let index: BTreeMap<SymbolId, usize> = nodes
            .iter()
            .enumerate()
            .map(|(position, record)| (record.id, position))
            .collect();
        let mut merged = Merged::new();
        for (from, record) in nodes.iter().enumerate() {
            let out =
                self.storage()
                    .neighbors(record.id, Direction::Out, min_confidence, MAX_FANOUT)?;
            for neighbor in out {
                let Some(&to) = index.get(&neighbor.symbol.id) else {
                    continue;
                };
                merge_edge(
                    &mut merged,
                    from,
                    to,
                    neighbor.edge.kind,
                    neighbor.edge.confidence,
                    1,
                );
            }
        }
        Ok(finish_edges(merged))
    }

    /// Builds a graph of symbols: the neighbourhood of a centre, or the most central symbols.
    ///
    /// # Errors
    /// Returns [`EngineError::NotFound`] or [`EngineError::Ambiguous`] when a centre is given and
    /// does not name exactly one symbol, and a storage error when the index cannot be read.
    ///
    /// # Examples
    /// ```no_run
    /// # fn demo(engine: &pn_ultramemory_engine::Engine) -> Result<(), pn_ultramemory_engine::EngineError> {
    /// use pn_ultramemory_engine::GraphQuery;
    ///
    /// let whole = engine.graph(&GraphQuery::default())?;
    /// let around = engine.graph(&GraphQuery {
    ///     center: Some("load_config".into()),
    ///     ..GraphQuery::default()
    /// })?;
    /// assert!(around.edges.iter().all(|edge| edge.from < around.nodes.len()));
    /// assert!(whole.nodes.len() <= 60);
    /// # Ok(())
    /// # }
    /// ```
    pub fn graph(&self, q: &GraphQuery) -> Result<SymbolGraph, EngineError> {
        if q.max_nodes == 0 {
            return Ok(SymbolGraph::default());
        }
        let records = match q.center.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
            Some(center) => self.neighbourhood(center, q)?,
            None => self
                .storage()
                .central_symbols(q.max_nodes, None)?
                .into_iter()
                .map(|(record, _)| record)
                .collect(),
        };
        let edges = self.edges_among(&records, q.min_confidence)?;
        let degrees = degrees(records.len(), &edges);
        let nodes = records
            .iter()
            .enumerate()
            .map(|(position, record)| GraphNodeInfo {
                id: format!("s{}", record.id.0),
                label: record.qualified_name.clone(),
                group: directory_of(&record.path),
                weight: degrees.get(position).copied().unwrap_or(0).max(1),
            })
            .collect();
        Ok(SymbolGraph { nodes, edges })
    }

    /// Builds a graph of directories: one node per module, one edge per pair that depends on the
    /// other.
    ///
    /// `depth` is how many leading directories name a module (`crates/core` at depth two), and
    /// `max_nodes` keeps the largest modules by symbol count. Edges are counted at
    /// [`Confidence::Heuristic`] and above, like every other structural aggregate of the tool.
    ///
    /// # Errors
    /// Returns a storage error when the index cannot be read.
    ///
    /// # Examples
    /// ```no_run
    /// # fn demo(engine: &pn_ultramemory_engine::Engine) -> Result<(), pn_ultramemory_engine::EngineError> {
    /// let graph = engine.module_graph(2, 40)?;
    /// assert!(graph.edges.iter().all(|edge| edge.to < graph.nodes.len()));
    /// # Ok(())
    /// # }
    /// ```
    pub fn module_graph(&self, depth: usize, max_nodes: usize) -> Result<ModuleGraph, EngineError> {
        if max_nodes == 0 {
            return Ok(ModuleGraph::default());
        }
        let modules: Vec<String> = self
            .storage()
            .module_stats(depth)?
            .into_iter()
            .take(max_nodes)
            .map(|module| module.name)
            .collect();
        let index: BTreeMap<&str, usize> = modules
            .iter()
            .enumerate()
            .map(|(position, name)| (name.as_str(), position))
            .collect();
        let (low, high) = MODULE_EDGE_BOUNDS;
        let limit = max_nodes
            .saturating_mul(MODULE_EDGES_PER_NODE)
            .clamp(low, high);
        let mut merged = Merged::new();
        for edge in self
            .storage()
            .module_edges(depth, Confidence::Heuristic, limit)?
        {
            let (Some(&from), Some(&to)) =
                (index.get(edge.from.as_str()), index.get(edge.to.as_str()))
            else {
                continue;
            };
            merge_edge(
                &mut merged,
                from,
                to,
                EdgeKind::Uses,
                Confidence::Heuristic,
                u32::try_from(edge.weight).unwrap_or(u32::MAX),
            );
        }
        let edges = finish_edges(merged);
        let degrees = degrees(modules.len(), &edges);
        let nodes = modules
            .iter()
            .enumerate()
            .map(|(position, name)| GraphNodeInfo {
                id: name.clone(),
                label: name.clone(),
                group: root_of(name),
                weight: degrees.get(position).copied().unwrap_or(0).max(1),
            })
            .collect();
        Ok(ModuleGraph { nodes, edges })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        GraphEdgeInfo, Merged, degrees, directory_of, finish_edges, kind_rank, merge_edge, root_of,
    };
    use pn_ultramemory_core::{Confidence, EdgeKind};

    /// A path's group is its directory, and a file at the root belongs to `.`.
    #[test]
    fn groups_come_from_the_directory() {
        assert_eq!(directory_of("crates/core/src/lib.rs"), "crates/core/src");
        assert_eq!(directory_of("src/config.rs"), "src");
        assert_eq!(directory_of("main.rs"), ".");
        assert_eq!(root_of("crates/core"), "crates");
        assert_eq!(root_of("."), ".");
        assert_eq!(root_of("src"), "src");
    }

    /// Every edge kind gets its own rank, so merging never confuses two kinds.
    #[test]
    fn kind_ranks_are_distinct() {
        let mut ranks: Vec<u8> = [EdgeKind::Calls, EdgeKind::Inherits, EdgeKind::Uses]
            .into_iter()
            .map(kind_rank)
            .collect();
        ranks.sort_unstable();
        ranks.dedup();
        assert_eq!(ranks.len(), 3);
    }

    /// Duplicates merge by summing weights and keeping the strongest confidence; self-loops are
    /// dropped; different kinds stay apart.
    #[test]
    fn merging_sums_weights_and_keeps_the_strongest() {
        let mut merged = Merged::new();
        merge_edge(&mut merged, 0, 1, EdgeKind::Calls, Confidence::Guess, 1);
        merge_edge(&mut merged, 0, 1, EdgeKind::Calls, Confidence::Resolved, 2);
        merge_edge(&mut merged, 0, 1, EdgeKind::Uses, Confidence::Heuristic, 1);
        merge_edge(&mut merged, 2, 2, EdgeKind::Calls, Confidence::Exact, 9);
        let edges = finish_edges(merged);
        assert_eq!(edges.len(), 2);
        assert_eq!(
            edges.first().copied(),
            Some(GraphEdgeInfo {
                from: 0,
                to: 1,
                kind: EdgeKind::Calls,
                confidence: Confidence::Resolved,
                weight: 3,
            })
        );
        assert_eq!(edges.get(1).map(|edge| edge.kind), Some(EdgeKind::Uses));
    }

    /// A degree counts both ends of every edge and ignores an endpoint outside the graph.
    #[test]
    fn degrees_count_both_ends() {
        let edge = |from, to| GraphEdgeInfo {
            from,
            to,
            kind: EdgeKind::Calls,
            confidence: Confidence::Exact,
            weight: 1,
        };
        assert_eq!(degrees(3, &[edge(0, 1), edge(1, 2), edge(1, 9)]), [1, 3, 1]);
        assert_eq!(degrees(0, &[edge(0, 1)]), Vec::<u32>::new());
    }
}
