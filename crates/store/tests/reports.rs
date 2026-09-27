// SPDX-License-Identifier: Apache-2.0
//! Tests of the report aggregates: module sizes and boundary-crossing edges, module-to-module
//! weights, documentation coverage and file totals. Hand-built projects pin the rules; random
//! projects are compared with a straightforward reference computed from the public queries.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // Test code: a failed check aborts the test.

mod common;

use std::collections::BTreeMap;

use common::{DraftExt, NOW, Rng, call, extract, file, func, put, store, sym};
use pn_ultramemory_core::{
    Confidence, Direction, DocCoverageRow, FileTotals, Language, ModuleEdge, ModuleStats,
    ResolveScope, Storage, SymbolKind, Visibility,
};
use pn_ultramemory_store::SqliteStorage;

/// The stats of one module, as the tests spell them.
fn stats(name: &str, files: u64, symbols: u64, incoming: u64, outgoing: u64) -> ModuleStats {
    ModuleStats {
        name: name.to_owned(),
        files,
        symbols,
        incoming,
        outgoing,
    }
}

/// An edge between modules, as the tests spell it.
fn edge(from: &str, to: &str, weight: u64) -> ModuleEdge {
    ModuleEdge {
        from: from.to_owned(),
        to: to.to_owned(),
        weight,
    }
}

/// A project with root files, nested directories and edges of every confidence.
///
/// * `main.rs` (root) calls `store_open`, unique in `crates/store`: one heuristic edge.
/// * `crates/store/src/lib.rs` calls `core_parse`, `core_lex` and `core_emit`, all unique in
///   `crates/core/src/lib.rs`: three heuristic edges.
/// * `crates/store/tests/t.rs` calls `store_open`: one heuristic edge, from the tests directory.
/// * `crates/core/src/types.rs` calls `type_helper` in its own file: a resolved edge that never
///   crosses a module.
/// * `docs/guide.rs` calls `twin`, defined in two places: two guesses.
fn project() -> SqliteStorage {
    let store = store();
    put(
        &store,
        "main.rs",
        vec![func("main"), func("store_open_caller")],
        vec![call("store_open", 0, 3)],
    );
    put(&store, "build.rs", vec![func("build")], vec![]);
    put(
        &store,
        "crates/core/src/lib.rs",
        vec![
            func("core_parse"),
            func("core_lex"),
            func("core_emit"),
            func("twin"),
        ],
        vec![],
    );
    put(
        &store,
        "crates/core/src/types.rs",
        vec![func("uses_helper"), func("type_helper")],
        vec![call("type_helper", 0, 4)],
    );
    put(
        &store,
        "crates/store/src/lib.rs",
        vec![
            func("store_open"),
            func("caller_a"),
            func("caller_b"),
            func("twin"),
        ],
        vec![
            call("core_parse", 1, 2),
            call("core_lex", 2, 3),
            call("core_emit", 2, 4),
        ],
    );
    put(
        &store,
        "crates/store/tests/t.rs",
        vec![func("test_open")],
        vec![call("store_open", 0, 5)],
    );
    put(
        &store,
        "docs/guide.rs",
        vec![func("guide")],
        vec![call("twin", 0, 2)],
    );
    store.resolve_edges(&ResolveScope::All).unwrap();
    store
}

