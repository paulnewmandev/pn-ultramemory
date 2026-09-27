// SPDX-License-Identifier: Apache-2.0
//! Tests of the language-family rule of reference resolution: a reference only resolves to
//! symbols of files in the same family, at every confidence tier, and a change of definitions in
//! one family never makes another family's references be resolved again.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // Test code: a failed check aborts the test.

mod common;

use common::{assert_consistent, call, edges_from, edges_of, file_store, func, put_in, store, sym};
use pn_ultramemory_core::{Confidence, Language, ResolveScope, Storage, SymbolKind};
use pn_ultramemory_store::SqliteStorage;
use rusqlite::Connection;

/// Resolves everything.
fn resolve_all(store: &SqliteStorage) {
    store.resolve_edges(&ResolveScope::All).unwrap();
}

/// A resolve of nothing but what the store itself remembers as changed.
fn resolve_remembered(store: &SqliteStorage) {
    store
        .resolve_edges(&ResolveScope::Touching {
            file_ids: vec![],
            names: vec![],
        })
        .unwrap();
}

/// The targets and confidences of the edges out of the first symbol of a file.
fn targets(store: &SqliteStorage, path: &str, qualified: &str) -> Vec<(String, Confidence)> {
    edges_from(store, path, qualified)
}

/// A TypeScript reference never links to a C# or Python symbol of the same name, at any tier.
#[test]
fn a_typescript_reference_never_links_to_other_languages() {
    let store = store();
    put_in(
        &store,
        "web/app.ts",
        Language::TypeScript,
        vec![func("render")],
        vec![call("Record", 0, 3), call("Item", 0, 4), call("Both", 0, 5)],
    );
    put_in(
        &store,
        "api/Record.cs",
        Language::CSharp,
        vec![sym("Record", SymbolKind::Class)],
        vec![],
    );
    put_in(
        &store,
        "tools/record.py",
        Language::Python,
        vec![sym("Record", SymbolKind::Class), func("Item")],
        vec![],
    );
    put_in(
        &store,
        "api/Item.cs",
        Language::CSharp,
        vec![func("Item"), func("Both")],
        vec![],
    );
    put_in(
        &store,
        "tools/both.py",
        Language::Python,
        vec![func("Both")],
        vec![],
    );
    resolve_all(&store);
    // Nothing in TypeScript is called `Record`, `Item` or `Both`: no edge, not even a guess
    // (`Item` and `Both` exist twice in other languages, `Record` twice as well).
    assert!(targets(&store, "web/app.ts", "render").is_empty());
    assert!(edges_of(&store).is_empty());
    // The centrality of the C# class is not inflated either.
    assert!(store.central_symbols(10, None).unwrap().is_empty());
}

/// JavaScript, TypeScript and TSX see each other, in every direction.
#[test]
fn ecmascript_languages_link_to_each_other() {
    let store = store();
    put_in(
        &store,
        "src/a.ts",
        Language::TypeScript,
        vec![func("caller_ts")],
        vec![call("from_js", 0, 1), call("from_tsx", 0, 2)],
    );
    put_in(
        &store,
        "src/b.js",
        Language::JavaScript,
        vec![func("from_js"), func("caller_js")],
        vec![call("from_ts", 1, 1)],
    );
    put_in(
        &store,
        "src/c.tsx",
        Language::Tsx,
        vec![func("from_tsx"), func("from_ts")],
        vec![],
    );
    resolve_all(&store);
    assert_eq!(
        targets(&store, "src/a.ts", "caller_ts"),
        [
            ("src/b.js::from_js".to_owned(), Confidence::Heuristic),
            ("src/c.tsx::from_tsx".to_owned(), Confidence::Heuristic)
        ]
    );
    assert_eq!(
        targets(&store, "src/b.js", "caller_js"),
        [("src/c.tsx::from_ts".to_owned(), Confidence::Heuristic)]
    );
}

/// C and C++ see each other, and neither sees Rust.
#[test]
fn c_and_cpp_link_to_each_other() {
    let store = store();
    put_in(
        &store,
        "src/main.c",
        Language::C,
        vec![func("c_main")],
        vec![call("cpp_helper", 0, 1), call("shared", 0, 2)],
    );
    put_in(
        &store,
        "src/lib.cpp",
        Language::Cpp,
        vec![func("cpp_helper"), func("cpp_main")],
        vec![call("c_main", 1, 3)],
    );
    put_in(
        &store,
        "src/lib.rs",
        Language::Rust,
        vec![func("shared")],
        vec![],
    );
    resolve_all(&store);
    assert_eq!(
        targets(&store, "src/main.c", "c_main"),
        [("src/lib.cpp::cpp_helper".to_owned(), Confidence::Heuristic)]
    );
    assert_eq!(
        targets(&store, "src/lib.cpp", "cpp_main"),
        [("src/main.c::c_main".to_owned(), Confidence::Heuristic)]
    );
}

