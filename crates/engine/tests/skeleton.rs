// SPDX-License-Identifier: Apache-2.0
//! End-to-end checks of the shared foundation of the engine: indexing, incremental re-indexing,
//! name resolution and safe source reading, against the real adapters.

// Test helpers outside a `#[test]` function are not covered by the `allow-*-in-tests` settings in
// `clippy.toml`, and a failed setup step in a test should stop that test loudly.
#![allow(
    clippy::expect_used,
    reason = "a failed setup step should stop the test loudly"
)]

mod common;

use common::{sample_repo, write};
use pn_ultramemory_engine::IndexOptions;

/// The first index parses every file, and a second run with no changes parses none.
#[test]
fn indexing_is_incremental() {
    let fixture = sample_repo();
    let again = fixture
        .engine
        .index(&IndexOptions::default())
        .expect("second index");
    assert_eq!(again.files_indexed, 0);
    assert_eq!(again.files_unchanged, 4);
    assert_eq!(again.files_removed, 0);
}

/// Editing one file re-parses only that file and reports what changed in it.
#[test]
fn editing_one_file_reindexes_only_it() {
    let fixture = sample_repo();
    write(
        fixture.dir.path(),
        "web/api.ts",
        "export function request(url: string): Promise<any> {\n  return fetch(url);\n}\n\
         export function ping(): void {}\n",
    );
    let report = fixture
        .engine
        .index(&IndexOptions::default())
        .expect("reindex");
    assert_eq!(report.files_indexed, 1);
    assert_eq!(report.files_unchanged, 3);
    assert!(report.symbols_added >= 1, "ping is new: {report:?}");
    assert!(
        report.symbols_removed >= 1,
        "fetchUser and User are gone: {report:?}"
    );
}

/// A deleted file leaves the index.
#[test]
fn deleted_files_are_removed() {
    let fixture = sample_repo();
    std::fs::remove_file(fixture.dir.path().join("app/server.py")).expect("delete");
    let report = fixture
        .engine
        .index(&IndexOptions::default())
        .expect("reindex");
    assert_eq!(report.files_removed, 1);
}

/// Names resolve by exact name, by id, and by file and name; ambiguity and typos explain
/// themselves.
#[test]
fn symbols_resolve_by_name_id_and_path() {
    let fixture = sample_repo();
    let engine = &fixture.engine;
    let by_name = engine.resolve_symbol("parse_config").expect("by name");
    assert_eq!(by_name.path, "src/config.rs");
    let by_id = engine
        .resolve_symbol(&by_name.id.to_string())
        .expect("by id");
    assert_eq!(by_id.id, by_name.id);
    let by_path = engine
        .resolve_symbol("src/config.rs:load_config")
        .expect("by path");
    assert_eq!(by_path.name, "load_config");
    let missing = engine.resolve_symbol("load_confg").expect_err("typo");
    assert!(missing.to_string().contains("not found"));
    let empty = engine.resolve_symbol("  ").expect_err("empty");
    assert_eq!(empty.exit_code(), 2);
}

/// A symbol's source is served only while the file still matches the index.
#[test]
fn source_is_refused_when_the_file_changed() {
    let fixture = sample_repo();
    let engine = &fixture.engine;
    let symbol = engine.resolve_symbol("load_config").expect("symbol");
    let source = engine.source_of(&symbol).expect("read").expect("source");
    assert!(source.contains("parse_config(&text)"));
    write(fixture.dir.path(), "src/config.rs", "// edited\n");
    assert!(engine.source_of(&symbol).expect("read").is_none());
}
