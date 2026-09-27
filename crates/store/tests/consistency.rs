// SPDX-License-Identifier: Apache-2.0
//! Consistency tests: an operation that fails half way leaves nothing behind, and after a long
//! random sequence of operations the database passes SQLite's own checks and its full-text
//! indexes describe exactly the rows they should.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // Test code: a failed check aborts the test.

mod common;

use std::collections::BTreeSet;

use common::{
    DraftExt, NOW, assert_consistent, call, edges_of, file_store, func, id_of, put, put_at,
};
use pn_ultramemory_core::{
    MemoryFilter, MemoryId, MemoryKind, NewMemory, Provenance, ReferenceDraft, ResolveScope,
    SearchQuery, Storage, StorageError, SymbolDraft, SymbolId,
};
use pn_ultramemory_store::SqliteStorage;
use rusqlite::Connection;

/// A failure in the middle of an upsert rolls the whole file back.
#[test]
fn a_failed_upsert_leaves_nothing_behind() {
    let (_dir, path, store) = file_store();
    put(
        &store,
        "src/a.rs",
        vec![func("alpha").doc("Original text."), func("beta")],
        vec![call("beta", 0, 3)],
    );
    let before = store.symbols_in_file("src/a.rs").unwrap();
    let hash_before = store.file_hash("src/a.rs").unwrap();

    // A trigger that makes inserting references fail, installed from outside.
    let raw = Connection::open(&path).unwrap();
    raw.execute_batch(
        "CREATE TRIGGER sabotage BEFORE INSERT ON refs \
         BEGIN SELECT RAISE(ABORT, 'refused for the test'); END;",
    )
    .unwrap();
    let mut input = common::file("src/a.rs");
    input.hash = "new-hash".into();
    let extract = common::extract(
        vec![func("gamma").doc("Replacement text."), func("delta")],
        vec![call("delta", 0, 4)],
    );
    let failed = store.upsert_file(&input, &extract, NOW + 1);
    assert!(
        matches!(failed, Err(StorageError::Backend(_))),
        "{failed:?}"
    );

    // Symbols, the file row and the search index are exactly as they were.
    assert_eq!(store.symbols_in_file("src/a.rs").unwrap(), before);
    assert_eq!(store.file_hash("src/a.rs").unwrap(), hash_before);
    let query = |text: &str| SearchQuery {
        text: text.into(),
        ..SearchQuery::default()
    };
    assert_eq!(
        store.search_symbols(&query("original"), 5).unwrap().len(),
        1
    );
    assert!(
        store
            .search_symbols(&query("replacement"), 5)
            .unwrap()
            .is_empty()
    );
    assert_consistent(&path);

    // Once the obstacle is gone, the same call succeeds.
    raw.execute_batch("DROP TRIGGER sabotage;").unwrap();
    store.upsert_file(&input, &extract, NOW + 2).unwrap();
    assert_eq!(store.symbols_in_file("src/a.rs").unwrap().len(), 2);
    assert_eq!(
        store
            .search_symbols(&query("replacement"), 5)
            .unwrap()
            .len(),
        1
    );
    assert_consistent(&path);
}

/// A failed memory write and a failed resolve are atomic too.
#[test]
fn other_failed_writes_are_atomic_too() {
    let (_dir, path, store) = file_store();
    put(
        &store,
        "src/a.rs",
        vec![func("alpha"), func("beta")],
        vec![call("beta", 0, 3)],
    );
    store.resolve_edges(&ResolveScope::All).unwrap();
    let edges = edges_of(&store);
    let alpha = id_of(&store, "src/a.rs", "alpha");

    let raw = Connection::open(&path).unwrap();
    raw.execute_batch(
        "CREATE TRIGGER no_anchors BEFORE INSERT ON memory_anchors \
         BEGIN SELECT RAISE(ABORT, 'refused for the test'); END; \
         CREATE TRIGGER no_edges BEFORE INSERT ON edges \
         BEGIN SELECT RAISE(ABORT, 'refused for the test'); END;",
    )
    .unwrap();
    let memory = NewMemory {
        kind: MemoryKind::Fact,
        text: "half written".into(),
        provenance: Provenance::Agent,
        about: vec![alpha],
    };
    assert!(store.add_memory(&memory, 5).is_err());
    assert_eq!(store.stats().unwrap().memories, 0);
    assert!(store.search_memories("half written", 5).unwrap().is_empty());

    // The resolve deletes every edge first and then fails to write; the deletion is undone.
    assert!(store.resolve_edges(&ResolveScope::All).is_err());
    assert_eq!(edges_of(&store), edges);
    assert_consistent(&path);
}

