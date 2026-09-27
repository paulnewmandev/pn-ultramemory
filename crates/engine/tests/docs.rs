// SPDX-License-Identifier: Apache-2.0
//! End-to-end checks of the documentation operations against the real SQLite, tree-sitter and file
//! system adapters: finding the gaps, reading a batch written in either format, writing it into
//! Rust, Python and TypeScript sources, and building a Markdown reference.
//!
//! The round trip is the point of this file: a gap is listed, a description is written for it, the
//! text lands in the source in that language's own comment syntax, the file is indexed again, and
//! the gap is gone.

// Test helpers outside a `#[test]` function are not covered by the `allow-*-in-tests` settings in
// `clippy.toml`, and a failed setup step in a test should stop that test loudly.
#![allow(
    clippy::expect_used,
    reason = "a failed setup step should stop the test loudly"
)]

mod common;

use common::{Fixture, sample_repo, write};
use pn_ultramemory_codec::estimate_tokens;
use pn_ultramemory_engine::{
    DocEntry, DocGapQuery, Engine, IndexOptions, doc_gaps_to_value, parse_doc_entries,
};

/// Every gap of the fixture repository, by qualified name.
fn gap_names(engine: &Engine) -> Vec<String> {
    engine
        .doc_gaps(&DocGapQuery::default())
        .expect("gaps")
        .into_iter()
        .map(|gap| gap.name)
        .collect()
}

/// One entry for each name given.
fn entries(pairs: &[(&str, &str)]) -> Vec<DocEntry> {
    pairs
        .iter()
        .map(|(symbol, text)| DocEntry {
            symbol: (*symbol).to_owned(),
            text: (*text).to_owned(),
        })
        .collect()
}

/// The text of one file of the fixture.
fn read(fixture: &Fixture, relative: &str) -> String {
    std::fs::read_to_string(fixture.dir.path().join(relative)).expect("read file")
}

/// The gaps are the public symbols with no documentation, in path and line order, and the limit
/// and the path prefix both narrow them.
#[test]
fn gaps_are_the_undocumented_public_symbols() {
    let fixture = sample_repo();
    let names = gap_names(&fixture.engine);
    assert!(names.contains(&"Server.stop".to_owned()), "{names:?}");
    assert!(names.contains(&"default_config".to_owned()), "{names:?}");
    assert!(names.contains(&"request".to_owned()), "{names:?}");
    assert!(
        !names.iter().any(|name| name.contains("parse_config")),
        "documented symbols are not gaps: {names:?}"
    );
    assert!(
        !names.iter().any(|name| name.contains("read_file")),
        "private symbols are not gaps: {names:?}"
    );

    let gaps = fixture
        .engine
        .doc_gaps(&DocGapQuery {
            path_prefix: Some("web/".to_owned()),
            limit: 1,
            with_context: false,
        })
        .expect("narrowed gaps");
    assert_eq!(gaps.len(), 1);
    assert_eq!(gaps[0].path, "web/api.ts");
    assert!(gaps[0].context.is_none());
    assert!(gaps[0].line >= 1);
    assert!(!gaps[0].signature.is_empty());
}

/// A context holds the signature, the callers, the callees and the head of the source, and stays
/// close to the promised size.
#[test]
fn contexts_are_compact_and_complete() {
    let fixture = sample_repo();
    let gaps = fixture
        .engine
        .doc_gaps(&DocGapQuery {
            path_prefix: None,
            limit: 50,
            with_context: true,
        })
        .expect("gaps with context");
    assert!(!gaps.is_empty());
    let mut total = 0_u32;
    for gap in &gaps {
        let context = gap.context.as_deref().expect("a context");
        assert!(context.starts_with(&gap.signature), "{context}");
        let cost = estimate_tokens(context);
        assert!(cost <= 140, "{} costs {cost}: {context}", gap.name);
        total += cost;
    }
    let mean = total / u32::try_from(gaps.len()).expect("a small count");
    assert!(mean <= 140, "mean context is {mean} tokens");

    let value = doc_gaps_to_value(&gaps);
    let rows = value["gaps"].as_array().expect("rows");
    assert_eq!(rows.len(), gaps.len());
    assert_eq!(rows[0].as_object().map(serde_json::Map::len), Some(7));

    let with_callers = gaps
        .iter()
        .filter_map(|gap| gap.context.as_deref())
        .any(|context| context.contains("calls: "));
    assert!(with_callers, "at least one gap calls something");
}

