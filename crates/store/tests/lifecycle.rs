// SPDX-License-Identifier: Apache-2.0
//! Lifecycle tests: opening files, creating directories, schema versions, refusing foreign and
//! newer databases, persistence across reopening, two handles on one file, and concurrent use
//! from several threads.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // Test code: a failed check aborts the test.

mod common;

use std::sync::Arc;
use std::thread;

use common::{DraftExt, call, edges_of, func, id_of, put, store};
use pn_ultramemory_core::{
    Confidence, Direction, MemoryFilter, MemoryKind, NewMemory, Provenance, ResolveScope,
    SearchQuery, SignalKind, Storage, StorageError, SymbolId, Target, UtilityState,
};
use pn_ultramemory_store::SqliteStorage;
use rusqlite::Connection;

/// Opens a raw connection to look at what the adapter wrote.
fn raw(path: &std::path::Path) -> Connection {
    Connection::open(path).expect("raw connection")
}

/// Reads a numeric pragma through a raw connection.
fn pragma(path: &std::path::Path, name: &str) -> i64 {
    raw(path)
        .pragma_query_value(None, name, |row| row.get(0))
        .expect("pragma")
}

/// The store can be shared between threads and used as a trait object.
#[test]
fn storage_is_send_sync_and_object_safe() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<SqliteStorage>();
    let store = store();
    let as_dyn: &dyn Storage = &store;
    assert_eq!(as_dyn.stats().unwrap().files, 0);
    assert!(format!("{store:?}").contains("SqliteStorage"));
}

/// Opening a path creates the missing directories and a database at the schema of this build.
#[test]
fn open_creates_directories_and_the_schema() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("deeply").join("nested").join("index.db");
    let store = SqliteStorage::open(&path).unwrap();
    assert!(path.exists());
    store.set_meta("k", "v").unwrap();
    drop(store);
    assert_eq!(
        pragma(&path, "user_version"),
        i64::from(SqliteStorage::SCHEMA_VERSION)
    );
    assert_eq!(pragma(&path, "application_id"), 0x504E_554D);
    let mode: String = raw(&path)
        .pragma_query_value(None, "journal_mode", |row| row.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
}

/// A path whose parent cannot be created is a backend error, not a panic.
#[test]
fn open_reports_unusable_locations() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("plain.txt");
    std::fs::write(&file, "x").unwrap();
    let below_a_file = SqliteStorage::open(&file.join("index.db"));
    assert!(
        matches!(below_a_file, Err(StorageError::Backend(_))),
        "{below_a_file:?}"
    );
    let a_directory = SqliteStorage::open(dir.path());
    assert!(a_directory.is_err());
}

/// Opening the same file again is safe, and an empty file is a valid new database.
#[test]
fn reopening_and_empty_files() {
    let dir = tempfile::tempdir().unwrap();
    let empty = dir.path().join("empty.db");
    std::fs::write(&empty, b"").unwrap();
    let first = SqliteStorage::open(&empty).unwrap();
    let second = SqliteStorage::open(&empty).unwrap();
    first.set_meta("a", "1").unwrap();
    assert_eq!(second.get_meta("a").unwrap().as_deref(), Some("1"));
    drop((first, second));
    let third = SqliteStorage::open(&empty).unwrap();
    assert_eq!(third.get_meta("a").unwrap().as_deref(), Some("1"));
}

/// A database with a newer schema version is refused as corrupt, and left untouched.
#[test]
fn newer_schema_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("index.db");
    drop(SqliteStorage::open(&path).unwrap());
    raw(&path)
        .execute_batch("PRAGMA user_version = 99;")
        .unwrap();
    let result = SqliteStorage::open(&path);
    match result {
        Err(StorageError::Corrupt(message)) => assert!(message.contains("99"), "{message}"),
        other => panic!("expected Corrupt, got {other:?}"),
    }
    assert_eq!(pragma(&path, "user_version"), 99);
}

/// A database that belongs to something else, or is not a database, is refused.
#[test]
fn foreign_files_are_refused() {
    let dir = tempfile::tempdir().unwrap();

    let foreign = dir.path().join("foreign.db");
    raw(&foreign)
        .execute_batch("CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT);")
        .unwrap();
    assert!(matches!(
        SqliteStorage::open(&foreign),
        Err(StorageError::Corrupt(_))
    ));

    let garbage = dir.path().join("garbage.db");
    std::fs::write(
        &garbage,
        "this is definitely not a sqlite database ".repeat(50),
    )
    .unwrap();
    assert!(matches!(
        SqliteStorage::open(&garbage),
        Err(StorageError::Corrupt(_))
    ));

    let wrong_app = dir.path().join("wrong_app.db");
    raw(&wrong_app)
        .execute_batch("PRAGMA user_version = 1; PRAGMA application_id = 7; CREATE TABLE t (x);")
        .unwrap();
    assert!(matches!(
        SqliteStorage::open(&wrong_app),
        Err(StorageError::Corrupt(_))
    ));
}

