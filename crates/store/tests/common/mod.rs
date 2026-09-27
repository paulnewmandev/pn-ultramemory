// SPDX-License-Identifier: Apache-2.0
//! Shared fixtures of the integration tests: builders for symbols, references and files, and
//! helpers that read the whole graph back through the public storage port only.
//!
//! Nothing here touches SQLite directly, so the tests written with it check the behavior a caller
//! of `Storage` can observe.
#![allow(dead_code)] // Each test binary uses a different subset of these helpers.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // Test code: a failed check aborts the test.

use std::collections::BTreeSet;

use pn_ultramemory_core::{
    Confidence, Direction, FileExtract, FileInput, Language, RefKind, ReferenceDraft, Span,
    Storage, SymbolDraft, SymbolId, SymbolKind, UpsertOutcome, Visibility, hash64,
};
use pn_ultramemory_store::SqliteStorage;
use rusqlite::Connection;

/// The time used by helpers that do not care about it.
pub(crate) const NOW: i64 = 1_000;

/// Opens a fresh in-memory store.
pub(crate) fn store() -> SqliteStorage {
    SqliteStorage::open_in_memory().expect("in-memory store")
}

/// The simple name of a qualified name: what follows the last `::` or `.`.
pub(crate) fn simple_name(qualified: &str) -> &str {
    let after_colons = qualified.rsplit("::").next().unwrap_or(qualified);
    after_colons.rsplit('.').next().unwrap_or(after_colons)
}

/// Builds a public symbol with a signature and hashes derived from its name.
pub(crate) fn sym(qualified: &str, kind: SymbolKind) -> SymbolDraft {
    let name = simple_name(qualified).to_owned();
    let signature = format!("fn {name}()");
    SymbolDraft {
        sig_hash: hash64(signature.as_bytes()),
        body_hash: hash64(format!("{qualified} {{}}").as_bytes()),
        name,
        qualified_name: qualified.to_owned(),
        kind,
        signature,
        doc: None,
        visibility: Visibility::Public,
        span: Span {
            start_line: 1,
            end_line: 1,
            start_byte: 0,
            end_byte: 10,
        },
        parent: None,
        outline: Vec::new(),
    }
}

/// Builds a function symbol.
pub(crate) fn func(qualified: &str) -> SymbolDraft {
    sym(qualified, SymbolKind::Function)
}

/// Builder-style tweaks of a symbol draft.
pub(crate) trait DraftExt: Sized {
    /// Sets the documentation.
    fn doc(self, text: &str) -> Self;
    /// Sets the signature and its hash.
    fn sig(self, signature: &str) -> Self;
    /// Sets the declaration hash from a body text.
    fn body(self, text: &str) -> Self;
    /// Sets the visibility.
    fn vis(self, visibility: Visibility) -> Self;
    /// Sets the parent index.
    fn parent(self, index: usize) -> Self;
    /// Sets the first and last line.
    fn lines(self, start: u32, end: u32) -> Self;
    /// Sets the outline.
    fn outline(self, names: &[&str]) -> Self;
}

impl DraftExt for SymbolDraft {
    fn doc(mut self, text: &str) -> Self {
        self.doc = Some(text.to_owned());
        self
    }

    fn sig(mut self, signature: &str) -> Self {
        signature.clone_into(&mut self.signature);
        self.sig_hash = hash64(signature.as_bytes());
        self
    }

    fn body(mut self, text: &str) -> Self {
        self.body_hash = hash64(text.as_bytes());
        self
    }

    fn vis(mut self, visibility: Visibility) -> Self {
        self.visibility = visibility;
        self
    }

    fn parent(mut self, index: usize) -> Self {
        self.parent = Some(index);
        self
    }

    fn lines(mut self, start: u32, end: u32) -> Self {
        self.span.start_line = start;
        self.span.end_line = end;
        self
    }

    fn outline(mut self, names: &[&str]) -> Self {
        self.outline = names.iter().map(|n| (*n).to_owned()).collect();
        self
    }
}