/// The path, symbols and references of a random file, drawn with `next`.
fn random_file_parts(
    next: &mut impl FnMut(usize) -> usize,
    names: &[&str],
) -> (String, Vec<SymbolDraft>, Vec<ReferenceDraft>) {
    let path = format!("src/dir{}/f{}.rs", next(3), next(8));
    let symbols: Vec<_> = (0..=next(5))
        .map(|i| {
            let name = names[next(names.len())];
            let qualified = if next(2) == 0 {
                name.to_owned()
            } else {
                format!("T{}::{name}", next(2))
            };
            func(&qualified)
                .doc(if next(2) == 0 { "Does the thing." } else { "" })
                .body(&format!("body {}", next(3)))
                .lines(
                    u32::try_from(i * 5 + 1).unwrap(),
                    u32::try_from(i * 5 + 4).unwrap(),
                )
        })
        .collect();
    let count = symbols.len();
    let refs = (0..next(6))
        .map(|_| {
            call(
                names[next(names.len())],
                next(count),
                u32::try_from(1 + next(20)).unwrap(),
            )
        })
        .collect();
    (path, symbols, refs)
}

/// One step of the churn: a pseudo-random operation on the store.
fn churn_step(store: &SqliteStorage, rng: &mut u64, present: &mut BTreeSet<String>, now: i64) {
    let mut next = |bound: usize| -> usize {
        *rng = rng
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        usize::try_from((*rng >> 33) % u64::try_from(bound).unwrap()).unwrap()
    };
    let names = ["run", "parse", "load", "save", "draw", "init"];
    match next(10) {
        0..=3 => {
            let (path, symbols, refs) = random_file_parts(&mut next, &names);
            let outcome = put_at(store, &path, symbols, refs, now);
            present.insert(path);
            if next(2) == 0 {
                store
                    .resolve_edges(&ResolveScope::Touching {
                        file_ids: vec![outcome.file_id.unwrap()],
                        names: outcome.changed_names,
                    })
                    .unwrap();
            }
        }
        4 => {
            // Three files in one transaction.
            let batch: Vec<_> = (0..3)
                .map(|_| {
                    let (path, symbols, refs) = random_file_parts(&mut next, &names);
                    present.insert(path.clone());
                    (common::file(&path), common::extract(symbols, refs))
                })
                .collect();
            let outcomes = store.upsert_files(&batch, now).unwrap();
            store
                .resolve_edges(&ResolveScope::Touching {
                    file_ids: outcomes.iter().filter_map(|o| o.file_id).collect(),
                    names: vec![],
                })
                .unwrap();
        }
        5 => {
            if let Some(path) = present.iter().nth(next(present.len().max(1))).cloned() {
                present.remove(&path);
                let keep: Vec<String> = present.iter().cloned().collect();
                store.remove_files_not_in(&keep, now).unwrap();
            }
        }
        6 => {
            store.resolve_edges(&ResolveScope::All).unwrap();
        }
        7 => {
            let symbols: Vec<SymbolId> = present
                .iter()
                .take(3)
                .flat_map(|p| store.symbols_in_file(p).unwrap())
                .map(|s| s.id)
                .take(2)
                .collect();
            store
                .add_memory(
                    &NewMemory {
                        kind: MemoryKind::Lesson,
                        text: format!("note number {} about parse and load", next(100)),
                        provenance: Provenance::Agent,
                        about: symbols,
                    },
                    now,
                )
                .unwrap();
        }
        8 => {
            let all = store.list_memories(&MemoryFilter::default()).unwrap();
            if let Some(memory) = all.get(next(all.len().max(1))) {
                if next(2) == 0 {
                    store.forget_memory(memory.id).unwrap();
                } else {
                    store.reanchor_memory(memory.id).unwrap();
                }
            }
        }
        _ => {
            let query = SearchQuery {
                text: names[next(names.len())].to_owned(),
                ..SearchQuery::default()
            };
            store.search_symbols(&query, 5).unwrap();
            store.search_memories("parse load", 5).unwrap();
            let _ = store.forget_memory(MemoryId(-1)).unwrap();
        }
    }
}