/// What one session leaves behind, to compare with what the next one finds.
struct Snapshot {
    /// Every edge of the graph.
    edges: Vec<common::EdgeView>,
    /// The symbols of `src/a.rs`.
    symbols: Vec<pn_ultramemory_core::SymbolRecord>,
    /// The memory that was written.
    memory: pn_ultramemory_core::MemoryId,
    /// The symbol everything was recorded about.
    alpha: SymbolId,
}

/// Fills a store with files, edges, a memory that is stale, learning and metadata.
fn populate(store: &SqliteStorage) -> Snapshot {
    put(
        store,
        "src/a.rs",
        vec![func("alpha").doc("First."), func("beta")],
        vec![call("beta", 0, 3), call("gamma", 1, 4)],
    );
    put(store, "src/g.rs", vec![func("gamma")], vec![]);
    store.resolve_edges(&ResolveScope::All).unwrap();
    let alpha = id_of(store, "src/a.rs", "alpha");
    let memory = store
        .add_memory(
            &NewMemory {
                kind: MemoryKind::Lesson,
                text: "alpha needs a warm cache".into(),
                provenance: Provenance::User,
                about: vec![alpha],
            },
            42,
        )
        .unwrap();
    let state = UtilityState {
        alpha: 2.0,
        beta: 1.0,
        updated_at: 9,
    };
    store
        .put_utility_state(Target::Symbol(alpha), state)
        .unwrap();
    store
        .log_signal(Target::Symbol(alpha), SignalKind::Used, 10)
        .unwrap();
    let gamma = id_of(store, "src/g.rs", "gamma");
    store.bump_coaccess(alpha, gamma, 1.5, 11).unwrap();
    store.set_meta("root", "/repo").unwrap();
    // Edit alpha, so that the memory becomes stale and the mark is persisted too.
    put(
        store,
        "src/a.rs",
        vec![func("alpha").body("edited"), func("beta")],
        vec![call("beta", 0, 3)],
    );
    store.resolve_edges(&ResolveScope::All).unwrap();
    Snapshot {
        edges: edges_of(store),
        symbols: store.symbols_in_file("src/a.rs").unwrap(),
        memory: memory.id,
        alpha,
    }
}

/// Everything survives closing and reopening the file.
#[test]
fn data_persists_across_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("index.db");
    let snapshot = populate(&SqliteStorage::open(&path).unwrap());
    let alpha = snapshot.alpha;

    let store = SqliteStorage::open(&path).unwrap();
    assert_eq!(edges_of(&store), snapshot.edges);
    assert_eq!(store.symbols_in_file("src/a.rs").unwrap(), snapshot.symbols);
    assert_eq!(
        store.file_hash("src/g.rs").unwrap().as_deref(),
        Some("h-src/g.rs")
    );
    let memories = store.list_memories(&MemoryFilter::default()).unwrap();
    assert_eq!(memories.len(), 1);
    assert_eq!(memories[0].id, snapshot.memory);
    assert_eq!(memories[0].stale_since, Some(common::NOW));
    assert_eq!(store.search_memories("warm cache", 5).unwrap().len(), 1);
    let query = SearchQuery {
        text: "gamma".into(),
        ..SearchQuery::default()
    };
    assert_eq!(store.search_symbols(&query, 5).unwrap().len(), 1);
    let utility = store.utility_states(&[Target::Symbol(alpha)]).unwrap();
    assert!((utility[0].1.alpha - 2.0).abs() < 1e-12);
    let status = store.learning_status().unwrap();
    assert_eq!(
        (
            status.signals,
            status.tracked_targets,
            status.coaccess_pairs
        ),
        (1, 1, 1)
    );
    assert_eq!(store.coaccess_neighbors(alpha, 5, 11).unwrap().len(), 1);
    assert_eq!(store.get_meta("root").unwrap().as_deref(), Some("/repo"));
    // Stable identities: storing the same file again gives the same ids as before the restart.
    put(
        &store,
        "src/a.rs",
        vec![func("alpha").body("edited"), func("beta")],
        vec![],
    );
    assert_eq!(id_of(&store, "src/a.rs", "alpha"), alpha);
}

