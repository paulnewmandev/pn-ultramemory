// SPDX-License-Identifier: Apache-2.0
//! The code graph as a Mermaid `flowchart LR`.
//!
//! Nodes get the safe ids `n0`, `n1`, ... and quoted labels in which every character that has a
//! meaning to Mermaid is replaced by its numeric entity (`#quot;`, `#124;`, ...), so a label can
//! never end its string, open a shape or inject HTML. With two or more groups each group becomes
//! a `subgraph` and its nodes share a `classDef` color. Edges show their weight as a label when
//! it is above one, and uncertain edges are dotted.
//!
//! Invariants: the output parses with the official Mermaid parser for any input; it is
//! deterministic (groups sorted by name, nodes and edges in prepared order); user text appears
//! only inside quoted labels.

use std::fmt::Write as _;

use super::palette;
use super::prepare::Prepared;
use super::svg::group_display;
use crate::lang::Lang;

/// Replaces every character Mermaid could interpret inside a quoted label by an entity code.
pub(crate) fn escape_label(label: &str) -> String {
    let mut out = String::with_capacity(label.len());
    for c in label.chars() {
        match c {
            '"' => out.push_str("#quot;"),
            '<' => out.push_str("#lt;"),
            '>' => out.push_str("#gt;"),
            '&' => out.push_str("#amp;"),
            '#' | '|' | '[' | ']' | '(' | ')' | '{' | '}' | '`' | ';' | '\\' | '%' => {
                let _ = write!(out, "#{};", u32::from(c));
            }
            _ => out.push(c),
        }
    }
    out
}

