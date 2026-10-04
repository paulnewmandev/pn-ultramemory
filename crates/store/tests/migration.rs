// SPDX-License-Identifier: Apache-2.0
//! Migration tests: a database written by schema version 1 opens with this build, is brought to
//! the current version with everything it held intact and the new columns filled in, and keeps
//! working afterwards.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // Test code: a failed check aborts the test.

mod common;

use common::{assert_consistent, call, func, put};
use pn_ultramemory_core::{
    Confidence, Direction, FileTotals, Language, MemoryFilter, ModuleEdge, ResolveScope,
    SearchQuery, Storage, SymbolId,
};
use pn_ultramemory_store::SqliteStorage;
use rusqlite::Connection;

/// The SQL of schema version 1, exactly as shipped.
const VERSION_1: &str = include_str!("../migrations/0001_initial.sql");

/// The SQL that takes version 1 to version 2.
const VERSION_2: &str = include_str!("../migrations/0002_report_totals.sql");

/// The SQL that takes version 2 to version 3.
const VERSION_3: &str = include_str!("../migrations/0003_language_families.sql");

/// Creates a version 1 database with a little of everything in it, the way version 1 wrote it:
/// edges with foreign keys, references without the `no_self` flag, files without parse errors.
fn version_1_database(path: &std::path::Path) {
    let raw = Connection::open(path).unwrap();
    raw.execute_batch(VERSION_1).unwrap();
    raw.execute_batch("PRAGMA user_version = 1;").unwrap();
    // Version 1 could end up with an edge to a missing symbol only through a bug; the migration
    // must survive one, so this database has one.
    raw.execute_batch("PRAGMA foreign_keys = OFF;").unwrap();
    raw.execute_batch(
        "INSERT INTO files (id, path, language, hash, size, mtime, lines, symbol_count, indexed_at) \
         VALUES (1, 'src/a.rs', 'rust', 'h1', 10, 0, 120, 2, 5), \
                (2, 'lib/b.rs', 'rust', 'h2', 10, 0, 80, 1, 5); \
         INSERT INTO symbols (id, file_id, seq, ordinal, name, qualified_name, kind, signature, doc, \
             visibility, start_line, end_line, start_byte, end_byte, parent_id, outline, sig_hash, \
             body_hash) VALUES \
           (101, 1, 0, 0, 'save', 'save', 'function', 'fn save()', NULL, 'public', 1, 5, 0, 50, NULL, '', 1, 2), \
           (102, 1, 1, 0, 'helper', 'helper', 'function', 'fn helper()', 'Helps.', 'public', 6, 9, 51, 90, NULL, '', 3, 4), \
           (201, 2, 0, 0, 'other', 'other', 'function', 'fn other()', NULL, 'private', 1, 3, 0, 30, NULL, '', 5, 6); \
         INSERT INTO symbol_fts (rowid, name, qname, sig, doc) VALUES \
           (101, 'save', 'save', 'fn save', ''), (102, 'helper', 'helper', 'fn helper', 'helps'), \
           (201, 'other', 'other', 'fn other', ''); \
         INSERT INTO refs (file_id, owner_id, name, kind, line, qualifier) VALUES \
           (1, 101, 'save', 'call', 3, 'db'), \
           (1, 101, 'save', 'call', 4, 'self'), \
           (1, 101, 'helper', 'call', 5, NULL), \
           (1, 102, 'save', 'call', 7, 'db'), \
           (1, NULL, 'other', 'call', 8, 'x'); \
         INSERT INTO edges (src, dst, kind, confidence, line) VALUES \
           (101, 201, 'calls', 1, 3), (102, 101, 'calls', 2, 7), (999, 101, 'calls', 1, 1); \
         INSERT INTO memories (id, kind, text, provenance, created_at, stale_since) \
           VALUES (1, 'fact', 'save writes through the cache', 'agent', 7, NULL); \
         INSERT INTO memory_anchors (memory_id, ordinal, symbol_id, qualified_name, path, sig_hash, body_hash) \
           VALUES (1, 0, 101, 'save', 'src/a.rs', 1, 2); \
         INSERT INTO memory_fts (rowid, text) VALUES (1, 'save writes through the cache'); \
         INSERT INTO meta (key, value) VALUES ('root', '/repo');",
    )
    .unwrap();
}

