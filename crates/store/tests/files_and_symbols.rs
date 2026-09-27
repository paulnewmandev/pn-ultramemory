// SPDX-License-Identifier: Apache-2.0
//! Indexing tests: storing files, stable symbol identities, the added/removed/modified
//! accounting of an upsert, removal of vanished files, and the symbol lookups.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // Test code: a failed check aborts the test.

mod common;

use common::{DraftExt, NOW, extract, file, func, id_of, put, put_at, store, sym};
use pn_ultramemory_core::{Language, Span, Storage, SymbolDraft, SymbolId, SymbolKind, Visibility};

/// A small file used by several tests: a struct, one of its methods and a free function.
fn parser_file() -> Vec<SymbolDraft> {
    vec![
        sym("Parser", SymbolKind::Struct)
            .doc("Parses text.")
            .lines(1, 20),
        sym("Parser::parse", SymbolKind::Method)
            .parent(0)
            .lines(3, 9)
            .outline(&["read", "next_token"]),
        func("helper").vis(Visibility::Private).lines(22, 30),
    ]
}

/// A stored file comes back with every field of every symbol intact, in source order.
#[test]
fn stores_and_returns_every_field() {
    let store = store();
    let symbols = parser_file();
    let outcome = put(&store, "src/a.rs", symbols.clone(), vec![]);
    assert!(outcome.file_id.is_some());
    assert_eq!(outcome.symbols_added, 3);
    assert_eq!(outcome.symbols_removed, 0);
    assert_eq!(outcome.symbols_modified, 0);
    assert_eq!(outcome.memories_marked_stale, 0);
    assert_eq!(outcome.changed_names, ["Parser", "helper", "parse"]);

    let stored = store.symbols_in_file("src/a.rs").unwrap();
    assert_eq!(stored.len(), 3);
    for (record, draft) in stored.iter().zip(&symbols) {
        assert_eq!(record.file_id, outcome.file_id.unwrap());
        assert_eq!(record.path, "src/a.rs");
        assert_eq!(record.language, Language::Rust);
        assert_eq!(record.name, draft.name);
        assert_eq!(record.qualified_name, draft.qualified_name);
        assert_eq!(record.kind, draft.kind);
        assert_eq!(record.signature, draft.signature);
        assert_eq!(record.doc, draft.doc);
        assert_eq!(record.visibility, draft.visibility);
        assert_eq!(record.span, draft.span);
        assert_eq!(record.outline, draft.outline);
        assert_eq!(record.sig_hash, draft.sig_hash);
        assert_eq!(record.body_hash, draft.body_hash);
    }
    assert_eq!(stored[0].parent, None);
    assert_eq!(stored[1].parent, Some(stored[0].id));
    assert_eq!(stored[2].parent, None);
    assert_eq!(
        store.symbol(stored[1].id).unwrap().as_ref(),
        Some(&stored[1])
    );
    assert_eq!(store.symbol(SymbolId(-1)).unwrap(), None);
    assert_eq!(store.symbol(SymbolId(i64::MAX)).unwrap(), None);
}

/// File-level accessors: hash, listing and statistics.
#[test]
fn reports_files_and_statistics() {
    let store = store();
    put(&store, "src/b.rs", vec![func("b")], vec![]);
    put(&store, "src/a.rs", parser_file(), vec![]);
    let mut python = file("scripts/x.py");
    python.language = Language::Python;
    let mut python_extract = extract(vec![func("x")], vec![]);
    python_extract.language = Language::Python;
    store.upsert_file(&python, &python_extract, NOW).unwrap();

    assert_eq!(
        store.file_hash("src/a.rs").unwrap().as_deref(),
        Some("h-src/a.rs")
    );
    assert_eq!(store.file_hash("src/none.rs").unwrap(), None);

    let files = store.list_files().unwrap();
    let paths: Vec<_> = files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, ["scripts/x.py", "src/a.rs", "src/b.rs"]);
    let a = &files[1];
    assert_eq!((a.lines, a.size, a.symbol_count), (100, 100, 3));
    assert_eq!(a.language, Language::Rust);
    assert_eq!(files[0].language, Language::Python);

    let stats = store.stats().unwrap();
    assert_eq!((stats.files, stats.symbols, stats.edges), (3, 5, 0));
    assert_eq!((stats.memories, stats.stale_memories), (0, 0));
    assert_eq!(
        stats.languages,
        [("rust".to_owned(), 2), ("python".to_owned(), 1)]
    );
}

