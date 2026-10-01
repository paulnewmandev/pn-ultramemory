// SPDX-License-Identifier: Apache-2.0
//! Full-text search tests: identifier splitting, prefixes, filters, ranking, determinism, the
//! index staying in step with the symbols, and hostile query text that must never fail.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // Test code: a failed check aborts the test.

mod common;

use common::{DraftExt, func, put, store, sym};
use pn_ultramemory_core::{SearchHit, SearchQuery, Storage, SymbolDraft, SymbolKind};
use pn_ultramemory_store::SqliteStorage;

/// A query with only text.
fn text(text: &str) -> SearchQuery {
    SearchQuery {
        text: text.to_owned(),
        ..SearchQuery::default()
    }
}

/// The qualified names of the hits, in order.
fn names(hits: &[SearchHit]) -> Vec<String> {
    hits.iter()
        .map(|h| h.symbol.qualified_name.clone())
        .collect()
}

/// Searches with a generous limit.
fn search(store: &SqliteStorage, query: &SearchQuery) -> Vec<SearchHit> {
    store.search_symbols(query, 50).expect("search")
}

/// A store with a handful of configuration-related symbols.
fn config_store() -> SqliteStorage {
    let store = store();
    put(
        &store,
        "src/config.rs",
        vec![
            func("parse_config")
                .doc("Reads the settings from disk.")
                .lines(1, 5),
            func("ParseConfigFile").lines(10, 12),
            func("load_settings")
                .doc("Parse the config file and validate it.")
                .lines(20, 25),
            sym("Config", SymbolKind::Struct).lines(30, 40),
            sym("Config::path", SymbolKind::Method)
                .parent(3)
                .lines(31, 33),
            func("unrelated_helper")
                .sig("fn unrelated_helper(timeout: u64) -> bool")
                .lines(50, 52),
        ],
        vec![],
    );
    put(
        &store,
        "lib/other.rs",
        vec![
            func("parseConfig").lines(1, 3),
            func("HTTPServer").lines(5, 9),
        ],
        vec![],
    );
    store
}

/// A `camelCase` query finds `snake_case`, `PascalCase` and `camelCase` spellings of the words.
#[test]
fn camel_case_queries_find_every_spelling() {
    let store = config_store();
    let hits = search(&store, &text("parseConfig"));
    let found = names(&hits);
    for expected in [
        "parse_config",
        "ParseConfigFile",
        "parseConfig",
        "load_settings",
    ] {
        assert!(
            found.iter().any(|n| n == expected),
            "{expected} missing from {found:?}"
        );
    }
    // Names outrank a match that is only in the documentation.
    assert_eq!(found.last().map(String::as_str), Some("load_settings"));
}

/// Case, separators and spelling style of the query make no difference.
#[test]
fn spelling_style_does_not_matter() {
    let store = config_store();
    let reference = search(&store, &text("parse config"));
    for variant in [
        "parseConfig",
        "parse_config",
        "PARSE CONFIG",
        "Parse-Config",
        "  parse   config  ",
    ] {
        assert_eq!(
            search(&store, &text(variant)),
            reference,
            "variant {variant:?}"
        );
    }
    assert!(!reference.is_empty());
}

/// The last word also matches by prefix, so half-typed names find their completions.
#[test]
fn last_word_matches_by_prefix() {
    let store = config_store();
    for partial in ["conf", "confi", "config", "Con"] {
        let found = names(&search(&store, &text(partial)));
        assert!(found.contains(&"Config".to_owned()), "{partial}: {found:?}");
    }
    let found = names(&search(&store, &text("parse conf")));
    assert!(found.contains(&"parse_config".to_owned()), "{found:?}");
    // Only the last word is a prefix: `pars config` would need the exact word `pars`.
    assert!(search(&store, &text("pars config")).is_empty());
    // A prefix of a middle word does not match.
    assert!(names(&search(&store, &text("HTTPServ"))).contains(&"HTTPServer".to_owned()));
}

