// SPDX-License-Identifier: Apache-2.0
//! Memory tests: anchoring, listing, search, staleness when the code changes, re-anchoring and
//! forgetting.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // Test code: a failed check aborts the test.

mod common;

use common::{DraftExt, func, id_of, put, put_at, store};
use pn_ultramemory_core::{
    MemoryFilter, MemoryId, MemoryKind, MemoryRecord, NewMemory, Provenance, Storage, StorageError,
    SymbolId,
};
use pn_ultramemory_store::SqliteStorage;

/// A memory of a kind about some symbols.
fn note(kind: MemoryKind, text: &str, about: &[SymbolId]) -> NewMemory {
    NewMemory {
        kind,
        text: text.to_owned(),
        provenance: Provenance::Agent,
        about: about.to_vec(),
    }
}

/// Stores a fact about some symbols at a time.
fn remember(store: &SqliteStorage, text: &str, about: &[SymbolId], now: i64) -> MemoryRecord {
    store
        .add_memory(&note(MemoryKind::Fact, text, about), now)
        .expect("add memory")
}

/// Reads one memory back through the listing.
fn fetch(store: &SqliteStorage, id: MemoryId) -> MemoryRecord {
    store
        .list_memories(&MemoryFilter {
            limit: 1_000,
            ..MemoryFilter::default()
        })
        .unwrap()
        .into_iter()
        .find(|m| m.id == id)
        .expect("memory exists")
}

/// A store with a file of two functions, and their ids.
fn project() -> (SqliteStorage, SymbolId, SymbolId) {
    let store = store();
    put(
        &store,
        "src/a.rs",
        vec![func("alpha").lines(1, 5), func("beta").lines(7, 9)],
        vec![],
    );
    let alpha = id_of(&store, "src/a.rs", "alpha");
    let beta = id_of(&store, "src/a.rs", "beta");
    (store, alpha, beta)
}

/// Adding a memory records the hashes the symbols have now, and returns what was stored.
#[test]
fn add_records_anchors_with_current_hashes() {
    let (store, alpha, beta) = project();
    let record = store
        .add_memory(
            &NewMemory {
                kind: MemoryKind::Decision,
                text: "Chose polling over callbacks.".into(),
                provenance: Provenance::User,
                about: vec![alpha, beta],
            },
            77,
        )
        .unwrap();
    assert_eq!(record.kind, MemoryKind::Decision);
    assert_eq!(record.provenance, Provenance::User);
    assert_eq!(record.created_at, 77);
    assert_eq!(record.stale_since, None);
    assert_eq!(record.anchors.len(), 2);
    let stored_alpha = store.symbol(alpha).unwrap().unwrap();
    let anchor = &record.anchors[0];
    assert_eq!(anchor.symbol, Some(alpha));
    assert_eq!(anchor.qualified_name, "alpha");
    assert_eq!(anchor.path, "src/a.rs");
    assert_eq!(anchor.sig_hash, stored_alpha.sig_hash);
    assert_eq!(anchor.body_hash, stored_alpha.body_hash);
    assert_eq!(record.anchors[1].symbol, Some(beta));
    // What is listed later is what was returned.
    assert_eq!(fetch(&store, record.id), record);
    assert_eq!(store.stats().unwrap().memories, 1);
}

/// A symbol that does not exist makes the whole call fail, and nothing is stored.
#[test]
fn unknown_symbols_are_refused() {
    let (store, alpha, _) = project();
    let result = store.add_memory(&note(MemoryKind::Fact, "x", &[alpha, SymbolId(424_242)]), 1);
    assert!(
        matches!(result, Err(StorageError::NotFound(_))),
        "{result:?}"
    );
    assert_eq!(store.stats().unwrap().memories, 0);
    assert!(store.search_memories("x", 10).unwrap().is_empty());
}

/// Repeated symbols are anchored once, and a memory about nothing is allowed.
#[test]
fn duplicates_collapse_and_empty_anchor_lists_work() {
    let (store, alpha, _) = project();
    let record = remember(&store, "twice", &[alpha, alpha, alpha], 1);
    assert_eq!(record.anchors.len(), 1);
    let loose = remember(&store, "about nothing in particular", &[], 2);
    assert!(loose.anchors.is_empty());
    // A memory with no anchors is never stale.
    put(&store, "src/a.rs", vec![func("other")], vec![]);
    assert_eq!(fetch(&store, loose.id).stale_since, None);
}