/// Storing a path again updates the row in place: same file id, new hash and language.
#[test]
fn updating_a_file_keeps_its_identity() {
    let store = store();
    let first = put(&store, "src/a.rs", parser_file(), vec![]);
    let mut input = file("src/a.rs");
    input.hash = "new-hash".into();
    input.size = 555;
    let second = store
        .upsert_file(&input, &extract(vec![func("only")], vec![]), NOW + 1)
        .unwrap();
    assert_eq!(first.file_id, second.file_id);
    assert_eq!(
        store.file_hash("src/a.rs").unwrap().as_deref(),
        Some("new-hash")
    );
    let listed = &store.list_files().unwrap()[0];
    assert_eq!((listed.size, listed.symbol_count), (555, 1));
    assert_eq!(store.stats().unwrap().files, 1);
}

/// Storing identical content again keeps every id and reports no change.
#[test]
fn identical_upsert_keeps_ids_and_reports_nothing() {
    let store = store();
    put(&store, "src/a.rs", parser_file(), vec![]);
    let before: Vec<_> = store.symbols_in_file("src/a.rs").unwrap();
    let outcome = put(&store, "src/a.rs", parser_file(), vec![]);
    let after = store.symbols_in_file("src/a.rs").unwrap();
    assert_eq!(before, after);
    assert_eq!(
        (
            outcome.symbols_added,
            outcome.symbols_removed,
            outcome.symbols_modified
        ),
        (0, 0, 0)
    );
    assert!(outcome.changed_names.is_empty());
}

/// Ids do not change when other files are stored, edited or removed.
#[test]
fn ids_survive_edits_of_other_files() {
    let store = store();
    put(&store, "src/a.rs", parser_file(), vec![]);
    let ids: Vec<_> = store
        .symbols_in_file("src/a.rs")
        .unwrap()
        .iter()
        .map(|s| s.id)
        .collect();
    put(
        &store,
        "src/b.rs",
        vec![func("helper"), func("Parser")],
        vec![],
    );
    put(
        &store,
        "src/b.rs",
        vec![func("helper").body("changed")],
        vec![],
    );
    store
        .remove_files_not_in(&["src/a.rs".to_owned()], NOW)
        .unwrap();
    put(&store, "src/c.rs", vec![func("Parser::parse")], vec![]);
    let after: Vec<_> = store
        .symbols_in_file("src/a.rs")
        .unwrap()
        .iter()
        .map(|s| s.id)
        .collect();
    assert_eq!(ids, after);
}

/// The same content gets the same ids in an independent store: identities are derived, not counted.
#[test]
fn ids_are_deterministic_across_stores() {
    let (left, right) = (store(), store());
    put(&left, "src/a.rs", parser_file(), vec![]);
    put(&right, "src/z.rs", vec![func("noise")], vec![]);
    put(&right, "src/a.rs", parser_file(), vec![]);
    let ids = |s: &pn_ultramemory_store::SqliteStorage| -> Vec<SymbolId> {
        s.symbols_in_file("src/a.rs")
            .unwrap()
            .iter()
            .map(|x| x.id)
            .collect()
    };
    assert_eq!(ids(&left), ids(&right));
    assert!(ids(&left).iter().all(|id| id.0 >= 0));
    let mut unique = ids(&left);
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), 3);
}

/// The same qualified name in two files gives two different symbols.
#[test]
fn same_name_in_two_files_is_two_symbols() {
    let store = store();
    put(&store, "src/a.rs", vec![func("run")], vec![]);
    put(&store, "src/b.rs", vec![func("run")], vec![]);
    assert_ne!(
        id_of(&store, "src/a.rs", "run"),
        id_of(&store, "src/b.rs", "run")
    );
}

/// Overloads share a qualified name and kind; their ordinal keeps them apart and stable.
#[test]
fn overloads_are_told_apart_by_ordinal() {
    let store = store();
    let first = func("f").sig("fn f(a: i32)").lines(1, 2);
    let second = func("f").sig("fn f(a: &str)").lines(3, 4);
    put(
        &store,
        "src/o.rs",
        vec![first.clone(), second.clone()],
        vec![],
    );
    let ids: Vec<_> = store
        .symbols_in_file("src/o.rs")
        .unwrap()
        .iter()
        .map(|s| s.id)
        .collect();
    assert_ne!(ids[0], ids[1]);

    // Dropping the second overload removes exactly it.
    let outcome = put(&store, "src/o.rs", vec![first.clone()], vec![]);
    assert_eq!((outcome.symbols_added, outcome.symbols_removed), (0, 1));
    assert_eq!(store.symbols_in_file("src/o.rs").unwrap()[0].id, ids[0]);
    assert!(store.symbol(ids[1]).unwrap().is_none());

    // Adding it back restores the same id.
    put(&store, "src/o.rs", vec![first, second], vec![]);
    let again: Vec<_> = store
        .symbols_in_file("src/o.rs")
        .unwrap()
        .iter()
        .map(|s| s.id)
        .collect();
    assert_eq!(ids, again);
}

