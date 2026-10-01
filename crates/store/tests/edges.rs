// SPDX-License-Identifier: Apache-2.0
//! Edge tests: the confidence rules of reference resolution, idempotence, agreement between a
//! partial and a full resolve, following edges, and centrality. The strongest test compares the
//! store with a small reference implementation of the rules on randomly generated projects.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // Test code: a failed check aborts the test.

mod common;

use common::{
    DraftExt, call, edges_from, edges_of, func, id_of, put, qualified_call, reference, store, sym,
};
use pn_ultramemory_core::{
    Confidence, Direction, EdgeKind, FileId, RefKind, ReferenceDraft, ResolveScope, Storage,
    SymbolKind,
};
use pn_ultramemory_store::SqliteStorage;

/// Resolves everything.
fn resolve_all(store: &SqliteStorage) -> u64 {
    store
        .resolve_edges(&ResolveScope::All)
        .unwrap()
        .edges_written
}

/// A reference made by symbol 0 of a file to `name`, on line 5.
fn call_from_first(name: &str) -> Vec<ReferenceDraft> {
    vec![call(name, 0, 5)]
}

/// A reference in the same file resolves with confidence `Resolved`, and nothing else is linked.
#[test]
fn same_file_reference_is_resolved() {
    let store = store();
    put(
        &store,
        "src/a.rs",
        vec![func("caller"), func("helper")],
        call_from_first("helper"),
    );
    put(&store, "src/b.rs", vec![func("helper")], vec![]);
    assert_eq!(resolve_all(&store), 1);
    assert_eq!(
        edges_from(&store, "src/a.rs", "caller"),
        [("src/a.rs::helper".to_owned(), Confidence::Resolved)]
    );
    let out = store
        .neighbors(
            id_of(&store, "src/a.rs", "caller"),
            Direction::Out,
            Confidence::Guess,
            10,
        )
        .unwrap();
    assert_eq!(out[0].edge.kind, EdgeKind::Calls);
    assert_eq!(out[0].edge.line, 5);
    assert_eq!(out[0].edge.src, id_of(&store, "src/a.rs", "caller"));
    assert_eq!(out[0].edge.dst, id_of(&store, "src/a.rs", "helper"));
}

/// A name that exists once in the whole index resolves with `Heuristic`.
#[test]
fn unique_name_is_heuristic() {
    let store = store();
    put(
        &store,
        "src/a.rs",
        vec![func("caller")],
        call_from_first("only_here"),
    );
    put(&store, "lib/b.rs", vec![func("only_here")], vec![]);
    resolve_all(&store);
    assert_eq!(
        edges_from(&store, "src/a.rs", "caller"),
        [("lib/b.rs::only_here".to_owned(), Confidence::Heuristic)]
    );
}

/// A name with several candidates is linked to at most four of them, at `Guess`, preferring the
/// paths that share the longest prefix with the referencing file.
#[test]
fn ambiguous_name_is_guessed_with_path_preference() {
    let store = store();
    put(
        &store,
        "ui/widgets/button.rs",
        vec![func("draw")],
        call_from_first("render"),
    );
    for path in [
        "ui/widgets/a.rs",
        "ui/widgets/b.rs",
        "ui/other.rs",
        "ui/x/y.rs",
        "core/z.rs",
        "misc/w.rs",
        "zzz/v.rs",
    ] {
        put(&store, path, vec![func("render")], vec![]);
    }
    resolve_all(&store);
    let edges = edges_from(&store, "ui/widgets/button.rs", "draw");
    let mut targets: Vec<_> = edges.iter().map(|(t, _)| t.as_str()).collect();
    targets.sort_unstable();
    assert_eq!(
        targets,
        [
            "ui/other.rs::render",
            "ui/widgets/a.rs::render",
            "ui/widgets/b.rs::render",
            "ui/x/y.rs::render"
        ]
    );
    assert!(edges.iter().all(|(_, c)| *c == Confidence::Guess));

    // From another directory the preference follows that directory.
    put(
        &store,
        "core/main.rs",
        vec![func("start")],
        call_from_first("render"),
    );
    resolve_all(&store);
    let edges = edges_from(&store, "core/main.rs", "start");
    assert_eq!(edges.len(), 4);
    assert_eq!(edges[0].0, "core/z.rs::render");
}