/// "Unique" counts within the family: a name defined once in TypeScript and once in Python is
/// still unique for a TypeScript reference, and for a Python one.
#[test]
fn uniqueness_is_counted_within_the_family() {
    let store = store();
    put_in(
        &store,
        "web/a.ts",
        Language::TypeScript,
        vec![func("web_caller")],
        vec![call("parse", 0, 1)],
    );
    put_in(
        &store,
        "web/parse.ts",
        Language::TypeScript,
        vec![func("parse")],
        vec![],
    );
    put_in(
        &store,
        "py/x.py",
        Language::Python,
        vec![func("py_caller")],
        vec![call("parse", 0, 1)],
    );
    put_in(
        &store,
        "py/parse.py",
        Language::Python,
        vec![func("parse")],
        vec![],
    );
    resolve_all(&store);
    assert_eq!(
        targets(&store, "web/a.ts", "web_caller"),
        [("web/parse.ts::parse".to_owned(), Confidence::Heuristic)]
    );
    assert_eq!(
        targets(&store, "py/x.py", "py_caller"),
        [("py/parse.py::parse".to_owned(), Confidence::Heuristic)]
    );
}

/// The ambiguous tier ranks and caps candidates within the family, and a name that is
/// ambiguous only because of other languages is not ambiguous.
#[test]
fn ambiguous_ranking_stays_inside_the_family() {
    let store = store();
    put_in(
        &store,
        "app/ui/view.ts",
        Language::TypeScript,
        vec![func("draw")],
        vec![call("render", 0, 1)],
    );
    // Six TypeScript candidates, and Python ones that share a longer path prefix.
    for path in [
        "app/ui/a.ts",
        "app/ui/b.ts",
        "app/x.ts",
        "lib/y.ts",
        "lib/z.ts",
        "zzz/w.ts",
    ] {
        put_in(
            &store,
            path,
            Language::TypeScript,
            vec![func("render")],
            vec![],
        );
    }
    for path in ["app/ui/view_helpers.py", "app/ui/more.py", "app/ui/most.py"] {
        put_in(&store, path, Language::Python, vec![func("render")], vec![]);
    }
    resolve_all(&store);
    let edges = targets(&store, "app/ui/view.ts", "draw");
    assert_eq!(edges.len(), 4);
    assert!(
        edges
            .iter()
            .all(|(t, c)| t.ends_with(".ts::render") && *c == Confidence::Guess),
        "{edges:?}"
    );
    let mut paths: Vec<_> = edges.iter().map(|(t, _)| t.as_str()).collect();
    paths.sort_unstable();
    assert_eq!(
        paths,
        [
            "app/ui/a.ts::render",
            "app/ui/b.ts::render",
            "app/x.ts::render",
            "lib/y.ts::render"
        ]
    );

    // One TypeScript definition and three Python ones: unique for TypeScript.
    put_in(
        &store,
        "app/ui/other.ts",
        Language::TypeScript,
        vec![func("solo_caller")],
        vec![call("only_ts", 0, 1)],
    );
    put_in(
        &store,
        "app/only.ts",
        Language::TypeScript,
        vec![func("only_ts")],
        vec![],
    );
    for path in ["p/1.py", "p/2.py", "p/3.py"] {
        put_in(
            &store,
            path,
            Language::Python,
            vec![func("only_ts")],
            vec![],
        );
    }
    resolve_all(&store);
    assert_eq!(
        targets(&store, "app/ui/other.ts", "solo_caller"),
        [("app/only.ts::only_ts".to_owned(), Confidence::Heuristic)]
    );
}

/// A same-file match wins as before, and the family rule does not touch it.
#[test]
fn same_file_resolution_is_unaffected() {
    let store = store();
    put_in(
        &store,
        "a.py",
        Language::Python,
        vec![func("caller"), func("helper")],
        vec![call("helper", 0, 3)],
    );
    put_in(&store, "b.rs", Language::Rust, vec![func("helper")], vec![]);
    resolve_all(&store);
    assert_eq!(
        targets(&store, "a.py", "caller"),
        [("a.py::helper".to_owned(), Confidence::Resolved)]
    );
}

