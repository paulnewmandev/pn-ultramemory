// SPDX-License-Identifier: Apache-2.0
//! End-to-end checks of the two reporting operations against the real adapters: the statistics of
//! an index, the usage counters read back from a metrics file, and the figures a report about a
//! repository is built from.
//!
//! The case worth the most here is the empty one: a repository that was never indexed has to produce
//! a report full of empty tables, because that is what a new user sees first.

// Test helpers outside a `#[test]` function are not covered by the `allow-*-in-tests` settings in
// `clippy.toml`, and a failed setup step in a test should stop that test loudly.
#![allow(
    clippy::expect_used,
    reason = "a failed setup step should stop the test loudly"
)]

mod common;

use std::path::Path;

use common::{engine_at, sample_repo};
use pn_ultramemory_engine::{EngineConfig, InsightOptions};

/// An engine over `root` whose metrics file is `metrics`.
fn engine_with_metrics(root: &Path, metrics: &Path) -> pn_ultramemory_engine::Engine {
    engine_at(
        root,
        EngineConfig {
            metrics_path: Some(metrics.to_path_buf()),
            ..EngineConfig::default()
        },
    )
}

/// The statistics count the fixture and carry the languages as a table; usage is absent while
/// metrics are off.
#[test]
fn statistics_describe_the_index() {
    let fixture = sample_repo();
    let report = fixture.engine.stats().expect("stats");
    assert_eq!(report.index.files, 4);
    assert!(report.index.symbols >= 10, "{report:?}");
    assert!(report.index.edges >= 1, "{report:?}");
    assert_eq!(report.index.memories, 0);
    assert!(report.totals.lines > 50, "{report:?}");
    assert_eq!(report.totals.parse_error_files, 0);
    assert!(report.usage.is_none(), "no metrics path is configured");

    let value = fixture.engine.storage_stats().expect("value");
    assert_eq!(value["index"]["files"], 4);
    assert_eq!(value["usage"], serde_json::Value::Null);
    let languages = value["index"]["languages"]
        .as_array()
        .expect("a language table");
    assert_eq!(languages.len(), 3, "{languages:?}");
    for row in languages {
        assert_eq!(row.as_object().map(serde_json::Map::len), Some(2));
    }
    assert_eq!(value["learning"]["signals"], 0);
    assert_eq!(value["totals"]["parse_error_files"], 0);
}

/// The usage summary reads the metrics file, ignores what it cannot parse, and stays absent when
/// there is no file to read.
#[test]
fn usage_is_read_from_the_metrics_file() {
    let fixture = sample_repo();
    let home = tempfile::tempdir().expect("temporary directory");
    let metrics = home.path().join("metrics.jsonl");

    let engine = engine_with_metrics(fixture.dir.path(), &metrics);
    assert!(engine.usage_summary().is_none(), "the file does not exist");

    std::fs::write(
        &metrics,
        "{\"event\":\"recall\",\"budget\":1000,\"used\":400}\n\
         not json at all\n\
         \n\
         [1,2,3]\n\
         {\"event\":\"recall\",\"budget\":1000,\"used\":600}\n\
         {\"event\":\"expand\",\"lines\":40}\n\
         {\"event\":\"remember\"}\n\
         {\"event\":\"doc_apply\",\"applied\":2}\n\
         {\"event\":\"impact\"}\n\
         {\"event\":\"something_else\"}\n\
         {\"used\":\"not a number\"}\n",
    )
    .expect("write metrics");

    let usage = engine.usage_summary().expect("a summary");
    assert_eq!(usage.recalls, 2);
    assert_eq!(usage.expands, 1);
    assert_eq!(usage.impacts, 1);
    assert_eq!(usage.remembers, 1);
    assert_eq!(usage.doc_applies, 1);
    assert_eq!(usage.tokens_served_total, 1000);
    assert_eq!(usage.avg_tokens_served, 500);
    assert_eq!(usage.avg_budget_percent, 50);

    let report = engine.stats().expect("stats");
    assert_eq!(report.usage, Some(usage));
    let value = engine.storage_stats().expect("value");
    assert_eq!(value["usage"]["recalls"], 2);
    assert_eq!(value["usage"]["avg_budget_percent"], 50);

    std::fs::write(&metrics, "nothing usable here\n").expect("rewrite metrics");
    let empty = engine.usage_summary().expect("a summary");
    assert_eq!(empty.recalls, 0);
    assert_eq!(empty.avg_tokens_served, 0);
}

