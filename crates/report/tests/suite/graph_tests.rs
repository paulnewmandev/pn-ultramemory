// SPDX-License-Identifier: Apache-2.0
//! Tests of the graph export in all four formats.

use pn_ultramemory_report::{Graph, GraphEdge, GraphFormat, GraphNode, Lang, render_graph};

use crate::fixtures::{NASTY, huge, sample};
use crate::html_check::{Mode, check_markup};
use crate::json::{Json, parse};

/// Builds a node.
fn node(id: &str, label: &str, group: &str, weight: u32) -> GraphNode {
    GraphNode {
        id: id.into(),
        label: label.into(),
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

/// The JSON export parses, lists every node and merges duplicate edges.
#[test]
fn json_is_valid_and_merged() {
    let graph = Graph {
        nodes: vec![node("a", "A", "g", 3), node("b", "B", "g", 1)],
        edges: vec![
            edge(0, 1, 2),
            edge(0, 1, 5),
            edge(1, 1, 1),
            edge(0, 9, 1),
            edge(7, 0, 1),
        ],
    };
    let text = render_graph(&graph, GraphFormat::Json, Lang::En);
    let json = parse(&text).expect("valid JSON");
    assert_eq!(json.get("nodes").expect("nodes").items().len(), 2);
    let edges = json.get("edges").expect("edges").items();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].get("weight").and_then(Json::as_f64), Some(7.0));
    assert_eq!(edges[0].get("source").and_then(Json::as_str), Some("a"));
    assert_eq!(edges[0].get("target").and_then(Json::as_str), Some("b"));
}

/// Hostile text round-trips through JSON exactly.
#[test]
fn json_preserves_hostile_text() {
    let graph = Graph {
        nodes: vec![
            node(NASTY, NASTY, NASTY, 1),
            node("\u{2028}\u{2029}\"\\/", "\u{1f}", "", 0),
        ],
        edges: vec![GraphEdge {
            from: 0,
            to: 1,
            weight: 1,
            kind: NASTY.into(),
            confidence: "\n".into(),
        }],
    };
    let json = parse(&render_graph(&graph, GraphFormat::Json, Lang::Es)).expect("valid JSON");
    let nodes = json.get("nodes").expect("nodes").items();
    assert_eq!(nodes[0].get("id").and_then(Json::as_str), Some(NASTY));
    assert_eq!(nodes[0].get("label").and_then(Json::as_str), Some(NASTY));
    assert_eq!(
        nodes[1].get("id").and_then(Json::as_str),
        Some("\u{2028}\u{2029}\"\\/")
    );
    assert_eq!(nodes[1].get("label").and_then(Json::as_str), Some("\u{1f}"));
    let edge = &json.get("edges").expect("edges").items()[0];
    assert_eq!(edge.get("kind").and_then(Json::as_str), Some(NASTY));
    assert_eq!(edge.get("confidence").and_then(Json::as_str), Some("\n"));
}

/// The JSON of a huge graph is complete and valid.
#[test]
fn json_of_a_huge_graph_is_complete() {
    let graph = huge().graph;
    let json = parse(&render_graph(&graph, GraphFormat::Json, Lang::En)).expect("valid JSON");
    assert_eq!(json.get("nodes").expect("nodes").items().len(), 500);
    assert!(json.get("edges").expect("edges").items().len() > 1_000);
}

/// The standalone SVG is well-formed XML, self-contained and accessible.
#[test]
fn svg_is_well_formed() {
    for lang in [Lang::En, Lang::Es] {
        let svg = render_graph(&sample().graph, GraphFormat::Svg, lang);
        check_markup(&svg, Mode::Xml).expect("well-formed SVG");
        // Counted by class rather than by tag: a drawn node is also surrounded by its halos and
        // met by the terminal of every edge arriving at it, and those are circles too.
        assert_eq!(
            svg.matches("class=\"n g").count(),
            4 + 3,
            "4 nodes and 3 legend entries"
        );
        assert!(svg.contains("role=\"img\""));
        assert!(svg.contains("<title>"));
        assert!(svg.contains("prefers-color-scheme:dark"));
        assert!(svg.contains("xmlns=\"http://www.w3.org/2000/svg\""));
    }
}

/// Hostile text in the SVG is escaped and the file stays well-formed.
#[test]
fn svg_escapes_hostile_text() {
    let graph = Graph {
        nodes: vec![node(NASTY, NASTY, NASTY, 5), node("b", "<b>", "g&g", 1)],
        edges: vec![edge(0, 1, 3), edge(1, 0, 2)],
    };
    let svg = render_graph(&graph, GraphFormat::Svg, Lang::En);
    check_markup(&svg, Mode::Xml).expect("well-formed SVG");
    assert!(!svg.contains("<script"));
    assert!(svg.contains("&lt;b&gt;"));
    assert!(svg.contains("g&amp;g"));
}

/// The SVG export caps at 150 nodes by weight and says how many were left out.
#[test]
fn svg_caps_at_150_nodes() {
    let graph = huge().graph;
    let svg = render_graph(&graph, GraphFormat::Svg, Lang::En);
    check_markup(&svg, Mode::Xml).expect("well-formed SVG");
    assert_eq!(
        svg.matches("class=\"n g").count(),
        150 + 12,
        "150 nodes and 12 legend entries"
    );
    assert!(svg.contains("+350 more nodes not shown"));
    assert!(
        render_graph(&graph, GraphFormat::Svg, Lang::Es).contains("+350 nodos más sin mostrar")
    );
    let heaviest = graph.nodes.iter().map(|n| n.weight).max().unwrap_or(0);
    assert!(heaviest > 900);
}