/// Added, removed and modified symbols are counted exactly, and the names are unique and sorted.
#[test]
fn accounting_of_added_removed_and_modified() {
    let store = store();
    put(
        &store,
        "src/a.rs",
        vec![func("a"), func("b"), func("c"), func("d")],
        vec![],
    );
    let outcome = put(
        &store,
        "src/a.rs",
        vec![
            func("a"),                    // unchanged
            func("b").sig("fn b(x: u8)"), // signature changed
            func("d").body("new body"),   // declaration changed
            func("e"),                    // new
        ],
        vec![],
    );
    assert_eq!(outcome.symbols_added, 1);
    assert_eq!(outcome.symbols_removed, 1);
    assert_eq!(outcome.symbols_modified, 2);
    assert_eq!(outcome.changed_names, ["b", "c", "d", "e"]);
}

/// A move within the file, a doc-only edit and a visibility edit do not count as modified.
#[test]
fn only_hash_changes_count_as_modified() {
    let store = store();
    put(
        &store,
        "src/a.rs",
        vec![func("a").lines(1, 3), func("b").lines(5, 7)],
        vec![],
    );
    let outcome = put(
        &store,
        "src/a.rs",
        vec![
            func("a").lines(11, 13).doc("now documented"),
            func("b").lines(15, 17).vis(Visibility::Private),
        ],
        vec![],
    );
    assert_eq!(outcome.symbols_modified, 0);
    assert!(outcome.changed_names.is_empty());
    let stored = store.symbols_in_file("src/a.rs").unwrap();
    assert_eq!(stored[0].span.start_line, 11);
    assert_eq!(stored[0].doc.as_deref(), Some("now documented"));
    assert_eq!(stored[1].visibility, Visibility::Private);
}

/// Changing the kind of a symbol replaces it: the old one is removed and a new one added.
#[test]
fn changing_the_kind_replaces_the_symbol() {
    let store = store();
    put(
        &store,
        "src/a.rs",
        vec![sym("Thing", SymbolKind::Struct)],
        vec![],
    );
    let old = id_of(&store, "src/a.rs", "Thing");
    let outcome = put(
        &store,
        "src/a.rs",
        vec![sym("Thing", SymbolKind::Enum)],
        vec![],
    );
    assert_eq!((outcome.symbols_added, outcome.symbols_removed), (1, 1));
    assert_eq!(outcome.changed_names, ["Thing"]);
    assert_ne!(id_of(&store, "src/a.rs", "Thing"), old);
}

/// Two symbols with the same simple name appear once in the changed names.
#[test]
fn changed_names_are_deduplicated() {
    let store = store();
    let outcome = put(
        &store,
        "src/a.rs",
        vec![
            sym("A", SymbolKind::Struct),
            sym("A::new", SymbolKind::Method),
            sym("B", SymbolKind::Struct),
            sym("B::new", SymbolKind::Method),
        ],
        vec![],
    );
    assert_eq!(outcome.changed_names, ["A", "B", "new"]);
    assert_eq!(outcome.symbols_added, 4);
}

/// Every stored symbol of a removed file disappears with the file, and only those.
#[test]
fn removes_files_that_are_gone() {
    let store = store();
    put(&store, "src/a.rs", parser_file(), vec![]);
    put(&store, "src/b.rs", vec![func("b")], vec![]);
    put(&store, "src/c.rs", vec![func("c")], vec![]);
    let doomed = id_of(&store, "src/b.rs", "b");

    let removed = store
        .remove_files_not_in(&["src/a.rs".to_owned(), "src/zzz.rs".to_owned()], NOW)
        .unwrap();
    assert_eq!(removed, 2);
    assert_eq!(store.list_files().unwrap().len(), 1);
    assert!(store.symbols_in_file("src/b.rs").unwrap().is_empty());
    assert!(store.symbol(doomed).unwrap().is_none());
    assert_eq!(store.file_hash("src/b.rs").unwrap(), None);
    assert_eq!(store.stats().unwrap().symbols, 3);

    assert_eq!(
        store
            .remove_files_not_in(&["src/a.rs".to_owned()], NOW)
            .unwrap(),
        0
    );
    assert_eq!(store.remove_files_not_in(&[], NOW).unwrap(), 1);
    assert_eq!(store.stats().unwrap().files, 0);
    assert_eq!(store.stats().unwrap().symbols, 0);
}