/// A word typed in one lowercase piece finds a camelCase identifier through its glued form.
#[test]
fn glued_lowercase_form_finds_camel_case() {
    let store = config_store();
    let found = names(&search(&store, &text("parseconfig")));
    assert!(found.contains(&"parseConfig".to_owned()), "{found:?}");
    assert!(found.contains(&"ParseConfigFile".to_owned()), "{found:?}");
    assert!(!found.contains(&"parse_config".to_owned()), "{found:?}");
    let found = names(&search(&store, &text("httpserver")));
    assert_eq!(found, ["HTTPServer"]);
}

/// Acronyms and digits split like the indexer split them.
#[test]
fn acronyms_and_digits() {
    let store = store();
    put(
        &store,
        "src/a.rs",
        vec![
            func("XMLHttpRequest"),
            func("sha256sum"),
            func("utf8_decode"),
            func("plain"),
        ],
        vec![],
    );
    assert_eq!(
        names(&search(&store, &text("xml http request"))),
        ["XMLHttpRequest"]
    );
    assert_eq!(names(&search(&store, &text("sha256"))), ["sha256sum"]);
    assert_eq!(names(&search(&store, &text("sha 256 sum"))), ["sha256sum"]);
    assert_eq!(names(&search(&store, &text("utf8"))), ["utf8_decode"]);
    assert_eq!(names(&search(&store, &text("UTF 8"))), ["utf8_decode"]);
}

/// Signatures and qualified names are searchable.
#[test]
fn searches_signatures_and_qualified_names() {
    let store = config_store();
    let found = names(&search(&store, &text("timeout u64")));
    assert_eq!(found, ["unrelated_helper"]);
    let found = names(&search(&store, &text("Config path")));
    assert_eq!(found.first().map(String::as_str), Some("Config::path"));
}

/// A match in a name ranks above one in the signature, which ranks above one in prose.
#[test]
fn columns_are_weighted() {
    let store = store();
    put(
        &store,
        "src/w.rs",
        vec![
            func("render").doc("Draws a widget on the screen."),
            func("build").sig("fn build(w: Widget)"),
            sym("Widget", SymbolKind::Struct),
            sym("Gadget::widget", SymbolKind::Method),
        ],
        vec![],
    );
    let found = names(&search(&store, &text("widget")));
    assert_eq!(found.len(), 4);
    assert_eq!(found[0], "Widget");
    let position = |name: &str| found.iter().position(|n| n == name).unwrap();
    assert!(position("Gadget::widget") < position("build"), "{found:?}");
    assert!(position("build") < position("render"), "{found:?}");
}

/// Scores are positive, and the list is in descending score order.
#[test]
fn scores_are_positive_and_descending() {
    let store = config_store();
    let hits = search(&store, &text("config"));
    assert!(hits.len() >= 4);
    for hit in &hits {
        assert!(
            hit.score.is_finite() && hit.score > 0.0,
            "score {}",
            hit.score
        );
    }
    assert!(hits.windows(2).all(|pair| pair[0].score >= pair[1].score));
}

/// The kind filter restricts results; repeated kinds are harmless.
#[test]
fn filters_by_kind() {
    let store = config_store();
    let query = SearchQuery {
        text: "config".into(),
        kinds: vec![SymbolKind::Struct],
        path_prefix: None,
        any_word: false,
    };
    assert_eq!(names(&search(&store, &query)), ["Config"]);
    let query = SearchQuery {
        kinds: vec![SymbolKind::Struct, SymbolKind::Method, SymbolKind::Struct],
        ..query
    };
    let found = names(&search(&store, &query));
    assert!(found.contains(&"Config".to_owned()) && found.contains(&"Config::path".to_owned()));
    assert!(found.iter().all(|n| n.starts_with("Config")));
    let query = SearchQuery {
        kinds: vec![SymbolKind::Macro],
        ..query
    };
    assert!(search(&store, &query).is_empty());
}

/// The path filter is a prefix of the file path.
#[test]
fn filters_by_path_prefix() {
    let store = config_store();
    let query = SearchQuery {
        text: "parse config".into(),
        kinds: vec![],
        path_prefix: Some("lib/".into()),
        any_word: false,
    };
    assert_eq!(names(&search(&store, &query)), ["parseConfig"]);
    let query = SearchQuery {
        path_prefix: Some("src/conf".into()),
        ..query
    };
    assert!(!names(&search(&store, &query)).contains(&"parseConfig".to_owned()));
    let query = SearchQuery {
        path_prefix: Some("nothing/".into()),
        ..query
    };
    assert!(search(&store, &query).is_empty());
}

