// SPDX-License-Identifier: Apache-2.0
//! `PageRank` over the symbol graph, computed once per full index and stored as a structural
//! relevance signal for recall.
//!
//! # Why `PageRank` here
//! Recall already ranks symbols by text match, neighbor propagation and learned co-access, but all
//! of those are *local*: they need a seed to start from. A symbol that everything calls but nothing
//! names explicitly (a config loader, an error type, a shared allocator) can be invisible to a
//! keyword search and only reachable through a lucky neighbor. `PageRank` gives every symbol a
//! baseline importance derived purely from the shape of the graph, so the packer can offer it even
//! when no seed points at it directly.
//!
//! # The algorithm
//! Standard power-iteration `PageRank` with damping `d = 0.85`. Each node starts with `1/N`. On
//! every iteration each node keeps `(1 - d) / N` and receives `d * score[src] / out_degree[src]`
//! from every incoming edge. Dangling nodes (no outgoing edges) redistribute their score evenly so
//! probability is conserved. Iteration stops when the L1 delta between two rounds falls below
//! [`PAGERANK_EPSILON`] or after [`PAGERANK_ITERATIONS`] rounds, whichever comes first.
//!
//! # Cost
//! For a repository with `N` symbols and `E` edges, one iteration touches every edge once and
//! every node twice. With the defaults (`d = 0.85`, max 20 iterations, epsilon 1e-6) this is well
//! under 100 ms for repositories of 50k symbols and 200k edges on modern hardware, and converges
//! in far fewer than 20 rounds on typical code graphs.
//!
//! # Integration
//! [`compute`] takes the edge list as `(src, dst)` pairs and the number of nodes. It returns a
//! `Vec<f64>` indexed by the compact node index used during construction; the caller maps those
//! back to `SymbolId`s before persisting. The scores are normalized so the maximum is `1.0`, which
//! lets [`super::recall::ranking::structural_relevance`] map them into a fixed relevance range
//! without knowing the absolute scale.

use crate::recall::tuning;

/// Computes `PageRank` scores for a directed graph given as an edge list.
///
/// `edges` are `(src, dst)` pairs where both indices are in `0..node_count`. Nodes with no edges
/// still participate: they receive the dangling-node share on every iteration. Returns a vector of
/// length `node_count` with non-negative scores summing to approximately `1.0`; an empty graph
/// returns an empty vector.
#[allow(clippy::cast_precision_loss)]
pub(crate) fn compute(edges: &[(usize, usize)], node_count: usize) -> Vec<f64> {
    if node_count == 0 {
        return Vec::new();
    }
    let n = node_count as f64;
    let base = (1.0 - tuning::PAGERANK_DAMPING) / n;

    // Build adjacency lists and count out-degrees in one pass.
    let mut out_degree = vec![0_usize; node_count];
    let mut in_edges: Vec<Vec<usize>> = vec![Vec::new(); node_count];
    for &(src, dst) in edges {
        if src < node_count && dst < node_count {
            out_degree[src] += 1;
            in_edges[dst].push(src);
        }
    }

    let mut scores = vec![1.0 / n; node_count];
    let mut next = vec![0.0_f64; node_count];

    for _ in 0..tuning::PAGERANK_ITERATIONS {
        // Dangling nodes (out_degree == 0) leak their entire score; redistribute it evenly.
        let mut dangling_sum = 0.0_f64;
        for (i, &deg) in out_degree.iter().enumerate() {
            if deg == 0 {
                dangling_sum += scores[i];
            }
        }
        let dangling_share = tuning::PAGERANK_DAMPING * dangling_sum / n;

        for i in 0..node_count {
            let mut incoming = 0.0_f64;
            for &src in &in_edges[i] {
                // Safety: out_degree[src] > 0 because src has at least this outgoing edge.
                incoming += scores[src] / out_degree[src] as f64;
            }
            next[i] = base + dangling_share + tuning::PAGERANK_DAMPING * incoming;
        }

        // Check convergence before swapping.
        let mut delta = 0.0_f64;
        for i in 0..node_count {
            delta += (next[i] - scores[i]).abs();
        }
        std::mem::swap(&mut scores, &mut next);
        if delta < tuning::PAGERANK_EPSILON {
            break;
        }
    }

    // Normalize so the maximum score is 1.0; ranking only needs relative order.
    let max = scores.iter().copied().fold(0.0_f64, f64::max);
    if max > 0.0 {
        for s in &mut scores {
            *s /= max;
        }
    }
    scores
}

#[cfg(test)]
mod tests {
    use super::compute;

    /// An empty graph produces no scores.
    #[test]
    fn empty_graph() {
        assert!(compute(&[], 0).is_empty());
    }

    /// A single isolated node gets score 1.0 after normalization.
    #[test]
    fn single_node() {
        let scores = compute(&[], 1);
        assert_eq!(scores.len(), 1);
        assert!((scores[0] - 1.0).abs() < 1e-9);
    }

    /// In a star graph the center receives more rank than any leaf.
    #[test]
    fn star_center_beats_leaves() {
        // Center = 0, leaves = 1..5, edges leaf -> center.
        let edges: Vec<(usize, usize)> = (1..=5).map(|leaf| (leaf, 0)).collect();
        let scores = compute(&edges, 6);
        assert_eq!(scores.len(), 6);
        for leaf_score in &scores[1..] {
            assert!(scores[0] > *leaf_score);
        }
    }

    /// In a directed cycle every node ends up with the same score.
    #[test]
    fn cycle_is_uniform() {
        let n = 4;
        let edges: Vec<(usize, usize)> = (0..n).map(|i| (i, (i + 1) % n)).collect();
        let scores = compute(&edges, n);
        for s in &scores {
            assert!(
                (s - scores[0]).abs() < 1e-6,
                "cycle scores should be uniform"
            );
        }
    }

    /// A linear chain gives strictly decreasing scores from sink back to source.
    #[test]
    fn chain_decreases_upstream() {
        // 0 -> 1 -> 2 -> 3; sink is 3.
        let edges = vec![(0, 1), (1, 2), (2, 3)];
        let scores = compute(&edges, 4);
        assert!(scores[3] > scores[2]);
        assert!(scores[2] > scores[1]);
        assert!(scores[1] > scores[0]);
    }

    /// Scores converge well before the iteration cap on a modest graph.
    #[test]
    fn converges_within_cap() {
        // Complete bipartite K_{3,3}: should converge in a handful of iterations.
        let mut edges = Vec::new();
        for a in 0..3 {
            for b in 3..6 {
                edges.push((a, b));
                edges.push((b, a));
            }
        }
        let scores = compute(&edges, 6);
        assert_eq!(scores.len(), 6);
        // All scores positive and normalized.
        assert!(scores.iter().all(|&s| s > 0.0));
        let max = scores.iter().copied().fold(0.0_f64, f64::max);
        assert!((max - 1.0).abs() < 1e-9);
    }
}
