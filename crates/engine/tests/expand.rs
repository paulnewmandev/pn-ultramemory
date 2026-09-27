// SPDX-License-Identifier: Apache-2.0
//! End-to-end checks of `expand`: whole symbols, explicit windows, the boundaries of a window,
//! paging a symbol too large for one reply, and the three refusals, against the real adapters.

// Test helpers outside a `#[test]` function are not covered by the `allow-*-in-tests` settings in
// `clippy.toml`, and a failed setup step in a test should stop that test loudly.
#![allow(
    clippy::expect_used,
    reason = "a failed setup step should stop the test loudly"
)]

mod common;

use std::fmt::Write as _;
use std::path::Path;

use common::{Fixture, engine_at, sample_repo, write};
use pn_ultramemory_codec::estimate_tokens;
use pn_ultramemory_engine::{
    EngineConfig, EngineError, Expansion, FeedbackTarget, IndexOptions, MAX_WINDOW_LINES,
    MAX_WINDOW_TOKENS,
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

/// The lines of a file that a symbol's own declaration covers, read straight from disk, without the
/// line terminator that closes the last one: a declaration slice ends at its last character.
fn declaration_of(root: &Path, window: &Expansion) -> String {
    let text = std::fs::read_to_string(root.join(&window.path)).expect("read the file");
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let first = usize::try_from(window.start_line).expect("start line") - 1;
    let last = usize::try_from(window.end_line).expect("end line");
    let block = lines
        .get(first..last)
        .expect("the declaration is inside the file")
        .concat();
    block
        .strip_suffix('\n')
        .map_or(block.clone(), str::to_owned)
}

/// A Rust file holding one function whose body is `body_lines` lines of the given shape.
fn long_function(name: &str, body_lines: usize, line: &str) -> String {
    let mut source = format!(
        "/// A function with a long body.\npub fn {name}(input: u32) -> u32 {{\n    let mut total = input;\n"
    );
    for index in 0..body_lines {
        let _ = writeln!(source, "{}", line.replace("NNNN", &format!("{index:04}")));
    }
    source.push_str("    total\n}\n");
    source
}

/// With no window the whole declaration comes back, documentation comment included.
#[test]
fn a_whole_symbol_comes_back_by_default() {
    let fixture = sample_repo();
    let window = fixture
        .engine
        .expand("parse_config", None)
        .expect("expand parse_config");
    assert_eq!(window.from, 1);
    assert_eq!(window.to, window.total_lines);
    assert_eq!(window.total_lines, 4);
    assert!(!window.has_more);
    assert_eq!(window.path, "src/config.rs");
    assert_eq!(window.start_line, 22);
    assert_eq!(window.end_line, 25);
    assert!(
        window.text.starts_with("/// Parses configuration text"),
        "{window:?}"
    );
    assert!(
        window
            .text
            .contains("pub fn parse_config(text: &str) -> Config")
    );
    assert_eq!(window.text, declaration_of(fixture.dir.path(), &window));
}

/// An explicit window returns exactly those lines of the symbol, counted from its first line.
#[test]
fn an_explicit_window_returns_those_lines() {
    let fixture = sample_repo();
    let window = fixture
        .engine
        .expand("parse_config", Some((2, Some(3))))
        .expect("expand a window");
    assert_eq!((window.from, window.to, window.total_lines), (2, 3, 4));
    assert!(window.has_more, "line 4 follows");
    assert!(window.text.starts_with("pub fn parse_config"), "{window:?}");
    assert!(window.text.contains("Config { port:"));
    assert!(!window.text.contains("/// Parses"));
}

/// The end of a window is clamped to the symbol, and the last line closes it.
#[test]
fn the_end_of_a_window_is_clamped() {
    let fixture = sample_repo();
    let clamped = fixture
        .engine
        .expand("parse_config", Some((3, Some(9_999))))
        .expect("expand past the end");
    assert_eq!((clamped.from, clamped.to), (3, 4));
    assert!(!clamped.has_more);
    let exact = fixture
        .engine
        .expand("parse_config", Some((3, Some(4))))
        .expect("expand to the last line");
    assert_eq!(clamped.text, exact.text);
    // A `from` of zero is raised to one rather than refused: lines are 1-based.
    let raised = fixture
        .engine
        .expand("parse_config", Some((0, Some(1))))
        .expect("expand from zero");
    assert_eq!((raised.from, raised.to), (1, 1));
}

/// A window of one line returns that line alone.
#[test]
fn from_equal_to_returns_one_line() {
    let fixture = sample_repo();
    for line in 1..=4 {
        let window = fixture
            .engine
            .expand("parse_config", Some((line, Some(line))))
            .expect("expand one line");
        assert_eq!((window.from, window.to), (line, line));
        assert_eq!(window.text.matches('\n').count(), usize::from(line < 4));
        assert_eq!(window.has_more, line < 4);
    }
}

/// The windows of a symbol tile its source: paging until nothing follows reproduces it exactly, and
/// no window breaks either cap.
#[test]
fn paging_reproduces_the_whole_source() {
    let cheap = long_function("walk_everything", 1000, "    total += NNNN;");
    let dear = long_function(
        "weigh_everything",
        600,
        "    total = total.wrapping_add(compute_weight(alpha, beta, NNNN)) ^ 0xNNNN;",
    );
    let fixture = repo_of(&[("src/big.rs", &cheap), ("src/dear.rs", &dear)]);

    for (name, expect_line_cap) in [("walk_everything", true), ("weigh_everything", false)] {
        let mut joined = String::new();
        let mut from = 1_u32;
        let mut pages = 0_u32;
        let mut whole = String::new();
        loop {
            let window = fixture
                .engine
                .expand(name, Some((from, None)))
                .expect("expand a page");
            assert!(window.to >= window.from);
            let lines = window.to - window.from + 1;
            assert!(lines <= MAX_WINDOW_LINES, "{name}: {lines} lines");
            assert!(
                lines == 1 || estimate_tokens(&window.text) <= MAX_WINDOW_TOKENS,
                "{name}: {} tokens",
                estimate_tokens(&window.text)
            );
            if expect_line_cap {
                assert_eq!(
                    lines == MAX_WINDOW_LINES,
                    window.has_more,
                    "{name}: the line cap bites first for cheap lines"
                );
            } else if window.has_more {
                assert!(
                    lines < MAX_WINDOW_LINES,
                    "{name}: the token cap bites first"
                );
            }
            joined.push_str(&window.text);
            pages += 1;
            if whole.is_empty() {
                whole = declaration_of(fixture.dir.path(), &window);
            }
            if !window.has_more {
                break;
            }
            from = window.to + 1;
        }
        assert!(pages > 1, "{name} needs more than one page");
        assert_eq!(joined, whole, "{name}");
    }
}

/// A symbol whose declaration is one line comes back in one window.
#[test]
fn a_one_line_symbol_is_one_window() {
    let fixture = repo_of(&[("src/tiny.rs", "pub fn tiny() -> u32 { 7 }\n")]);
    let window = fixture.engine.expand("tiny", None).expect("expand tiny");
    assert_eq!(
        (window.from, window.to, window.total_lines),
        (1, 1, 1),
        "{window:?}"
    );
    assert!(!window.has_more);
    assert_eq!(window.text.trim_end(), "pub fn tiny() -> u32 { 7 }");
    assert_eq!(window.start_line, window.end_line);
}

/// A `from` past the end of the symbol is refused, and the refusal names a command to run.
#[test]
fn a_window_past_the_end_is_refused() {
    let fixture = sample_repo();
    let error = fixture
        .engine
        .expand("parse_config", Some((99, None)))
        .expect_err("line 99 does not exist");
    let message = error.to_string();
    assert!(matches!(error, EngineError::Invalid(_)), "{error:?}");
    assert!(message.contains("no line 99"), "{message}");
    assert!(message.contains("pn-ultramemory expand"), "{message}");
}

/// A window that ends before it starts is refused, with the corrected command in the message.
#[test]
fn a_backwards_window_is_refused() {
    let fixture = sample_repo();
    let error = fixture
        .engine
        .expand("parse_config", Some((4, Some(2))))
        .expect_err("4-2 is backwards");
    let message = error.to_string();
    assert!(matches!(error, EngineError::Invalid(_)), "{error:?}");
    assert!(message.contains("ends before it starts"), "{message}");
    assert!(
        message.contains("--from 2 --to 4"),
        "the message must name the fix: {message}"
    );
}

/// A file edited after indexing is never sliced: the refusal names the command that fixes it.
#[test]
fn a_changed_file_is_refused_and_names_the_index_command() {
    let fixture = sample_repo();
    assert!(fixture.engine.expand("parse_config", None).is_ok());
    write(
        fixture.dir.path(),
        "src/config.rs",
        "// The file moved on.\npub fn parse_config(text: &str) -> u8 { 0 }\n",
    );
    let error = fixture
        .engine
        .expand("parse_config", None)
        .expect_err("the stored position is stale");
    let message = error.to_string();
    assert!(matches!(error, EngineError::Invalid(_)), "{error:?}");
    assert!(
        message.contains("changed since it was indexed"),
        "{message}"
    );
    assert!(message.contains("run `pn-ultramemory index`"), "{message}");
}

/// A name matching several symbols is refused with the candidates, so the caller can retry with an
/// id, and that id then works.
#[test]
fn an_ambiguous_name_is_refused_with_candidates() {
    let fixture = repo_of(&[
        (
            "src/a.rs",
            "/// Parses an a.\npub fn parse() -> u32 { 1 }\n",
        ),
        ("src/b.rs", "/// Parses a b.\npub fn parse() -> u32 { 2 }\n"),
    ]);
    let error = fixture
        .engine
        .expand("parse", None)
        .expect_err("two symbols are named parse");
    let EngineError::Ambiguous { candidates, .. } = &error else {
        panic!("expected an ambiguity: {error:?}");
    };
    assert_eq!(candidates.len(), 2, "{candidates:?}");
    let first = candidates
        .first()
        .and_then(|line| line.split(' ').next())
        .expect("an id in the candidate line")
        .to_owned();
    let window = fixture
        .engine
        .expand(&first, None)
        .expect("the id resolves on its own");
    assert_eq!(window.name, "parse");
    // Naming the file works too.
    let by_path = fixture
        .engine
        .expand("src/b.rs:parse", None)
        .expect("file and name resolve");
    assert_eq!(by_path.path, "src/b.rs");
    assert!(by_path.text.contains('2'), "{by_path:?}");
}

/// The structured and the readable forms carry the same window, and the header says where it is.
#[test]
fn both_output_forms_describe_the_window() {
    let fixture = sample_repo();
    let window = fixture
        .engine
        .expand("load_config", Some((1, Some(2))))
        .expect("expand load_config");
    let value = window.to_value();
    assert_eq!(value["name"], "load_config");
    assert_eq!(value["path"], "src/config.rs");
    assert_eq!(value["from"], 1);
    assert_eq!(value["to"], 2);
    assert_eq!(value["total_lines"], 5);
    assert_eq!(value["has_more"], true);
    assert_eq!(value["text"], window.text);
    assert_eq!(value["kind"], "function");

    let rendered = window.render_text();
    let header = rendered.lines().next().expect("a header line");
    assert_eq!(
        header,
        "src/config.rs:16-20 load_config (lines 1-2 of 5, more follow)"
    );
    assert!(rendered.contains("pub fn load_config"), "{rendered}");
    assert!(!rendered.ends_with('\n'));

    let whole = fixture
        .engine
        .expand("load_config", None)
        .expect("expand the whole symbol");
    assert!(
        whole
            .render_text()
            .starts_with("src/config.rs:16-20 load_config (lines 1-5 of 5)"),
        "{}",
        whole.render_text()
    );
}

/// Expanding a symbol is the strongest signal that it was useful, so it is recorded for learning.
#[test]
fn expanding_teaches_that_the_symbol_was_used() {
    let fixture = sample_repo();
    let target = FeedbackTarget::Symbol("parse_config".into());
    let before = fixture.engine.utility_of(&target).expect("utility before");
    assert!(
        (before.multiplier - 1.0).abs() < f64::EPSILON,
        "no evidence yet: {before:?}"
    );
    fixture
        .engine
        .expand("parse_config", None)
        .expect("expand once");
    let after = fixture.engine.utility_of(&target).expect("utility after");
    assert!(after.alpha > before.alpha, "{before:?} then {after:?}");
    assert!(after.mean > 0.5, "{after:?}");
}

/// The same request always returns the same window.
#[test]
fn expansion_is_deterministic() {
    let fixture = sample_repo();
    for window in [None, Some((2, None)), Some((1, Some(3)))] {
        let first = fixture.engine.expand("load_config", window).expect("first");
        let second = fixture
            .engine
            .expand("load_config", window)
            .expect("second");
        assert_eq!(first, second, "{window:?}");
    }
}