/// Empty and wordless text, and a zero limit, give an empty result and no error.
#[test]
fn empty_queries_give_nothing() {
    let store = config_store();
    for blank in ["", " ", "\n\t", "()", "\"\"", "***", "---", "_"] {
        assert!(search(&store, &text(blank)).is_empty(), "{blank:?}");
    }
    assert!(store.search_symbols(&text("config"), 0).unwrap().is_empty());
    assert_eq!(store.search_symbols(&text("config"), 2).unwrap().len(), 2);
    assert!(
        store
            .search_symbols(&text("config"), usize::MAX)
            .unwrap()
            .len()
            >= 4
    );
}

/// Thirty hostile query strings: none may fail, whatever they contain.
#[test]
fn hostile_queries_never_fail() {
    let store = config_store();
    let long_word = "a".repeat(100_000);
    let many_words = "word ".repeat(20_000);
    let hostile: Vec<&str> = vec![
        "\"",
        "\"unbalanced",
        "unbalanced\"",
        "\"\"\"\"",
        "'",
        "' OR 1=1 --",
        "'; DROP TABLE symbols; --",
        "config)",
        "(config",
        "((config) OR (path",
        "config AND",
        "OR config",
        "NOT config",
        "config NOT",
        "NEAR(config path, 3)",
        "NEAR/2",
        "name:config",
        "{name doc}: config",
        "-config",
        "^config",
        "config*",
        "*",
        "**config**",
        "config OR OR path",
        "col:\"x",
        "\\",
        "%_[]",
        "\0",
        "con\0fig",
        "\u{202e}config\u{202c}",
        "\u{200b}\u{feff}",
        "\u{0903}",
        "e\u{0301}\u{0301}\u{0301}",
        "🦀🦀🦀",
        "日本語 のテスト",
        "ｃｏｎｆｉｇ",
        &long_word,
        &many_words,
    ];
    assert!(hostile.len() >= 30);
    let kinds = [vec![], vec![SymbolKind::Function, SymbolKind::Struct]];
    for input in &hostile {
        for kinds in &kinds {
            for prefix in [None, Some("src/".to_owned()), Some("\"; --".to_owned())] {
                let query = SearchQuery {
                    text: (*input).to_owned(),
                    kinds: kinds.clone(),
                    path_prefix: prefix,
                    any_word: false,
                };
                let result = store.search_symbols(&query, 20);
                assert!(result.is_ok(), "{input:.40?} failed: {result:?}");
            }
        }
    }
    // The store is intact afterwards.
    assert!(store.stats().unwrap().symbols >= 8);
}

/// A symbol whose own name looks like an attack is stored and found like any other.
#[test]
fn attack_shaped_names_are_ordinary_data() {
    let store = store();
    put(
        &store,
        "src/x.rs",
        vec![
            func("evil\") OR 1=1 --"),
            func("NEAR"),
            func("AND"),
            func("normal"),
        ],
        vec![],
    );
    assert_eq!(names(&search(&store, &text("evil"))), ["evil\") OR 1=1 --"]);
    assert_eq!(names(&search(&store, &text("near"))), ["NEAR"]);
    assert_eq!(names(&search(&store, &text("and"))), ["AND"]);
    assert_eq!(names(&search(&store, &text("evil\") OR 1=1 --"))).len(), 1);
    assert_eq!(store.stats().unwrap().symbols, 4);
}

/// Results are deterministic, and equal scores break by path, then line, then id.
#[test]
fn ties_break_deterministically() {
    let store = store();
    for path in ["src/c.rs", "src/a.rs", "src/b.rs"] {
        put(
            &store,
            path,
            vec![func("twin").lines(7, 8), func("twin_two").lines(3, 4)],
            vec![],
        );
    }
    let first = search(&store, &text("twin"));
    assert_eq!(first, search(&store, &text("twin")));
    let order: Vec<_> = first
        .iter()
        .map(|h| (h.symbol.path.as_str(), h.symbol.span.start_line))
        .collect();
    // Same name and same score: sorted by path. `twin_two` has a different score.
    let twins: Vec<_> = first.iter().filter(|h| h.symbol.name == "twin").collect();
    assert_eq!(
        twins
            .iter()
            .map(|h| h.symbol.path.as_str())
            .collect::<Vec<_>>(),
        ["src/a.rs", "src/b.rs", "src/c.rs"]
    );
    assert!(
        twins
            .windows(2)
            .all(|p| p[0].score.total_cmp(&p[1].score).is_eq())
    );
    assert_eq!(order.len(), 6);
}

