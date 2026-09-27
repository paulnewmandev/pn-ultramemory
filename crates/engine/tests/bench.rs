// SPDX-License-Identifier: Apache-2.0
//! End-to-end checks of the offline retrieval benchmark against the real adapters.
//!
//! Three things are worth proving here and are hard to prove anywhere else: that a fixed seed gives
//! the same numbers twice, that the capsule really is cheaper than reading whole files on a
//! repository large enough for the comparison to mean anything, and that an index with nothing in it
//! reports absent figures instead of flattering zeros.

// Test helpers outside a `#[test]` function are not covered by the `allow-*-in-tests` settings in
// `clippy.toml`, and a failed setup step in a test should stop that test loudly.
#![allow(
    clippy::expect_used,
    reason = "a failed setup step should stop the test loudly"
)]

mod common;

use common::{engine_at, generated, sample_repo};
use pn_ultramemory_engine::{BenchOptions, BenchReport, Engine, EngineConfig, IndexOptions};
use serde_json::Value;

/// The metrics of a report without the two wall-clock figures, which no two runs can share.
fn reproducible(report: &BenchReport) -> Value {
    let mut value = report.to_value();
    if let Some(metrics) = value["metrics"].as_array() {
        let kept: Vec<Value> = metrics
            .iter()
            .filter(|row| {
                row["metric"]
                    .as_str()
                    .is_some_and(|name| !name.ends_with("_ms"))
            })
            .cloned()
            .collect();
        value["metrics"] = Value::Array(kept);
    }
    value
}

/// A repository of `files` generated files, indexed and ready to measure.
fn generated_repo(files: usize) -> (tempfile::TempDir, Engine) {
    let dir = tempfile::tempdir().expect("temporary directory");
    generated::generate_repo(dir.path(), files, 7);
    let engine = engine_at(dir.path(), EngineConfig::default());
    engine.index(&IndexOptions::default()).expect("index");
    (dir, engine)
}

/// Every family is measured at every budget, every row carries the eight named figures, and the
/// notes say what the numbers do not mean.
#[test]
fn every_family_is_measured_at_every_budget() {
    let fixture = sample_repo();
    let options = BenchOptions {
        tasks: 10,
        seed: 3,
        budgets: vec![2000, 500, 500],
        baseline_files: 2,
    };
    let report = fixture.engine.bench(&options).expect("bench");
    assert_eq!(report.indexed_files, 4);
    assert_eq!(report.rows.len(), 4, "two families, two distinct budgets");

    let families: Vec<&str> = report.rows.iter().map(|row| row.family.as_str()).collect();
    assert_eq!(families, ["description", "description", "name", "name"]);
    let budgets: Vec<u32> = report.rows.iter().map(|row| row.budget).collect();
    assert_eq!(budgets, [500, 2000, 500, 2000]);

    for row in &report.rows {
        let names: Vec<&str> = row.metrics.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "hit_rate",
                "mean_tokens",
                "baseline_hit_rate",
                "baseline_mean_tokens",
                "tokens_saved_ratio",
                "p50_ms",
                "p95_ms",
                "task_success",
            ]
        );
        assert_eq!(row.value("task_success"), None, "no model ran");
        let mean = row.value("mean_tokens").expect("a mean");
        assert!(mean <= f64::from(row.budget), "{mean} over {}", row.budget);
        let hit = row.value("hit_rate").expect("a hit rate");
        assert!((0.0..=1.0).contains(&hit), "{hit}");
    }

    let notes = report.notes.join(" ");
    assert!(notes.contains("known-item retrieval"), "{notes}");
    assert!(notes.contains("lexical ranking of whole files"), "{notes}");
    assert!(
        notes.contains("not that a model solved anything"),
        "{notes}"
    );
    assert!(notes.contains("4.5 %"), "{notes}");
    assert!(notes.contains("depend on the repository"), "{notes}");
    assert!(notes.contains("`task_success` has no value"), "{notes}");

    let value = report.to_value();
    let rows = value["metrics"].as_array().expect("a metric table");
    assert_eq!(rows.len(), report.rows.len() * 8);
    for row in rows {
        assert_eq!(row.as_object().map(serde_json::Map::len), Some(6));
    }
    let unobservable: Vec<&Value> = rows
        .iter()
        .filter(|row| row["derivation"] == "unobservable")
        .collect();
    assert_eq!(unobservable.len(), report.rows.len());
    for row in unobservable {
        assert_eq!(row["value"], Value::Null, "never a fabricated zero");
    }
    assert!(report.summary().contains("task_success: not measured"));
}

