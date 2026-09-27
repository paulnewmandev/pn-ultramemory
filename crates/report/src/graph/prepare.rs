// SPDX-License-Identifier: Apache-2.0
//! Turns a raw [`Graph`] into the clean, capped graph every renderer draws.
//!
//! The raw graph comes from outside and may be inconsistent. Preparing it means: dropping edges
//! whose endpoints are out of range or equal (self-loops), merging duplicate edges by summing
//! their weights, keeping only the heaviest nodes when the graph is too big to read, keeping only
//! the heaviest edges among the surviving nodes, and computing the display text and group index
//! of every node.
//!
//! Invariants: the result is a pure function of the input; nodes keep their original relative
//! order; edges are sorted by `(from, to)` with indices into the surviving nodes; every edge has
//! a weight of at least one; no index is out of range. Cost is `O(N log N + E log E)`.

use std::collections::BTreeMap;

use crate::model::{Graph, GraphEdge, GraphNode};
use crate::text::{CLIP_NAME, single_line};

/// Maximum characters of a node label shown next to the node.
pub(crate) const SHORT_LABEL: usize = 24;

/// One surviving node, with cleaned display text.
#[derive(Debug, Clone)]
pub(crate) struct PNode<'a> {
    /// The node as given by the caller.
    pub raw: &'a GraphNode,
    /// Index of the node in the raw graph.
    pub source_index: usize,
    /// Clean single-line label, never empty.
    pub label: String,
    /// Clean single-line group name; empty when the node has no group.
    pub group: String,
    /// Index of the group in [`Prepared::groups`].
    pub group_index: usize,
}

/// One surviving edge, expressed with indices into [`Prepared::nodes`].
#[derive(Debug, Clone, Copy)]
pub(crate) struct PEdge<'a> {
    /// Index of the source node.
    pub from: usize,
    /// Index of the target node.
    pub to: usize,
    /// Sum of the weights of the merged duplicates, at least one.
    pub weight: u64,
    /// Kind of the relationship, as given.
    pub kind: &'a str,
    /// Confidence label, as given.
    pub confidence: &'a str,
}

impl PEdge<'_> {
    /// Returns `true` when the indexer was not sure about the relationship (a `heuristic` or
    /// `guess` confidence), which renderers draw dashed.
    pub(crate) fn is_uncertain(&self) -> bool {
        uncertain(self.confidence)
    }
}

/// Returns `true` for the two weakest confidence labels.
fn uncertain(confidence: &str) -> bool {
    let label = confidence.trim();
    label.eq_ignore_ascii_case("heuristic") || label.eq_ignore_ascii_case("guess")
}

/// A graph ready to be laid out and drawn.
#[derive(Debug, Clone)]
pub(crate) struct Prepared<'a> {
    /// Surviving nodes, in their original order.
    pub nodes: Vec<PNode<'a>>,
    /// Surviving edges, sorted by `(from, to)`.
    pub edges: Vec<PEdge<'a>>,
    /// Nodes of the raw graph.
    pub total_nodes: usize,
    /// Valid, merged edges of the raw graph (before any cap).
    pub total_edges: usize,
    /// Sorted, distinct group names of the surviving nodes; the empty group sorts first.
    pub groups: Vec<String>,
}

impl Prepared<'_> {
    /// Number of nodes dropped by the cap.
    pub(crate) fn hidden_nodes(&self) -> usize {
        self.total_nodes - self.nodes.len()
    }

    /// Largest node weight, at least one.
    pub(crate) fn max_node_weight(&self) -> u32 {
        self.nodes
            .iter()
            .map(|node| node.raw.weight)
            .max()
            .unwrap_or(0)
            .max(1)
    }

    /// Largest edge weight, at least one.
    pub(crate) fn max_edge_weight(&self) -> u64 {
        self.edges
            .iter()
            .map(|edge| edge.weight)
            .max()
            .unwrap_or(1)
            .max(1)
    }
}