/// Names that match nothing, references without an owner and references from a missing owner
/// produce no edge.
#[test]
fn unresolved_references_produce_no_edges() {
    let store = store();
    let refs = vec![
        call("nowhere", 0, 1),
        ReferenceDraft {
            owner: None,
            ..call("target", 0, 2)
        },
        call("target", 42, 3),
    ];
    put(
        &store,
        "src/a.rs",
        vec![func("caller"), func("target")],
        refs,
    );
    assert_eq!(resolve_all(&store), 0);
    assert!(edges_of(&store).is_empty());
    assert_eq!(store.stats().unwrap().edges, 0);
}

/// Reference kinds map to edge kinds, and two kinds to one target are two edges.
#[test]
fn reference_kinds_map_to_edge_kinds() {
    let store = store();
    let refs = vec![
        call("Thing", 0, 4),
        reference("Thing", RefKind::Type, 0, 6),
        reference("Base", RefKind::Inherit, 1, 8),
    ];
    put(
        &store,
        "src/a.rs",
        vec![
            func("user"),
            sym("Derived", SymbolKind::Class),
            sym("Thing", SymbolKind::Class),
            sym("Base", SymbolKind::Class),
        ],
        refs,
    );
    resolve_all(&store);
    let edges = edges_of(&store);
    let kinds: Vec<_> = edges.iter().map(|e| (e.1.as_str(), e.2, e.4)).collect();
    assert_eq!(
        kinds,
        [
            ("src/a.rs::Base", "inherits", 8),
            ("src/a.rs::Thing", "calls", 4),
            ("src/a.rs::Thing", "uses", 6)
        ]
    );
}

/// Several references from one owner to one target become one edge, remembering the first line.
#[test]
fn repeated_references_collapse_into_one_edge() {
    let store = store();
    let refs = vec![
        call("helper", 0, 30),
        call("helper", 0, 12),
        call("helper", 0, 20),
    ];
    put(
        &store,
        "src/a.rs",
        vec![func("caller"), func("helper")],
        refs,
    );
    assert_eq!(resolve_all(&store), 1);
    let edges = edges_of(&store);
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].4, 12);
}

/// Self-links: allowed for recursion (unqualified or on `self`), refused when the reference is
/// qualified by something else.
#[test]
fn self_links_only_for_recursion() {
    let store = store();
    let refs = vec![
        call("fact", 0, 2),                   // recursion
        qualified_call("save", "self", 1, 8), // recursion through self
        qualified_call("save", "db", 2, 14),  // another object's save
    ];
    put(
        &store,
        "src/a.rs",
        vec![func("fact"), func("save").lines(6, 9), func("wrapper_save")],
        refs,
    );
    put(&store, "src/b.rs", vec![func("save")], vec![]);
    resolve_all(&store);
    assert_eq!(
        edges_from(&store, "src/a.rs", "fact"),
        [("src/a.rs::fact".to_owned(), Confidence::Resolved)]
    );
    assert_eq!(
        edges_from(&store, "src/a.rs", "save"),
        [("src/a.rs::save".to_owned(), Confidence::Resolved)]
    );
    // `wrapper_save` calls `db.save`: a call on another object, and nothing says that object is
    // either `save` of the repository, so both are only guesses. The same-file `save` is not
    // assumed, because `db` does not point at it.
    assert_eq!(
        edges_from(&store, "src/a.rs", "wrapper_save"),
        [
            ("src/a.rs::save".to_owned(), Confidence::Guess),
            ("src/b.rs::save".to_owned(), Confidence::Guess)
        ]
    );

    // A symbol that calls `db.save()` while being `save` itself must not link to itself; the
    // other `save` is the only candidate left, and `db` does not point at it either.
    let other = store_with_qualified_self_call();
    assert_eq!(
        edges_from(&other, "src/x.rs", "save"),
        [("src/y.rs::save".to_owned(), Confidence::Guess)]
    );
}

