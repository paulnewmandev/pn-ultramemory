// SPDX-License-Identifier: Apache-2.0
//! End-to-end checks of `brief`: that a session with no context is told what the repository is,
//! that it fits its budget, and that what survives a tight budget is what cannot be recovered any
//! other way.

// Test helpers outside a `#[test]` function are not covered by the `allow-*-in-tests` settings in
// `clippy.toml`, and a failed setup step in a test should stop that test loudly.
#![allow(
    clippy::expect_used,
    reason = "a failed setup step should stop the test loudly"
)]

mod common;

use common::sample_repo;
use pn_ultramemory_core::{MemoryKind, Provenance};
use pn_ultramemory_engine::RememberInput;

/// The shape of the repository is always there, whatever the budget.
#[test]
fn a_session_is_told_what_the_repository_is() {
    let fixture = sample_repo();
    let brief = fixture.engine.brief("my-project", None).expect("brief");
    let value = brief.to_value();

    assert_eq!(value["brief"]["repo"], "my-project");
    assert!(value["brief"]["files"].as_u64().unwrap_or(0) > 0);
    assert!(value["brief"]["symbols"].as_u64().unwrap_or(0) > 0);
    assert!(
        value["brief"]["languages"]
            .as_str()
            .unwrap_or_default()
            .contains("rust"),
        "the fixture holds Rust: {:?}",
        value["brief"]["languages"]
    );
    // It always says what to ask next, so a session never has to guess the commands.
    assert!(value["next"].as_array().is_some_and(|n| !n.is_empty()));
}

/// An empty name falls back rather than printing a blank heading.
#[test]
fn an_unnamed_repository_still_has_a_name() {
    let fixture = sample_repo();
    let brief = fixture.engine.brief("   ", None).expect("brief");
    assert_eq!(brief.name, "repository");
    assert_eq!(brief.to_value()["brief"]["repo"], "repository");
}

/// Whatever the budget, the brief fits it, and it never becomes empty.
#[test]
fn a_brief_fits_its_budget() {
    let fixture = sample_repo();
    for budget in [20, 60, 120, 250, 500, 1000, 4000] {
        let brief = fixture
            .engine
            .brief("fixture", Some(budget))
            .expect("brief");
        let value = brief.to_value();
        // The shape and the next steps survive every budget: a brief that says nothing is worse
        // than no brief at all.
        assert!(
            value["brief"]["files"].as_u64().is_some(),
            "budget {budget}"
        );
        assert!(value["next"].as_array().is_some(), "budget {budget}");
        if budget >= 120 {
            assert!(
                brief.tokens <= budget,
                "budget {budget}: spent {}",
                brief.tokens
            );
        }
    }
}

/// What someone already decided outlives the structure around it.
///
/// This is the whole ordering decision of the module, so it is pinned here. Structure can be
/// re-derived by reading the code; a decision and its reason cannot, so when the budget will only
/// carry one of the two it carries the decision.
#[test]
fn a_decision_outlives_the_structure_around_it() {
    let fixture = sample_repo();
    for text in [
        "Money is stored as integer minor units, never floating point",
        "The importer reads the ledger in one pass and holds no row in memory",
    ] {
        fixture
            .engine
            .remember(&RememberInput {
                kind: MemoryKind::Decision,
                text: text.to_owned(),
                about: vec!["Config".to_owned()],
                provenance: Provenance::User,
                session: None,
            })
            .expect("remember");
    }

    let full = fixture.engine.brief("fixture", Some(4000)).expect("brief");
    let full_value = full.to_value();
    assert!(
        full_value["known"]
            .as_array()
            .is_some_and(|k| !k.is_empty())
    );
    assert!(full_value["central"].as_array().is_some());

    // Squeeze it until something has to go. The busiest symbols go before the decisions do.
    let mut lost_central_first = false;
    for budget in [150, 200, 260, 320, 400] {
        let value = fixture
            .engine
            .brief("fixture", Some(budget))
            .expect("brief")
            .to_value();
        let has_central = value.get("central").is_some();
        let has_known = value.get("known").is_some();
        if !has_central && has_known {
            lost_central_first = true;
        }
        assert!(
            has_known || !has_central,
            "budget {budget}: the decisions went before the busiest symbols did"
        );
    }
    assert!(
        lost_central_first,
        "no budget in the sweep was tight enough to drop a section"
    );
}

/// A repository with nothing indexed says so, instead of printing an empty shape.
#[test]
fn an_empty_repository_says_what_to_do() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let engine = common::engine_at(dir.path(), pn_ultramemory_engine::EngineConfig::default());
    let value = engine.brief("empty", None).expect("brief").to_value();
    assert_eq!(value["brief"]["files"], 0);
    assert!(
        value["brief"]["note"]
            .as_str()
            .unwrap_or_default()
            .contains("pn-ultramemory index"),
        "it must name the command that fixes it: {:?}",
        value["brief"]["note"]
    );
}