/// Module sizes at each depth, with root files as `.`, shallow paths using what directories
/// they have, and the ordering by symbols then name.
#[test]
fn module_stats_group_by_directory_depth() {
    let store = project();
    // Depth 1: `.` sends one edge into `crates`. The edges between `crates/core` and
    // `crates/store` stay inside `crates`, and the guesses of `docs` are below the threshold.
    assert_eq!(
        store.module_stats(1).unwrap(),
        [
            stats("crates", 4, 11, 1, 0),
            stats(".", 2, 3, 0, 1),
            stats("docs", 1, 1, 0, 0),
        ]
    );
    assert_eq!(
        store.module_stats(2).unwrap(),
        [
            stats("crates/core", 2, 6, 3, 0),
            stats("crates/store", 2, 5, 1, 3),
            stats(".", 2, 3, 0, 1),
            stats("docs", 1, 1, 0, 0),
        ]
    );
    // Depth 3 splits `crates/store` in its sources and its tests; `docs/guide.rs` has only one
    // directory and keeps it.
    assert_eq!(
        store.module_stats(3).unwrap(),
        [
            stats("crates/core/src", 2, 6, 3, 0),
            stats("crates/store/src", 1, 4, 2, 3),
            stats(".", 2, 3, 0, 1),
            stats("crates/store/tests", 1, 1, 0, 1),
            stats("docs", 1, 1, 0, 0),
        ]
    );
    // Depth 0 puts everything in one module, whose edges all stay inside it; a huge depth is the
    // same as the deepest directory of each file.
    assert_eq!(store.module_stats(0).unwrap(), [stats(".", 7, 15, 0, 0)]);
    assert_eq!(
        store.module_stats(usize::MAX).unwrap(),
        store.module_stats(3).unwrap()
    );
}

/// Weights between modules, heaviest first, ties by names, limited, and filtered by confidence.
#[test]
fn module_edges_are_summed_ordered_and_filtered() {
    let store = project();
    let heuristic = Confidence::Heuristic;
    assert_eq!(
        store.module_edges(2, heuristic, 10).unwrap(),
        [
            edge("crates/store", "crates/core", 3),
            edge(".", "crates/store", 1),
        ]
    );
    // With guesses: `docs` guesses both `twin` symbols.
    assert_eq!(
        store.module_edges(2, Confidence::Guess, 10).unwrap(),
        [
            edge("crates/store", "crates/core", 3),
            edge(".", "crates/store", 1),
            edge("docs", "crates/core", 1),
            edge("docs", "crates/store", 1),
        ]
    );
    // Depth 3: the tests directory now calls into the sources of the same crate.
    assert_eq!(
        store.module_edges(3, heuristic, 10).unwrap(),
        [
            edge("crates/store/src", "crates/core/src", 3),
            edge(".", "crates/store/src", 1),
            edge("crates/store/tests", "crates/store/src", 1),
        ]
    );
    assert_eq!(store.module_edges(3, heuristic, 2).unwrap().len(), 2);
    assert_eq!(store.module_edges(3, heuristic, 1).unwrap()[0].weight, 3);
    assert!(store.module_edges(3, heuristic, 0).unwrap().is_empty());
    // Resolved edges stay inside a file, so no module pair has any.
    assert!(
        store
            .module_edges(3, Confidence::Resolved, 10)
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .module_edges(0, Confidence::Guess, 10)
            .unwrap()
            .is_empty()
    );
    // Asking twice gives the same answer: nothing is left behind by a call.
    assert_eq!(
        store.module_edges(3, heuristic, 10).unwrap(),
        store.module_edges(3, heuristic, 10).unwrap()
    );
}

/// An empty index gives empty reports and zero totals.
#[test]
fn empty_index_reports_nothing() {
    let store = store();
    assert!(store.module_stats(2).unwrap().is_empty());
    assert!(
        store
            .module_edges(2, Confidence::Guess, 10)
            .unwrap()
            .is_empty()
    );
    assert!(store.doc_coverage().unwrap().is_empty());
    assert_eq!(store.file_totals().unwrap(), FileTotals::default());
}

/// Odd paths (empty directories, a leading slash, spaces, dots) are grouped by the same rule.
#[test]
fn odd_paths_are_grouped_without_failing() {
    let store = store();
    for path in [
        "a//b.rs",
        "/abs/x.rs",
        "dir with space/y.rs",
        ".hidden/z.rs",
        "no_dir.rs",
        "a/b/c/d/e.rs",
    ] {
        put(&store, path, vec![func("f")], vec![]);
    }
    let names = |depth: usize| -> Vec<String> {
        let mut names: Vec<_> = store
            .module_stats(depth)
            .unwrap()
            .into_iter()
            .map(|m| m.name)
            .collect();
        names.sort();
        names
    };
    assert_eq!(names(1), [".", ".hidden", "a", "dir with space"]);
    assert_eq!(
        names(2),
        [".", ".hidden", "/abs", "a/", "a/b", "dir with space"]
    );
    assert_eq!(names(0), ["."]);
}

