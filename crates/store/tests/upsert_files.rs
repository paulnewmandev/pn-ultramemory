// SPDX-License-Identifier: Apache-2.0
//! Tests of the batch upsert: outcomes in input order, atomicity, and equivalence with storing
//! the same files one at a time, including the bulk-load path that builds indexes at the end and
//! the retry when a derived symbol id is already taken.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // Test code: a failed check aborts the test.

mod common;

use std::collections::BTreeMap;

use common::{
    DraftExt, NOW, Rng, assert_consistent, call, edges_of, extract, file, file_store, func, put,
    store, sym,
};
use pn_ultramemory_core::{
    FileExtract, FileInput, FileRecord, IndexStats, MemoryFilter, MemoryKind, NewMemory,
    Provenance, ResolveScope, SearchQuery, Storage, SymbolKind, SymbolRecord, UpsertOutcome,
    hash64,
};
use pn_ultramemory_store::SqliteStorage;
use rusqlite::Connection;

/// The names that generated symbols and references share, so that many are ambiguous.
const POOL: [&str; 10] = [
    "new", "run", "parse", "load", "save", "render", "draw", "init", "stop", "flush",
];

/// Generates one file of a random project.
fn random_file(
    rng: &mut Rng,
    index: usize,
    symbols: usize,
    refs: usize,
) -> (FileInput, FileExtract) {
    let symbols: Vec<_> = (0..symbols)
        .map(|i| {
            let name = if rng.below(3) == 0 {
                POOL[rng.below(POOL.len())].to_owned()
            } else {
                format!("item_{}_{i}", rng.below(60))
            };
            let qualified = if rng.below(4) == 0 {
                format!("Type{}::{name}", rng.below(3))
            } else {
                name
            };
            let line = u32::try_from(1 + i * 7).unwrap();
            let mut symbol = func(&qualified)
                .lines(line, line + 5)
                .body(&format!("body {}", rng.below(3)));
            if rng.below(2) == 0 {
                symbol = symbol.doc("Does something useful with the pipeline.");
            }
            symbol
        })
        .collect();
    let count = symbols.len();
    let references = (0..refs)
        .map(|_| {
            let name = if rng.below(2) == 0 {
                POOL[rng.below(POOL.len())].to_owned()
            } else {
                format!("item_{}_{}", rng.below(60), rng.below(count))
            };
            call(
                &name,
                rng.below(count),
                u32::try_from(1 + rng.below(90)).unwrap(),
            )
        })
        .collect();
    let path = format!("src/dir{}/file{index}.rs", index % 7);
    (file(&path), extract(symbols, references))
}

/// Generates a random project.
fn random_project(
    seed: u64,
    files: usize,
    symbols: usize,
    refs: usize,
) -> Vec<(FileInput, FileExtract)> {
    let mut rng = Rng(seed);
    (0..files)
        .map(|index| random_file(&mut rng, index, symbols, refs))
        .collect()
}

/// Everything observable about a store, for comparing two of them.
#[derive(Debug, PartialEq)]
struct Snapshot {
    /// The files.
    files: Vec<FileRecord>,
    /// The symbols of every file, in source order.
    symbols: BTreeMap<String, Vec<SymbolRecord>>,
    /// The counts of the index.
    stats: IndexStats,
    /// Every edge.
    edges: Vec<common::EdgeView>,
    /// The results of some searches: path, qualified name and score.
    searches: Vec<Vec<(String, String, u64)>>,
    /// Lookups by name.
    lookups: Vec<Vec<i64>>,
}

/// Reads a snapshot.
fn snapshot(store: &SqliteStorage) -> Snapshot {
    let files = store.list_files().unwrap();
    let symbols = files
        .iter()
        .map(|f| (f.path.clone(), store.symbols_in_file(&f.path).unwrap()))
        .collect();
    let searches = [
        "new",
        "parse config",
        "item",
        "useful pipeline",
        "flu",
        "type0 run",
    ]
    .iter()
    .map(|text| {
        let query = SearchQuery {
            text: (*text).to_owned(),
            ..SearchQuery::default()
        };
        store
            .search_symbols(&query, 25)
            .unwrap()
            .into_iter()
            .map(|hit| {
                (
                    hit.symbol.path,
                    hit.symbol.qualified_name,
                    hit.score.to_bits(),
                )
            })
            .collect()
    })
    .collect();
    let lookups = ["new", "Type1::run", "PARSE", "item_3_1"]
        .iter()
        .map(|name| {
            store
                .find_symbols(name, 50)
                .unwrap()
                .iter()
                .map(|s| s.id.0)
                .collect()
        })
        .collect();
    Snapshot {
        files,
        symbols,
        stats: store.stats().unwrap(),
        edges: edges_of(store),
        searches,
        lookups,
    }
}

