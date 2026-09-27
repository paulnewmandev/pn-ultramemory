// SPDX-License-Identifier: Apache-2.0
//! End-to-end checks of `outline`: that it describes a whole file, that it never drops a symbol
//! whatever the budget, that the detail falls in the documented order, and that it refuses a path
//! it does not have, against the real SQLite and tree-sitter adapters.

// Test helpers outside a `#[test]` function are not covered by the `allow-*-in-tests` settings in
// `clippy.toml`, and a failed setup step in a test should stop that test loudly.
#![allow(
    clippy::expect_used,
    reason = "a failed setup step should stop the test loudly"
)]

mod common;

use std::fmt::Write as _;

use common::sample_repo;
use pn_ultramemory_engine::{EngineError, OutlineDetail};

/// An outline lists every symbol the index has for that file, in the order they appear.
#[test]
fn an_outline_describes_the_whole_file() {
    let fixture = sample_repo();
    let outline = fixture
        .engine
        .outline("src/config.rs", None)
        .expect("outline the file");

    let stored = fixture
        .engine
        .symbols_in("src/config.rs")
        .expect("the symbols of the file");
    assert_eq!(
        outline.symbols.len(),
        stored.len(),
        "the outline must hold every symbol the index has"
    );
    assert!(!outline.symbols.is_empty(), "the fixture declares symbols");

    let lines: Vec<u32> = outline.symbols.iter().map(|row| row.line).collect();
    let mut sorted = lines.clone();
    sorted.sort_unstable();
    assert_eq!(lines, sorted, "symbols come in the order they appear");

    assert_eq!(outline.path, "src/config.rs");
    assert_eq!(outline.detail, OutlineDetail::Documented);
    assert!(outline.tokens > 0);
}

/// The cost of the file is always reported beside the cost of the outline, and the two agree with
/// each other: a file the outline does not beat is marked as cheaper to read.
///
/// The fixture's files are a few dozen lines, which is exactly the range where an outline *loses*:
/// its per-symbol columns cost more than the bodies it leaves out. That is not a defect to hide,
/// it is the honest boundary of the command, so the test pins the reporting rather than asserting
/// a saving the tool cannot always deliver.
#[test]
fn the_cost_of_the_file_is_always_reported() {
    let fixture = sample_repo();
    let outline = fixture
        .engine
        .outline("src/config.rs", None)
        .expect("outline the file");
    let whole = outline
        .whole_file_tokens
        .expect("the file is readable and matches the index");
    assert!(whole > 0);

    let value = outline.to_value();
    if outline.cheaper_to_read() {
        assert!(outline.tokens > whole);
        assert_eq!(value["file"]["cheaper_to_read"], true);
        assert!(
            value["file"]["note"]
                .as_str()
                .unwrap_or_default()
                .contains("reading it outright")
        );
    } else {
        assert!(outline.tokens <= whole);
        assert!(value["file"].get("cheaper_to_read").is_none());
        let saving = outline.saving().expect("a saving is reported");
        assert!((0.0..=1.0).contains(&saving), "saving {saving}");
    }
}

/// A file large enough for the bodies to dominate is cheaper as an outline, which is the case the
/// command exists for. Built here rather than taken from the fixture, so the boundary is explicit.
#[test]
fn a_large_file_is_cheaper_as_an_outline() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let mut source = String::from("//! A module with long function bodies.\n\n");
    for n in 0..24 {
        let _ = write!(
            source,
            "/// Step {n} of the pipeline.\npub fn step_{n}(input: u64) -> u64 {{\n"
        );
        for line in 0..18 {
            let _ = writeln!(
                source,
                "    let value_{line} = input.wrapping_mul({}).wrapping_add({line});",
                line + 3
            );
        }
        source.push_str("    input\n}\n\n");
    }
    common::write(dir.path(), "src/big.rs", &source);
    let engine = common::engine_at(dir.path(), pn_ultramemory_engine::EngineConfig::default());
    engine
        .index(&pn_ultramemory_engine::IndexOptions::default())
        .expect("index");

    let outline = engine.outline("src/big.rs", None).expect("outline");
    let whole = outline.whole_file_tokens.expect("readable");
    assert!(
        !outline.cheaper_to_read(),
        "outline {} tokens, whole file {whole}",
        outline.tokens
    );
    assert_eq!(outline.symbols.len(), 24, "every function is listed");
    let saving = outline.saving().expect("a saving is reported");
    assert!(
        saving > 0.5,
        "saving only {saving} on a file of long bodies"
    );
}