/// A symbol with the given documentation, visibility and kind.
fn documented(
    name: &str,
    doc: Option<&str>,
    visibility: Visibility,
    kind: SymbolKind,
) -> pn_ultramemory_core::SymbolDraft {
    let mut symbol = sym(name, kind).vis(visibility);
    symbol.doc = doc.map(str::to_owned);
    symbol
}

/// Public symbols per language, how many are documented, modules and private ones left out,
/// whitespace-only documentation counted as missing, most public symbols first.
#[test]
fn doc_coverage_counts_public_symbols_per_language() {
    use SymbolKind::{Enum, Function, Method, Module, Struct};
    use Visibility::{Private, Public, Unknown};
    let store = store();
    put(
        &store,
        "src/a.rs",
        vec![
            documented("a1", Some("Documented."), Public, Function),
            documented("a2", None, Public, Function),
            documented("a3", Some(""), Public, Struct),
            documented("a4", Some("  \n\t "), Public, Method),
            documented("a5", Some("Documented too."), Public, Enum),
            documented("m", Some("A module."), Public, Module),
            documented("p", Some("Private."), Private, Function),
            documented("u", None, Unknown, Function),
        ],
        vec![],
    );
    let in_language = |path: &str, language: Language, symbols| {
        let mut input = file(path);
        input.language = language;
        let mut parsed = extract(symbols, vec![]);
        parsed.language = language;
        (input, parsed)
    };
    let python = in_language(
        "tools/x.py",
        Language::Python,
        vec![
            documented("py1", Some("Yes."), Public, Function),
            documented("py2", Some("Yes."), Public, Function),
        ],
    );
    let go = in_language(
        "cmd/main.go",
        Language::Go,
        vec![
            documented("g1", Some("x"), Public, Function),
            documented("g2", None, Public, Function),
        ],
    );
    store.upsert_files(&[python, go], NOW).unwrap();

    let row = |language: &str, public_symbols, documented| DocCoverageRow {
        language: language.to_owned(),
        public_symbols,
        documented,
    };
    let coverage = store.doc_coverage().unwrap();
    assert_eq!(
        coverage,
        [row("rust", 5, 2), row("go", 2, 1), row("python", 2, 2)]
    );
    // It agrees with `undocumented_public`.
    let missing = u64::try_from(store.undocumented_public(100, None).unwrap().len()).unwrap();
    let public: u64 = coverage.iter().map(|r| r.public_symbols).sum();
    let documented_total: u64 = coverage.iter().map(|r| r.documented).sum();
    assert_eq!(missing, public - documented_total);
}

/// Total lines and the number of files that had parse errors, following the latest version of
/// each file.
#[test]
fn file_totals_sum_lines_and_count_files_with_parse_errors() {
    let store = store();
    let input = |path: &str, lines: u32, errors: u32| {
        let mut parsed = extract(vec![func(&format!("f_{lines}"))], vec![]);
        parsed.line_count = lines;
        parsed.parse_errors = errors;
        store.upsert_file(&file(path), &parsed, NOW).unwrap();
    };
    input("a.rs", 100, 0);
    input("b.rs", 250, 3);
    input("c.rs", 50, 0);
    input("d.rs", 10, 1);
    assert_eq!(
        store.file_totals().unwrap(),
        FileTotals {
            lines: 410,
            parse_error_files: 2
        }
    );
    // A file that parses cleanly afterwards no longer counts; one that starts failing does.
    input("b.rs", 260, 0);
    input("c.rs", 50, 7);
    assert_eq!(
        store.file_totals().unwrap(),
        FileTotals {
            lines: 420,
            parse_error_files: 2
        }
    );
    store
        .remove_files_not_in(&["a.rs".to_owned(), "b.rs".to_owned()], NOW)
        .unwrap();
    assert_eq!(
        store.file_totals().unwrap(),
        FileTotals {
            lines: 360,
            parse_error_files: 0
        }
    );
}