/// Stores files one by one.
fn store_one_by_one(
    store: &SqliteStorage,
    files: &[(FileInput, FileExtract)],
) -> Vec<UpsertOutcome> {
    files
        .iter()
        .map(|(input, parsed)| store.upsert_file(input, parsed, NOW).unwrap())
        .collect()
}

/// Resolves what the outcomes touch, the way an indexer would.
fn resolve_touched(store: &SqliteStorage, outcomes: &[UpsertOutcome]) {
    store
        .resolve_edges(&ResolveScope::Touching {
            file_ids: outcomes.iter().filter_map(|o| o.file_id).collect(),
            names: vec![],
        })
        .unwrap();
}

/// A batch returns one outcome per file, in the order of the input, with the numbers a
/// separate upsert of that file would give.
#[test]
fn outcomes_come_back_in_input_order() {
    let store = store();
    let batch: Vec<_> = (0..6)
        .map(|i| {
            let symbols: Vec<_> = (0..=i).map(|n| func(&format!("f{i}_{n}"))).collect();
            (file(&format!("src/f{i}.rs")), extract(symbols, vec![]))
        })
        .collect();
    let outcomes = store.upsert_files(&batch, NOW).unwrap();
    assert_eq!(outcomes.len(), 6);
    for (i, outcome) in outcomes.iter().enumerate() {
        assert_eq!(usize::try_from(outcome.symbols_added).unwrap(), i + 1);
        assert_eq!(outcome.changed_names.len(), i + 1);
    }
    let listed = store.list_files().unwrap();
    for (outcome, record) in outcomes.iter().zip(&listed) {
        assert_eq!(outcome.file_id, Some(record.id));
    }
    assert!(store.upsert_files(&[], NOW).unwrap().is_empty());
    assert_eq!(store.stats().unwrap().files, 6);
}

/// Storing a project as one batch gives exactly what storing its files one at a time gives:
/// outcomes, ids, listings, edges, search results and lookups.
#[test]
fn a_batch_equals_one_by_one() {
    for seed in [1, 2, 3] {
        let project = random_project(seed, 30, 12, 30);
        let (single, batch) = (store(), store());
        let expected = store_one_by_one(&single, &project);
        let outcomes = batch.upsert_files(&project, NOW).unwrap();
        assert_eq!(outcomes, expected, "seed {seed}");
        resolve_touched(&single, &expected);
        resolve_touched(&batch, &outcomes);
        assert_eq!(snapshot(&batch), snapshot(&single), "seed {seed}");
    }
}

/// A batch big enough to be a bulk load (the secondary indexes are built at the end) is still
/// equal to storing one by one, and leaves every index in place and the database sound.
#[test]
fn a_bulk_load_equals_one_by_one_and_keeps_its_indexes() {
    let project = random_project(11, 150, 30, 110);
    let rows: usize = project
        .iter()
        .map(|(_, e)| e.symbols.len() + e.references.len())
        .sum();
    assert!(rows >= 20_000, "the batch must be a bulk load");
    let (_dir, path, batch) = file_store();
    let single = store();
    let expected = store_one_by_one(&single, &project);
    let outcomes = batch.upsert_files(&project, NOW).unwrap();
    assert_eq!(outcomes, expected);
    resolve_touched(&single, &expected);
    resolve_touched(&batch, &outcomes);
    assert_eq!(snapshot(&batch), snapshot(&single));
    // A full resolve agrees with the partial one.
    batch.resolve_edges(&ResolveScope::All).unwrap();
    assert_eq!(edges_of(&batch), edges_of(&single));

    {
        let conn = Connection::open(&path).unwrap();
        let mut statement = conn
            .prepare("SELECT name FROM sqlite_schema WHERE type = 'index' AND name LIKE 'idx_%'")
            .unwrap();
        let indexes: Vec<String> = statement
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        for wanted in [
            "idx_symbols_name",
            "idx_symbols_name_nocase",
            "idx_symbols_qname_nocase",
            "idx_symbols_file_seq",
            "idx_refs_name",
            "idx_refs_file",
            "idx_edges_dst",
        ] {
            assert!(
                indexes.iter().any(|i| i == wanted),
                "{wanted} missing from {indexes:?}"
            );
        }
    }
    assert_consistent(&path);
}

