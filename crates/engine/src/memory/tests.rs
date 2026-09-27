// SPDX-License-Identifier: Apache-2.0
//! Unit tests for the parts of the memory layer that need no storage: cleaning text, counting
//! corroborations from evidence, and collapsing near-duplicates for display.
//!
//! The rules that need a store (deduplication on write, the independence window, contradictions
//! across stored memories, staleness) are exercised by the integration tests, which wire the real
//! adapters.

use pn_ultramemory_core::{
    Anchor, MemoryId, MemoryKind, MemoryRecord, Provenance, SignalKind, UtilityState,
};

use super::{
    MAX_CORROBORATIONS, corroborations_of, dedupe_for_capsule, memories_to_value, sanitize,
};

/// A memory with the given identity, kind, text and anchor names.
fn memory(id: i64, kind: MemoryKind, text: &str, about: &[&str]) -> MemoryRecord {
    MemoryRecord {
        id: MemoryId(id),
        kind,
        text: text.to_owned(),
        provenance: Provenance::User,
        created_at: 0,
        stale_since: None,
        anchors: about
            .iter()
            .map(|name| Anchor {
                symbol: None,
                qualified_name: (*name).to_owned(),
                path: "src/lib.rs".to_owned(),
                sig_hash: 1,
                body_hash: 2,
            })
            .collect(),
    }
}

/// Cleaning normalizes line endings, drops control characters and trims the ends.
#[test]
fn cleaning_normalizes_text() {
    // Leading spaces survive: a memory often quotes code, and its indentation carries meaning.
    assert_eq!(sanitize("  hello \r\n  world  "), "hello\n  world");
    assert_eq!(sanitize("a\rb"), "a\nb");
    assert_eq!(sanitize("keeps\ta tab"), "keeps\ta tab");
    assert_eq!(sanitize("drops\u{7}a bell"), "dropsa bell");
    assert_eq!(sanitize("   \n\n   "), "");
}

/// A long run of blank lines collapses to at most two.
#[test]
fn cleaning_collapses_blank_runs() {
    assert_eq!(sanitize("a\n\n\n\n\n\nb"), "a\n\n\nb");
    assert_eq!(sanitize("a\n\nb"), "a\n\nb");
}

/// Cleaning never panics, whatever it is given.
#[test]
fn cleaning_handles_hostile_text() {
    let long = "x".repeat(100_000);
    assert_eq!(sanitize(&long).len(), 100_000);
    assert_eq!(sanitize("\u{0}\u{1}\u{2}"), "");
    assert!(sanitize("日本語 🎉 ñ").contains('🎉'));
}

/// The corroboration count is read from the evidence, and it is capped.
#[test]
fn corroborations_are_counted_from_evidence() {
    let (positive, _) = SignalKind::Useful.deltas();
    assert_eq!(corroborations_of(&UtilityState::empty(0)), 0);

    let one = UtilityState {
        alpha: positive,
        beta: 0.0,
        updated_at: 0,
    };
    assert_eq!(corroborations_of(&one), 1);

    let three = UtilityState {
        alpha: positive * 3.0,
        beta: 0.0,
        updated_at: 0,
    };
    assert_eq!(corroborations_of(&three), 3);

    // However much evidence piles up, the count stops at the cap, so repetition cannot dominate.
    let flood = UtilityState {
        alpha: positive * 1_000.0,
        beta: 0.0,
        updated_at: 0,
    };
    assert_eq!(corroborations_of(&flood), MAX_CORROBORATIONS);
}

/// Negative or nonsensical evidence counts as none, never as a negative number.
#[test]
fn odd_evidence_counts_as_none() {
    let negative = UtilityState {
        alpha: -5.0,
        beta: 0.0,
        updated_at: 0,
    };
    assert_eq!(corroborations_of(&negative), 0);
    let tiny = UtilityState {
        alpha: 0.01,
        beta: 0.0,
        updated_at: 0,
    };
    assert_eq!(corroborations_of(&tiny), 0);
}

