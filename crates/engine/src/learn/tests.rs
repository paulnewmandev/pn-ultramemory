// SPDX-License-Identifier: Apache-2.0
//! Unit tests for the learning rules that need no storage: the settlement window, the pairing
//! window and the shape of the explanation.
//!
//! The rules that need a store (signals reaching storage, decay over a controlled clock, the
//! effect on ranking) are exercised by the integration tests, which wire the real adapters.

use pn_ultramemory_core::{Detail, MemoryId, SymbolId};

use super::{
    FeedbackTarget, IGNORE_WINDOW_SECS, MAX_IGNORED_PER_SETTLEMENT, Session, UtilityReport,
    ignored_symbols, round4,
};

/// A session that showed `count` symbols at `detail`, `now` seconds ago.
fn shown_session(count: i64, detail: Detail, shown_at: i64) -> Session {
    Session {
        shown: (1..=count).map(|id| (SymbolId(id), detail)).collect(),
        shown_at: Some(shown_at),
        ..Session::default()
    }
}

/// A capsule that is never followed by another teaches nothing, because a session that simply
/// ended says nothing about whether its results were relevant.
#[test]
fn a_capsule_with_no_successor_teaches_nothing() {
    let session = Session::default();
    assert!(ignored_symbols(&session, 100).is_empty());
}

/// Symbols shown at a level richer than the name and then not expanded count as ignored.
#[test]
fn unexpanded_symbols_are_ignored() {
    let session = shown_session(3, Detail::Signature, 0);
    let ignored = ignored_symbols(&session, 60);
    assert_eq!(ignored, vec![SymbolId(1), SymbolId(2), SymbolId(3)]);
}

/// A symbol shown only as a name is not evidence of anything: the agent was never offered enough
/// of it to judge.
#[test]
fn name_level_symbols_are_not_ignored() {
    let session = shown_session(3, Detail::Name, 0);
    assert!(ignored_symbols(&session, 60).is_empty());
}

/// A symbol the agent expanded is not ignored, even though it was shown.
#[test]
fn expanded_symbols_are_not_ignored() {
    let mut session = shown_session(3, Detail::Summary, 0);
    session.expanded.push(SymbolId(2));
    assert_eq!(
        ignored_symbols(&session, 60),
        vec![SymbolId(1), SymbolId(3)]
    );
}

/// Silence for longer than the window says nothing, so nothing is recorded.
#[test]
fn silence_beyond_the_window_teaches_nothing() {
    let session = shown_session(3, Detail::Signature, 0);
    assert!(ignored_symbols(&session, IGNORE_WINDOW_SECS).is_empty());
    assert!(ignored_symbols(&session, IGNORE_WINDOW_SECS + 1).is_empty());
    assert_eq!(ignored_symbols(&session, IGNORE_WINDOW_SECS - 1).len(), 3);
}

/// One settlement never records more than the cap, however large the capsule was.
#[test]
fn settlement_is_capped() {
    let session = shown_session(200, Detail::Outline, 0);
    assert_eq!(
        ignored_symbols(&session, 1).len(),
        MAX_IGNORED_PER_SETTLEMENT
    );
}

/// A symbol listed twice in one capsule is recorded once.
#[test]
fn duplicates_within_a_capsule_count_once() {
    let mut session = shown_session(2, Detail::Signature, 0);
    session.shown.push((SymbolId(1), Detail::Source));
    assert_eq!(
        ignored_symbols(&session, 10),
        vec![SymbolId(1), SymbolId(2)]
    );
}

/// Time running backwards, which a clock change can cause, never produces a settlement.
#[test]
fn a_clock_that_went_backwards_is_harmless() {
    let session = shown_session(3, Detail::Signature, 1_000);
    assert_eq!(ignored_symbols(&session, 900).len(), 3);
}

/// Feedback targets are distinguishable, so a symbol named "3" is never confused with memory 3.
#[test]
fn feedback_targets_are_distinct() {
    assert_ne!(
        FeedbackTarget::Symbol("3".into()),
        FeedbackTarget::Memory(MemoryId(3))
    );
}

/// The explanation rounds to four decimals, so its output is stable to compare and to print.
#[test]
fn the_explanation_is_rounded() {
    assert!((round4(1.234_567_8) - 1.2346).abs() < f64::EPSILON);
    assert!((round4(-0.000_04) - 0.0).abs() < f64::EPSILON);
    let report = UtilityReport {
        target: "symbol 1 a.rs f".into(),
        alpha: 2.000_04,
        beta: 0.5,
        mean: 0.666_666_6,
        multiplier: 1.111_111,
        evidence: 2.500_04,
    };
    let value = report.to_value();
    assert_eq!(value["alpha"], 2.0);
    assert_eq!(value["mean"], 0.6667);
    assert_eq!(value["target"], "symbol 1 a.rs f");
}
