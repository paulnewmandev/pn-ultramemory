// SPDX-License-Identifier: Apache-2.0
//! Resolution that reads the receiver of a call: a method name shared with a library object must
//! not become a confident edge, and a receiver that names one of several candidates picks it.
//!
//! Each test builds the smallest repository that shows one rule, resolves everything and reads the
//! edges back with their confidence.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // Test code: a failed check aborts the test.

mod common;

use common::{call, edges_from, func, put, qualified_call, store, sym};
use pn_ultramemory_core::{
    Confidence, EXPRESSION_QUALIFIER, ResolveScope, Storage, SymbolDraft, SymbolKind,
};
use pn_ultramemory_store::SqliteStorage;

/// A class and one method inside it.
fn class_with_method(class: &str, method: &str) -> Vec<SymbolDraft> {
    vec![
        sym(class, SymbolKind::Class),
        SymbolDraft {
            parent: Some(0),
            ..sym(&format!("{class}.{method}"), SymbolKind::Method)
        },
    ]
}

/// Resolves every reference of the store.
fn resolve(store: &SqliteStorage) {
    store.resolve_edges(&ResolveScope::All).expect("resolve");
}

/// `$request->validate()` is Laravel's, not the repository's only `validate`; the call made on
/// the service that declares it is the real one.
#[test]
fn a_library_receiver_is_only_a_guess() {
    let store = store();
    put(
        &store,
        "app/Services/CouponService.php",
        class_with_method("CouponService", "validate"),
        vec![],
    );
    put(
        &store,
        "app/Http/CouponController.php",
        vec![func("store"), func("check")],
        vec![
            qualified_call("validate", "$request", 0, 3),
            qualified_call("validate", "$this->couponService", 1, 9),
        ],
    );
    resolve(&store);
    let target = "app/Services/CouponService.php::CouponService.validate".to_owned();
    assert_eq!(
        edges_from(&store, "app/Http/CouponController.php", "store"),
        [(target.clone(), Confidence::Guess)]
    );
    assert_eq!(
        edges_from(&store, "app/Http/CouponController.php", "check"),
        [(target, Confidence::Heuristic)]
    );
}

/// Of several `recall` methods, `engine.recall()` is the one declared by `Engine`, and only that
/// one is linked; an unqualified call still guesses among all of them.
#[test]
fn a_receiver_picks_one_of_several_candidates() {
    let store = store();
    put(
        &store,
        "src/core.rs",
        class_with_method("Engine", "recall"),
        vec![],
    );
    put(
        &store,
        "src/mcp.rs",
        class_with_method("Backend", "recall"),
        vec![],
    );
    put(
        &store,
        "src/main.rs",
        vec![func("run"), func("other")],
        vec![
            qualified_call("recall", "engine", 0, 2),
            call("recall", 1, 6),
        ],
    );
    resolve(&store);
    assert_eq!(
        edges_from(&store, "src/main.rs", "run"),
        [(
            "src/core.rs::Engine.recall".to_owned(),
            Confidence::Heuristic
        )]
    );
    let guessed = edges_from(&store, "src/main.rs", "other");
    assert_eq!(guessed.len(), 2);
    assert!(guessed.iter().all(|(_, c)| *c == Confidence::Guess));
}

/// A same-named method of another type in the caller's own file is not assumed when the receiver
/// names a different type.
#[test]
fn the_same_file_does_not_win_over_the_receiver() {
    let store = store();
    put(
        &store,
        "src/core.rs",
        class_with_method("Engine", "recall"),
        vec![],
    );
    let mut symbols = class_with_method("McpBackend", "recall");
    symbols.push(func("run"));
    put(
        &store,
        "src/main.rs",
        symbols,
        vec![
            qualified_call("recall", "engine", 2, 9),
            qualified_call("recall", "self", 1, 4),
        ],
    );
    resolve(&store);
    assert_eq!(
        edges_from(&store, "src/main.rs", "run"),
        [(
            "src/core.rs::Engine.recall".to_owned(),
            Confidence::Heuristic
        )]
    );
}

/// A module or file name as receiver points at the function of that file.
#[test]
fn a_module_receiver_points_at_its_file() {
    let store = store();
    put(&store, "src/utils.py", vec![func("helper")], vec![]);
    put(
        &store,
        "src/app.py",
        vec![func("main"), func("elsewhere")],
        vec![
            qualified_call("helper", "utils", 0, 2),
            qualified_call("helper", "json", 1, 5),
        ],
    );
    resolve(&store);
    assert_eq!(
        edges_from(&store, "src/app.py", "main"),
        [("src/utils.py::helper".to_owned(), Confidence::Heuristic)]
    );
    assert_eq!(
        edges_from(&store, "src/app.py", "elsewhere"),
        [("src/utils.py::helper".to_owned(), Confidence::Guess)]
    );
}

/// A call made on an expression (`items().is_empty()`) says nothing about its target, so the only
/// method of that name is a guess.
#[test]
fn an_expression_receiver_is_only_a_guess() {
    let store = store();
    put(
        &store,
        "src/candidates.rs",
        class_with_method("CandidateSet", "is_empty"),
        vec![],
    );
    put(
        &store,
        "src/agents.rs",
        vec![func("resolve")],
        vec![qualified_call("is_empty", EXPRESSION_QUALIFIER, 0, 3)],
    );
    resolve(&store);
    assert_eq!(
        edges_from(&store, "src/agents.rs", "resolve"),
        [(
            "src/candidates.rs::CandidateSet.is_empty".to_owned(),
            Confidence::Guess
        )]
    );
}

/// A Rust method declared in an `impl` away from its type has no enclosing symbol, only a
/// qualified name, and that name is enough: `engine.recall()` is `Engine::recall`, not the
/// `mod recall` that lives under the same `engine` directory.
#[test]
fn a_type_named_in_the_qualified_name_beats_a_directory() {
    let store = store();
    put(
        &store,
        "crates/engine/src/lib.rs",
        vec![sym("recall", SymbolKind::Module)],
        vec![],
    );
    put(
        &store,
        "crates/engine/src/recall.rs",
        vec![sym("Engine::recall", SymbolKind::Method)],
        vec![],
    );
    put(
        &store,
        "crates/cli/src/main.rs",
        vec![func("run")],
        vec![qualified_call("recall", "engine", 0, 4)],
    );
    resolve(&store);
    assert_eq!(
        edges_from(&store, "crates/cli/src/main.rs", "run"),
        [(
            "crates/engine/src/recall.rs::Engine::recall".to_owned(),
            Confidence::Heuristic
        )]
    );
}