/// Builds a call reference made by the symbol at index `owner`.
pub(crate) fn call(name: &str, owner: usize, line: u32) -> ReferenceDraft {
    ReferenceDraft {
        name: name.to_owned(),
        kind: RefKind::Call,
        line,
        owner: Some(owner),
        qualifier: None,
    }
}

/// Builds a reference of another kind.
pub(crate) fn reference(name: &str, kind: RefKind, owner: usize, line: u32) -> ReferenceDraft {
    ReferenceDraft {
        kind,
        ..call(name, owner, line)
    }
}

/// Builds a call written with a qualifier, such as `db.save()`.
pub(crate) fn qualified_call(
    name: &str,
    qualifier: &str,
    owner: usize,
    line: u32,
) -> ReferenceDraft {
    ReferenceDraft {
        qualifier: Some(qualifier.to_owned()),
        ..call(name, owner, line)
    }
}

/// Builds the file record of a path.
pub(crate) fn file(path: &str) -> FileInput {
    FileInput {
        path: path.to_owned(),
        language: Language::Rust,
        hash: format!("h-{path}"),
        size: 100,
        mtime_secs: 0,
    }
}

/// Builds an extraction of a Rust file.
pub(crate) fn extract(symbols: Vec<SymbolDraft>, references: Vec<ReferenceDraft>) -> FileExtract {
    FileExtract {
        language: Language::Rust,
        symbols,
        references,
        imports: Vec::new(),
        line_count: 100,
        parse_errors: 0,
    }
}

/// Builds the file record of a path in a given language.
pub(crate) fn file_in(path: &str, language: Language) -> FileInput {
    FileInput {
        language,
        ..file(path)
    }
}

/// Stores a file written in a given language, at [`NOW`].
pub(crate) fn put_in(
    store: &SqliteStorage,
    path: &str,
    language: Language,
    symbols: Vec<SymbolDraft>,
    references: Vec<ReferenceDraft>,
) -> UpsertOutcome {
    let parsed = FileExtract {
        language,
        ..extract(symbols, references)
    };
    store
        .upsert_file(&file_in(path, language), &parsed, NOW)
        .expect("upsert")
}

/// Stores a file with the given symbols and references, at [`NOW`].
pub(crate) fn put(
    store: &SqliteStorage,
    path: &str,
    symbols: Vec<SymbolDraft>,
    references: Vec<ReferenceDraft>,
) -> UpsertOutcome {
    put_at(store, path, symbols, references, NOW)
}

/// Stores a file with the given symbols and references at a given time.
pub(crate) fn put_at(
    store: &SqliteStorage,
    path: &str,
    symbols: Vec<SymbolDraft>,
    references: Vec<ReferenceDraft>,
    now: i64,
) -> UpsertOutcome {
    store
        .upsert_file(&file(path), &extract(symbols, references), now)
        .expect("upsert")
}

/// The id of the first symbol with a qualified name in a file.
pub(crate) fn id_of(store: &SqliteStorage, path: &str, qualified: &str) -> SymbolId {
    store
        .symbols_in_file(path)
        .expect("symbols")
        .into_iter()
        .find(|s| s.qualified_name == qualified)
        .unwrap_or_else(|| panic!("no symbol {qualified} in {path}"))
        .id
}