/// Reads rows of integers with a raw query.
fn rows(path: &std::path::Path, sql: &str, columns: usize) -> Vec<Vec<i64>> {
    let raw = Connection::open(path).unwrap();
    let mut statement = raw.prepare(sql).unwrap();
    statement
        .query_map([], |row| {
            (0..columns)
                .map(|i| row.get::<_, i64>(i))
                .collect::<Result<Vec<_>, _>>()
        })
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// A version 1 database migrates cleanly: content intact, new columns backfilled, and the
/// version recorded.
#[test]
fn a_version_1_database_migrates_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.db");
    version_1_database(&path);
    assert_eq!(rows(&path, "PRAGMA user_version", 1), [[1]]);

    let store = SqliteStorage::open(&path).unwrap();
    assert_eq!(
        rows(&path, "PRAGMA user_version", 1),
        [[i64::from(SqliteStorage::SCHEMA_VERSION)]]
    );
    assert_eq!(SqliteStorage::SCHEMA_VERSION, 7);

    // What version 1 stored is all still there.
    assert_eq!(store.list_files().unwrap().len(), 2);
    assert_eq!(store.symbols_in_file("src/a.rs").unwrap().len(), 2);
    assert_eq!(store.get_meta("root").unwrap().as_deref(), Some("/repo"));
    let memories = store.list_memories(&MemoryFilter::default()).unwrap();
    assert_eq!(memories.len(), 1);
    assert_eq!(memories[0].anchors[0].symbol, Some(SymbolId(101)));
    let out = store
        .neighbors(SymbolId(101), Direction::Out, Confidence::Guess, 10)
        .unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].symbol.id, SymbolId(201));
    let hits = store
        .search_symbols(
            &SearchQuery {
                text: "save".into(),
                ..SearchQuery::default()
            },
            10,
        )
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(store.search_memories("cache", 5).unwrap().len(), 1);

    // The edge that pointed at a symbol that did not exist is gone; the others got their files.
    assert_eq!(
        rows(
            &path,
            "SELECT src, dst, src_file, dst_file FROM edges ORDER BY src",
            4
        ),
        [[101, 201, 1, 2], [102, 101, 1, 1]]
    );
    // The flag of references is what the rule says: only a call qualified by something other
    // than "this symbol" from a symbol of the same name.
    assert_eq!(
        rows(&path, "SELECT no_self FROM refs ORDER BY id", 1),
        [[1], [0], [0], [0], [0]]
    );
    // Parse errors were not known: zero.
    assert_eq!(rows(&path, "SELECT parse_errors FROM files", 1), [[0], [0]]);
    assert_eq!(
        store.file_totals().unwrap(),
        FileTotals {
            lines: 200,
            parse_error_files: 0
        }
    );
    // Edges no longer have foreign keys, and the full-text write buffer is larger.
    assert_eq!(
        rows(
            &path,
            "SELECT count(*) FROM pragma_foreign_key_list('edges')",
            1
        ),
        [[0]]
    );
    assert_eq!(
        rows(
            &path,
            "SELECT v FROM symbol_fts_config WHERE k = 'hashsize'",
            1
        ),
        [[16_777_216]]
    );
    // The reports work on migrated edges.
    assert_eq!(
        store.module_edges(1, Confidence::Heuristic, 10).unwrap(),
        [ModuleEdge {
            from: "src".into(),
            to: "lib".into(),
            weight: 1
        }]
    );
    assert_consistent(&path);
}

/// After migrating, the store behaves like a fresh one: writes, resolves, removals (the delete
/// trigger takes the edges of removed symbols) and the parse-error count all work.
#[test]
fn a_migrated_database_keeps_working() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.db");
    version_1_database(&path);
    let store = SqliteStorage::open(&path).unwrap();

    // Removing a file removes the edges that touch its symbols, through the trigger.
    store
        .remove_files_not_in(&["src/a.rs".to_owned()], 9)
        .unwrap();
    assert_eq!(rows(&path, "SELECT src, dst FROM edges", 2), [[102, 101]]);
    assert_consistent(&path);

    // New files store and resolve, with the files of both ends written into the edges.
    put(
        &store,
        "lib/c.rs",
        vec![func("cee")],
        vec![call("save", 0, 2), call("helper", 0, 3)],
    );
    store.resolve_edges(&ResolveScope::All).unwrap();
    assert_eq!(
        rows(
            &path,
            "SELECT count(*) FROM edges WHERE src_file = 0 OR dst_file = 0",
            1
        ),
        [[0]]
    );
    assert!(store.stats().unwrap().edges >= 3);

    // The parse-error count is recorded for files stored from now on.
    let mut parsed = common::extract(vec![func("broken")], vec![]);
    parsed.parse_errors = 4;
    store
        .upsert_file(&common::file("src/broken.rs"), &parsed, 10)
        .unwrap();
    assert_eq!(store.file_totals().unwrap().parse_error_files, 1);
    assert_consistent(&path);
}