/// A batch reads the same from TOON and from JSON, and every refusal names the entry.
#[test]
fn a_batch_reads_from_toon_and_from_json() {
    let toon = "docs[2]{symbol,text}:\n  default_config,The built-in defaults.\n  \
                request,Sends one request.";
    let json = "[{\"symbol\":\"default_config\",\"text\":\"The built-in defaults.\"},\
                {\"symbol\":\"request\",\"text\":\"Sends one request.\"}]";
    let object = "{\"docs\":[{\"symbol\":\"default_config\",\"text\":\"The built-in defaults.\"},\
                  {\"symbol\":\"request\",\"text\":\"Sends one request.\"}]}";
    let expected = entries(&[
        ("default_config", "The built-in defaults."),
        ("request", "Sends one request."),
    ]);
    assert_eq!(parse_doc_entries(toon).expect("toon"), expected);
    assert_eq!(parse_doc_entries(json).expect("json"), expected);
    assert_eq!(parse_doc_entries(object).expect("object"), expected);

    for broken in [
        "",
        "   ",
        "{\"other\":[]}",
        "[{\"symbol\":\"a\"}]",
        "[{\"symbol\":\"a\",\"text\":\"\"}]",
        "[{\"symbol\":\"a\",\"text\":42}]",
        "docs[9]{symbol,text}:\n  a,b",
    ] {
        let error = parse_doc_entries(broken).expect_err("refused");
        assert_eq!(error.exit_code(), 2, "{broken}");
        assert!(!error.to_string().is_empty());
    }
    let numbered = parse_doc_entries("[{\"symbol\":\"a\",\"text\":\"T.\"},{\"text\":\"T.\"}]")
        .expect_err("refused");
    assert!(numbered.to_string().contains("entry 2"), "{numbered}");
}

/// Documentation written for a Rust, a Python and a TypeScript symbol lands in each language's own
/// syntax, and the gap is gone once the file has been indexed again.
#[test]
fn documentation_round_trips_in_every_language() {
    let fixture = sample_repo();
    let engine = &fixture.engine;
    let batch = entries(&[
        (
            "default_config",
            "The configuration used when none was given.",
        ),
        ("Server.stop", "Stops listening and releases the port."),
        ("request", "Sends one request and decodes the response."),
    ]);
    let report = engine.doc_apply(&batch, false).expect("apply");
    assert_eq!(report.skipped, Vec::new(), "{report:?}");
    assert_eq!(report.applied.len(), 3, "{report:?}");
    assert!(!report.dry_run);
    assert_eq!(
        report.files_changed,
        vec![
            "app/server.py".to_owned(),
            "src/config.rs".to_owned(),
            "web/api.ts".to_owned()
        ]
    );

    let rust = read(&fixture, "src/config.rs");
    assert!(
        rust.contains("/// The configuration used when none was given.\npub fn default_config"),
        "{rust}"
    );
    let python = read(&fixture, "app/server.py");
    assert!(
        python.contains(
            "    def stop(self):\n        \"\"\"Stops listening and releases the port.\"\"\""
        ),
        "{python}"
    );
    let typescript = read(&fixture, "web/api.ts");
    assert!(
        typescript.contains(
            "/**\n * Sends one request and decodes the response.\n */\nexport function request"
        ),
        "{typescript}"
    );

    for name in ["default_config", "stop", "request"] {
        let symbol = engine.resolve_symbol(name).expect("resolve");
        assert!(symbol.doc.is_some(), "{name} is documented now");
    }
    let names = gap_names(engine);
    for gone in ["default_config", "Server.stop", "request"] {
        assert!(!names.contains(&gone.to_owned()), "{gone} is still a gap");
    }

    // The re-index the apply did leaves nothing for the next run to do.
    let again = engine.index(&IndexOptions::default()).expect("reindex");
    assert_eq!(again.files_indexed, 0, "{again:?}");
}

/// A dry run reports what it would do and writes nothing at all.
#[test]
fn a_dry_run_changes_nothing() {
    let fixture = sample_repo();
    let before = read(&fixture, "src/config.rs");
    let gaps_before = gap_names(&fixture.engine);
    let batch = entries(&[("default_config", "The built-in configuration.")]);
    let report = fixture.engine.doc_apply(&batch, true).expect("dry run");
    assert!(report.dry_run);
    assert_eq!(report.applied, vec!["default_config".to_owned()]);
    assert_eq!(report.files_changed, vec!["src/config.rs".to_owned()]);
    assert!(report.summary().contains("would change"), "{report:?}");
    assert_eq!(read(&fixture, "src/config.rs"), before);
    assert_eq!(gap_names(&fixture.engine), gaps_before);
}