/// Renders the graph as Mermaid text, ending with a newline.
pub(crate) fn render(prep: &Prepared<'_>, lang: Lang) -> String {
    let mut out = String::from("flowchart LR\n");
    let _ = writeln!(
        out,
        "    %% pn-ultramemory code graph: {} nodes, {} edges",
        prep.nodes.len(),
        prep.edges.len()
    );
    if prep.hidden_nodes() > 0 {
        let _ = writeln!(out, "    %% +{} more nodes not shown", prep.hidden_nodes());
    }
    let grouped = prep.groups.len() >= 2;
    if grouped {
        for (group_index, group) in prep.groups.iter().enumerate() {
            let _ = writeln!(
                out,
                "    subgraph sg{group_index}[\"{}\"]",
                escape_label(&group_display(group, lang))
            );
            for (index, node) in prep.nodes.iter().enumerate() {
                if node.group_index == group_index {
                    let _ = writeln!(out, "        n{index}[\"{}\"]", escape_label(&node.label));
                }
            }
            out.push_str("    end\n");
        }
    } else {
        for (index, node) in prep.nodes.iter().enumerate() {
            let _ = writeln!(out, "    n{index}[\"{}\"]", escape_label(&node.label));
        }
    }
    for edge in &prep.edges {
        let arrow = if edge.is_uncertain() { "-.->" } else { "-->" };
        if edge.weight > 1 {
            let _ = writeln!(
                out,
                "    n{} {arrow}|{}| n{}",
                edge.from, edge.weight, edge.to
            );
        } else {
            let _ = writeln!(out, "    n{} {arrow} n{}", edge.from, edge.to);
        }
    }
    if grouped {
        for group_index in 0..prep.groups.len() {
            let _ = writeln!(
                out,
                "    classDef c{group_index} fill:{},stroke:{},color:#111827",
                palette::soft(group_index),
                palette::main(group_index)
            );
            let members: Vec<String> = prep
                .nodes
                .iter()
                .enumerate()
                .filter(|(_, node)| node.group_index == group_index)
                .map(|(index, _)| format!("n{index}"))
                .collect();
            if !members.is_empty() {
                let _ = writeln!(out, "    class {} c{group_index}", members.join(","));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{escape_label, render};
    use crate::graph::prepare::prepare;
    use crate::lang::Lang;
    use crate::model::{Graph, GraphEdge, GraphNode};

    /// Builds a node.
    fn node(label: &str, group: &str) -> GraphNode {
        GraphNode {
            id: label.into(),
            label: label.into(),
            group: group.into(),
            weight: 1,
        }
    }

    /// Every character with a meaning to Mermaid is neutralized.
    #[test]
    fn labels_are_neutralized() {
        let hostile = "a\"b<c>d|e[f]g(h){i}`j;k\\l%m#n&o";
        let safe = escape_label(hostile);
        for forbidden in [
            '"', '<', '>', '|', '[', ']', '(', ')', '{', '}', '`', '\\', '%',
        ] {
            assert!(!safe.contains(forbidden), "{forbidden} survived in {safe}");
        }
        assert_eq!(
            safe,
            "a#quot;b#lt;c#gt;d#124;e#91;f#93;g#40;h#41;#123;i#125;#96;j#59;k#92;l#37;m#35;n#amp;o"
        );
        assert_eq!(escape_label("plain ñandú 🙂"), "plain ñandú 🙂");
    }

    /// A single-group graph is flat, uses safe ids and labels edges only when the weight exceeds one.
    #[test]
    fn flat_graph_layout() {
        let graph = Graph {
            nodes: vec![node("a", ""), node("b", ""), node("c", "")],
            edges: vec![
                GraphEdge {
                    from: 0,
                    to: 1,
                    weight: 3,
                    ..GraphEdge::default()
                },
                GraphEdge {
                    from: 1,
                    to: 2,
                    weight: 1,
                    ..GraphEdge::default()
                },
                GraphEdge {
                    from: 2,
                    to: 0,
                    weight: 2,
                    confidence: "heuristic".into(),
                    ..GraphEdge::default()
                },
            ],
        };
        let text = render(&prepare(&graph, 150, 500), Lang::En);
        assert!(text.starts_with("flowchart LR\n"));
        assert!(text.contains("    n0[\"a\"]\n"));
        assert!(text.contains("    n0 -->|3| n1\n"));
        assert!(text.contains("    n1 --> n2\n"));
        assert!(text.contains("    n2 -.->|2| n0\n"));
        assert!(!text.contains("subgraph"));
        assert!(!text.contains("classDef"));
    }

    /// Two or more groups produce one subgraph and one color class each, in sorted order.
    #[test]
    fn grouped_graph_has_subgraphs() {
        let graph = Graph {
            nodes: vec![
                node("a", "beta"),
                node("b", "alpha"),
                node("c", "beta"),
                node("d", ""),
            ],
            edges: vec![GraphEdge {
                from: 0,
                to: 1,
                weight: 1,
                ..GraphEdge::default()
            }],
        };
        let text = render(&prepare(&graph, 150, 500), Lang::Es);
        assert!(text.contains("subgraph sg0[\"Sin grupo\"]"));
        assert!(text.contains("subgraph sg1[\"alpha\"]"));
        assert!(text.contains("subgraph sg2[\"beta\"]"));
        assert_eq!(text.matches("    end\n").count(), 3);
        assert!(text.contains("classDef c2 fill:#c2ebdc,stroke:#009e73,color:#111827"));
        assert!(text.contains("class n0,n2 c2"));
    }

    /// Hostile text never breaks out of its quoted label.
    #[test]
    fn hostile_text_stays_inside_labels() {
        let graph = Graph {
            nodes: vec![
                node("x\"]\n    n9[\"pwned", "g\"1"),
                node("<img src=x onerror=alert(1)>", "g2"),
            ],
            edges: vec![],
        };
        let text = render(&prepare(&graph, 150, 500), Lang::En);
        assert!(!text.contains("n9["));
        assert!(!text.contains("<img"));
        for line in text.lines() {
            assert_eq!(
                line.matches('"').count() % 2,
                0,
                "unbalanced quotes in {line}"
            );
        }
    }

    /// A capped graph notes the omission in a comment, and an empty graph is still valid text.
    #[test]
    fn caps_and_empty_graphs() {
        let nodes: Vec<GraphNode> = (0..10).map(|i| node(&i.to_string(), "")).collect();
        let text = render(
            &prepare(
                &Graph {
                    nodes,
                    edges: vec![],
                },
                4,
                10,
            ),
            Lang::En,
        );
        assert!(text.contains("%% +6 more nodes not shown"));
        let empty = render(&prepare(&Graph::default(), 4, 10), Lang::En);
        assert_eq!(
            empty,
            "flowchart LR\n    %% pn-ultramemory code graph: 0 nodes, 0 edges\n"
        );
    }
}
