// SPDX-License-Identifier: Apache-2.0
//! A question asked in Spanish finds code written in English.
//!
//! The sample repository is written in English (`load_config`, `Server.stop`, `fetchUser`) and every
//! question here is Spanish, with its accents, its question marks and its function words. Each one
//! shares no word with the code it is about, so only the glossary can connect them.

// Test helpers outside a `#[test]` function are not covered by the `allow-*-in-tests` settings in
// `clippy.toml`, and a failed setup step in a test should stop that test loudly.
#![allow(
    clippy::expect_used,
    reason = "a failed setup step should stop the test loudly"
)]

mod common;

use common::{Fixture, sample_repo};
use pn_ultramemory_engine::RecallQuery;

/// The qualified names a question brings back, best first.
fn names(fixture: &Fixture, question: &str) -> Vec<String> {
    fixture
        .engine
        .recall(&RecallQuery {
            text: question.into(),
            budget: Some(800),
            ..RecallQuery::default()
        })
        .expect("recall")
        .symbols
        .into_iter()
        .map(|symbol| symbol.name)
        .collect()
}

/// Whether the first `top` names hold one that ends with `name`.
fn finds(names: &[String], name: &str, top: usize) -> bool {
    names.iter().take(top).any(|found| found.ends_with(name))
}

/// Spanish questions reach the English code they are about, near the top.
#[test]
fn a_spanish_question_finds_english_code() {
    let fixture = sample_repo();
    for (question, expected) in [
        ("¿Dónde se carga la configuración?", "load_config"),
        ("detener el servidor", "stop"),
        ("obtener un usuario", "fetchUser"),
    ] {
        let found = names(&fixture, question);
        assert!(
            finds(&found, expected, 3),
            "{question:?} should find {expected} near the top, found {found:?}"
        );
    }
}

/// An English question is not translated, and still finds what it found before.
#[test]
fn an_english_question_is_unchanged() {
    let fixture = sample_repo();
    let found = names(&fixture, "where is the configuration loaded");
    assert!(finds(&found, "load_config", 3), "found {found:?}");
}