/// The module of a path at a depth, spelled the obvious way.
fn reference_module(path: &str, depth: usize) -> String {
    let parts: Vec<&str> = path.split('/').collect();
    let directories = &parts[..parts.len() - 1];
    let module = directories[..depth.min(directories.len())].join("/");
    if module.is_empty() {
        ".".to_owned()
    } else {
        module
    }
}

/// Generates a random project with files at many depths and edges of every confidence.
fn random_store(seed: u64) -> SqliteStorage {
    let mut rng = Rng(seed);
    let store = store();
    let dirs = [
        "",
        "crates/a/",
        "crates/a/src/",
        "crates/b/src/deep/",
        "docs/",
        "tools/x/",
        "a//",
    ];
    let names = [
        "new", "run", "parse", "load", "save", "render", "draw", "init",
    ];
    for index in 0..40 {
        let path = format!("{}f{index}.rs", dirs[rng.below(dirs.len())]);
        let symbols: Vec<_> = (0..=rng.below(6))
            .map(|i| {
                let name = if rng.below(2) == 0 {
                    names[rng.below(names.len())].to_owned()
                } else {
                    format!("u_{index}_{i}")
                };
                let mut symbol = func(&name).lines(
                    u32::try_from(1 + i * 5).unwrap(),
                    u32::try_from(5 + i * 5).unwrap(),
                );
                if rng.below(3) == 0 {
                    symbol = symbol.vis(Visibility::Private);
                }
                if rng.below(2) == 0 {
                    symbol = symbol.doc("Documented.");
                }
                symbol
            })
            .collect();
        let count = symbols.len();
        let refs = (0..rng.below(12))
            .map(|_| {
                let name = if rng.below(2) == 0 {
                    names[rng.below(names.len())].to_owned()
                } else {
                    format!("u_{}_{}", rng.below(40), rng.below(6))
                };
                call(
                    &name,
                    rng.below(count),
                    u32::try_from(1 + rng.below(30)).unwrap(),
                )
            })
            .collect();
        put(&store, &path, symbols, refs);
    }
    store.resolve_edges(&ResolveScope::All).unwrap();
    store
}

/// Reference module stats from the public queries: files, symbols, and crossing edges.
fn reference_stats(store: &SqliteStorage, depth: usize) -> Vec<ModuleStats> {
    let mut modules: BTreeMap<String, ModuleStats> = BTreeMap::new();
    let files = store.list_files().unwrap();
    for record in &files {
        let module = modules
            .entry(reference_module(&record.path, depth))
            .or_insert_with_key(|name| stats(name, 0, 0, 0, 0));
        module.files += 1;
        module.symbols += u64::from(record.symbol_count);
    }
    for record in &files {
        for symbol in store.symbols_in_file(&record.path).unwrap() {
            let from = reference_module(&symbol.path, depth);
            for neighbor in store
                .neighbors(symbol.id, Direction::Out, Confidence::Heuristic, usize::MAX)
                .unwrap()
            {
                let to = reference_module(&neighbor.symbol.path, depth);
                if from != to {
                    modules.get_mut(&from).unwrap().outgoing += 1;
                    modules.get_mut(&to).unwrap().incoming += 1;
                }
            }
        }
    }
    let mut list: Vec<_> = modules.into_values().collect();
    list.sort_by(|a, b| b.symbols.cmp(&a.symbols).then(a.name.cmp(&b.name)));
    list
}