/// Prepares `graph`, keeping at most `max_nodes` nodes and `max_edges` edges.
///
/// Nodes are ranked by weight, then by the total weight of their edges, then by original index;
/// edges are ranked by weight, then by position. Ties never depend on hashing.
pub(crate) fn prepare(graph: &Graph, max_nodes: usize, max_edges: usize) -> Prepared<'_> {
    let merged = merge_edges(graph);
    let total_edges = merged.len();
    let keep = choose_nodes(graph, &merged, max_nodes);

    let mut remap = vec![usize::MAX; graph.nodes.len()];
    for (new_index, &old_index) in keep.iter().enumerate() {
        remap[old_index] = new_index;
    }

    let mut edges: Vec<PEdge<'_>> = merged
        .into_iter()
        .filter(|((from, to), _)| remap[*from] != usize::MAX && remap[*to] != usize::MAX)
        .map(|((from, to), acc)| PEdge {
            from: remap[from],
            to: remap[to],
            weight: acc.weight,
            kind: acc.kind,
            confidence: acc.confidence,
        })
        .collect();
    if edges.len() > max_edges {
        let mut order: Vec<usize> = (0..edges.len()).collect();
        order.sort_by(|&a, &b| edges[b].weight.cmp(&edges[a].weight).then(a.cmp(&b)));
        order.truncate(max_edges);
        order.sort_unstable();
        edges = order.into_iter().map(|index| edges[index]).collect();
    }

    let mut nodes: Vec<PNode<'_>> = keep
        .iter()
        .map(|&source_index| {
            let raw = &graph.nodes[source_index];
            PNode {
                raw,
                source_index,
                label: node_label(raw, source_index),
                group: single_line(&raw.group, CLIP_NAME),
                group_index: 0,
            }
        })
        .collect();
    let mut groups: Vec<String> = nodes.iter().map(|node| node.group.clone()).collect();
    groups.sort();
    groups.dedup();
    for node in &mut nodes {
        node.group_index = groups.binary_search(&node.group).unwrap_or(0);
    }

    Prepared {
        nodes,
        edges,
        total_nodes: graph.nodes.len(),
        total_edges,
        groups,
    }
}

/// The accumulator used while merging duplicate edges.
struct Merged<'a> {
    /// Sum of weights.
    weight: u64,
    /// Kind of the strongest duplicate seen first.
    kind: &'a str,
    /// Confidence of the strongest duplicate seen first.
    confidence: &'a str,
}

/// Drops invalid edges and merges duplicates, keyed and ordered by `(from, to)`.
fn merge_edges(graph: &Graph) -> BTreeMap<(usize, usize), Merged<'_>> {
    let count = graph.nodes.len();
    let mut merged: BTreeMap<(usize, usize), Merged<'_>> = BTreeMap::new();
    for edge in &graph.edges {
        if edge.from >= count || edge.to >= count || edge.from == edge.to {
            continue;
        }
        let weight = u64::from(edge.weight.max(1));
        merged
            .entry((edge.from, edge.to))
            .and_modify(|acc| absorb(acc, edge, weight))
            .or_insert(Merged {
                weight,
                kind: &edge.kind,
                confidence: &edge.confidence,
            });
    }
    merged
}

/// Adds a duplicate edge to the accumulator; a confident duplicate replaces an uncertain label.
fn absorb<'a>(acc: &mut Merged<'a>, edge: &'a GraphEdge, weight: u64) {
    acc.weight = acc.weight.saturating_add(weight);
    if uncertain(acc.confidence) && !uncertain(&edge.confidence) {
        acc.kind = &edge.kind;
        acc.confidence = &edge.confidence;
    }
}

/// Returns the original indices of the nodes to keep, in ascending order.
fn choose_nodes(
    graph: &Graph,
    merged: &BTreeMap<(usize, usize), Merged<'_>>,
    max_nodes: usize,
) -> Vec<usize> {
    let count = graph.nodes.len();
    if count <= max_nodes {
        return (0..count).collect();
    }
    let mut degree = vec![0u64; count];
    for (&(from, to), acc) in merged {
        degree[from] = degree[from].saturating_add(acc.weight);
        degree[to] = degree[to].saturating_add(acc.weight);
    }
    let mut order: Vec<usize> = (0..count).collect();
    order.sort_by(|&a, &b| {
        graph.nodes[b]
            .weight
            .cmp(&graph.nodes[a].weight)
            .then(degree[b].cmp(&degree[a]))
            .then(a.cmp(&b))
    });
    order.truncate(max_nodes);
    order.sort_unstable();
    order
}

