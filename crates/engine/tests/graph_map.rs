// SPDX-License-Identifier: Apache-2.0
//! End-to-end checks of `graph`, `module_graph` and `repo_map` against the real adapters: the node
//! and depth caps, the confidence floor, the shape of the exported edges, determinism, and the
//! promise that a map never exceeds its token budget.

// Test helpers outside a `#[test]` function are not covered by the `allow-*-in-tests` settings in
// `clippy.toml`, and a failed setup step in a test should stop that test loudly.
#![allow(
    clippy::expect_used,
    reason = "a failed setup step should stop the test loudly"
)]

mod common;

use common::generated::generate_repo;
use common::{Fixture, engine_at, sample_repo};
use pn_ultramemory_codec::{Format, estimate_tokens};
use pn_ultramemory_core::Confidence;
use pn_ultramemory_engine::{
    Engine, EngineConfig, GraphQuery, MapQuery, ModuleGraph, RepoMap, SymbolGraph,
};
use pn_ultramemory_toon::Delimiter;

/// A generated repository of `files` files, indexed and ready to query.
fn generated_repo(files: usize, seed: u64) -> Fixture {
    let dir = tempfile::tempdir().expect("temporary directory");
    generate_repo(dir.path(), files, seed);
    let engine = engine_at(dir.path(), EngineConfig::default());
    engine
        .index(&pn_ultramemory_engine::IndexOptions::default())
        .expect("index the generated repository");
    Fixture { dir, engine }
}

/// Fails unless every edge of a symbol graph points at a node that exists and not at itself.
fn check_symbol_edges(graph: &SymbolGraph, what: &str) {
    for edge in &graph.edges {
        assert!(edge.from < graph.nodes.len(), "{what}: {edge:?}");
        assert!(edge.to < graph.nodes.len(), "{what}: {edge:?}");
        assert_ne!(edge.from, edge.to, "{what}: a self-loop survived");
        assert!(edge.weight >= 1, "{what}: {edge:?}");
    }
    for node in &graph.nodes {
        assert!(node.weight >= 1, "{what}: {node:?}");
        assert!(!node.id.is_empty() && !node.label.is_empty(), "{what}");
    }
}

/// Fails unless every edge of a module graph points at a node that exists.
fn check_module_edges(graph: &ModuleGraph, what: &str) {
    for edge in &graph.edges {
        assert!(edge.from < graph.nodes.len(), "{what}: {edge:?}");
        assert!(edge.to < graph.nodes.len(), "{what}: {edge:?}");
        assert_ne!(edge.from, edge.to, "{what}: a module never links to itself");
    }
}

/// Up to this budget every single value is tried by the budget property test.
const DENSE_BUDGET: u32 = 1200;

/// The tokens a map costs as the default format prints it.
fn printed_tokens(map: &RepoMap) -> u32 {
    estimate_tokens(&map.render(Format::Toon, Delimiter::Comma))
}

/// A centred graph holds the centre first and reaches only as far as the depth allows.
#[test]
fn a_centred_graph_stays_within_its_depth() {
    let fixture = sample_repo();
    let centred = |depth: u32| {
        fixture
            .engine
            .graph(&GraphQuery {
                center: Some("read_file".into()),
                depth,
                ..GraphQuery::default()
            })
            .expect("graph around read_file")
    };
    let one = centred(1);
    check_symbol_edges(&one, "depth 1");
    assert_eq!(
        one.nodes.first().map(|node| node.label.as_str()),
        Some("read_file")
    );
    let labels: Vec<&str> = one.nodes.iter().map(|node| node.label.as_str()).collect();
    assert_eq!(labels, ["read_file", "load_config"], "{one:?}");
    assert_eq!(one.edges.len(), 1, "{one:?}");
    assert_eq!(
        one.edges.first().map(|edge| (edge.from, edge.to)),
        Some((1, 0))
    );

    let two = centred(2);
    check_symbol_edges(&two, "depth 2");
    assert!(two.nodes.len() > one.nodes.len(), "{two:?}");
    let deeper: Vec<&str> = two.nodes.iter().map(|node| node.label.as_str()).collect();
    assert!(deeper.contains(&"parse_config"), "{deeper:?}");
    assert!(deeper.contains(&"main"), "{deeper:?}");
    // Groups are the directory of the file.
    assert!(
        two.nodes.iter().all(|node| node.group == "src"),
        "{:?}",
        two.nodes
    );
    assert!(two.nodes.iter().all(|node| node.id.starts_with('s')));
}