/// A `save` that calls `db.save()`, with a second `save` elsewhere.
fn store_with_qualified_self_call() -> SqliteStorage {
    let store = store();
    put(
        &store,
        "src/x.rs",
        vec![func("save")],
        vec![qualified_call("save", "db", 0, 3)],
    );
    put(&store, "src/y.rs", vec![func("save")], vec![]);
    resolve_all(&store);
    store
}

/// When the only candidate is the owner itself and the call is qualified, there is no edge.
#[test]
fn qualified_call_to_own_name_without_other_candidates_links_nothing() {
    let store = store();
    put(
        &store,
        "src/x.rs",
        vec![func("save")],
        vec![qualified_call("save", "db", 0, 3)],
    );
    resolve_all(&store);
    assert!(edges_of(&store).is_empty());
}

/// More than four same-file candidates: only the four nearest lines are linked.
#[test]
fn same_file_links_are_capped_at_four() {
    let store = store();
    let mut symbols = vec![func("caller").lines(100, 110)];
    for i in 0..7_u32 {
        symbols.push(sym(&format!("Class{i}::run"), SymbolKind::Method).lines(10 * i, 10 * i + 5));
    }
    put(&store, "src/a.rs", symbols, vec![call("run", 0, 105)]);
    resolve_all(&store);
    let edges = edges_from(&store, "src/a.rs", "caller");
    assert_eq!(edges.len(), 4);
    assert!(edges.iter().all(|(_, c)| *c == Confidence::Resolved));
    let mut names: Vec<_> = edges.into_iter().map(|(t, _)| t).collect();
    names.sort();
    assert_eq!(
        names,
        [
            "src/a.rs::Class3::run",
            "src/a.rs::Class4::run",
            "src/a.rs::Class5::run",
            "src/a.rs::Class6::run"
        ]
    );
}

/// Resolving twice writes the same edges, never duplicates, and matches the count reported.
#[test]
fn resolving_is_idempotent() {
    let store = store();
    put(
        &store,
        "src/a.rs",
        vec![func("a1"), func("a2"), func("shared")],
        vec![
            call("shared", 0, 1),
            call("far", 0, 2),
            call("shared", 1, 3),
            call("dup", 1, 4),
        ],
    );
    put(&store, "src/b.rs", vec![func("far"), func("dup")], vec![]);
    put(&store, "src/c.rs", vec![func("dup")], vec![]);
    let written = resolve_all(&store);
    let first = edges_of(&store);
    assert_eq!(usize::try_from(written).unwrap(), first.len());
    assert_eq!(resolve_all(&store), written);
    assert_eq!(edges_of(&store), first);
    assert_eq!(store.stats().unwrap().edges, written);
    let touching = store
        .resolve_edges(&ResolveScope::Touching {
            file_ids: store.list_files().unwrap().iter().map(|f| f.id).collect(),
            names: vec![],
        })
        .unwrap();
    assert_eq!(touching.edges_written, written);
    assert_eq!(edges_of(&store), first);
}

/// Resolving an empty index, or with scopes that match nothing, is a no-op and not an error.
#[test]
fn resolving_nothing_is_fine() {
    let store = store();
    assert_eq!(resolve_all(&store), 0);
    let scope = ResolveScope::Touching {
        file_ids: vec![FileId(12345)],
        names: vec!["ghost".into(), String::new()],
    };
    assert_eq!(store.resolve_edges(&scope).unwrap().edges_written, 0);
}