/// Whatever the budget, every symbol is still listed: only the detail falls. This is the whole
/// contract of the command, because an outline that silently drops symbols is read as the file not
/// having them.
#[test]
fn a_budget_lowers_the_detail_and_never_drops_a_symbol() {
    let fixture = sample_repo();
    let full = fixture
        .engine
        .outline("src/config.rs", None)
        .expect("outline the file");

    let mut previous = OutlineDetail::Name;
    for budget in [1, 20, 60, 150, 400, 1000, 4000] {
        let outline = fixture
            .engine
            .outline("src/config.rs", Some(budget))
            .expect("outline the file");
        assert_eq!(
            outline.symbols.len(),
            full.symbols.len(),
            "budget {budget} dropped symbols"
        );
        assert!(
            outline.detail >= previous || budget <= 60,
            "budget {budget}: detail went down as the budget went up"
        );
        previous = outline.detail;
        // Either it fits, or it is the cheapest level there is and it says it does not fit.
        assert!(
            outline.within_budget() || outline.detail == OutlineDetail::Name,
            "budget {budget}: {} tokens at {:?}",
            outline.tokens,
            outline.detail
        );
    }
}

/// A budget too small even for bare names still answers, and admits it went over.
#[test]
fn an_impossible_budget_is_honest_about_it() {
    let fixture = sample_repo();
    let outline = fixture
        .engine
        .outline("src/config.rs", Some(1))
        .expect("outline the file");
    assert!(!outline.symbols.is_empty());
    assert_eq!(outline.detail, OutlineDetail::Name);
    assert!(!outline.within_budget());
    let value = outline.to_value();
    assert_eq!(value["file"]["over_budget"], true);
    assert!(
        value["file"]["note"]
            .as_str()
            .unwrap_or_default()
            .contains("every symbol is listed")
    );
}

/// The cheaper levels carry less text, and the name is never dropped.
#[test]
fn the_levels_carry_what_they_promise() {
    let fixture = sample_repo();
    let documented = fixture
        .engine
        .outline("src/config.rs", None)
        .expect("outline");
    let bare = fixture
        .engine
        .outline("src/config.rs", Some(1))
        .expect("outline");

    assert!(bare.tokens < documented.tokens);
    for row in &bare.symbols {
        assert!(!row.name.is_empty(), "a name is never dropped");
        assert!(
            row.signature.is_empty(),
            "the bare level carries no signature"
        );
        assert!(
            row.doc.is_empty(),
            "the bare level carries no documentation"
        );
    }
    assert!(
        documented.symbols.iter().any(|row| !row.doc.is_empty()),
        "the fixture documents some symbols"
    );
    assert!(
        documented
            .symbols
            .iter()
            .any(|row| !row.signature.is_empty()),
        "signatures are carried at the richest level"
    );
}

/// Nesting is reported, so the shape of the file survives having the bodies removed.
#[test]
fn nesting_is_reported() {
    let fixture = sample_repo();
    let outline = fixture
        .engine
        .outline("src/config.rs", None)
        .expect("outline");
    assert!(
        outline.symbols.iter().any(|row| row.depth > 0),
        "the fixture nests a method inside a type"
    );
    assert!(
        outline.symbols.iter().any(|row| row.depth == 0),
        "something sits at the top of the file"
    );
    assert!(outline.symbols.iter().all(|row| row.depth < 32));
}

/// A path the index does not have is refused by name, with the command that would fix it.
#[test]
fn an_unknown_path_is_refused_with_a_way_forward() {
    let fixture = sample_repo();
    let error = fixture
        .engine
        .outline("src/nowhere.rs", None)
        .expect_err("no such file");
    assert!(matches!(error, EngineError::NotFound(_)), "{error:?}");
    let message = error.to_string();
    assert!(message.contains("src/nowhere.rs"), "{message}");
    assert!(message.contains("pn-ultramemory index"), "{message}");
}

/// A leading `./` is accepted, because that is how a shell completes a path.
#[test]
fn a_leading_dot_slash_is_accepted() {
    let fixture = sample_repo();
    let plain = fixture
        .engine
        .outline("src/config.rs", None)
        .expect("outline");
    let dotted = fixture
        .engine
        .outline("./src/config.rs", None)
        .expect("outline");
    assert_eq!(plain.symbols.len(), dotted.symbols.len());
    assert_eq!(plain.path, dotted.path);
}