/// An extractor that reports nonsense parents and owners cannot break the store.
#[test]
fn hostile_parents_and_owners_are_dropped() {
    let store = store();
    let mut symbols = vec![
        func("a").parent(0), // itself
        func("b").parent(5), // out of range
        func("c").parent(3), // forward
        func("d").parent(0), // valid: earlier symbol
    ];
    symbols[3].parent = Some(0);
    let refs = vec![
        common::call("x", 99, 1),
        common::call("y", usize::MAX, 2),
        common::call("z", 0, 3),
    ];
    put(&store, "src/a.rs", symbols, refs);
    let stored = store.symbols_in_file("src/a.rs").unwrap();
    assert_eq!(stored[0].parent, None);
    assert_eq!(stored[1].parent, None);
    assert_eq!(stored[2].parent, None);
    assert_eq!(stored[3].parent, Some(stored[0].id));
}

/// Odd strings and extreme numbers round-trip unchanged.
#[test]
fn awkward_values_round_trip() {
    let store = store();
    let mut odd = sym("we\"ird::na'me;--", SymbolKind::Other)
        .doc("line one\nline two\0nul \u{1F980} \\ \"quoted\"")
        .sig("fn f(\u{202e}x: 日本語)")
        .outline(&["a\nb", "back\\slash", "", "\\n", "ünï"]);
    odd.sig_hash = u64::MAX;
    odd.body_hash = 1 << 63;
    odd.span = Span {
        start_line: u32::MAX,
        end_line: u32::MAX,
        start_byte: u32::MAX,
        end_byte: u32::MAX,
    };
    let mut zero = func("zero").doc("");
    zero.sig_hash = 0;
    zero.body_hash = 0;
    put(
        &store,
        "we ird/päth with spaces.rs",
        vec![odd.clone(), zero.clone()],
        vec![],
    );
    let stored = store.symbols_in_file("we ird/päth with spaces.rs").unwrap();
    assert_eq!(stored[0].qualified_name, odd.qualified_name);
    assert_eq!(stored[0].doc, odd.doc);
    assert_eq!(stored[0].signature, odd.signature);
    assert_eq!(stored[0].outline, odd.outline);
    assert_eq!(stored[0].sig_hash, u64::MAX);
    assert_eq!(stored[0].body_hash, 1 << 63);
    assert_eq!(stored[0].span, odd.span);
    assert_eq!(stored[1].doc.as_deref(), Some(""));
    assert_eq!((stored[1].sig_hash, stored[1].body_hash), (0, 0));
}

/// Languages without a grammar (the lexical fallback ones) round-trip by name too.
#[test]
fn fallback_languages_round_trip() {
    let store = store();
    let mut input = file("app/Main.kt");
    input.language = Language::Other("kotlin");
    let mut parsed = extract(vec![func("main")], vec![]);
    parsed.language = Language::Other("kotlin");
    store.upsert_file(&input, &parsed, NOW).unwrap();
    assert_eq!(
        store.symbols_in_file("app/Main.kt").unwrap()[0].language,
        Language::Other("kotlin")
    );
    assert_eq!(
        store.list_files().unwrap()[0].language,
        Language::Other("kotlin")
    );
    assert_eq!(store.stats().unwrap().languages, [("kotlin".to_owned(), 1)]);
}

/// An empty extraction stores an empty file and later removes the old symbols.
#[test]
fn empty_extraction_clears_the_file() {
    let store = store();
    put(&store, "src/a.rs", parser_file(), vec![]);
    let outcome = put(&store, "src/a.rs", vec![], vec![]);
    assert_eq!(outcome.symbols_removed, 3);
    assert!(store.symbols_in_file("src/a.rs").unwrap().is_empty());
    assert_eq!(store.list_files().unwrap()[0].symbol_count, 0);
    assert_eq!(outcome.changed_names, ["Parser", "helper", "parse"]);
}

/// A large file goes through in one call.
#[test]
fn stores_a_large_file() {
    let store = store();
    let symbols: Vec<_> = (0..2_000)
        .map(|i| func(&format!("f{i}")).lines(i, i + 1))
        .collect();
    let outcome = put(&store, "src/big.rs", symbols, vec![]);
    assert_eq!(outcome.symbols_added, 2_000);
    assert_eq!(store.stats().unwrap().symbols, 2_000);
    assert_eq!(store.symbols_in_file("src/big.rs").unwrap().len(), 2_000);
}