/// Every kind and provenance survives storage.
#[test]
fn kinds_and_provenance_round_trip() {
    let (store, alpha, _) = project();
    for kind in MemoryKind::ALL {
        for provenance in [Provenance::Tool, Provenance::Agent, Provenance::User] {
            let added = store
                .add_memory(
                    &NewMemory {
                        kind,
                        text: format!("{kind} {provenance}"),
                        provenance,
                        about: vec![alpha],
                    },
                    5,
                )
                .unwrap();
            let read = fetch(&store, added.id);
            assert_eq!((read.kind, read.provenance), (kind, provenance));
        }
    }
    // Derived rather than written out, so adding a memory kind does not break this test.
    let expected = (MemoryKind::ALL.len() * 3) as u64;
    assert_eq!(store.stats().unwrap().memories, expected);
}

/// Memories for symbols come newest first, once each, and honor the limit.
#[test]
fn memories_for_symbols_are_newest_first() {
    let (store, alpha, beta) = project();
    let old = remember(&store, "old", &[alpha], 10);
    let both = remember(&store, "both", &[alpha, beta], 20);
    let only_beta = remember(&store, "beta only", &[beta], 30);
    let tie = remember(&store, "tie", &[alpha], 30);

    let for_alpha: Vec<_> = store
        .memories_for_symbols(&[alpha], 10)
        .unwrap()
        .iter()
        .map(|m| m.id)
        .collect();
    assert_eq!(for_alpha, [tie.id, both.id, old.id]);
    let for_both: Vec<_> = store
        .memories_for_symbols(&[alpha, beta, alpha], 10)
        .unwrap()
        .iter()
        .map(|m| m.id)
        .collect();
    assert_eq!(for_both, [tie.id, only_beta.id, both.id, old.id]);
    assert_eq!(
        store.memories_for_symbols(&[alpha, beta], 2).unwrap().len(),
        2
    );
    assert!(store.memories_for_symbols(&[alpha], 0).unwrap().is_empty());
    assert!(store.memories_for_symbols(&[], 10).unwrap().is_empty());
    assert!(
        store
            .memories_for_symbols(&[SymbolId(9)], 10)
            .unwrap()
            .is_empty()
    );
}

/// Listing: newest first, by kind, only stale, with an explicit and a default limit.
#[test]
fn listing_filters_and_limits() {
    let (store, alpha, _) = project();
    for i in 0..120 {
        let kind = if i % 3 == 0 {
            MemoryKind::Lesson
        } else {
            MemoryKind::Fact
        };
        store
            .add_memory(&note(kind, &format!("memory {i}"), &[alpha]), i)
            .unwrap();
    }
    let default = store.list_memories(&MemoryFilter::default()).unwrap();
    assert_eq!(default.len(), 100);
    assert_eq!(default[0].text, "memory 119");
    assert!(
        default
            .windows(2)
            .all(|p| p[0].created_at >= p[1].created_at)
    );

    let five = store
        .list_memories(&MemoryFilter {
            limit: 5,
            ..MemoryFilter::default()
        })
        .unwrap();
    assert_eq!(five.len(), 5);
    let lessons = store
        .list_memories(&MemoryFilter {
            kind: Some(MemoryKind::Lesson),
            limit: 1_000,
            only_stale: false,
        })
        .unwrap();
    assert_eq!(lessons.len(), 40);
    assert!(lessons.iter().all(|m| m.kind == MemoryKind::Lesson));
    assert!(
        store
            .list_memories(&MemoryFilter {
                kind: Some(MemoryKind::Task),
                ..MemoryFilter::default()
            })
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .list_memories(&MemoryFilter {
                only_stale: true,
                ..MemoryFilter::default()
            })
            .unwrap()
            .is_empty()
    );
}

/// Re-storing identical code, moving it, or editing another file leaves memories fresh.
#[test]
fn unchanged_code_keeps_memories_fresh() {
    let (store, alpha, beta) = project();
    let memory = remember(&store, "about alpha", &[alpha], 1);
    let outcome = put(
        &store,
        "src/a.rs",
        vec![
            func("alpha").lines(11, 15).doc("now documented"),
            func("beta").lines(17, 19),
        ],
        vec![],
    );
    assert_eq!(outcome.memories_marked_stale, 0);
    put(
        &store,
        "src/b.rs",
        vec![func("alpha").body("different"), func("beta")],
        vec![],
    );
    let read = fetch(&store, memory.id);
    assert_eq!(read.stale_since, None);
    assert_eq!(read.anchors[0].symbol, Some(alpha));
    assert_eq!(store.stats().unwrap().stale_memories, 0);
    let _ = beta;
}