/// Returns the clean label of a node, falling back to its id and then to its position.
fn node_label(node: &GraphNode, index: usize) -> String {
    let label = single_line(&node.label, CLIP_NAME);
    if !label.is_empty() {
        return label;
    }
    let id = single_line(&node.id, CLIP_NAME);
    if id.is_empty() {
        format!("#{index}")
    } else {
        id
    }
}

/// Shortens a clean label for display next to a node.
pub(crate) fn short_label(label: &str) -> String {
    single_line(label, SHORT_LABEL)
}

#[cfg(test)]
mod tests {
    use super::{prepare, short_label};
    use crate::model::{Graph, GraphEdge, GraphNode};

    /// Builds a node.
    fn node(id: &str, group: &str, weight: u32) -> GraphNode {
        GraphNode {
            id: id.into(),
            label: id.into(),
            group: group.into(),
            weight,
        }
    }

    /// Builds an edge.
    fn edge(from: usize, to: usize, weight: u32) -> GraphEdge {
        GraphEdge {
            from,
            to,
            weight,
            kind: "calls".into(),
            confidence: "exact".into(),
        }
    }

    /// Edges with bad endpoints and self-loops are ignored.
    #[test]
    fn invalid_edges_are_dropped() {
        let graph = Graph {
            nodes: vec![node("a", "", 1), node("b", "", 1)],
            edges: vec![edge(0, 0, 3), edge(0, 5, 1), edge(9, 1, 1), edge(0, 1, 2)],
        };
        let prep = prepare(&graph, 10, 10);
        assert_eq!(prep.edges.len(), 1);
        assert_eq!(
            (prep.edges[0].from, prep.edges[0].to, prep.edges[0].weight),
            (0, 1, 2)
        );
        assert_eq!(prep.total_edges, 1);
    }

    /// Duplicate edges merge by summing weights, and a zero weight counts as one.
    #[test]
    fn duplicate_edges_are_merged() {
        let graph = Graph {
            nodes: vec![node("a", "", 1), node("b", "", 1)],
            edges: vec![
                edge(0, 1, 2),
                edge(0, 1, 3),
                edge(1, 0, 0),
                edge(0, 1, u32::MAX),
            ],
        };
        let prep = prepare(&graph, 10, 10);
        assert_eq!(prep.edges.len(), 2);
        assert_eq!(prep.edges[0].weight, 5 + u64::from(u32::MAX));
        assert_eq!(prep.edges[1].weight, 1);
    }

    /// A confident duplicate replaces the label of an uncertain one.
    #[test]
    fn confident_duplicates_win() {
        let mut weak = edge(0, 1, 1);
        weak.confidence = "Guess".into();
        weak.kind = "maybe".into();
        let graph = Graph {
            nodes: vec![node("a", "", 1), node("b", "", 1)],
            edges: vec![weak.clone(), edge(0, 1, 1)],
        };
        let prep = prepare(&graph, 10, 10);
        assert!(!prep.edges[0].is_uncertain());
        assert_eq!(prep.edges[0].kind, "calls");
        let alone = Graph {
            nodes: graph.nodes.clone(),
            edges: vec![weak],
        };
        assert!(prepare(&alone, 10, 10).edges[0].is_uncertain());
    }

    /// The node cap keeps the heaviest nodes in their original order and remaps edges.
    #[test]
    fn node_cap_keeps_the_heaviest() {
        let nodes = vec![
            node("a", "", 1),
            node("b", "", 9),
            node("c", "", 5),
            node("d", "", 7),
        ];
        let graph = Graph {
            nodes,
            edges: vec![edge(0, 1, 1), edge(1, 3, 4), edge(2, 3, 2)],
        };
        let prep = prepare(&graph, 2, 10);
        let ids: Vec<&str> = prep.nodes.iter().map(|n| n.raw.id.as_str()).collect();
        assert_eq!(ids, ["b", "d"]);
        assert_eq!(prep.hidden_nodes(), 2);
        assert_eq!(prep.edges.len(), 1);
        assert_eq!((prep.edges[0].from, prep.edges[0].to), (0, 1));
        assert_eq!(prep.total_edges, 3);
        assert_eq!(prep.total_nodes, 4);
    }