/// A file edited since it was indexed is refused whole, with a reason naming the index command.
#[test]
fn a_changed_file_is_refused_whole() {
    let fixture = sample_repo();
    let original = read(&fixture, "web/api.ts");
    write(
        fixture.dir.path(),
        "web/api.ts",
        &format!("// a comment added after indexing\n{original}"),
    );
    let batch = entries(&[
        ("request", "Sends one request."),
        ("User", "One account of the service."),
    ]);
    let report = fixture.engine.doc_apply(&batch, false).expect("apply");
    assert!(report.applied.is_empty(), "{report:?}");
    assert_eq!(report.skipped.len(), 2, "{report:?}");
    for (_, reason) in &report.skipped {
        assert!(reason.contains("pn-ultramemory index"), "{reason}");
    }
    assert!(report.files_changed.is_empty());
    assert!(
        read(&fixture, "web/api.ts").starts_with("// a comment added after indexing"),
        "the file is untouched"
    );
}

/// Two entries in one file are applied from the bottom upwards, so both land where they belong.
#[test]
fn several_entries_in_one_file_apply_bottom_up() {
    let fixture = sample_repo();
    let batch = entries(&[
        ("request", "Sends one request to the service."),
        ("User", "One account of the service."),
    ]);
    let report = fixture.engine.doc_apply(&batch, false).expect("apply");
    assert_eq!(report.skipped, Vec::new(), "{report:?}");
    assert_eq!(
        report.applied,
        vec!["request".to_owned(), "User".to_owned()],
        "applied are reported in source order"
    );
    let text = read(&fixture, "web/api.ts");
    let request_doc = text
        .find("Sends one request to the service.")
        .expect("request documentation");
    let request_decl = text.find("export function request").expect("request");
    let user_doc = text
        .find("One account of the service.")
        .expect("user documentation");
    let user_decl = text.find("export interface User").expect("user");
    assert!(request_doc < request_decl, "{text}");
    assert!(request_decl < user_doc, "{text}");
    assert!(user_doc < user_decl, "{text}");
}

/// Whatever is applied, every file still parses afterwards: awkward text is refused rather than
/// written badly.
#[test]
fn applying_never_breaks_the_source() {
    let fixture = sample_repo();
    let engine = &fixture.engine;
    assert_eq!(engine.stats().expect("stats").totals.parse_error_files, 0);
    let symbols_before = engine.stats().expect("stats").index.symbols;

    let awkward = "Ends a block */ and opens \"\"\" a docstring \\\\ with <angle> brackets and \
                   a // line comment.";
    let batch: Vec<DocEntry> = engine
        .doc_gaps(&DocGapQuery::default())
        .expect("gaps")
        .into_iter()
        .map(|gap| DocEntry {
            symbol: gap.name,
            text: awkward.to_owned(),
        })
        .collect();
    assert!(!batch.is_empty());
    let report = engine.doc_apply(&batch, false).expect("apply");

    let after = engine.index(&IndexOptions::default()).expect("reindex");
    assert_eq!(after.files_skipped, Vec::new(), "{after:?}");
    let stats = engine.stats().expect("stats");
    assert_eq!(stats.totals.parse_error_files, 0, "{report:?}");
    assert_eq!(
        stats.index.symbols, symbols_before,
        "documentation adds no symbols and removes none"
    );
}

/// The Markdown reference lists the public symbols of every file, escaped, and the prefix narrows
/// it to one directory.
#[test]
fn markdown_reference_is_built_from_the_index() {
    let fixture = sample_repo();
    let engine = &fixture.engine;
    let all = engine
        .doc_markdown(None, Some("Fixture API"))
        .expect("markdown");
    assert!(all.starts_with("# Fixture API\n"), "{all}");
    assert!(all.contains("## src/config.rs\n"), "{all}");
    assert!(
        all.contains("| kind | name | signature | summary |"),
        "{all}"
    );
    assert!(all.contains("| function | load_config |"), "{all}");
    assert!(
        all.contains("Loads the configuration from a file."),
        "the summary comes from the documentation: {all}"
    );
    assert!(!all.contains("read_file"), "private symbols are left out");
    assert!(!all.contains("Promise<User>"), "angle brackets are escaped");
    assert!(all.contains("Promise&lt;User&gt;"), "{all}");

    let narrowed = engine.doc_markdown(Some("web/"), None).expect("markdown");
    assert!(narrowed.starts_with("# API reference\n"), "{narrowed}");
    assert!(narrowed.contains("## web/api.ts"), "{narrowed}");
    assert!(!narrowed.contains("src/config.rs"), "{narrowed}");

    // The document depends on the index alone, so it is the same twice.
    assert_eq!(
        engine.doc_markdown(Some("web/"), None).expect("markdown"),
        narrowed
    );
}