/// A batch into a store that already has content (edits, new files, a path twice, a file that
/// shrinks) equals the same edits applied one at a time, memories included.
#[test]
fn edits_in_a_batch_equal_edits_one_by_one() {
    for seed in [21, 22, 23] {
        let first = random_project(seed, 20, 10, 25);
        let (single, batch) = (store(), store());
        for target in [&single, &batch] {
            store_one_by_one(target, &first);
            target.resolve_edges(&ResolveScope::All).unwrap();
            // Memories about symbols that the batch will change, remove or keep.
            for (index, (input, _)) in first.iter().enumerate().take(8) {
                let symbol = target.symbols_in_file(&input.path).unwrap()[index % 4].id;
                target
                    .add_memory(
                        &NewMemory {
                            kind: MemoryKind::Fact,
                            text: format!("note {index}"),
                            provenance: Provenance::Agent,
                            about: vec![symbol],
                        },
                        7,
                    )
                    .unwrap();
            }
        }
        let mut edits = random_project(seed + 100, 12, 9, 20);
        // Overwrite some existing paths, add some new ones, and store one path twice.
        for (i, (input, _)) in edits.iter_mut().enumerate() {
            if i < 8 {
                *input = first[i * 2].0.clone();
            }
        }
        edits.push((first[0].0.clone(), extract(vec![func("only_one")], vec![])));
        edits.push(random_file(&mut Rng(seed), 90, 3, 5));

        let expected: Vec<_> = edits
            .iter()
            .map(|(input, parsed)| single.upsert_file(input, parsed, 99).unwrap())
            .collect();
        let outcomes = batch.upsert_files(&edits, 99).unwrap();
        assert_eq!(outcomes, expected, "seed {seed}");
        assert!(outcomes.iter().any(|o| o.symbols_removed > 0));
        assert!(outcomes.iter().any(|o| o.memories_marked_stale > 0));
        resolve_touched(&single, &expected);
        resolve_touched(&batch, &outcomes);
        assert_eq!(snapshot(&batch), snapshot(&single), "seed {seed}");
        let all = MemoryFilter {
            limit: 100,
            ..MemoryFilter::default()
        };
        assert_eq!(
            batch.list_memories(&all).unwrap(),
            single.list_memories(&all).unwrap()
        );
    }
}

/// If one file of a batch fails, nothing of the batch is stored, whatever came before it.
#[test]
fn a_failing_file_rolls_back_the_whole_batch() {
    let (_dir, path, store) = file_store();
    put(
        &store,
        "src/kept.rs",
        vec![func("kept").doc("Original.")],
        vec![call("kept", 0, 1)],
    );
    let before = snapshot(&store);

    let raw = Connection::open(&path).unwrap();
    raw.execute_batch(
        "CREATE TRIGGER refuse BEFORE INSERT ON refs WHEN new.name = 'boom' \
         BEGIN SELECT RAISE(ABORT, 'refused for the test'); END;",
    )
    .unwrap();
    let batch = vec![
        (file("src/kept.rs"), extract(vec![func("changed")], vec![])),
        (
            file("src/new_one.rs"),
            extract(vec![func("fresh_one")], vec![]),
        ),
        (
            file("src/bad.rs"),
            extract(vec![func("bad")], vec![call("boom", 0, 1)]),
        ),
        (file("src/after.rs"), extract(vec![func("never")], vec![])),
    ];
    assert!(store.upsert_files(&batch, NOW + 1).is_err());
    assert_eq!(snapshot(&store), before);
    assert_consistent(&path);

    raw.execute_batch("DROP TRIGGER refuse;").unwrap();
    let outcomes = store.upsert_files(&batch, NOW + 2).unwrap();
    assert_eq!(outcomes.len(), 4);
    assert_eq!(store.stats().unwrap().files, 4);
    assert_consistent(&path);
}