/// Without a centre the graph is the most central symbols of the repository.
#[test]
fn an_uncentred_graph_shows_the_central_symbols() {
    let fixture = sample_repo();
    let graph = fixture
        .engine
        .graph(&GraphQuery::default())
        .expect("uncentred graph");
    check_symbol_edges(&graph, "uncentred");
    assert_eq!(
        graph.nodes.first().map(|node| node.label.as_str()),
        Some("Config"),
        "Config is the most referenced symbol: {graph:?}"
    );
    assert!(graph.nodes.len() >= 8, "{graph:?}");
    assert!(!graph.edges.is_empty());
    // Every edge joins two symbols that are both in the graph.
    let ids: Vec<&str> = graph.nodes.iter().map(|node| node.id.as_str()).collect();
    assert_eq!(ids.len(), graph.nodes.len());
}

/// Both caps hold: the node count never passes `max_nodes`, and zero gives an empty graph.
#[test]
fn the_node_cap_holds() {
    let fixture = generated_repo(40, 11);
    for max_nodes in [0, 1, 2, 5, 17, 60, 500] {
        for center in [None, Some("1".to_owned())] {
            let query = GraphQuery {
                center: center.clone(),
                depth: 3,
                max_nodes,
                ..GraphQuery::default()
            };
            let Ok(graph) = fixture.engine.graph(&query) else {
                continue;
            };
            assert!(graph.nodes.len() <= max_nodes, "{max_nodes}: {graph:?}");
            check_symbol_edges(&graph, &format!("max_nodes {max_nodes}"));
        }
    }
    let empty = fixture
        .engine
        .graph(&GraphQuery {
            max_nodes: 0,
            ..GraphQuery::default()
        })
        .expect("an empty graph");
    assert_eq!(empty, SymbolGraph::default());
}

/// Raising the confidence floor removes the edges that do not meet it.
#[test]
fn the_confidence_floor_removes_edges() {
    let fixture = sample_repo();
    let at = |confidence: Confidence| {
        fixture
            .engine
            .graph(&GraphQuery {
                center: Some("Config".into()),
                depth: 1,
                min_confidence: confidence,
                ..GraphQuery::default()
            })
            .expect("graph around Config")
    };
    let heuristic = at(Confidence::Heuristic);
    let resolved = at(Confidence::Resolved);
    check_symbol_edges(&heuristic, "heuristic");
    check_symbol_edges(&resolved, "resolved");
    assert!(
        heuristic.nodes.len() > resolved.nodes.len(),
        "`run` reaches Config only heuristically: {heuristic:?} then {resolved:?}"
    );
    assert!(
        heuristic
            .edges
            .iter()
            .any(|edge| edge.confidence == Confidence::Heuristic)
    );
    assert!(
        resolved
            .edges
            .iter()
            .all(|edge| edge.confidence.is_structural())
    );
    let exact = at(Confidence::Exact);
    assert_eq!(exact.nodes.len(), 1, "only the centre is left: {exact:?}");
    assert!(exact.edges.is_empty());
}

/// A symbol with no relationships is still a node, with weight one and no edges.
#[test]
fn a_node_with_no_edges_is_still_a_node() {
    let fixture = sample_repo();
    let graph = fixture
        .engine
        .graph(&GraphQuery {
            center: Some("Server.__init__".into()),
            ..GraphQuery::default()
        })
        .expect("graph around a lonely symbol");
    assert_eq!(graph.nodes.len(), 1, "{graph:?}");
    assert!(graph.edges.is_empty());
    let node = graph.nodes.first().expect("the one node");
    assert_eq!(node.label, "Server.__init__");
    assert_eq!(node.group, "app");
    assert_eq!(node.weight, 1, "a weight is never zero");
    let value = graph.to_value();
    assert_eq!(value["nodes"].as_array().map(Vec::len), Some(1));
    assert_eq!(value["edges"].as_array().map(Vec::len), Some(0));
}

