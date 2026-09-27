// SPDX-License-Identifier: Apache-2.0
//! The code graph as JSON, written by hand so the crate needs no serializer.
//!
//! The document is `{"nodes": [...], "edges": [...]}`. Nodes carry `id`, `label`, `group` and
//! `weight`; edges carry `from` and `to` (indices into `nodes`), the ids of both ends as `source`
//! and `target`, `weight` (duplicates merged), `kind` and `confidence`. Text is exported exactly
//! as given, with full JSON escaping: quotes, backslashes, control characters as `\u00XX`, and
//! the line and paragraph separators U+2028 and U+2029 so the text is also valid JavaScript.
//!
//! Invariants: the output is always valid JSON; it is deterministic; edges with bad endpoints are
//! dropped and duplicates merged before writing.

use std::fmt::Write as _;

use super::prepare::Prepared;

/// Appends `text` as a JSON string literal, including the quotes.
pub(crate) fn push_string(out: &mut String, text: &str) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if u32::from(c) < 0x20 || c == '\u{2028}' || c == '\u{2029}' || c == '\u{7f}' => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Renders the graph as pretty-printed JSON with one node or edge per line.
pub(crate) fn render(prep: &Prepared<'_>) -> String {
    let mut out = String::from("{\n  \"nodes\": [");
    for (index, node) in prep.nodes.iter().enumerate() {
        out.push_str(if index == 0 { "\n    " } else { ",\n    " });
        out.push_str("{\"id\": ");
        push_string(&mut out, &node.raw.id);
        out.push_str(", \"label\": ");
        push_string(&mut out, &node.raw.label);
        out.push_str(", \"group\": ");
        push_string(&mut out, &node.raw.group);
        let _ = write!(out, ", \"weight\": {}}}", node.raw.weight);
    }
    out.push_str(if prep.nodes.is_empty() {
        "],\n  \"edges\": ["
    } else {
        "\n  ],\n  \"edges\": ["
    });
    for (index, edge) in prep.edges.iter().enumerate() {
        out.push_str(if index == 0 { "\n    " } else { ",\n    " });
        let _ = write!(
            out,
            "{{\"from\": {}, \"to\": {}, \"source\": ",
            edge.from, edge.to
        );
        push_string(&mut out, &prep.nodes[edge.from].raw.id);
        out.push_str(", \"target\": ");
        push_string(&mut out, &prep.nodes[edge.to].raw.id);
        let _ = write!(out, ", \"weight\": {}, \"kind\": ", edge.weight);
        push_string(&mut out, edge.kind);
        out.push_str(", \"confidence\": ");
        push_string(&mut out, edge.confidence);
        out.push('}');
    }
    out.push_str(if prep.edges.is_empty() {
        "]\n}\n"
    } else {
        "\n  ]\n}\n"
    });
    out
}

#[cfg(test)]
mod tests {
    use super::{push_string, render};
    use crate::graph::prepare::prepare;
    use crate::model::{Graph, GraphEdge, GraphNode};

    /// Returns `text` as a JSON string literal.
    fn literal(text: &str) -> String {
        let mut out = String::new();
        push_string(&mut out, text);
        out
    }

    /// Quotes, backslashes and control characters are escaped, other text is kept as UTF-8.
    #[test]
    fn string_escaping() {
        assert_eq!(literal("a\"b\\c"), "\"a\\\"b\\\\c\"");
        assert_eq!(literal("l1\nl2\r\t"), "\"l1\\nl2\\r\\t\"");
        assert_eq!(literal("\u{0}\u{1f}\u{7f}"), "\"\\u0000\\u001f\\u007f\"");
        assert_eq!(literal("\u{8}\u{c}"), "\"\\b\\f\"");
        assert_eq!(literal("\u{2028}\u{2029}"), "\"\\u2028\\u2029\"");
        assert_eq!(
            literal("ñandú 🙂 שלום </script>"),
            "\"ñandú 🙂 שלום </script>\""
        );
        assert_eq!(literal(""), "\"\"");
    }

    /// The document lists nodes and edges with indices, ids and merged weights.
    #[test]
    fn document_shape() {
        let node = |id: &str, w| GraphNode {
            id: id.into(),
            label: id.to_uppercase(),
            group: "g".into(),
            weight: w,
        };
        let graph = Graph {
            nodes: vec![node("a", 3), node("b", 1)],
            edges: vec![
                GraphEdge {
                    from: 0,
                    to: 1,
                    weight: 2,
                    kind: "calls".into(),
                    confidence: "exact".into(),
                },
                GraphEdge {
                    from: 0,
                    to: 1,
                    weight: 3,
                    kind: "calls".into(),
                    confidence: "exact".into(),
                },
                GraphEdge {
                    from: 1,
                    to: 7,
                    weight: 3,
                    ..GraphEdge::default()
                },
            ],
        };
        let text = render(&prepare(&graph, usize::MAX, usize::MAX));
        assert_eq!(
            text,
            "{\n  \"nodes\": [\n    {\"id\": \"a\", \"label\": \"A\", \"group\": \"g\", \"weight\": 3},\n    \
             {\"id\": \"b\", \"label\": \"B\", \"group\": \"g\", \"weight\": 1}\n  ],\n  \"edges\": [\n    \
             {\"from\": 0, \"to\": 1, \"source\": \"a\", \"target\": \"b\", \"weight\": 5, \
             \"kind\": \"calls\", \"confidence\": \"exact\"}\n  ]\n}\n"
        );
    }

    /// An empty graph is `{"nodes": [], "edges": []}` with the same framing.
    #[test]
    fn empty_document() {
        assert_eq!(
            render(&prepare(&Graph::default(), usize::MAX, usize::MAX)),
            "{\n  \"nodes\": [],\n  \"edges\": []\n}\n"
        );
    }
}
