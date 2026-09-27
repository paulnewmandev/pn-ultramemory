// SPDX-License-Identifier: Apache-2.0
//! End-to-end checks of `impact`: each epistemic outcome reached on purpose, the depth and
//! confidence limits, a cycle, truncation, test-file detection and the commands the report hands
//! back, against the real adapters.

// Test helpers outside a `#[test]` function are not covered by the `allow-*-in-tests` settings in
// `clippy.toml`, and a failed setup step in a test should stop that test loudly.
#![allow(
    clippy::expect_used,
    reason = "a failed setup step should stop the test loudly"
)]

mod common;

use common::{Fixture, engine_at, sample_repo, write};
use pn_ultramemory_core::Confidence;
use pn_ultramemory_engine::{
    Engine, EngineConfig, Epistemic, ImpactQuery, ImpactReport, IndexOptions, MAX_IMPACT_DEPTH,
};

/// Builds a repository of the given files, indexes it and returns the fixture.
fn repo_of(files: &[(&str, &str)]) -> Fixture {
    let dir = tempfile::tempdir().expect("temporary directory");
    for (path, content) in files {
        write(dir.path(), path, content);
    }
    let engine = engine_at(dir.path(), EngineConfig::default());
    engine.index(&IndexOptions::default()).expect("index");
    Fixture { dir, engine }
}

/// The report for one symbol at the given depth, with every other setting left at its default.
fn at_depth(engine: &Engine, symbol: &str, depth: u32) -> ImpactReport {
    engine
        .impact(&ImpactQuery {
            symbol: symbol.to_owned(),
            depth: Some(depth),
            ..ImpactQuery::default()
        })
        .expect("impact")
}

/// The qualified names of the callers of a report.
fn names(report: &ImpactReport) -> Vec<&str> {
    report
        .callers
        .iter()
        .map(|caller| caller.name.as_str())
        .collect()
}

/// A private symbol reached only through resolved edges gets the strong claim: the list is whole.
#[test]
fn resolved_edges_into_a_private_symbol_are_exact() {
    let fixture = sample_repo();
    // `read_file` is private and its only caller, `load_config`, is a resolved call.
    let report = at_depth(&fixture.engine, "read_file", 1);
    assert_eq!(report.epistemic, Epistemic::Exact, "{report:?}");
    assert_eq!(names(&report), ["load_config"]);
    assert_eq!((report.direct, report.total), (1, 1));
    assert!(!report.truncated);
    assert_eq!(report.files, ["src/config.rs"]);
    assert!(report.tests.is_empty());

    // The same holds across languages: `Server._bind` is private and called once, resolved.
    let python = at_depth(&fixture.engine, "Server._bind", 2);
    assert_eq!(python.epistemic, Epistemic::Exact, "{python:?}");
    assert_eq!(names(&python), ["Server.start"]);
}

/// A private symbol nothing calls is exact with an empty list: it cannot be reached from outside.
#[test]
fn a_private_symbol_with_no_callers_is_exact() {
    let fixture = sample_repo();
    let report = at_depth(&fixture.engine, "main", 2);
    assert!(report.callers.is_empty(), "{report:?}");
    assert_eq!(report.epistemic, Epistemic::Exact);
    assert_eq!((report.direct, report.total), (0, 0));
    assert!(report.files.is_empty());
}

/// A public symbol nothing calls is unknown, never safe: a caller outside the index may exist.
#[test]
fn a_public_symbol_with_no_callers_is_unknown() {
    let fixture = sample_repo();
    for symbol in ["default_config", "Server.stop", "fetchUser"] {
        let report = at_depth(&fixture.engine, symbol, 2);
        assert!(report.callers.is_empty(), "{symbol}: {report:?}");
        assert_eq!(report.epistemic, Epistemic::Unknown, "{symbol}");
        assert!(
            report.render_text().contains("unknown"),
            "{}",
            report.render_text()
        );
    }
}