/// The index follows the data: renames, identical re-upserts and removals leave no ghosts.
#[test]
fn index_stays_in_step_with_symbols() {
    let store = store();
    put(
        &store,
        "src/a.rs",
        vec![func("alpha_task"), func("beta_task")],
        vec![],
    );
    assert_eq!(search(&store, &text("task")).len(), 2);

    // Identical content again: still exactly one row per symbol.
    put(
        &store,
        "src/a.rs",
        vec![func("alpha_task"), func("beta_task")],
        vec![],
    );
    assert_eq!(search(&store, &text("task")).len(), 2);

    // A rename replaces the old spelling.
    put(
        &store,
        "src/a.rs",
        vec![func("alpha_task"), func("gamma_job")],
        vec![],
    );
    assert!(search(&store, &text("beta")).is_empty());
    assert_eq!(names(&search(&store, &text("gamma"))), ["gamma_job"]);
    assert_eq!(names(&search(&store, &text("task"))), ["alpha_task"]);

    // Documentation edits are searchable, and the old text is gone.
    put(
        &store,
        "src/a.rs",
        vec![
            func("alpha_task").doc("Frobnicates the widget."),
            func("gamma_job"),
        ],
        vec![],
    );
    assert_eq!(names(&search(&store, &text("frobnicates"))), ["alpha_task"]);
    put(
        &store,
        "src/a.rs",
        vec![func("alpha_task"), func("gamma_job")],
        vec![],
    );
    assert!(search(&store, &text("frobnicates")).is_empty());

    // Removing the file removes its rows.
    store.remove_files_not_in(&[], 1).unwrap();
    assert!(search(&store, &text("task")).is_empty());
    assert!(search(&store, &text("gamma")).is_empty());
}

/// Accents and other scripts work, on both sides of the match.
#[test]
fn unicode_text_is_searchable() {
    let store = store();
    put(
        &store,
        "src/u.rs",
        vec![func("café_menu"), func("日本語_parser"), func("Ünïcode")],
        vec![],
    );
    assert_eq!(names(&search(&store, &text("cafe"))), ["café_menu"]);
    assert_eq!(names(&search(&store, &text("café"))), ["café_menu"]);
    assert_eq!(names(&search(&store, &text("日本語"))), ["日本語_parser"]);
    assert_eq!(names(&search(&store, &text("unicode"))), ["Ünïcode"]);
}

/// Overlong documentation and names are indexed up to a bound and never break anything.
#[test]
fn oversized_text_is_bounded() {
    let store = store();
    let huge_doc = format!("{} needle_at_the_end", "filler ".repeat(50_000));
    let huge_name = "x".repeat(200_000);
    put(
        &store,
        "src/h.rs",
        vec![func("big").doc(&huge_doc), func(&huge_name)],
        vec![],
    );
    assert_eq!(names(&search(&store, &text("filler"))), ["big"]);
    assert!(search(&store, &text("needle_at_the_end")).is_empty());
    assert_eq!(search(&store, &text("big")).len(), 1);
}

/// Relevance on a bigger corpus: the exact name is first, regardless of insertion order.
#[test]
fn ranks_exact_names_first_in_a_larger_corpus() {
    let store = store();
    let mut symbols: Vec<SymbolDraft> = (0..400).map(|i| func(&format!("handler_{i}"))).collect();
    symbols.push(func("handler"));
    symbols.extend((0..100).map(|i| func(&format!("request_handler_{i}"))));
    put(&store, "src/many.rs", symbols, vec![]);
    let hits = search(&store, &text("handler"));
    assert_eq!(hits[0].symbol.name, "handler");
    let hits = store.search_symbols(&text("handler"), 5).unwrap();
    assert_eq!(hits.len(), 5);
    assert_eq!(hits[0].symbol.name, "handler");
}