/// The module graph mirrors the directories, and its edges stay inside the node list.
#[test]
fn the_module_graph_joins_directories() {
    let fixture = sample_repo();
    let graph = fixture.engine.module_graph(1, 40).expect("module graph");
    check_module_edges(&graph, "sample repo");
    let names: Vec<&str> = graph.nodes.iter().map(|node| node.id.as_str()).collect();
    assert!(names.contains(&"src"), "{names:?}");
    assert!(names.contains(&"app"), "{names:?}");
    assert!(names.contains(&"web"), "{names:?}");
    assert!(
        graph
            .nodes
            .iter()
            .all(|node| node.id == node.label && node.weight >= 1)
    );
    assert!(
        graph
            .edges
            .iter()
            .all(|edge| edge.kind.as_str() == "uses" && edge.weight >= 1),
        "{:?}",
        graph.edges
    );
    assert_eq!(
        fixture.engine.module_graph(1, 0).expect("no nodes"),
        ModuleGraph::default()
    );
    let deep = fixture.engine.module_graph(2, 40).expect("deeper modules");
    check_module_edges(&deep, "depth 2");
    let value = deep.to_value();
    assert!(value["nodes"].as_array().is_some_and(|n| !n.is_empty()));
}

/// Building the same graph twice gives the same value, on both repositories.
#[test]
fn graphs_are_deterministic() {
    let generated = generated_repo(30, 5);
    let sample = sample_repo();
    for engine in [&sample.engine, &generated.engine] {
        for center in [None, Some("1".to_owned())] {
            for depth in [1_u32, 3] {
                let query = GraphQuery {
                    center: center.clone(),
                    depth,
                    max_nodes: 25,
                    min_confidence: Confidence::Heuristic,
                };
                let Ok(first) = engine.graph(&query) else {
                    continue;
                };
                let second = engine.graph(&query).expect("the second build");
                assert_eq!(first, second, "{center:?} at depth {depth}");
                check_symbol_edges(&first, "determinism");
            }
        }
        for depth in [1_usize, 2, 3] {
            let first = engine.module_graph(depth, 30).expect("first module graph");
            let second = engine.module_graph(depth, 30).expect("second module graph");
            assert_eq!(first, second, "module depth {depth}");
            check_module_edges(&first, "determinism");
        }
    }
}

/// Over a wide range of centres and caps, no edge ever points outside the node list.
#[test]
fn no_edge_index_is_ever_out_of_range() {
    let fixture = generated_repo(40, 23);
    let central = fixture
        .engine
        .graph(&GraphQuery {
            max_nodes: 30,
            ..GraphQuery::default()
        })
        .expect("central symbols");
    let mut checked = 0_u32;
    for node in &central.nodes {
        let id = node.id.strip_prefix('s').unwrap_or(&node.id).to_owned();
        for depth in [1_u32, 2, 4] {
            for max_nodes in [1_usize, 4, 30] {
                let graph = fixture
                    .engine
                    .graph(&GraphQuery {
                        center: Some(id.clone()),
                        depth,
                        max_nodes,
                        min_confidence: Confidence::Guess,
                    })
                    .expect("a centred graph");
                check_symbol_edges(&graph, &node.id);
                checked += 1;
            }
        }
    }
    assert!(checked >= 30, "only {checked} graphs were checked");
}

/// A map never exceeds its budget, over the budgets 200 to 8000 on two repositories.
///
/// On the fixture every single budget up to [`DENSE_BUDGET`] is tried, because that is where the
/// frame of the map is a large enough share of the budget for the guarantee to be tight; the rest of
/// the range, and the larger generated tree, are swept in steps.
#[test]
fn a_map_never_exceeds_its_budget() {
    let sample = sample_repo();
    let generated = generated_repo(30, 17);
    let mut checked = 0_u32;
    for (engine, dense_to, step) in [
        (&sample.engine, DENSE_BUDGET, 23_u32),
        (&generated.engine, 0, 31),
    ] {
        let mut budget = 200_u32;
        while budget <= 8000 {
            let map = engine
                .repo_map(&MapQuery {
                    budget: Some(budget),
                    path_prefix: None,
                })
                .expect("a map");
            assert_eq!(map.budget, budget);
            assert!(map.used <= budget, "budget {budget}: used {}", map.used);
            assert!(
                printed_tokens(&map) <= budget,
                "budget {budget}: printed {} tokens",
                printed_tokens(&map)
            );
            checked += 1;
            budget += if budget < dense_to { 1 } else { step };
        }
    }
    assert!(checked > 1200, "only {checked} budgets were checked");
}