/// Looking up by name: exact matches first, then case-only differences, then ordered by path.
#[test]
fn find_symbols_orders_exact_before_case_insensitive() {
    let store = store();
    put(&store, "src/a.rs", vec![func("Parse")], vec![]);
    put(&store, "src/b.rs", vec![func("parse")], vec![]);
    put(
        &store,
        "src/c.rs",
        vec![
            sym("Config", SymbolKind::Struct),
            sym("Config::parse", SymbolKind::Method).parent(0),
        ],
        vec![],
    );
    put(&store, "src/d.rs", vec![func("PARSE")], vec![]);

    let found = store.find_symbols("parse", 10).unwrap();
    let described: Vec<_> = found
        .iter()
        .map(|s| format!("{}:{}", s.path, s.qualified_name))
        .collect();
    assert_eq!(
        described,
        [
            "src/b.rs:parse",
            "src/c.rs:Config::parse",
            "src/a.rs:Parse",
            "src/d.rs:PARSE"
        ]
    );

    let qualified = store.find_symbols("Config::parse", 10).unwrap();
    assert_eq!(qualified.len(), 1);
    assert_eq!(qualified[0].name, "parse");
    let insensitive = store.find_symbols("config::PARSE", 10).unwrap();
    assert_eq!(insensitive.len(), 1);

    assert_eq!(store.find_symbols("parse", 1).unwrap().len(), 1);
    assert!(store.find_symbols("parse", 0).unwrap().is_empty());
    assert!(store.find_symbols("missing", 10).unwrap().is_empty());
    assert!(store.find_symbols("", 10).unwrap().is_empty());
    assert!(store.find_symbols("' OR 1=1 --", 10).unwrap().is_empty());
    assert_eq!(store.find_symbols("parse", usize::MAX).unwrap().len(), 4);
}

/// Undocumented public symbols: only public, documented ones and modules are skipped.
#[test]
fn undocumented_public_lists_the_gaps() {
    let store = store();
    put(
        &store,
        "src/a.rs",
        vec![
            sym("m", SymbolKind::Module),               // module: skipped
            func("documented").doc("Has docs."),        // documented: skipped
            func("bare").lines(10, 12),                 // listed
            func("blank").doc("  \n\t ").lines(20, 22), // whitespace doc counts as missing
            func("hidden").vis(Visibility::Private),    // private: skipped
            func("unsure").vis(Visibility::Unknown),    // unknown: skipped
        ],
        vec![],
    );
    put(&store, "lib/b.rs", vec![func("also_bare")], vec![]);
    let all = store.undocumented_public(10, None).unwrap();
    let names: Vec<_> = all.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["also_bare", "bare", "blank"]);
    let scoped = store.undocumented_public(10, Some("src/")).unwrap();
    assert_eq!(scoped.len(), 2);
    assert_eq!(store.undocumented_public(1, None).unwrap().len(), 1);
    assert!(store.undocumented_public(0, None).unwrap().is_empty());
    assert!(
        store
            .undocumented_public(10, Some("nope/"))
            .unwrap()
            .is_empty()
    );
}

/// A path prefix is a literal prefix: pattern characters in it mean nothing special.
#[test]
fn path_prefixes_are_literal() {
    let store = store();
    put(&store, "src/a_b.rs", vec![func("one")], vec![]);
    put(&store, "src/axb.rs", vec![func("two")], vec![]);
    put(&store, "src/100%.rs", vec![func("three")], vec![]);
    assert_eq!(
        store.undocumented_public(10, Some("src/a_")).unwrap().len(),
        1
    );
    assert_eq!(
        store.undocumented_public(10, Some("src/%")).unwrap().len(),
        0
    );
    assert_eq!(
        store
            .undocumented_public(10, Some("src/100%"))
            .unwrap()
            .len(),
        1
    );
    assert_eq!(store.undocumented_public(10, Some("")).unwrap().len(), 3);
}

/// Storing at a later time does not change what an upsert reports for identical content.
#[test]
fn time_does_not_affect_identity() {
    let store = store();
    put_at(&store, "src/a.rs", parser_file(), vec![], 1);
    let first = store.symbols_in_file("src/a.rs").unwrap();
    put_at(&store, "src/a.rs", parser_file(), vec![], 9_999_999);
    assert_eq!(first, store.symbols_in_file("src/a.rs").unwrap());
}