/// A changed signature or body marks the memory stale, once, at the time of the change.
#[test]
fn changed_code_marks_memories_stale_once() {
    let (store, alpha, beta) = project();
    let about_alpha = remember(&store, "about alpha", &[alpha], 1);
    let about_beta = remember(&store, "about beta", &[beta], 2);
    let about_both = remember(&store, "about both", &[alpha, beta], 3);

    let outcome = put_at(
        &store,
        "src/a.rs",
        vec![
            func("alpha").body("changed body").lines(1, 5),
            func("beta").lines(7, 9),
        ],
        vec![],
        500,
    );
    assert_eq!(outcome.memories_marked_stale, 2);
    assert_eq!(fetch(&store, about_alpha.id).stale_since, Some(500));
    assert_eq!(fetch(&store, about_both.id).stale_since, Some(500));
    assert_eq!(fetch(&store, about_beta.id).stale_since, None);
    assert_eq!(store.stats().unwrap().stale_memories, 2);

    // Changing the signature of beta later stales its memory, and leaves the first mark alone.
    let outcome = put_at(
        &store,
        "src/a.rs",
        vec![
            func("alpha").body("changed again").lines(1, 5),
            func("beta").sig("fn beta(x: u8)").lines(7, 9),
        ],
        vec![],
        900,
    );
    assert_eq!(outcome.memories_marked_stale, 1);
    assert_eq!(fetch(&store, about_alpha.id).stale_since, Some(500));
    assert_eq!(fetch(&store, about_both.id).stale_since, Some(500));
    assert_eq!(fetch(&store, about_beta.id).stale_since, Some(900));

    let stale = store
        .list_memories(&MemoryFilter {
            only_stale: true,
            limit: 10,
            kind: None,
        })
        .unwrap();
    assert_eq!(stale.len(), 3);
    assert_eq!(store.stats().unwrap().stale_memories, 3);
}

/// A vanished symbol makes the memory stale and keeps what the anchor recorded.
#[test]
fn vanished_symbols_leave_recorded_anchors() {
    let (store, alpha, beta) = project();
    let memory = remember(&store, "about alpha", &[alpha], 1);
    let recorded = memory.anchors[0].clone();
    let outcome = put_at(
        &store,
        "src/a.rs",
        vec![func("beta").lines(7, 9)],
        vec![],
        40,
    );
    assert_eq!(outcome.memories_marked_stale, 1);
    let read = fetch(&store, memory.id);
    assert_eq!(read.stale_since, Some(40));
    let anchor = &read.anchors[0];
    assert_eq!(anchor.symbol, None);
    assert_eq!(anchor.qualified_name, recorded.qualified_name);
    assert_eq!(anchor.path, recorded.path);
    assert_eq!(
        (anchor.sig_hash, anchor.body_hash),
        (recorded.sig_hash, recorded.body_hash)
    );
    // The anchor is decided by the domain rule.
    assert!(anchor.is_stale_against(None));
    let _ = beta;
}

/// Removing a file marks the memories anchored in it, and only those.
#[test]
fn removing_files_marks_their_memories_stale() {
    let store = store();
    put(&store, "src/a.rs", vec![func("alpha")], vec![]);
    put(&store, "src/b.rs", vec![func("beta")], vec![]);
    let in_a = remember(&store, "a", &[id_of(&store, "src/a.rs", "alpha")], 1);
    let in_b = remember(&store, "b", &[id_of(&store, "src/b.rs", "beta")], 1);
    store
        .remove_files_not_in(&["src/b.rs".to_owned()], 88)
        .unwrap();
    assert_eq!(fetch(&store, in_a.id).stale_since, Some(88));
    assert_eq!(fetch(&store, in_a.id).anchors[0].symbol, None);
    assert_eq!(fetch(&store, in_b.id).stale_since, None);
    // Removing again does not move the mark.
    store.remove_files_not_in(&[], 99).unwrap();
    assert_eq!(fetch(&store, in_a.id).stale_since, Some(88));
    assert_eq!(fetch(&store, in_b.id).stale_since, Some(99));
}