/// One heuristic edge anywhere on a path makes the whole answer a lower bound, and so does a public
/// symbol that does have callers.
#[test]
fn a_heuristic_step_or_a_public_symbol_gives_a_lower_bound() {
    let fixture = sample_repo();
    // Depth 2 from `read_file` reaches `main` through a heuristic call into `load_config`.
    let deep = at_depth(&fixture.engine, "read_file", 2);
    assert_eq!(names(&deep), ["load_config", "main"]);
    assert_eq!(deep.epistemic, Epistemic::LowerBound, "{deep:?}");
    let weakest = deep
        .callers
        .iter()
        .find(|caller| caller.name == "main")
        .map(|caller| caller.confidence);
    assert_eq!(
        weakest,
        Some(Confidence::Heuristic),
        "the weakest edge on the path wins: {deep:?}"
    );

    // `parse_config` is public and called once, through a resolved edge.
    let public = at_depth(&fixture.engine, "parse_config", 1);
    assert_eq!(names(&public), ["load_config"]);
    assert_eq!(public.epistemic, Epistemic::LowerBound, "{public:?}");
}

/// Depth bounds the walk, zero is raised to one and anything above the maximum is lowered to it.
#[test]
fn depth_bounds_the_walk() {
    let fixture = sample_repo();
    assert_eq!(at_depth(&fixture.engine, "read_file", 1).total, 1);
    assert_eq!(at_depth(&fixture.engine, "read_file", 2).total, 2);
    assert_eq!(at_depth(&fixture.engine, "read_file", 0).total, 1);
    let deepest = at_depth(&fixture.engine, "read_file", MAX_IMPACT_DEPTH);
    let beyond = at_depth(&fixture.engine, "read_file", 99);
    assert_eq!(deepest.callers, beyond.callers);
    // The default is two steps.
    let default = fixture
        .engine
        .impact(&ImpactQuery {
            symbol: "read_file".into(),
            ..ImpactQuery::default()
        })
        .expect("impact");
    assert_eq!(
        default.callers,
        at_depth(&fixture.engine, "read_file", 2).callers
    );
}

/// A cycle is walked once: every symbol of it appears exactly one time, at its shallowest depth.
#[test]
fn a_cycle_is_visited_once() {
    let fixture = repo_of(&[(
        "src/ring.rs",
        "/// The first step.\npub fn alpha(n: u32) -> u32 { beta(n) }\n\n\
         /// The second step.\npub fn beta(n: u32) -> u32 { gamma(n) }\n\n\
         /// The third step, which closes the ring.\npub fn gamma(n: u32) -> u32 { alpha(n) }\n",
    )]);
    let report = fixture
        .engine
        .impact(&ImpactQuery {
            symbol: "alpha".into(),
            depth: Some(MAX_IMPACT_DEPTH),
            ..ImpactQuery::default()
        })
        .expect("impact around a cycle");
    let mut found = names(&report);
    found.sort_unstable();
    assert_eq!(found, ["beta", "gamma"], "{report:?}");
    assert_eq!(report.total, 2, "alpha is not its own caller");
    let depths: Vec<u32> = report.callers.iter().map(|caller| caller.depth).collect();
    assert_eq!(depths, [1, 2], "{report:?}");
}

/// Raising the confidence floor drops the edges that do not meet it.
#[test]
fn the_confidence_floor_filters_edges() {
    let fixture = sample_repo();
    let query = |confidence: Option<Confidence>| ImpactQuery {
        symbol: "Config".into(),
        depth: Some(1),
        min_confidence: confidence,
        limit: None,
    };
    let heuristic = fixture
        .engine
        .impact(&query(None))
        .expect("heuristic floor");
    assert_eq!(
        names(&heuristic),
        ["load_config", "parse_config", "default_config", "run"],
        "{heuristic:?}"
    );
    let resolved = fixture
        .engine
        .impact(&query(Some(Confidence::Resolved)))
        .expect("resolved floor");
    assert_eq!(
        names(&resolved),
        ["load_config", "parse_config", "default_config"],
        "`run` reaches Config only heuristically: {resolved:?}"
    );
    assert!(
        resolved
            .callers
            .iter()
            .all(|c| c.confidence.is_structural())
    );
    let exact = fixture
        .engine
        .impact(&query(Some(Confidence::Exact)))
        .expect("exact floor");
    assert!(exact.callers.is_empty(), "{exact:?}");
    assert_eq!(exact.epistemic, Epistemic::Unknown, "Config is public");
}