/// Storing a file again drops the edges that start in it and keeps the ones that end at symbols
/// that survive; a partial resolve of the file brings back the dropped ones.
#[test]
fn upsert_drops_outgoing_edges_and_keeps_incoming_ones() {
    let store = store();
    put(
        &store,
        "src/a.rs",
        vec![func("caller")],
        call_from_first("callee"),
    );
    put(
        &store,
        "src/b.rs",
        vec![func("callee"), func("other")],
        call_from_first("other"),
    );
    resolve_all(&store);
    let before = edges_of(&store);
    assert_eq!(before.len(), 2);

    let outcome = put(
        &store,
        "src/b.rs",
        vec![func("callee"), func("other")],
        call_from_first("other"),
    );
    // `callee -> other` started in b.rs and is gone; `caller -> callee` ends at a symbol that
    // survived, so it stays.
    let after: Vec<_> = edges_of(&store).into_iter().map(|e| (e.0, e.1)).collect();
    assert_eq!(
        after,
        [("src/a.rs::caller".to_owned(), "src/b.rs::callee".to_owned())]
    );
    store
        .resolve_edges(&ResolveScope::Touching {
            file_ids: vec![outcome.file_id.unwrap()],
            names: vec![],
        })
        .unwrap();
    assert_eq!(edges_of(&store), before);
}

/// Editing a symbol without adding or removing one keeps every edge into the file.
#[test]
fn modifying_symbols_keeps_incoming_edges() {
    let store = store();
    put(
        &store,
        "src/a.rs",
        vec![func("caller")],
        call_from_first("callee"),
    );
    put(
        &store,
        "src/b.rs",
        vec![func("callee"), func("spare")],
        vec![],
    );
    resolve_all(&store);
    let before = edges_of(&store);
    let outcome = put(
        &store,
        "src/b.rs",
        vec![func("callee").body("edited").lines(20, 30), func("spare")],
        vec![],
    );
    assert_eq!(outcome.symbols_modified, 1);
    assert_eq!(edges_of(&store), before);
    // Nothing needs resolving for an edit like this.
    let written = store
        .resolve_edges(&ResolveScope::Touching {
            file_ids: vec![outcome.file_id.unwrap()],
            names: vec![],
        })
        .unwrap()
        .edges_written;
    assert_eq!(written, 0);
    assert_eq!(edges_of(&store), before);
}

/// Adding a definition of a referenced name changes what the reference means, and a partial
/// resolve that is told nothing still notices.
#[test]
fn adding_a_definition_is_noticed_by_the_next_partial_resolve() {
    let store = store();
    put(
        &store,
        "src/a.rs",
        vec![func("caller")],
        call_from_first("dup"),
    );
    put(&store, "src/b.rs", vec![func("dup")], vec![]);
    resolve_all(&store);
    assert_eq!(
        edges_from(&store, "src/a.rs", "caller"),
        [("src/b.rs::dup".to_owned(), Confidence::Heuristic)]
    );
    put(&store, "src/c.rs", vec![func("dup")], vec![]);
    store
        .resolve_edges(&ResolveScope::Touching {
            file_ids: vec![],
            names: vec![],
        })
        .unwrap();
    let edges = edges_from(&store, "src/a.rs", "caller");
    assert_eq!(edges.len(), 2);
    assert!(edges.iter().all(|(_, c)| *c == Confidence::Guess));
}

/// Removing a symbol removes the edges that pointed at it, and only those.
#[test]
fn removing_a_symbol_removes_its_edges() {
    let store = store();
    put(
        &store,
        "src/a.rs",
        vec![func("caller")],
        vec![call("gone", 0, 1), call("stays", 0, 2)],
    );
    put(
        &store,
        "src/b.rs",
        vec![func("gone"), func("stays")],
        vec![],
    );
    resolve_all(&store);
    assert_eq!(edges_of(&store).len(), 2);
    put(&store, "src/b.rs", vec![func("stays")], vec![]);
    let left = edges_of(&store);
    assert!(left.iter().all(|e| e.1 == "src/b.rs::stays"), "{left:?}");
    assert_eq!(
        store.stats().unwrap().edges,
        u64::try_from(left.len()).unwrap()
    );
}