/// More budget buys more of the repository, and never less.
#[test]
fn a_bigger_budget_shows_more() {
    let fixture = generated_repo(60, 3);
    let map_at = |budget: u32| {
        fixture
            .engine
            .repo_map(&MapQuery {
                budget: Some(budget),
                path_prefix: None,
            })
            .expect("a map")
    };
    let small = map_at(250);
    let medium = map_at(1500);
    let large = map_at(8000);
    assert!(small.files.len() < large.files.len(), "{small:?}");
    assert!(medium.files.len() <= large.files.len());
    assert!(small.omitted_files > large.omitted_files);
    assert!(small.used <= small.budget && large.used <= large.budget);
    // Every file of the repository is either listed or counted as omitted.
    let listed = u32::try_from(large.files.len()).expect("a count");
    assert_eq!(listed + large.omitted_files, 60);
    assert_eq!(
        large.omitted_files, 0,
        "8000 tokens fit 60 files: {large:?}"
    );
}

/// The path prefix narrows the map to one part of the tree.
#[test]
fn the_path_prefix_narrows_the_map() {
    let fixture = sample_repo();
    let whole = fixture
        .engine
        .repo_map(&MapQuery {
            budget: Some(4000),
            path_prefix: None,
        })
        .expect("the whole map");
    assert_eq!(whole.files.len(), 4, "{whole:?}");
    for prefix in ["src", "./src", " src "] {
        let narrowed = fixture
            .engine
            .repo_map(&MapQuery {
                budget: Some(4000),
                path_prefix: Some(prefix.to_owned()),
            })
            .expect("a narrowed map");
        assert_eq!(narrowed.files.len(), 2, "{prefix}: {narrowed:?}");
        assert!(
            narrowed
                .files
                .iter()
                .all(|file| file.path.starts_with("src/")),
            "{prefix}: {narrowed:?}"
        );
        assert_eq!(narrowed.omitted_files, 0, "{prefix}");
    }
    let nothing = fixture
        .engine
        .repo_map(&MapQuery {
            budget: Some(4000),
            path_prefix: Some("does/not/exist".to_owned()),
        })
        .expect("an empty map");
    assert!(nothing.files.is_empty());
    assert_eq!(nothing.omitted_files, 0);
    assert!(nothing.used <= nothing.budget);
}

/// Breadth comes before depth: while files are still missing, nothing is spent on signatures.
#[test]
fn breadth_is_bought_before_depth() {
    let fixture = generated_repo(60, 17);
    let mut signature_rows = Vec::new();
    for budget in [200_u32, 400, 800, 1500, 3000, 8000] {
        let map = fixture
            .engine
            .repo_map(&MapQuery {
                budget: Some(budget),
                path_prefix: None,
            })
            .expect("a map");
        let rows: usize = map.files.iter().map(|file| file.signatures.len()).sum();
        if map.omitted_files > 0 {
            assert_eq!(
                rows, 0,
                "budget {budget} still omits {} files: {map:?}",
                map.omitted_files
            );
        }
        signature_rows.push(rows);
    }
    assert!(
        signature_rows.windows(2).all(|pair| pair[0] <= pair[1]),
        "detail never shrinks as the budget grows: {signature_rows:?}"
    );
    assert!(
        signature_rows.last().is_some_and(|rows| *rows > 0),
        "a generous budget buys signatures: {signature_rows:?}"
    );
}