/// A partial resolve after any change equals a full resolve, with several families sharing
/// names.
#[test]
fn a_partial_resolve_equals_a_full_one_across_families() {
    let build = |store: &SqliteStorage, with_python_dup: bool, with_ts_dup: bool| {
        put_in(
            store,
            "web/a.ts",
            Language::TypeScript,
            vec![func("caller")],
            vec![call("dup", 0, 1), call("solo", 0, 2)],
        );
        put_in(
            store,
            "web/b.ts",
            Language::TypeScript,
            vec![func("dup"), func("solo")],
            vec![],
        );
        if with_python_dup {
            put_in(
                store,
                "py/c.py",
                Language::Python,
                vec![func("dup"), func("solo")],
                vec![],
            );
        }
        if with_ts_dup {
            put_in(
                store,
                "web/d.ts",
                Language::TypeScript,
                vec![func("dup")],
                vec![],
            );
        }
    };
    let incremental = store();
    build(&incremental, false, false);
    resolve_all(&incremental);
    // Add a definition in another family, then in the same family, then drop the first.
    put_in(
        &incremental,
        "py/c.py",
        Language::Python,
        vec![func("dup"), func("solo")],
        vec![],
    );
    resolve_remembered(&incremental);
    put_in(
        &incremental,
        "web/d.ts",
        Language::TypeScript,
        vec![func("dup")],
        vec![],
    );
    resolve_remembered(&incremental);
    incremental
        .remove_files_not_in(
            &[
                "web/a.ts".to_owned(),
                "web/b.ts".to_owned(),
                "web/d.ts".to_owned(),
            ],
            1,
        )
        .unwrap();
    resolve_remembered(&incremental);

    let fresh = store();
    build(&fresh, false, true);
    resolve_all(&fresh);
    assert_eq!(edges_of(&incremental), edges_of(&fresh));
}

/// A definition changing in one family does not make another family's references be resolved
/// again, and one changing in the reference's own family does.
#[test]
fn a_change_in_one_family_leaves_other_families_alone() {
    let (_dir, path, store) = file_store();
    put_in(
        &store,
        "web/a.ts",
        Language::TypeScript,
        vec![func("caller")],
        vec![call("dup", 0, 2)],
    );
    put_in(
        &store,
        "web/b.ts",
        Language::TypeScript,
        vec![func("dup")],
        vec![],
    );
    resolve_all(&store);
    // Plant a confidence no resolve writes (exact) on the TypeScript edge.
    let raw = Connection::open(&path).unwrap();
    raw.execute("UPDATE edges SET confidence = 3", []).unwrap();
    let confidences = || -> Vec<i64> {
        let mut statement = raw
            .prepare("SELECT confidence FROM edges ORDER BY src, dst")
            .unwrap();
        statement
            .query_map([], |row| row.get::<_, i64>(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    assert_eq!(confidences(), [3]);

    // A Python definition of the same name appears: nothing about TypeScript changed.
    put_in(
        &store,
        "py/dup.py",
        Language::Python,
        vec![func("dup")],
        vec![],
    );
    resolve_remembered(&store);
    assert_eq!(
        confidences(),
        [3],
        "the TypeScript reference must not be revisited"
    );

    // A second TypeScript definition does change it: two guesses now.
    put_in(
        &store,
        "web/c.ts",
        Language::TypeScript,
        vec![func("dup")],
        vec![],
    );
    resolve_remembered(&store);
    assert_eq!(confidences(), [0, 0]);
    assert_consistent(&path);
}

/// A file that changes language moves between families: edges into and out of it follow, and a
/// partial resolve agrees with a full one.
#[test]
fn a_file_that_changes_family_is_relinked() {
    let store = store();
    put_in(
        &store,
        "a.ts",
        Language::TypeScript,
        vec![func("caller")],
        vec![call("target", 0, 1)],
    );
    put_in(
        &store,
        "b.ts",
        Language::TypeScript,
        vec![func("target"), func("py_caller")],
        vec![call("py_target", 1, 2)],
    );
    put_in(
        &store,
        "c.py",
        Language::Python,
        vec![func("py_target")],
        vec![],
    );
    resolve_all(&store);
    assert_eq!(targets(&store, "a.ts", "caller").len(), 1);
    assert!(targets(&store, "b.ts", "py_caller").is_empty());

    // `b.ts` is now read as Python: it leaves the family of `a.ts`, and joins that of `c.py`.
    let outcome = put_in(
        &store,
        "b.ts",
        Language::Python,
        vec![func("target"), func("py_caller")],
        vec![call("py_target", 1, 2)],
    );
    store
        .resolve_edges(&ResolveScope::Touching {
            file_ids: vec![outcome.file_id.unwrap()],
            names: vec![],
        })
        .unwrap();
    assert!(targets(&store, "a.ts", "caller").is_empty());
    assert_eq!(
        targets(&store, "b.ts", "py_caller"),
        [("c.py::py_target".to_owned(), Confidence::Heuristic)]
    );
    let partial = edges_of(&store);
    resolve_all(&store);
    assert_eq!(edges_of(&store), partial);
}