/// Neighbors come by descending confidence, then path and line; filters and limits apply.
#[test]
fn neighbors_are_ordered_and_filtered() {
    let store = store();
    let refs = vec![
        call("same_late", 0, 30),
        call("same_early", 0, 10),
        call("only_b", 0, 20),
        call("dup", 0, 40),
    ];
    put(
        &store,
        "src/a.rs",
        vec![func("caller"), func("same_late"), func("same_early")],
        refs,
    );
    put(&store, "src/b.rs", vec![func("only_b")], vec![]);
    put(&store, "x/1.rs", vec![func("dup")], vec![]);
    put(&store, "y/2.rs", vec![func("dup")], vec![]);
    resolve_all(&store);
    let caller = id_of(&store, "src/a.rs", "caller");

    let all = store
        .neighbors(caller, Direction::Out, Confidence::Guess, 100)
        .unwrap();
    let described: Vec<_> = all
        .iter()
        .map(|n| {
            (
                n.symbol.qualified_name.as_str(),
                n.edge.confidence,
                n.edge.line,
            )
        })
        .collect();
    assert_eq!(
        described,
        [
            ("same_early", Confidence::Resolved, 10),
            ("same_late", Confidence::Resolved, 30),
            ("only_b", Confidence::Heuristic, 20),
            ("dup", Confidence::Guess, 40),
            ("dup", Confidence::Guess, 40),
        ]
    );
    assert_eq!(all[3].symbol.path, "x/1.rs");
    assert_eq!(all[4].symbol.path, "y/2.rs");

    assert_eq!(
        store
            .neighbors(caller, Direction::Out, Confidence::Heuristic, 100)
            .unwrap()
            .len(),
        3
    );
    assert_eq!(
        store
            .neighbors(caller, Direction::Out, Confidence::Resolved, 100)
            .unwrap()
            .len(),
        2
    );
    assert!(
        store
            .neighbors(caller, Direction::Out, Confidence::Exact, 100)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        store
            .neighbors(caller, Direction::Out, Confidence::Guess, 1)
            .unwrap()
            .len(),
        1
    );
    assert!(
        store
            .neighbors(caller, Direction::Out, Confidence::Guess, 0)
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .neighbors(caller, Direction::In, Confidence::Guess, 100)
            .unwrap()
            .is_empty()
    );

    let target = id_of(&store, "src/a.rs", "same_early");
    let incoming = store
        .neighbors(target, Direction::In, Confidence::Guess, 100)
        .unwrap();
    assert_eq!(incoming.len(), 1);
    assert_eq!(incoming[0].symbol.id, caller);
    assert_eq!(incoming[0].edge.src, caller);
    assert_eq!(incoming[0].edge.dst, target);

    let unknown = store
        .neighbors(
            pn_ultramemory_core::SymbolId(-5),
            Direction::Out,
            Confidence::Guess,
            10,
        )
        .unwrap();
    assert!(unknown.is_empty());
}

/// Incoming neighbors order by path and line of the caller.
#[test]
fn incoming_neighbors_order_by_path_and_line() {
    let store = store();
    put(
        &store,
        "src/z.rs",
        vec![func("late_caller")],
        call_from_first("target"),
    );
    put(
        &store,
        "src/a.rs",
        vec![func("first"), func("second")],
        vec![call("target", 1, 9), call("target", 0, 3)],
    );
    put(&store, "src/t.rs", vec![func("target")], vec![]);
    resolve_all(&store);
    let target = id_of(&store, "src/t.rs", "target");
    let callers: Vec<_> = store
        .neighbors(target, Direction::In, Confidence::Guess, 10)
        .unwrap()
        .iter()
        .map(|n| {
            (
                n.symbol.path.clone(),
                n.symbol.qualified_name.clone(),
                n.edge.line,
            )
        })
        .collect();
    assert_eq!(
        callers,
        [
            ("src/a.rs".to_owned(), "first".to_owned(), 3),
            ("src/a.rs".to_owned(), "second".to_owned(), 9),
            ("src/z.rs".to_owned(), "late_caller".to_owned(), 5),
        ]
    );
}