/// The limit cuts the list short, says so, and the report names the command that widens it.
#[test]
fn the_limit_truncates_and_offers_a_wider_command() {
    let fixture = sample_repo();
    let report = fixture
        .engine
        .impact(&ImpactQuery {
            symbol: "Config".into(),
            depth: Some(2),
            min_confidence: None,
            limit: Some(2),
        })
        .expect("impact with a small limit");
    assert!(report.truncated, "{report:?}");
    assert_eq!(report.total, 2);
    assert!(
        report
            .next
            .iter()
            .any(|command| command.contains("--limit 4")),
        "{:?}",
        report.next
    );
    assert_eq!(
        report.epistemic,
        Epistemic::LowerBound,
        "a cut list is never exact"
    );
    let whole = fixture
        .engine
        .impact(&ImpactQuery {
            symbol: "Config".into(),
            depth: Some(2),
            min_confidence: None,
            limit: None,
        })
        .expect("impact without a limit");
    assert!(!whole.truncated);
    assert!(whole.total > report.total, "{whole:?}");

    // A limit of zero refuses to claim anything.
    let none = fixture
        .engine
        .impact(&ImpactQuery {
            symbol: "read_file".into(),
            depth: Some(1),
            min_confidence: None,
            limit: Some(0),
        })
        .expect("impact with no room");
    assert!(none.truncated && none.callers.is_empty());
    assert_eq!(none.epistemic, Epistemic::LowerBound);
}

/// Test files are listed apart from the rest, by directory and by the shape of the file name.
#[test]
fn test_files_are_listed_apart() {
    let fixture = repo_of(&[
        (
            "src/parser.rs",
            "/// Parses the input.\npub fn parse_input(text: &str) -> usize { text.len() }\n",
        ),
        (
            "tests/parser_suite.rs",
            "/// Checks the parser.\npub fn covers_the_parser() -> bool { parse_input(\"x\") == 1 }\n",
        ),
        (
            "src/parser_test.rs",
            "/// Checks the parser again.\npub fn also_covers() -> bool { parse_input(\"y\") == 1 }\n",
        ),
        (
            "app/use_parser.rs",
            "/// Uses the parser for real.\npub fn run_it() -> usize { parse_input(\"z\") }\n",
        ),
    ]);
    let report = at_depth(&fixture.engine, "parse_input", 1);
    assert_eq!(
        report.files,
        [
            "app/use_parser.rs",
            "src/parser_test.rs",
            "tests/parser_suite.rs"
        ],
        "{report:?}"
    );
    assert_eq!(
        report.tests,
        ["src/parser_test.rs", "tests/parser_suite.rs"],
        "{report:?}"
    );
}

/// Alternatives point at the other symbols of the same name, so the wrong one is easy to spot.
#[test]
fn alternatives_list_the_other_symbols_of_the_same_name() {
    let fixture = repo_of(&[
        (
            "src/a.rs",
            "/// Encodes for a.\npub fn encode(v: u32) -> u32 { v }\n/// Uses a's encode.\npub fn use_a() -> u32 { encode(1) }\n",
        ),
        (
            "src/b.rs",
            "/// Encodes for b.\npub fn encode(v: u32) -> u32 { v + 1 }\n",
        ),
        (
            "src/c.rs",
            "/// Encodes for c.\npub fn encode(v: u32) -> u32 { v + 2 }\n",
        ),
    ]);
    let report = fixture
        .engine
        .impact(&ImpactQuery {
            symbol: "src/a.rs:encode".into(),
            ..ImpactQuery::default()
        })
        .expect("impact on one of three");
    assert_eq!(report.alternatives.len(), 2, "{:?}", report.alternatives);
    assert!(
        report
            .alternatives
            .iter()
            .all(|line| line.split(' ').count() >= 3 && !line.contains("src/a.rs")),
        "each alternative is `id path qualified_name`: {:?}",
        report.alternatives
    );
    assert!(report.alternatives.len() <= 5);
}