/// Opening a migrated database again changes nothing, and an empty version 1 database migrates
/// too.
#[test]
fn migrating_is_idempotent_and_works_on_an_empty_database() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.db");
    version_1_database(&path);
    drop(SqliteStorage::open(&path).unwrap());
    let first = rows(
        &path,
        "SELECT src, dst, confidence, line, src_file, dst_file FROM edges ORDER BY src",
        6,
    );
    drop(SqliteStorage::open(&path).unwrap());
    assert_eq!(
        rows(
            &path,
            "SELECT src, dst, confidence, line, src_file, dst_file FROM edges ORDER BY src",
            6,
        ),
        first
    );

    let empty = dir.path().join("empty-v1.db");
    let raw = Connection::open(&empty).unwrap();
    raw.execute_batch(VERSION_1).unwrap();
    raw.execute_batch("PRAGMA user_version = 1;").unwrap();
    drop(raw);
    let store = SqliteStorage::open(&empty).unwrap();
    assert_eq!(store.stats().unwrap().files, 0);
    put(&store, "src/a.rs", vec![func("a")], vec![]);
    assert_consistent(&empty);
}

/// A version 2 database (no language families yet) migrates to version 3: every file gets the
/// family the code computes for its language, edges between families disappear, and every name
/// is marked as changed so that the next partial resolve redoes the tiers.
#[test]
fn a_version_2_database_gets_language_families() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v2.db");
    let raw = Connection::open(&path).unwrap();
    raw.execute_batch(VERSION_1).unwrap();
    raw.execute_batch(VERSION_2).unwrap();
    raw.execute_batch("PRAGMA user_version = 2; PRAGMA foreign_keys = OFF;")
        .unwrap();
    // One file per language the code knows, each with a symbol called `<language>_fn`.
    let mut languages: Vec<&str> = Language::GRAMMAR_BACKED.iter().map(|l| l.name()).collect();
    languages.extend(["kotlin", "swift", "lua"]);
    for (index, name) in languages.iter().enumerate() {
        let id = i64::try_from(index).unwrap() + 1;
        raw.execute(
            "INSERT INTO files (id, path, language, hash, size, mtime, lines, symbol_count, \
             indexed_at, parse_errors) VALUES (?1, ?2, ?3, 'h', 1, 0, 10, 1, 0, 0)",
            rusqlite::params![id, format!("f{id}.x"), name],
        )
        .unwrap();
        raw.execute(
            "INSERT INTO symbols (id, file_id, seq, ordinal, name, qualified_name, kind, \
             signature, doc, visibility, start_line, end_line, start_byte, end_byte, parent_id, \
             outline, sig_hash, body_hash) VALUES (?1, ?2, 0, 0, ?3, ?3, 'function', 'fn', NULL, \
             'public', 1, 2, 0, 5, NULL, '', 1, 1)",
            rusqlite::params![id * 10, id, format!("{name}_fn")],
        )
        .unwrap();
        raw.execute(
            "INSERT INTO symbol_fts (rowid, name, qname, sig, doc) VALUES (?1, ?2, ?2, 'fn', '')",
            rusqlite::params![id * 10, format!("{name}_fn")],
        )
        .unwrap();
    }
    // An edge from a TypeScript symbol to a JavaScript one (same family) and one to a C# one.
    let file_of =
        |name: &str| i64::try_from(languages.iter().position(|l| *l == name).unwrap()).unwrap() + 1;
    let (ts, js, cs) = (
        file_of("typescript"),
        file_of("javascript"),
        file_of("csharp"),
    );
    for (src, dst) in [(ts, js), (ts, cs)] {
        raw.execute(
            "INSERT INTO edges (src, dst, kind, confidence, line, src_file, dst_file) \
             VALUES (?1, ?2, 'calls', 1, 1, ?3, ?4)",
            rusqlite::params![src * 10, dst * 10, src, dst],
        )
        .unwrap();
    }
    drop(raw);

    let store = SqliteStorage::open(&path).unwrap();
    assert_eq!(
        rows(&path, "PRAGMA user_version", 1),
        [[i64::from(SqliteStorage::SCHEMA_VERSION)]]
    );

    // The backfilled families are exactly what the code says.
    let raw = Connection::open(&path).unwrap();
    let mut statement = raw.prepare("SELECT language, family FROM files").unwrap();
    let stored: Vec<(String, String)> = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(stored.len(), languages.len());
    for (language, family) in &stored {
        assert_eq!(
            family,
            Language::from_name(language).unwrap().family(),
            "family of {language}"
        );
    }
    // The edge across families is gone; the one inside a family stays.
    assert_eq!(
        rows(&path, "SELECT src, dst FROM edges", 2),
        [[ts * 10, js * 10]]
    );
    // Every name is marked as changed in its own family.
    assert_eq!(
        rows(&path, "SELECT count(*) FROM dirty_names", 1),
        [[i64::try_from(languages.len()).unwrap()]]
    );
    assert_consistent(&path);
    // And the store works: a partial resolve of nothing revisits what was marked.
    store
        .resolve_edges(&ResolveScope::Touching {
            file_ids: vec![],
            names: vec![],
        })
        .unwrap();
    assert_eq!(rows(&path, "SELECT count(*) FROM dirty_names", 1), [[0]]);
}