/// The same failure inside a bulk load (indexes dropped and rebuilt) restores the indexes too.
#[test]
fn a_failed_bulk_load_leaves_the_schema_as_it_was() {
    let (_dir, path, store) = file_store();
    let raw = Connection::open(&path).unwrap();
    let count_indexes = |raw: &Connection| -> i64 {
        raw.query_row(
            "SELECT count(*) FROM sqlite_schema WHERE type = 'index'",
            [],
            |row| row.get(0),
        )
        .unwrap()
    };
    let indexes_before = count_indexes(&raw);
    raw.execute_batch(
        "CREATE TRIGGER refuse BEFORE INSERT ON refs WHEN new.name = 'boom' \
         BEGIN SELECT RAISE(ABORT, 'refused for the test'); END;",
    )
    .unwrap();
    let mut project = random_project(5, 150, 30, 110);
    project.push((
        file("src/bad.rs"),
        extract(vec![func("bad")], vec![call("boom", 0, 1)]),
    ));
    assert!(store.upsert_files(&project, NOW).is_err());
    assert_eq!(store.stats().unwrap().files, 0);
    assert_eq!(
        count_indexes(&raw),
        indexes_before,
        "every index is still there"
    );
    assert_consistent(&path);
    raw.execute_batch("DROP TRIGGER refuse;").unwrap();
    project.pop();
    store.upsert_files(&project, NOW).unwrap();
    assert_consistent(&path);
}

/// The derivation of a symbol id, written out again here so that a collision can be provoked.
fn derived_id(path: &str, qualified: &str, kind: &str, ordinal: u32, attempt: u32) -> i64 {
    let mut bytes = Vec::new();
    for part in [path, qualified, kind] {
        bytes.extend_from_slice(&(part.len() as u64).to_le_bytes());
        bytes.extend_from_slice(part.as_bytes());
    }
    bytes.extend_from_slice(&ordinal.to_le_bytes());
    bytes.extend_from_slice(&attempt.to_le_bytes());
    i64::try_from(hash64(&bytes) & 0x7FFF_FFFF_FFFF_FFFF).unwrap()
}

/// When the id a new symbol derives is already taken by another symbol, the next attempt is
/// used, the other symbol is untouched, and the choice is stable when the file is stored again.
#[test]
fn a_taken_id_makes_the_next_attempt() {
    let (_dir, path, store) = file_store();
    put(&store, "src/other.rs", vec![func("occupant")], vec![]);
    let taken = derived_id("src/a.rs", "target", "function", 0, 0);
    let raw = Connection::open(&path).unwrap();
    // Move the occupant onto the id that `target` in `src/a.rs` would get.
    raw.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    raw.execute(
        "UPDATE symbols SET id = ?1 WHERE name = 'occupant'",
        [taken],
    )
    .unwrap();
    drop(raw);

    // A batch of two files, so that the conflict is found while other rows are pending.
    let batch = vec![
        (
            file("src/a.rs"),
            extract(vec![func("target"), func("neighbor")], vec![]),
        ),
        (file("src/b.rs"), extract(vec![func("bystander")], vec![])),
    ];
    store.upsert_files(&batch, NOW).unwrap();
    let stored = store.symbols_in_file("src/a.rs").unwrap();
    let target = stored.iter().find(|s| s.name == "target").unwrap();
    assert_eq!(
        target.id.0,
        derived_id("src/a.rs", "target", "function", 0, 1)
    );
    assert_eq!(
        store
            .symbol(pn_ultramemory_core::SymbolId(taken))
            .unwrap()
            .unwrap()
            .name,
        "occupant"
    );
    let stable = store.upsert_files(&batch[..1], NOW + 1).unwrap();
    assert_eq!(stable[0].symbols_added, 0);
    assert_eq!(stable[0].symbols_removed, 0);
    let again = store.symbols_in_file("src/a.rs").unwrap();
    assert_eq!(
        again.iter().find(|s| s.name == "target").unwrap().id,
        target.id
    );
}

/// A parent link of a symbol whose id was moved by a collision follows the new id.
#[test]
fn children_of_a_moved_symbol_point_at_its_new_id() {
    let (_dir, path, store) = file_store();
    put(&store, "src/other.rs", vec![func("occupant")], vec![]);
    let taken = derived_id("src/a.rs", "Parent", "struct", 0, 0);
    let raw = Connection::open(&path).unwrap();
    raw.execute(
        "UPDATE symbols SET id = ?1 WHERE name = 'occupant'",
        [taken],
    )
    .unwrap();
    drop(raw);
    let symbols = vec![
        sym("Parent", SymbolKind::Struct),
        sym("Parent::child", SymbolKind::Method).parent(0),
    ];
    store
        .upsert_files(
            &[(
                file("src/a.rs"),
                extract(symbols, vec![call("child", 1, 3)]),
            )],
            NOW,
        )
        .unwrap();
    let stored = store.symbols_in_file("src/a.rs").unwrap();
    assert_ne!(stored[0].id.0, taken);
    assert_eq!(stored[1].parent, Some(stored[0].id));
}