/// After hundreds of random operations the file passes every check, and a full resolve agrees
/// with what the partial resolves left, once both are brought up to date.
#[test]
fn random_churn_keeps_the_database_consistent() {
    for seed in [1_u64, 2, 3] {
        let (_dir, path, store) = file_store();
        let mut rng = seed;
        let mut present = BTreeSet::new();
        for step in 0..250 {
            churn_step(&store, &mut rng, &mut present, i64::from(step));
            if step % 50 == 49 {
                assert_consistent(&path);
            }
        }
        assert_consistent(&path);
        store.resolve_edges(&ResolveScope::All).unwrap();
        let full = edges_of(&store);
        store.resolve_edges(&ResolveScope::All).unwrap();
        assert_eq!(edges_of(&store), full);
        let stats = store.stats().unwrap();
        assert_eq!(usize::try_from(stats.files).unwrap(), present.len());
        assert_consistent(&path);
    }
}

/// Optionally marks every edge with a confidence no resolve would write for it (exact), then
/// reads all confidences back, through a raw connection.
fn plant_and_read(path: &std::path::Path, plant: bool) -> Vec<i64> {
    let raw = Connection::open(path).unwrap();
    if plant {
        raw.execute("UPDATE edges SET confidence = 3", []).unwrap();
    }
    let mut statement = raw
        .prepare("SELECT confidence FROM edges ORDER BY src, dst")
        .unwrap();
    statement
        .query_map([], |row| row.get::<_, i64>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// Only a change in the set of definitions of a name makes a partial resolve revisit the
/// references to it: an edit that merely modifies a symbol leaves its incoming edges alone, and
/// adding or removing a symbol of that name makes them be resolved again.
#[test]
fn only_definition_changes_make_names_dirty() {
    let (_dir, path, store) = file_store();
    put(
        &store,
        "src/a.rs",
        vec![func("caller")],
        vec![call("dup", 0, 2)],
    );
    put(&store, "src/b.rs", vec![func("dup")], vec![]);
    store.resolve_edges(&ResolveScope::All).unwrap();
    assert_eq!(plant_and_read(&path, true), [3]);
    let nothing = ResolveScope::Touching {
        file_ids: vec![],
        names: vec![],
    };

    // Modified only: the planted mark survives a partial resolve, so the reference was not
    // looked at again.
    put(&store, "src/b.rs", vec![func("dup").body("edited")], vec![]);
    store.resolve_edges(&nothing).unwrap();
    assert_eq!(plant_and_read(&path, false), [3]);

    // A second definition appears: the reference is resolved again (now two guesses).
    put(&store, "src/c.rs", vec![func("dup")], vec![]);
    store.resolve_edges(&nothing).unwrap();
    assert_eq!(plant_and_read(&path, false), [0, 0]);

    // One definition disappears: resolved again once more.
    assert_eq!(plant_and_read(&path, true), [3, 3]);
    store
        .remove_files_not_in(&["src/a.rs".to_owned(), "src/b.rs".to_owned()], 5)
        .unwrap();
    store.resolve_edges(&nothing).unwrap();
    assert_eq!(plant_and_read(&path, false), [1]);
    assert_consistent(&path);
}