/// A generous budget buys names and signatures, within their caps, best file first.
#[test]
fn detail_is_bought_within_its_caps() {
    let fixture = sample_repo();
    let map_at = |budget: u32| {
        fixture
            .engine
            .repo_map(&MapQuery {
                budget: Some(budget),
                path_prefix: None,
            })
            .expect("a map")
    };
    let tight = map_at(120);
    let generous = map_at(8000);
    assert!(
        generous
            .files
            .iter()
            .any(|file| !file.signatures.is_empty()),
        "{generous:?}"
    );
    assert!(
        generous.files.iter().all(|file| file.top.len() <= 3),
        "{generous:?}"
    );
    assert!(
        generous.files.iter().all(|file| file.signatures.len() <= 8),
        "{generous:?}"
    );
    assert!(
        tight.files.iter().all(|file| file.signatures.is_empty()),
        "a tight budget spends nothing on signatures: {tight:?}"
    );
    assert!(tight.used <= tight.budget && generous.used <= generous.budget);
    // The best file comes first, and it is the one whose symbols are referenced most.
    assert_eq!(
        generous.files.first().map(|file| file.path.as_str()),
        Some("src/config.rs"),
        "{generous:?}"
    );
    // The signature table only appears when something fills it.
    let tight_value = tight.to_value();
    assert!(tight_value.get("signatures").is_none(), "{tight_value}");
    let generous_value = generous.to_value();
    let rows = generous_value["signatures"]
        .as_array()
        .expect("a signature table");
    assert!(!rows.is_empty());
    assert!(
        rows.iter()
            .all(|row| row["f"].as_u64().is_some_and(|f| f < 4)),
        "every signature points at a listed file: {generous_value}"
    );
    assert_eq!(generous_value["map"]["budget"], 8000);
    assert_eq!(generous_value["map"]["used"], u64::from(generous.used));
}

/// Each output format prints the map, and the tables carry what the columns promise.
#[test]
fn every_format_prints_the_map() {
    let fixture = sample_repo();
    let map = fixture
        .engine
        .repo_map(&MapQuery {
            budget: Some(3000),
            path_prefix: None,
        })
        .expect("a map");
    let toon = map.render(Format::Toon, Delimiter::Comma);
    assert!(toon.starts_with("map:\n  budget: 3000"), "{toon}");
    assert!(
        toon.contains("files[4]{path,lang,lines,symbols,top}:"),
        "{toon}"
    );
    assert!(toon.contains("src/config.rs,rust,33,6,"), "{toon}");
    for format in [Format::Toon, Format::Json, Format::Text] {
        for delimiter in [Delimiter::Comma, Delimiter::Tab, Delimiter::Pipe] {
            let printed = map.render(format, delimiter);
            assert!(!printed.is_empty(), "{format:?}");
            assert!(!printed.ends_with('\n'), "{format:?}");
            assert!(printed.contains("src/config.rs"), "{format:?}");
        }
    }
    let json = map.render(Format::Json, Delimiter::Comma);
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid json");
    assert_eq!(parsed, map.to_value());
}

/// The same repository and budget always give the same map.
#[test]
fn maps_are_deterministic() {
    let generated = generated_repo(40, 29);
    let sample = sample_repo();
    for engine in [&sample.engine, &generated.engine] {
        for budget in [200_u32, 900, 5000] {
            let query = MapQuery {
                budget: Some(budget),
                path_prefix: None,
            };
            let first = engine.repo_map(&query).expect("first map");
            let second = engine.repo_map(&query).expect("second map");
            assert_eq!(first, second, "budget {budget}");
        }
    }
}

/// An empty index gives an empty map rather than a failure.
#[test]
fn an_empty_index_gives_an_empty_map() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let engine: Engine = engine_at(dir.path(), EngineConfig::default());
    let map = engine.repo_map(&MapQuery::default()).expect("a map");
    assert!(map.files.is_empty());
    assert_eq!(map.omitted_files, 0);
    assert_eq!(map.budget, EngineConfig::default().default_budget);
    assert!(map.used <= map.budget);
    let graph = engine.graph(&GraphQuery::default()).expect("a graph");
    assert_eq!(graph, SymbolGraph::default());
    assert_eq!(
        engine.module_graph(2, 20).expect("a module graph"),
        ModuleGraph::default()
    );
}