/// Version 4 adds the receiver word of a reference. A version 3 database cannot know the words of
/// what it stored, so its references keep none, which resolves them exactly as version 3 did, and
/// every file is marked as changed so that the next `index` reads it again and records them.
#[test]
fn a_version_3_database_is_marked_for_reading_again() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v3.db");
    let raw = Connection::open(&path).unwrap();
    raw.execute_batch(VERSION_1).unwrap();
    raw.execute_batch(VERSION_2).unwrap();
    raw.execute_batch(VERSION_3).unwrap();
    raw.execute_batch(
        "PRAGMA user_version = 3; \
         INSERT INTO files (id, path, language, hash, size, mtime, lines, symbol_count, \
             indexed_at, parse_errors, family) \
         VALUES (1, 'src/a.rs', 'rust', 'h1', 10, 0, 20, 1, 5, 0, 'rust'), \
                (2, 'src/b.rs', 'rust', 'h2', 10, 0, 20, 1, 5, 0, 'rust'); \
         INSERT INTO symbols (id, file_id, seq, ordinal, name, qualified_name, kind, signature, \
             doc, visibility, start_line, end_line, start_byte, end_byte, parent_id, outline, \
             sig_hash, body_hash) VALUES \
           (101, 1, 0, 0, 'run', 'run', 'function', 'fn run()', NULL, 'public', 1, 5, 0, 50, NULL, '', 1, 2), \
           (201, 2, 0, 0, 'save', 'save', 'function', 'fn save()', NULL, 'public', 1, 3, 0, 30, NULL, '', 3, 4); \
         INSERT INTO symbol_fts (rowid, name, qname, sig, doc) VALUES \
           (101, 'run', 'run', 'fn run', ''), (201, 'save', 'save', 'fn save', ''); \
         INSERT INTO refs (file_id, owner_id, name, kind, line, qualifier, no_self) \
           VALUES (1, 101, 'save', 'call', 3, 'db', 0);",
    )
    .unwrap();
    drop(raw);

    let store = SqliteStorage::open(&path).unwrap();
    assert_eq!(rows(&path, "PRAGMA user_version", 1), [[7]]);
    // Every file reads as changed, and the stored reference has no receiver word.
    assert_eq!(store.file_hash("src/a.rs").unwrap().as_deref(), Some(""));
    assert_eq!(
        rows(&path, "SELECT count(*) FROM refs WHERE recv IS NULL", 1),
        [[1]]
    );
    // Without a word, `db.save()` resolves as version 3 resolved it: the only `save`, at
    // `Heuristic`.
    store.resolve_edges(&ResolveScope::All).unwrap();
    let out = store
        .neighbors(SymbolId(101), Direction::Out, Confidence::Guess, 10)
        .unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].edge.confidence, Confidence::Heuristic);
    assert_consistent(&path);
}