    /// Equal weights fall back to connectivity and then to position.
    #[test]
    fn node_cap_breaks_ties_deterministically() {
        let nodes = vec![node("a", "", 1), node("b", "", 1), node("c", "", 1)];
        let graph = Graph {
            nodes,
            edges: vec![edge(2, 1, 5)],
        };
        let prep = prepare(&graph, 2, 10);
        let ids: Vec<&str> = prep.nodes.iter().map(|n| n.raw.id.as_str()).collect();
        assert_eq!(ids, ["b", "c"]);
    }

    /// The edge cap keeps the heaviest edges, sorted by position.
    #[test]
    fn edge_cap_keeps_the_heaviest() {
        let nodes: Vec<GraphNode> = (0..4).map(|i| node(&i.to_string(), "", 1)).collect();
        let graph = Graph {
            nodes,
            edges: vec![edge(0, 1, 1), edge(1, 2, 9), edge(2, 3, 5), edge(3, 0, 5)],
        };
        let prep = prepare(&graph, 10, 2);
        let pairs: Vec<(usize, usize)> = prep.edges.iter().map(|e| (e.from, e.to)).collect();
        assert_eq!(pairs, [(1, 2), (2, 3)]);
        assert_eq!(prep.total_edges, 4);
    }

    /// Groups are sorted and distinct, the empty group sorts first, and indices point at them.
    #[test]
    fn groups_are_indexed() {
        let graph = Graph {
            nodes: vec![
                node("a", "zeta", 1),
                node("b", "", 1),
                node("c", "alpha", 1),
                node("d", "zeta", 1),
            ],
            edges: vec![],
        };
        let prep = prepare(&graph, 10, 10);
        assert_eq!(prep.groups, ["", "alpha", "zeta"]);
        let indices: Vec<usize> = prep.nodes.iter().map(|n| n.group_index).collect();
        assert_eq!(indices, [2, 0, 1, 2]);
    }

    /// Labels fall back to the id and then to the position, and are clipped.
    #[test]
    fn labels_have_fallbacks() {
        let graph = Graph {
            nodes: vec![
                GraphNode {
                    id: "the-id".into(),
                    label: " \n ".into(),
                    ..GraphNode::default()
                },
                GraphNode::default(),
                GraphNode {
                    label: "x".repeat(1000),
                    ..GraphNode::default()
                },
            ],
            edges: vec![],
        };
        let prep = prepare(&graph, 10, 10);
        assert_eq!(prep.nodes[0].label, "the-id");
        assert_eq!(prep.nodes[1].label, "#1");
        assert_eq!(prep.nodes[2].label.chars().count(), 161);
        assert_eq!(short_label(&prep.nodes[2].label).chars().count(), 25);
    }

    /// An empty graph prepares to an empty result.
    #[test]
    fn empty_graph() {
        let graph = Graph::default();
        let prep = prepare(&graph, 10, 10);
        assert!(prep.nodes.is_empty() && prep.edges.is_empty());
        assert_eq!(prep.max_node_weight(), 1);
        assert_eq!(prep.max_edge_weight(), 1);
        assert_eq!(prep.hidden_nodes(), 0);
    }

    /// Preparing the same graph twice gives identical results.
    #[test]
    fn preparation_is_deterministic() {
        let nodes: Vec<GraphNode> = (0..50)
            .map(|i| node(&format!("n{i}"), "g", i % 7))
            .collect();
        let edges: Vec<GraphEdge> = (0..200)
            .map(|i| {
                edge(
                    i % 50,
                    (i * 7 + 3) % 50,
                    1 + u32::try_from(i % 4).unwrap_or(1),
                )
            })
            .collect();
        let graph = Graph { nodes, edges };
        let first = prepare(&graph, 20, 30);
        let second = prepare(&graph, 20, 30);
        let key = |p: &super::Prepared<'_>| {
            (
                p.nodes.iter().map(|n| n.source_index).collect::<Vec<_>>(),
                p.edges
                    .iter()
                    .map(|e| (e.from, e.to, e.weight))
                    .collect::<Vec<_>>(),
            )
        };
        assert_eq!(key(&first), key(&second));
    }
}