/// Every report hands back at most three literal commands, starting with an expansion.
#[test]
fn next_holds_literal_commands() {
    let fixture = sample_repo();
    for symbol in ["read_file", "Config", "default_config", "main"] {
        let report = at_depth(&fixture.engine, symbol, 2);
        assert!(!report.next.is_empty(), "{symbol}");
        assert!(report.next.len() <= 3, "{symbol}: {:?}", report.next);
        assert!(
            report
                .next
                .iter()
                .all(|command| command.starts_with("pn-ultramemory ")),
            "{symbol}: {:?}",
            report.next
        );
        assert!(
            report
                .next
                .first()
                .is_some_and(|command| command.starts_with("pn-ultramemory expand ")),
            "{symbol}: {:?}",
            report.next
        );
        assert!(
            report
                .next
                .iter()
                .any(|command| command.starts_with("pn-ultramemory recall ")),
            "{symbol}: {:?}",
            report.next
        );
    }
    // The expansion names the top caller when there is one, and the symbol itself otherwise.
    let with_caller = at_depth(&fixture.engine, "read_file", 1);
    let top = with_caller
        .callers
        .first()
        .map(|caller| caller.name.clone())
        .expect("a caller");
    assert_eq!(top, "load_config");
    let id = with_caller
        .next
        .first()
        .and_then(|command| command.rsplit(' ').next())
        .expect("an id")
        .to_owned();
    let expanded = fixture
        .engine
        .expand(&id, None)
        .expect("the suggested command names a real symbol");
    assert_eq!(expanded.name, "load_config");
}

/// Both output forms carry the epistemic, the counts and the callers.
#[test]
fn both_output_forms_carry_the_verdict() {
    let fixture = sample_repo();
    let report = at_depth(&fixture.engine, "read_file", 2);
    let value = report.to_value();
    assert_eq!(value["impact"]["symbol"], "read_file");
    assert_eq!(value["impact"]["epistemic"], "lower-bound");
    assert_eq!(value["impact"]["visibility"], "private");
    assert_eq!(value["impact"]["direct"], 1);
    assert_eq!(value["impact"]["total"], 2);
    assert_eq!(value["impact"].get("truncated"), None);
    let callers = value["callers"].as_array().expect("a callers table");
    assert_eq!(callers.len(), 2);
    assert_eq!(callers[0]["name"], "load_config");
    assert_eq!(callers[0]["depth"], 1);
    assert_eq!(callers[0]["confidence"], "resolved");
    assert_eq!(callers[1]["confidence"], "heuristic");
    assert_eq!(value["files"].as_array().map(Vec::len), Some(2));
    assert!(value.get("tests").is_none(), "no test files here");
    assert!(value["next"].as_array().is_some_and(|n| !n.is_empty()));

    let text = report.render_text();
    assert!(
        text.starts_with("read_file (function, src/config.rs:31, private)"),
        "{text}"
    );
    assert!(text.contains("lower-bound: 1 direct, 2 in total"), "{text}");
    assert!(
        text.contains("d1 resolved calls load_config src/config.rs:18"),
        "{text}"
    );
    assert!(!text.ends_with('\n'));
}

/// An unknown name is refused with suggestions rather than answered with an empty report.
#[test]
fn an_unknown_symbol_is_refused() {
    let fixture = sample_repo();
    let error = fixture
        .engine
        .impact(&ImpactQuery {
            symbol: "parse_configuration".into(),
            ..ImpactQuery::default()
        })
        .expect_err("no such symbol");
    let message = error.to_string();
    assert!(message.contains("no symbol named"), "{message}");
    assert!(message.contains("did you mean"), "{message}");
}

/// The same question always gets the same answer.
#[test]
fn impact_is_deterministic() {
    let fixture = sample_repo();
    for symbol in ["Config", "read_file", "Server._bind", "request"] {
        let first = at_depth(&fixture.engine, symbol, 3);
        let second = at_depth(&fixture.engine, symbol, 3);
        assert_eq!(first.callers, second.callers, "{symbol}");
        assert_eq!(first.files, second.files, "{symbol}");
        assert_eq!(first.epistemic, second.epistemic, "{symbol}");
        assert_eq!(first.next, second.next, "{symbol}");
        assert_eq!(first.to_value(), second.to_value(), "{symbol}");
    }
}