/// Re-anchoring accepts the code as it is now, clears the mark, and later identical stores stay quiet.
#[test]
fn reanchoring_clears_staleness() {
    let (store, alpha, _) = project();
    let memory = remember(&store, "about alpha", &[alpha], 1);
    put_at(
        &store,
        "src/a.rs",
        vec![func("alpha").body("v2"), func("beta").lines(7, 9)],
        vec![],
        50,
    );
    assert_eq!(fetch(&store, memory.id).stale_since, Some(50));

    assert!(store.reanchor_memory(memory.id).unwrap());
    let read = fetch(&store, memory.id);
    assert_eq!(read.stale_since, None);
    let current = store.symbol(alpha).unwrap().unwrap();
    assert_eq!(read.anchors[0].body_hash, current.body_hash);
    assert_eq!(read.anchors[0].sig_hash, current.sig_hash);

    let outcome = put_at(
        &store,
        "src/a.rs",
        vec![func("alpha").body("v2"), func("beta").lines(7, 9)],
        vec![],
        60,
    );
    assert_eq!(outcome.memories_marked_stale, 0);
    assert_eq!(fetch(&store, memory.id).stale_since, None);
    // The next real change stales it again.
    let outcome = put_at(
        &store,
        "src/a.rs",
        vec![func("alpha").body("v3"), func("beta").lines(7, 9)],
        vec![],
        70,
    );
    assert_eq!(outcome.memories_marked_stale, 1);
    assert_eq!(fetch(&store, memory.id).stale_since, Some(70));

    assert!(!store.reanchor_memory(MemoryId(999_999)).unwrap());
}

/// A memory about a symbol that vanished and came back is found again, stays stale until
/// re-anchored, and is then linked to the new symbol.
#[test]
fn symbols_that_come_back_are_relinked_by_reanchoring() {
    let (store, alpha, _) = project();
    let memory = remember(&store, "about alpha", &[alpha], 1);
    put_at(
        &store,
        "src/a.rs",
        vec![func("beta").lines(7, 9)],
        vec![],
        10,
    );
    assert_eq!(fetch(&store, memory.id).anchors[0].symbol, None);

    // The symbol returns unchanged, so it has the same identity and the same hashes.
    put_at(
        &store,
        "src/a.rs",
        vec![func("alpha").lines(1, 5), func("beta").lines(7, 9)],
        vec![],
        20,
    );
    assert_eq!(id_of(&store, "src/a.rs", "alpha"), alpha);
    let back: Vec<_> = store.memories_for_symbols(&[alpha], 10).unwrap();
    assert_eq!(back.len(), 1, "found by path and qualified name");
    assert_eq!(
        back[0].stale_since,
        Some(10),
        "still stale until someone confirms it"
    );

    assert!(store.reanchor_memory(memory.id).unwrap());
    let read = fetch(&store, memory.id);
    assert_eq!(read.stale_since, None);
    assert_eq!(read.anchors[0].symbol, Some(alpha));
}

/// Re-anchoring a memory whose symbol is gone for good keeps the recorded anchor.
#[test]
fn reanchoring_a_vanished_symbol_keeps_the_record() {
    let (store, alpha, _) = project();
    let memory = remember(&store, "about alpha", &[alpha], 1);
    put_at(
        &store,
        "src/a.rs",
        vec![func("beta").lines(7, 9)],
        vec![],
        10,
    );
    assert!(store.reanchor_memory(memory.id).unwrap());
    let read = fetch(&store, memory.id);
    assert_eq!(read.stale_since, None);
    assert_eq!(read.anchors[0].qualified_name, "alpha");
    assert_eq!(read.anchors[0].symbol, None);
    // The next index of the file notices that the symbol is still missing.
    let outcome = put_at(
        &store,
        "src/a.rs",
        vec![func("beta").lines(7, 9)],
        vec![],
        30,
    );
    assert_eq!(outcome.memories_marked_stale, 1);
}