/// Centrality counts edges that are at least heuristic, breaks ties by path and name, and
/// honors the prefix and the limit.
#[test]
fn central_symbols_rank_by_in_degree() {
    let store = store();
    put(&store, "hub/hub.rs", vec![func("hub")], vec![]);
    put(&store, "mid/mid.rs", vec![func("mid")], vec![]);
    put(&store, "mid/aaa.rs", vec![func("also_mid")], vec![]);
    put(&store, "g/g1.rs", vec![func("guessy")], vec![]);
    put(&store, "g/g2.rs", vec![func("guessy")], vec![]);
    for (i, targets) in [
        ["hub", "mid"],
        ["hub", "also_mid"],
        ["hub", "guessy"],
        ["guessy", "guessy"],
    ]
    .iter()
    .enumerate()
    {
        let refs = targets.iter().map(|t| call(t, 0, 1)).collect();
        put(
            &store,
            &format!("callers/c{i}.rs"),
            vec![func(&format!("c{i}"))],
            refs,
        );
    }
    resolve_all(&store);
    let central = store.central_symbols(10, None).unwrap();
    let summary: Vec<_> = central.iter().map(|(s, d)| (s.name.as_str(), *d)).collect();
    // `guessy` only has Guess edges, which do not count. Ties: `also_mid` (mid/aaa.rs) before
    // `mid` (mid/mid.rs).
    assert_eq!(summary, [("hub", 3), ("also_mid", 1), ("mid", 1)]);
    assert_eq!(store.central_symbols(1, None).unwrap().len(), 1);
    assert!(store.central_symbols(0, None).unwrap().is_empty());
    let scoped = store.central_symbols(10, Some("mid/")).unwrap();
    assert_eq!(scoped.len(), 2);
    assert!(
        store
            .central_symbols(10, Some("nothing"))
            .unwrap()
            .is_empty()
    );
    assert!(store.central_symbols(10, Some("g/")).unwrap().is_empty());
}

/// A partial resolve after a change equals a full resolve, for a hand-made set of edits.
#[test]
fn touching_matches_full_after_edits() {
    let build = |store: &SqliteStorage, with_dup: bool, with_target: bool| {
        put(
            store,
            "src/a.rs",
            vec![func("caller")],
            vec![call("target", 0, 1), call("dup", 0, 2)],
        );
        put(store, "src/b.rs", vec![func("dup")], vec![]);
        if with_dup {
            put(store, "src/c.rs", vec![func("dup")], vec![]);
        }
        if with_target {
            put(store, "src/d.rs", vec![func("target")], vec![]);
        }
    };
    // Scenario: start with dup ambiguous and no target; add target, drop the second dup.
    let incremental = store();
    build(&incremental, true, false);
    resolve_all(&incremental);
    let added = put(&incremental, "src/d.rs", vec![func("target")], vec![]);
    incremental
        .resolve_edges(&ResolveScope::Touching {
            file_ids: vec![added.file_id.unwrap()],
            names: added.changed_names,
        })
        .unwrap();
    incremental
        .remove_files_not_in(
            &[
                "src/a.rs".to_owned(),
                "src/b.rs".to_owned(),
                "src/d.rs".to_owned(),
            ],
            1,
        )
        .unwrap();
    // Nothing was passed: the store remembers that `dup` changed.
    incremental
        .resolve_edges(&ResolveScope::Touching {
            file_ids: vec![],
            names: vec![],
        })
        .unwrap();

    let fresh = store();
    build(&fresh, false, true);
    resolve_all(&fresh);
    assert_eq!(edges_of(&incremental), edges_of(&fresh));
    assert_eq!(
        edges_from(&incremental, "src/a.rs", "caller"),
        [
            ("src/b.rs::dup".to_owned(), Confidence::Heuristic),
            ("src/d.rs::target".to_owned(), Confidence::Heuristic)
        ]
    );
}