/// The same seed over the same index gives the same numbers, apart from the two wall-clock ones.
#[test]
fn a_seed_makes_the_run_reproducible() {
    let (_dir, engine) = generated_repo(12);
    let options = BenchOptions {
        tasks: 8,
        seed: 42,
        budgets: vec![800],
        baseline_files: 3,
    };
    let first = engine.bench(&options).expect("first run");
    let second = engine.bench(&options).expect("second run");
    assert_eq!(reproducible(&first), reproducible(&second));
    assert_eq!(
        serde_json::to_string(&reproducible(&first)).expect("json"),
        serde_json::to_string(&reproducible(&second)).expect("json"),
        "byte-identical output"
    );

    // A different seed draws a different sample, so the run is a sample and not a fixed list.
    let other = engine
        .bench(&BenchOptions { seed: 9, ..options })
        .expect("third run");
    assert_eq!(other.rows.len(), first.rows.len());

    // Running the benchmark teaches the learner nothing, so nothing was recorded.
    assert_eq!(engine.learning_status().expect("learning").signals, 0);
}

/// On a repository big enough for the comparison to mean anything, a capsule costs a small
/// fraction of reading the files a lexical ranking would choose.
#[test]
fn a_capsule_costs_far_less_than_reading_files() {
    let (_dir, engine) = generated_repo(30);
    let report = engine
        .bench(&BenchOptions {
            tasks: 20,
            seed: 1,
            budgets: vec![1000],
            baseline_files: 3,
        })
        .expect("bench");
    println!("{}", report.summary());

    assert!(report.indexed_files >= 30, "{report:?}");
    for row in &report.rows {
        assert!(row.tasks >= 5, "{row:?}");
        let ours = row.value("mean_tokens").expect("a mean");
        let theirs = row.value("baseline_mean_tokens").expect("a baseline mean");
        assert!(theirs > ours * 3.0, "{ours} against {theirs}");
        let saved = row.value("tokens_saved_ratio").expect("a ratio");
        assert!(saved > 0.5, "saved {saved}");
        let hit = row.value("hit_rate").expect("a hit rate");
        assert!(hit > 0.0, "the wanted symbol is found at least sometimes");
    }
    let by_name = report
        .rows
        .iter()
        .find(|row| row.family == "name")
        .expect("the name family");
    assert!(
        by_name.value("hit_rate").expect("a hit rate") > 0.5,
        "a name is the easiest query there is: {by_name:?}"
    );
}

/// An index with no documented symbol reports absent figures and says why.
#[test]
fn an_empty_index_reports_absent_figures() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let engine = engine_at(dir.path(), EngineConfig::default());
    let report = engine.bench(&BenchOptions::default()).expect("bench");

    assert_eq!(report.indexed_files, 0);
    assert_eq!(report.indexed_symbols, 0);
    assert_eq!(report.rows.len(), 6, "two families, three budgets");
    for row in &report.rows {
        assert_eq!(row.tasks, 0);
        for metric in &row.metrics {
            assert_eq!(metric.value, None, "{}", metric.name);
        }
    }
    let notes = report.notes.join(" ");
    assert!(notes.contains("pn-ultramemory index"), "{notes}");
    assert!(report.summary().contains("indexed 0 symbols in 0 files"));
}

/// Asking for nothing in particular falls back to the documented defaults.
#[test]
fn the_defaults_are_used_when_nothing_is_asked_for() {
    let fixture = sample_repo();
    let report = fixture
        .engine
        .bench(&BenchOptions {
            tasks: 0,
            seed: 1,
            budgets: Vec::new(),
            baseline_files: 0,
        })
        .expect("bench");
    let budgets: Vec<u32> = report
        .rows
        .iter()
        .filter(|row| row.family == "description")
        .map(|row| row.budget)
        .collect();
    assert_eq!(budgets, [500, 1000, 2000]);
}