/// Forgetting removes the memory, its anchors, its search entry and what was learned about it.
#[test]
fn forgetting_removes_everything_about_a_memory() {
    let (store, alpha, _) = project();
    let keep = remember(&store, "keep this widget", &[alpha], 1);
    let gone = remember(&store, "discard this widget", &[alpha], 2);
    store
        .put_utility_state(
            pn_ultramemory_core::Target::Memory(gone.id),
            pn_ultramemory_core::UtilityState {
                alpha: 2.0,
                beta: 1.0,
                updated_at: 5,
            },
        )
        .unwrap();
    assert!(store.forget_memory(gone.id).unwrap());
    assert!(!store.forget_memory(gone.id).unwrap());
    assert!(!store.forget_memory(MemoryId(-1)).unwrap());
    let left: Vec<_> = store
        .search_memories("widget", 10)
        .unwrap()
        .iter()
        .map(|m| m.id)
        .collect();
    assert_eq!(left, [keep.id]);
    assert_eq!(store.memories_for_symbols(&[alpha], 10).unwrap().len(), 1);
    assert_eq!(store.stats().unwrap().memories, 1);
    assert_eq!(store.learning_status().unwrap().tracked_targets, 0);
    // Identities are not reused for a different memory.
    let next = remember(&store, "next", &[], 3);
    assert!(next.id.0 > gone.id.0);
}

/// Memory search: identifiers split like symbol names, prefixes, ranking, and no results.
#[test]
fn searches_memory_text() {
    let (store, alpha, _) = project();
    let a = remember(
        &store,
        "We call parseConfig before starting the server.",
        &[alpha],
        1,
    );
    let b = remember(&store, "The cache is flushed on shutdown.", &[], 2);
    let c = remember(
        &store,
        "parse errors are reported with line numbers",
        &[],
        3,
    );

    let ids = |text: &str| -> Vec<MemoryId> {
        store
            .search_memories(text, 10)
            .unwrap()
            .iter()
            .map(|m| m.id)
            .collect()
    };
    assert_eq!(ids("parse config"), [a.id]);
    assert_eq!(ids("parse_config"), [a.id]);
    assert_eq!(ids("cache flushed"), [b.id]);
    assert_eq!(ids("shutd"), [b.id], "the last word matches by prefix");
    assert_eq!(ids("parse").len(), 2);
    assert_eq!(ids("PARSE")[0..1].len(), 1);
    assert!(ids("nothing matches this at all").is_empty());
    assert!(ids("").is_empty());
    assert!(ids("(((").is_empty());
    assert!(store.search_memories("parse", 0).unwrap().is_empty());
    assert_eq!(store.search_memories("parse", 1).unwrap().len(), 1);
    let _ = c;
}

/// A question written as a sentence still finds the best memory: when no memory has every word,
/// any word will do, and better matches come first.
#[test]
fn sentences_fall_back_to_any_word() {
    let (store, _, _) = project();
    let wal = remember(
        &store,
        "Use WAL mode so readers do not block the writer.",
        &[],
        1,
    );
    let other = remember(&store, "Retry the request twice before giving up.", &[], 2);
    let found: Vec<_> = store
        .search_memories("why did we choose WAL mode for concurrency?", 10)
        .unwrap()
        .iter()
        .map(|m| m.id)
        .collect();
    assert_eq!(found.first(), Some(&wal.id));
    assert!(!found.contains(&other.id) || found.len() == 2);
}

/// Hostile text never fails a memory search.
#[test]
fn hostile_memory_queries_never_fail() {
    let (store, alpha, _) = project();
    remember(&store, "plain text about things", &[alpha], 1);
    for input in [
        "\"",
        "'",
        "' OR 1=1 --",
        "a AND",
        "NEAR(a b)",
        "text:things",
        "-things",
        "^text",
        "*",
        "(",
        ")",
        "\0",
        "\u{202e}text",
        "🦀",
        "\\",
        "%",
        "text OR OR things",
        "{a}",
        "col:\"",
    ] {
        assert!(store.search_memories(input, 5).is_ok(), "{input:?}");
    }
}

/// Very long and unusual memory text is stored and found.
#[test]
fn stores_unusual_text() {
    let (store, _, _) = project();
    let long = format!("{} unique_marker_word", "padding ".repeat(200_000));
    let record = remember(&store, &long, &[], 1);
    assert_eq!(fetch(&store, record.id).text, long);
    let empty = remember(&store, "", &[], 2);
    assert_eq!(fetch(&store, empty.id).text, "");
    let odd = remember(&store, "línea\n日本語 \"quoted\" \\ \0 end", &[], 3);
    assert_eq!(
        fetch(&store, odd.id).text,
        "línea\n日本語 \"quoted\" \\ \0 end"
    );
    assert_eq!(store.search_memories("日本語", 5).unwrap().len(), 1);
}