/// Reference module edges from the public queries.
fn reference_edges(store: &SqliteStorage, depth: usize, min: Confidence) -> Vec<ModuleEdge> {
    let mut pairs: BTreeMap<(String, String), u64> = BTreeMap::new();
    for record in store.list_files().unwrap() {
        for symbol in store.symbols_in_file(&record.path).unwrap() {
            let from = reference_module(&symbol.path, depth);
            for neighbor in store
                .neighbors(symbol.id, Direction::Out, min, usize::MAX)
                .unwrap()
            {
                let to = reference_module(&neighbor.symbol.path, depth);
                if from != to {
                    *pairs.entry((from.clone(), to)).or_default() += 1;
                }
            }
        }
    }
    let mut list: Vec<ModuleEdge> = pairs
        .into_iter()
        .map(|((from, to), weight)| ModuleEdge { from, to, weight })
        .collect();
    list.sort_by(|a, b| {
        b.weight
            .cmp(&a.weight)
            .then(a.from.cmp(&b.from))
            .then(a.to.cmp(&b.to))
    });
    list
}

/// On random projects every aggregate equals the reference computed from the public queries, at
/// several depths, confidences and limits.
#[test]
fn aggregates_match_the_reference_on_random_projects() {
    for seed in 1..=6 {
        let store = random_store(seed);
        assert!(
            store.stats().unwrap().edges > 20,
            "seed {seed} must have edges"
        );
        for depth in 0..=5 {
            assert_eq!(
                store.module_stats(depth).unwrap(),
                reference_stats(&store, depth),
                "seed {seed} depth {depth}"
            );
            for min in Confidence::ALL {
                let expected = reference_edges(&store, depth, min);
                for limit in [1, 5, 1_000] {
                    let mut wanted = expected.clone();
                    wanted.truncate(limit);
                    assert_eq!(
                        store.module_edges(depth, min, limit).unwrap(),
                        wanted,
                        "seed {seed} depth {depth} {min} limit {limit}"
                    );
                }
            }
        }
        // Documentation coverage from the symbol lists.
        let mut expected: BTreeMap<String, (u64, u64)> = BTreeMap::new();
        for record in store.list_files().unwrap() {
            for symbol in store.symbols_in_file(&record.path).unwrap() {
                if symbol.visibility == Visibility::Public && symbol.kind != SymbolKind::Module {
                    let entry = expected
                        .entry(record.language.name().to_owned())
                        .or_default();
                    entry.0 += 1;
                    entry.1 +=
                        u64::from(symbol.doc.as_deref().is_some_and(|d| !d.trim().is_empty()));
                }
            }
        }
        let mut rows: Vec<_> = expected
            .into_iter()
            .map(|(language, (public_symbols, documented))| DocCoverageRow {
                language,
                public_symbols,
                documented,
            })
            .collect();
        rows.sort_by(|a, b| {
            b.public_symbols
                .cmp(&a.public_symbols)
                .then(a.language.cmp(&b.language))
        });
        assert_eq!(store.doc_coverage().unwrap(), rows, "seed {seed}");
        let lines: u64 = store
            .list_files()
            .unwrap()
            .iter()
            .map(|f| u64::from(f.lines))
            .sum();
        assert_eq!(store.file_totals().unwrap().lines, lines);
    }
}

/// Reports do not disturb later work: the scratch tables are empty after a call.
#[test]
fn reports_leave_no_state_behind() {
    let store = project();
    let before = store.module_stats(2).unwrap();
    for depth in 0..4 {
        let _ = store.module_stats(depth).unwrap();
        let _ = store.module_edges(depth, Confidence::Guess, 5).unwrap();
    }
    assert_eq!(store.module_stats(2).unwrap(), before);
    put(
        &store,
        "extra/new.rs",
        vec![func("extra_fn")],
        vec![call("core_parse", 0, 1)],
    );
    store.resolve_edges(&ResolveScope::All).unwrap();
    assert!(
        store
            .module_stats(2)
            .unwrap()
            .iter()
            .any(|m| m.name == "extra")
    );
}
