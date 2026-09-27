// SPDX-License-Identifier: Apache-2.0
//! Tests for the refusal ratchet, driven by the fixture files under `xtask/tests/fixtures`.
//!
//! Each fixture is a small Rust file exercising one case, so a failure names the rule that broke
//! rather than a line in a large sample.

use std::path::Path;

use super::{Outcome, collect};
use crate::source::{self, SourceFile};

/// Loads one fixture as if it were a source file of a crate called `fx`.
fn fixture(name: &str) -> SourceFile {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("cannot read fixture {name}: {err}"));
    SourceFile::from_text(format!("crates/fx/src/{name}"), text).expect("the fixture parses")
}

/// Runs the collector over one fixture.
fn analyse(name: &str) -> Outcome {
    let files = vec![fixture(name)];
    let test_only = source::classify_test_only(&files);
    collect(&files, &test_only)
}

/// Returns the findings of one fixture as plain strings.
fn findings(name: &str) -> Vec<String> {
    analyse(name).findings.into_iter().collect()
}

/// A message naming a subcommand is a site with a continuation, and not a finding.
#[test]
fn a_message_with_a_command_is_not_a_finding() {
    let outcome = analyse("refusal_with_command.rs");
    assert_eq!(outcome.total, 1);
    assert_eq!(outcome.with_continuation, 1);
    assert!(outcome.findings.is_empty());
    assert!(outcome.problems.is_empty());
}

/// A message that only explains is a finding, keyed by path and text with no line number.
#[test]
fn a_message_without_a_command_is_a_finding() {
    let outcome = analyse("refusal_without_command.rs");
    assert_eq!(outcome.total, 1);
    assert_eq!(outcome.with_continuation, 0);
    assert_eq!(
        outcome
            .findings
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec!["fx/src/refusal_without_command.rs\tthe capsule exceeds the token budget"]
    );
}

/// Each shape of the closed vocabulary excuses its site.
#[test]
fn each_by_design_shape_excuses_its_site() {
    for name in [
        "refusal_operator_knowledge.rs",
        "refusal_world_action.rs",
        "refusal_human_authority.rs",
    ] {
        let outcome = analyse(name);
        assert_eq!(outcome.total, 1, "{name}");
        assert_eq!(outcome.by_design, 1, "{name}");
        assert!(outcome.findings.is_empty(), "{name}");
        assert!(outcome.problems.is_empty(), "{name}");
    }
}

/// A shape outside the vocabulary is a failure, not a silent suppression.
#[test]
fn an_unknown_shape_fails() {
    let outcome = analyse("refusal_unknown_shape.rs");
    assert_eq!(outcome.problems.len(), 1);
    assert!(outcome.problems[0].contains("unknown refusal shape"));
    assert_eq!(outcome.by_design, 0);
    assert_eq!(
        outcome.findings.len(),
        1,
        "the site is still reported as a finding"
    );
}

/// A marker that excuses nothing fails, for the same reason a stale baseline entry fails.
#[test]
fn a_marker_that_excuses_nothing_fails() {
    let outcome = analyse("refusal_unused_marker.rs");
    assert_eq!(outcome.with_continuation, 1);
    assert!(outcome.findings.is_empty());
    assert_eq!(outcome.problems.len(), 1);
    assert!(outcome.problems[0].contains("excuses nothing"));
}

/// All four carriers are read: the attribute, the error `Display`, the constructor and the field.
#[test]
fn every_carrier_is_read() {
    let outcome = analyse("refusal_carriers.rs");
    let carriers: Vec<&str> = outcome.by_carrier.keys().copied().collect();
    assert_eq!(
        carriers,
        vec![
            "constructor",
            "error Display",
            "error attribute",
            "explanatory field"
        ]
    );
    assert_eq!(outcome.total, 4);
    assert_eq!(outcome.findings.len(), 4);
}

/// Non-ASCII text survives, a message split over source lines becomes one entry, and an embedded
/// tab never creates a second field.
#[test]
fn unicode_and_multiline_messages_are_normalised() {
    let found = findings("refusal_unicode_multiline.rs");
    assert_eq!(
        found.len(),
        2,
        "the multi-line message names a command: {found:?}"
    );
    for entry in &found {
        assert_eq!(
            entry.matches('\t').count(),
            1,
            "one separator only: {entry}"
        );
        assert_eq!(entry.lines().count(), 1);
    }
    assert!(
        found
            .iter()
            .any(|entry| entry.contains("caf\u{e9} \u{2014} dash"))
    );
    assert!(
        found
            .iter()
            .any(|entry| entry.ends_with("two fields are missing"))
    );
}

/// A message that only exists in test code is not a message a user can see.
#[test]
fn test_code_is_skipped() {
    let outcome = analyse("refusal_test_code.rs");
    assert_eq!(outcome.total, 0);
    assert!(outcome.findings.is_empty());
}

/// Every analysed file becomes an anchor, even one with no findings, so that a baseline entry for a
/// still-analysed file counts as fixed rather than as unverifiable.
#[test]
fn every_analysed_file_is_an_anchor() {
    let outcome = analyse("refusal_with_command.rs");
    assert_eq!(outcome.analysed, 1);
    assert!(outcome.anchors.contains("fx/src/refusal_with_command.rs"));
}

/// A run that enumerates no source file fails instead of passing vacuously.
#[test]
fn an_empty_crate_set_fails() {
    let root = std::env::temp_dir().join(format!("xtask-refusal-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("crates")).expect("creates the directory");
    let error = match source::collect_crate_sources(&root) {
        Ok(files) => panic!("an empty tree must fail, got {} files", files.len()),
        Err(error) => error,
    };
    assert!(error.contains("cannot pass"), "{error}");
    let _ = std::fs::remove_dir_all(&root);
}