/// Two handles on one file see each other's writes, and writes from both succeed.
#[test]
fn two_handles_share_one_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("index.db");
    let left = Arc::new(SqliteStorage::open(&path).unwrap());
    let right = Arc::new(SqliteStorage::open(&path).unwrap());
    put(&left, "src/a.rs", vec![func("alpha")], vec![]);
    assert_eq!(right.list_files().unwrap().len(), 1);

    let writers: Vec<_> = [(Arc::clone(&left), "l"), (Arc::clone(&right), "r")]
        .into_iter()
        .map(|(store, tag)| {
            thread::spawn(move || {
                for i in 0..40 {
                    let path = format!("src/{tag}{i}.rs");
                    put(&store, &path, vec![func(&format!("f_{tag}_{i}"))], vec![]);
                    store.set_meta(&format!("{tag}{i}"), "x").unwrap();
                }
            })
        })
        .collect();
    for writer in writers {
        writer.join().unwrap();
    }
    assert_eq!(left.stats().unwrap().files, 81);
    assert_eq!(right.stats().unwrap().files, 81);
}

/// Several processes (here, threads with their own handle) opening a brand new file at the same
/// moment all succeed: the migration runs once and nobody sees a half-built schema.
#[test]
fn simultaneous_first_opens_are_safe() {
    for round in 0..5 {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(format!("race{round}.db"));
        let barrier = Arc::new(std::sync::Barrier::new(6));
        let handles: Vec<_> = (0..6)
            .map(|n| {
                let (path, barrier) = (path.clone(), Arc::clone(&barrier));
                thread::spawn(move || {
                    barrier.wait();
                    let store = SqliteStorage::open(&path).unwrap();
                    store.set_meta(&format!("opened{n}"), "yes").unwrap();
                    store
                })
            })
            .collect();
        let stores: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(
            pragma(&path, "user_version"),
            i64::from(SqliteStorage::SCHEMA_VERSION)
        );
        for n in 0..6 {
            assert_eq!(
                stores[0]
                    .get_meta(&format!("opened{n}"))
                    .unwrap()
                    .as_deref(),
                Some("yes")
            );
        }
    }
}

/// Several threads read and write one store at once without errors, and the totals add up.
#[test]
fn concurrent_smoke_test() {
    const THREADS: usize = 8;
    const FILES_PER_THREAD: usize = 15;
    let store = Arc::new(store());
    let handles: Vec<_> = (0..THREADS)
        .map(|t| {
            let store = Arc::clone(&store);
            thread::spawn(move || {
                for i in 0..FILES_PER_THREAD {
                    let path = format!("t{t}/file{i}.rs");
                    let symbols = vec![
                        func(&format!("worker_{t}_{i}")).lines(1, 5),
                        func("shared_helper").lines(6, 9),
                    ];
                    let outcome = put(&store, &path, symbols, vec![call("shared_helper", 0, 2)]);
                    store
                        .resolve_edges(&ResolveScope::Touching {
                            file_ids: vec![outcome.file_id.unwrap()],
                            names: outcome.changed_names,
                        })
                        .unwrap();
                    let hits = store
                        .search_symbols(
                            &SearchQuery {
                                text: format!("worker {t}"),
                                ..SearchQuery::default()
                            },
                            5,
                        )
                        .unwrap();
                    assert!(!hits.is_empty());
                    let id = id_of(&store, &path, &format!("worker_{t}_{i}"));
                    store
                        .add_memory(
                            &NewMemory {
                                kind: MemoryKind::Fact,
                                text: format!("thread {t} wrote file {i}"),
                                provenance: Provenance::Tool,
                                about: vec![id],
                            },
                            i64::try_from(i).unwrap(),
                        )
                        .unwrap();
                    store.bump_coaccess(id, SymbolId(1), 1.0, 5).unwrap();
                    let _ = store
                        .neighbors(id, Direction::Out, Confidence::Guess, 10)
                        .unwrap();
                    let _ = store.stats().unwrap();
                    let _ = store.central_symbols(3, None).unwrap();
                }
            })
        })
        .collect();
    for handle in handles {
        handle.join().unwrap();
    }
    let stats = store.stats().unwrap();
    assert_eq!(stats.files, (THREADS * FILES_PER_THREAD) as u64);
    assert_eq!(stats.symbols, (THREADS * FILES_PER_THREAD * 2) as u64);
    assert_eq!(stats.memories, (THREADS * FILES_PER_THREAD) as u64);
    // The final graph equals a fresh full resolve.
    let incremental = edges_of(&store);
    store.resolve_edges(&ResolveScope::All).unwrap();
    assert_eq!(edges_of(&store), incremental);
}
