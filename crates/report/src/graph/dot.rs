// SPDX-License-Identifier: Apache-2.0
//! The code graph as Graphviz DOT.
//!
//! Every identifier and label is a quoted string with backslashes and quotes escaped, so user
//! text cannot end a string or add attributes. Node ids are the ids of the graph made unique
//! (a repeated id gets a `#2`, `#3`, ... suffix; an empty one becomes `n<index>`). Nodes are
//! filled by group color, and edges use `penwidth` proportional to their weight.
//!
//! Invariants: the output is a syntactically valid `digraph` for any input; it is deterministic;
//! no user text ever appears outside a quoted string.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use super::layout::edge_width;
use super::palette;
use super::prepare::Prepared;
use crate::numfmt::{self, coord};
use crate::text::single_line;

/// Escapes text for use inside a DOT double-quoted string.
pub(crate) fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            _ => out.push(c),
        }
    }
    out
}

/// Returns one unique, clean id per node.
fn unique_ids(prep: &Prepared<'_>) -> Vec<String> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut ids = Vec::with_capacity(prep.nodes.len());
    for (index, node) in prep.nodes.iter().enumerate() {
        let mut base = single_line(&node.raw.id, 256);
        if base.is_empty() {
            base = format!("n{index}");
        }
        let mut candidate = base.clone();
        let mut counter = 2u32;
        while !seen.insert(candidate.clone()) {
            candidate = format!("{base}#{counter}");
            counter += 1;
        }
        ids.push(candidate);
    }
    ids
}

/// Renders the graph as DOT text, ending with a newline.
pub(crate) fn render(prep: &Prepared<'_>) -> String {
    let ids = unique_ids(prep);
    let mut out = String::from("digraph \"pn-ultramemory\" {\n");
    out.push_str("    graph [rankdir=LR, fontname=\"Helvetica\", bgcolor=\"transparent\"];\n");
    out.push_str(
        "    node [shape=box, style=\"rounded,filled\", fontname=\"Helvetica\", fontsize=11, \
         color=\"#64748b\", fillcolor=\"#f1f5f9\", fontcolor=\"#111827\"];\n",
    );
    out.push_str(
        "    edge [color=\"#64748b\", arrowsize=0.8, fontname=\"Helvetica\", fontsize=9];\n",
    );
    for (node, id) in prep.nodes.iter().zip(&ids) {
        let _ = writeln!(
            out,
            "    \"{}\" [label=\"{}\", fillcolor=\"{}\", color=\"{}\"];",
            escape(id),
            escape(&node.label),
            palette::soft(node.group_index),
            palette::main(node.group_index)
        );
    }
    let max_weight = prep.max_edge_weight();
    for edge in &prep.edges {
        let mut attributes = format!("penwidth={}", coord(edge_width(edge.weight, max_weight)));
        if edge.weight > 1 {
            let _ = write!(
                attributes,
                ", label=\"{}\"",
                numfmt::int(edge.weight, crate::lang::Lang::En)
            );
        }
        if edge.is_uncertain() {
            attributes.push_str(", style=dashed");
        }
        let _ = writeln!(
            out,
            "    \"{}\" -> \"{}\" [{attributes}];",
            escape(&ids[edge.from]),
            escape(&ids[edge.to])
        );
    }
    out.push_str("}\n");
    out
}

#[cfg(test)]
mod tests {
    use super::{escape, render, unique_ids};
    use crate::graph::prepare::prepare;
    use crate::model::{Graph, GraphEdge, GraphNode};

    /// Builds a node with an explicit id.
    fn node(id: &str, label: &str, group: &str) -> GraphNode {
        GraphNode {
            id: id.into(),
            label: label.into(),
            group: group.into(),
            weight: 1,
        }
    }

    /// Quotes and backslashes are escaped, everything else is untouched.
    #[test]
    fn escaping() {
        assert_eq!(escape("a\"b\\c"), "a\\\"b\\\\c");
        assert_eq!(escape("\\n"), "\\\\n");
        assert_eq!(escape("ñandú 🙂"), "ñandú 🙂");
    }

    /// Repeated and empty ids are made unique.
    #[test]
    fn ids_are_unique() {
        let graph = Graph {
            nodes: vec![
                node("a", "A", ""),
                node("a", "A2", ""),
                node("", "E", ""),
                node("a#2", "X", ""),
            ],
            edges: vec![],
        };
        let prep = prepare(&graph, 10, 10);
        assert_eq!(unique_ids(&prep), ["a", "a#2", "n2", "a#2#2"]);
    }

    /// The output has the digraph frame, colored nodes and weighted, dashed edges.
    #[test]
    fn digraph_structure() {
        let graph = Graph {
            nodes: vec![node("a", "Alpha", "g1"), node("b", "Beta", "g2")],
            edges: vec![
                GraphEdge {
                    from: 0,
                    to: 1,
                    weight: 5,
                    ..GraphEdge::default()
                },
                GraphEdge {
                    from: 1,
                    to: 0,
                    weight: 1,
                    confidence: "guess".into(),
                    ..GraphEdge::default()
                },
            ],
        };
        let text = render(&prepare(&graph, 10, 10));
        assert!(text.starts_with("digraph \"pn-ultramemory\" {\n"));
        assert!(text.ends_with("}\n"));
        assert!(
            text.contains("\"a\" [label=\"Alpha\", fillcolor=\"#cfe3f5\", color=\"#0072b2\"];")
        );
        assert!(text.contains("\"b\" [label=\"Beta\", fillcolor=\"#fbe5b6\", color=\"#e69f00\"];"));
        assert!(text.contains("\"a\" -> \"b\" [penwidth=3.2, label=\"5\"];"));
        assert!(text.contains("\"b\" -> \"a\" [penwidth=1.87, style=dashed];"));
    }

    /// Hostile ids and labels stay inside their quotes.
    #[test]
    fn hostile_text_is_quoted() {
        let graph = Graph {
            nodes: vec![
                node("x\" -> \"evil", "l\"]; system(\"rm\"); [\"", ""),
                node("b\\", "line\nbreak", ""),
            ],
            edges: vec![GraphEdge {
                from: 0,
                to: 1,
                weight: 1,
                ..GraphEdge::default()
            }],
        };
        let text = render(&prepare(&graph, 10, 10));
        for line in text.lines().skip(1) {
            let unescaped = line.replace("\\\\", "").replace("\\\"", "");
            assert_eq!(
                unescaped.matches('"').count() % 2,
                0,
                "unbalanced quotes in {line}"
            );
        }
        assert!(!text.contains("\nbreak"));
    }

    /// An empty graph is a valid empty digraph.
    #[test]
    fn empty_graph() {
        let text = render(&prepare(&Graph::default(), 10, 10));
        assert!(text.ends_with(
            "edge [color=\"#64748b\", arrowsize=0.8, fontname=\"Helvetica\", fontsize=9];\n}\n"
        ));
    }
}