/// The layout of 150 nodes runs well under a second, even in a debug build.
#[test]
fn svg_of_150_nodes_is_fast() {
    let graph = Graph {
        nodes: (0..150)
            .map(|i| {
                node(
                    &format!("n{i}"),
                    &format!("Node {i}"),
                    &format!("g{}", i % 6),
                    1 + u32::try_from(i % 17).unwrap_or(1),
                )
            })
            .collect(),
        edges: (0..150)
            .flat_map(|i| [edge(i, (i + 1) % 150, 2), edge(i, (i * 7 + 3) % 150, 1)])
            .collect(),
    };
    let started = std::time::Instant::now();
    let svg = render_graph(&graph, GraphFormat::Svg, Lang::En);
    let elapsed = started.elapsed();
    println!("150-node svg: {} bytes in {elapsed:?}", svg.len());
    assert!(elapsed.as_millis() < 1_000, "{elapsed:?}");
    assert!(check_markup(&svg, Mode::Xml).is_ok());
}

/// Every export is deterministic.
#[test]
fn exports_are_deterministic() {
    let graph = huge().graph;
    for format in GraphFormat::ALL {
        for lang in [Lang::En, Lang::Es] {
            assert_eq!(
                render_graph(&graph, format, lang),
                render_graph(&graph, format, lang),
                "{format:?}"
            );
        }
    }
}

/// An empty graph exports to valid, minimal documents in every format.
#[test]
fn empty_graph_exports() {
    let graph = Graph::default();
    let json = parse(&render_graph(&graph, GraphFormat::Json, Lang::En)).expect("valid JSON");
    assert!(json.get("nodes").expect("nodes").items().is_empty());
    assert!(render_graph(&graph, GraphFormat::Mermaid, Lang::En).starts_with("flowchart LR"));
    assert!(render_graph(&graph, GraphFormat::Dot, Lang::En).starts_with("digraph"));
    let svg = render_graph(&graph, GraphFormat::Svg, Lang::En);
    check_markup(&svg, Mode::Xml).expect("well-formed SVG");
}

/// Mermaid output: safe ids, subgraphs per group, weight labels, neutralized labels.
#[test]
fn mermaid_output() {
    let graph = Graph {
        nodes: vec![
            node("a", "Alpha \"1\" <b> |x| [y]", "one", 3),
            node("b", "Beta", "two", 1),
            node("c", "Gamma", "two", 1),
        ],
        edges: vec![edge(0, 1, 3), edge(1, 2, 1)],
    };
    let text = render_graph(&graph, GraphFormat::Mermaid, Lang::En);
    assert!(text.starts_with("flowchart LR\n"));
    assert_eq!(text.matches("subgraph ").count(), 2);
    assert!(text.contains("n0 -->|3| n1"));
    assert!(text.contains("n1 --> n2"));
    let label_line = text.lines().find(|l| l.contains("n0[")).expect("node line");
    for forbidden in ['<', '>', '|', '[', ']'] {
        assert_eq!(
            label_line.matches(forbidden).count(),
            usize::from(forbidden == '[' || forbidden == ']'),
            "{forbidden} in {label_line}"
        );
    }
    let single = Graph {
        nodes: vec![node("a", "A", "only", 1)],
        edges: vec![],
    };
    assert!(!render_graph(&single, GraphFormat::Mermaid, Lang::En).contains("subgraph"));
}

/// DOT output: quoted ids and labels, fill by group, penwidth by weight.
#[test]
fn dot_output() {
    let graph = Graph {
        nodes: vec![
            node("a\"b", "Al\\pha", "one", 3),
            node("b", "Beta", "two", 1),
        ],
        edges: vec![edge(0, 1, 8), edge(1, 0, 1)],
    };
    let text = render_graph(&graph, GraphFormat::Dot, Lang::En);
    assert!(text.starts_with("digraph \"pn-ultramemory\" {\n"));
    assert!(text.contains("\"a\\\"b\" [label=\"Al\\\\pha\", fillcolor="));
    assert!(text.contains("penwidth=3.2"));
    assert!(text.trim_end().ends_with('}'));
    let opens = text.matches('{').count();
    assert_eq!(opens, text.matches('}').count());
}

/// Bad endpoints are ignored in every format.
#[test]
fn bad_edges_are_ignored_everywhere() {
    let graph = Graph {
        nodes: vec![node("a", "A", "", 1), node("b", "B", "", 1)],
        edges: vec![
            edge(0, 5, 1),
            edge(5, 0, 1),
            edge(1, 1, 1),
            edge(usize::MAX, usize::MAX, 1),
        ],
    };
    for format in GraphFormat::ALL {
        let text = render_graph(&graph, format, Lang::En);
        assert!(!text.contains("n5"), "{format:?}");
    }
    assert!(!render_graph(&graph, GraphFormat::Mermaid, Lang::En).contains("-->"));
    assert!(!render_graph(&graph, GraphFormat::Dot, Lang::En).contains("->"));
}