/// One edge described by names, for comparing graphs: source, target, kind, confidence, line.
pub(crate) type EdgeView = (String, String, &'static str, Confidence, u32);

/// Every edge of the store, read through `neighbors`, sorted.
pub(crate) fn edges_of(store: &SqliteStorage) -> Vec<EdgeView> {
    let mut all = Vec::new();
    for file in store.list_files().expect("files") {
        for symbol in store.symbols_in_file(&file.path).expect("symbols") {
            let out = store
                .neighbors(symbol.id, Direction::Out, Confidence::Guess, usize::MAX)
                .expect("neighbors");
            for neighbor in out {
                all.push((
                    format!("{}::{}", symbol.path, symbol.qualified_name),
                    format!(
                        "{}::{}",
                        neighbor.symbol.path, neighbor.symbol.qualified_name
                    ),
                    neighbor.edge.kind.as_str(),
                    neighbor.edge.confidence,
                    neighbor.edge.line,
                ));
            }
        }
    }
    all.sort();
    all
}

/// The edges of a store out of the symbol `path::qualified`, as `(target, confidence)`.
pub(crate) fn edges_from(
    store: &SqliteStorage,
    path: &str,
    qualified: &str,
) -> Vec<(String, Confidence)> {
    let id = id_of(store, path, qualified);
    store
        .neighbors(id, Direction::Out, Confidence::Guess, usize::MAX)
        .expect("neighbors")
        .into_iter()
        .map(|n| {
            (
                format!("{}::{}", n.symbol.path, n.symbol.qualified_name),
                n.edge.confidence,
            )
        })
        .collect()
}

/// A store over a file in a temporary directory, and the path of that file.
pub(crate) fn file_store() -> (tempfile::TempDir, std::path::PathBuf, SqliteStorage) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("index.db");
    let store = SqliteStorage::open(&path).unwrap();
    (dir, path, store)
}

/// Reads a set of integers with a raw query.
pub(crate) fn ids(conn: &Connection, sql: &str) -> BTreeSet<i64> {
    let mut statement = conn.prepare(sql).unwrap();
    statement
        .query_map([], |row| row.get::<_, i64>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// Runs SQLite's checks and the adapter's own invariants against the file.
pub(crate) fn assert_consistent(path: &std::path::Path) {
    let conn = Connection::open(path).unwrap();
    let integrity: String = conn
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .unwrap();
    assert_eq!(integrity, "ok");
    let violations = ids(&conn, "SELECT rowid FROM pragma_foreign_key_check");
    assert!(
        violations.is_empty(),
        "foreign key violations: {violations:?}"
    );
    // The symbol index describes exactly the symbols, and the memory index the memories.
    assert_eq!(
        ids(&conn, "SELECT id FROM symbols"),
        ids(&conn, "SELECT id FROM symbol_fts_docsize"),
        "symbol_fts rows differ from the symbols"
    );
    assert_eq!(
        ids(&conn, "SELECT id FROM memories"),
        ids(&conn, "SELECT id FROM memory_fts_docsize"),
        "memory_fts rows differ from the memories"
    );
    // Every file counts its symbols right.
    let wrong = ids(
        &conn,
        "SELECT f.id FROM files f WHERE f.symbol_count <> \
         (SELECT COUNT(*) FROM symbols s WHERE s.file_id = f.id)",
    );
    assert!(
        wrong.is_empty(),
        "files with a wrong symbol count: {wrong:?}"
    );
    // Edges only connect symbols that exist (the delete trigger keeps them so).
    let dangling = ids(
        &conn,
        "SELECT src FROM edges WHERE src NOT IN (SELECT id FROM symbols) \
         OR dst NOT IN (SELECT id FROM symbols)",
    );
    assert!(
        dangling.is_empty(),
        "edges to missing symbols: {dangling:?}"
    );
    // Parents and reference owners stay inside their file.
    let strays = ids(
        &conn,
        "SELECT s.id FROM symbols s JOIN symbols p ON p.id = s.parent_id \
         WHERE p.file_id <> s.file_id",
    );
    assert!(strays.is_empty(), "parents in other files: {strays:?}");
    let strays = ids(
        &conn,
        "SELECT r.id FROM refs r JOIN symbols o ON o.id = r.owner_id WHERE o.file_id <> r.file_id",
    );
    assert!(strays.is_empty(), "owners in other files: {strays:?}");
}

/// A small deterministic pseudo-random generator, so failures reproduce.
pub(crate) struct Rng(pub(crate) u64);

impl Rng {
    /// The next raw value.
    pub(crate) fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }

    /// A value in `0..bound` (`bound` must be positive).
    pub(crate) fn below(&mut self, bound: usize) -> usize {
        let bound = u64::try_from(bound).unwrap();
        usize::try_from(self.next() % bound).unwrap()
    }
}