/// The insights of the fixture name its languages, its modules, its hotspots, its documentation
/// coverage and a few of its gaps, and repeat exactly on a second call.
#[test]
fn insights_describe_the_repository() {
    let fixture = sample_repo();
    let engine = &fixture.engine;
    let insights = engine
        .insights(&InsightOptions::default())
        .expect("insights");

    assert_eq!(insights.stats.index.files, 4);
    let names: Vec<&str> = insights
        .languages
        .iter()
        .map(|row| row.name.as_str())
        .collect();
    assert_eq!(names.first(), Some(&"rust"), "{names:?}");
    assert!(names.contains(&"python"), "{names:?}");
    assert!(names.contains(&"typescript"), "{names:?}");
    let files: u64 = insights.languages.iter().map(|row| row.files).sum();
    assert_eq!(files, insights.stats.index.files);

    assert!(!insights.modules.is_empty(), "{:?}", insights.modules);
    let module_files: u64 = insights.modules.iter().map(|module| module.files).sum();
    assert_eq!(module_files, insights.stats.index.files);

    assert!(!insights.hotspots.is_empty());
    for pair in insights.hotspots.windows(2) {
        assert!(
            pair[0].callers >= pair[1].callers,
            "{:?}",
            insights.hotspots
        );
    }
    let hot: Vec<&str> = insights
        .hotspots
        .iter()
        .map(|spot| spot.name.as_str())
        .collect();
    assert!(
        hot.contains(&"parse_config") || hot.contains(&"Config"),
        "{hot:?}"
    );

    assert!(!insights.documentation.is_empty());
    for row in &insights.documentation {
        assert!(row.documented <= row.public_symbols, "{row:?}");
    }
    assert!(!insights.undocumented.is_empty());
    assert!(insights.undocumented.len() <= 10);
    assert!(insights.memories.is_empty(), "the fixture stores none");

    let value = insights.to_value();
    assert_eq!(value["stats"]["index"]["files"], 4);
    assert_eq!(
        value["languages"].as_array().map(Vec::len),
        Some(insights.languages.len())
    );
    assert_eq!(
        value["undocumented"].as_array().map(Vec::len),
        Some(insights.undocumented.len())
    );
    assert!(value["documentation"][0]["percent"].is_number());

    let again = engine
        .insights(&InsightOptions::default())
        .expect("insights");
    assert_eq!(again, insights, "the same index gives the same figures");
}

/// The limits of the options are honoured, and asking for none of something gives none.
#[test]
fn insight_limits_are_honoured() {
    let fixture = sample_repo();
    let insights = fixture
        .engine
        .insights(&InsightOptions {
            module_depth: 1,
            max_hotspots: 2,
            max_memories: 0,
            max_undocumented: 1,
        })
        .expect("insights");
    assert!(insights.hotspots.len() <= 2);
    assert_eq!(insights.undocumented.len(), 1);
    assert!(insights.memories.is_empty());
    assert!(!insights.modules.is_empty());
}

/// A repository that was never indexed reports empty tables and zero counts rather than failing.
#[test]
fn an_empty_repository_reports_empty_tables() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let engine = engine_at(dir.path(), EngineConfig::default());
    let insights = engine
        .insights(&InsightOptions::default())
        .expect("insights");

    assert_eq!(insights.stats.index.files, 0);
    assert_eq!(insights.stats.index.symbols, 0);
    assert!(insights.stats.usage.is_none());
    assert!(insights.languages.is_empty());
    assert!(insights.modules.is_empty());
    assert!(insights.module_edges.is_empty());
    assert!(insights.hotspots.is_empty());
    assert!(insights.documentation.is_empty());
    assert!(insights.undocumented.is_empty());
    assert!(insights.memories.is_empty());

    let value = insights.to_value();
    for key in [
        "languages",
        "modules",
        "module_edges",
        "hotspots",
        "documentation",
        "undocumented",
        "memories",
    ] {
        assert_eq!(value[key].as_array().map(Vec::len), Some(0), "{key}");
    }
    assert_eq!(engine.storage_stats().expect("value")["index"]["files"], 0);
}