/// Near-identical memories collapse to one line with a count of the rest.
#[test]
fn display_collapses_near_duplicates() {
    let memories = vec![
        memory(
            1,
            MemoryKind::Fact,
            "Money is stored as integer minor units, never floating point",
            &["Ledger"],
        ),
        memory(
            2,
            MemoryKind::Fact,
            "Money is stored as integer minor units and never floating point",
            &[],
        ),
        memory(
            3,
            MemoryKind::Fact,
            "Money is stored as integer minor units, never as floating point",
            &[],
        ),
        memory(
            4,
            MemoryKind::Fact,
            "Dashboard charts read from a view refreshed every five minutes",
            &[],
        ),
    ];
    let collapsed = dedupe_for_capsule(memories);
    assert_eq!(
        collapsed.len(),
        2,
        "the three ways of saying one thing become one line"
    );
    // The kept memory is the one with the most anchors.
    assert_eq!(collapsed[0].0.id, MemoryId(1));
    assert_eq!(collapsed[0].1, 2, "it stands for two more");
    assert_eq!(collapsed[1].1, 0);
}

/// Memories of different kinds are never collapsed together, however alike their text.
#[test]
fn display_never_collapses_across_kinds() {
    let text = "Money is stored as integer minor units, never as a floating point number";
    let memories = vec![
        memory(1, MemoryKind::Fact, text, &[]),
        memory(2, MemoryKind::Convention, text, &[]),
    ];
    assert_eq!(dedupe_for_capsule(memories).len(), 2);
}

/// Collapsing does not depend on the order the memories arrive in.
#[test]
fn display_collapsing_is_deterministic() {
    let build = || {
        vec![
            memory(
                7,
                MemoryKind::Lesson,
                "Tests for the export need a fixed clock to stay stable",
                &[],
            ),
            memory(
                3,
                MemoryKind::Lesson,
                "Tests for the export need a fixed clock so they stay stable",
                &[],
            ),
            memory(
                5,
                MemoryKind::Lesson,
                "Retry the upload once before reporting a failure",
                &[],
            ),
        ]
    };
    let forward = dedupe_for_capsule(build());
    let mut reversed = build();
    reversed.reverse();
    let backward = dedupe_for_capsule(reversed);
    let ids =
        |list: &[(MemoryRecord, u32)]| list.iter().map(|(m, n)| (m.id, *n)).collect::<Vec<_>>();
    assert_eq!(ids(&forward), ids(&backward));
}

/// An empty list collapses to an empty list.
#[test]
fn display_handles_an_empty_list() {
    assert!(dedupe_for_capsule(Vec::new()).is_empty());
}

/// The table form carries every field a capsule prints, with the anchors as one string.
#[test]
fn the_table_form_carries_every_field() {
    let mut stale = memory(
        9,
        MemoryKind::Decision,
        "Use a file, not environment variables",
        &["Config", "load"],
    );
    stale.stale_since = Some(10);
    let value = memories_to_value(&[stale]);
    let row = &value["memories"][0];
    assert_eq!(row["id"], 9);
    assert_eq!(row["kind"], "decision");
    assert_eq!(row["by"], "user");
    assert_eq!(row["stale"], "yes");
    assert_eq!(row["about"], "Config load");
    assert_eq!(row["text"], "Use a file, not environment variables");
}

/// A denial scores as highly similar to what it denies, because negating a sentence changes
/// almost none of its words. This is why a contradiction must be checked before similarity: the
/// two mean opposite things, and merging them would let a memory be reinforced by its opposite.
#[test]
fn a_denial_scores_as_similar_which_is_why_it_is_checked_first() {
    let stated = "The token estimator is calibrated against a real tokenizer, not guessed";
    let denied = "The token estimator is not calibrated against a real tokenizer";
    assert!(
        super::similarity(stated, denied) >= super::similarity::SIMILAR,
        "the text measure cannot see the negation, which is the point"
    );
    assert!(
        super::contradicts(stated, denied, MemoryKind::Decision).is_some(),
        "the contradiction check must catch what the text measure cannot"
    );
}
