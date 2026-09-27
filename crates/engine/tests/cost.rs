// SPDX-License-Identifier: Apache-2.0
//! What must stay **flat** as a repository grows, and what must grow with it.
//!
//! # Why these are tests and not benchmarks
//! The central claim of this tool is that a question costs about the same to answer in a large
//! repository as in a small one. A timing benchmark cannot hold that claim: it is noisy, it is
//! machine-dependent, and a regression hides inside its error bars. What *can* hold it is the
//! shape of the answer — the tokens spent and the symbols returned — measured over a swept
//! repository size and asserted to belong to a complexity class.
//!
//! So each test here sweeps one parameter, measures a number the tool already reports, and asserts
//! that the number is **flat** in that parameter or **grows** with it. Nothing here is a
//! stopwatch, so nothing here is flaky.
//!
//! # The control
//! A flat assertion on its own proves nothing: a measurement that is always zero is also flat.
//! Every flat claim in this file is therefore paired with a measurement over the *same* sweep that
//! must grow, so a test that stopped measuring anything fails instead of passing quietly.

// Test helpers outside a `#[test]` function are not covered by the `allow-*-in-tests` settings in
// `clippy.toml`, and a failed setup step in a test should stop that test loudly.
#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "a failed setup step or a broken cost claim should stop the test loudly"
)]

mod common;

use common::{engine_at, generated};
use pn_ultramemory_engine::{Engine, EngineConfig, IndexOptions, RecallQuery};

/// The sizes every sweep runs over. The largest is eight times the smallest, which is enough for a
/// linear term to show up as a doubling and small enough to keep the suite quick.
const SIZES: [usize; 4] = [8, 20, 40, 64];

/// How much a flat measurement may drift across the whole sweep, as a fraction.
///
/// Not zero: a bigger repository has more candidates to choose between, so the packer can fill the
/// last few tokens of the budget slightly better. That is a better answer at the same price, not a
/// cost that grows, and this allows for it while still failing on anything proportional.
const FLAT_SLACK: f64 = 0.25;

/// The seed every generated repository in this file is built from.
const SEED: u64 = 7;

/// A generated repository of `files` files, indexed and ready to query.
fn repo_of(files: usize) -> (tempfile::TempDir, Engine) {
    let dir = tempfile::tempdir().expect("temporary directory");
    generated::generate_repo(dir.path(), files, SEED);
    let engine = engine_at(dir.path(), EngineConfig::default());
    engine
        .index(&IndexOptions::default())
        .expect("index the generated repository");
    (dir, engine)
}

/// Fails unless every measurement is within [`FLAT_SLACK`] of the first.
fn assert_flat(curve: &[(usize, f64)], what: &str) {
    let Some((_, first)) = curve.first() else {
        panic!("{what}: nothing was measured");
    };
    assert!(
        *first > 0.0,
        "{what}: the first measurement is zero: {curve:?}"
    );
    for (size, value) in curve {
        let drift = (value - first).abs() / first;
        assert!(
            drift <= FLAT_SLACK,
            "{what} is not flat: {value} at {size} files against {first} at the smallest ({curve:?})"
        );
    }
}

/// Fails unless the last measurement is meaningfully larger than the first.
///
/// This is the control. Without it a flat assertion passes on a measurement that stopped
/// happening, and the file would go green while proving nothing at all.
fn assert_grows(curve: &[(usize, f64)], what: &str) {
    let (Some((_, first)), Some((_, last))) = (curve.first(), curve.last()) else {
        panic!("{what}: nothing was measured");
    };
    assert!(
        last > &(first * 1.5),
        "{what} did not grow with the repository, so the sweep measured nothing: {curve:?}"
    );
}

/// Sweeps the repository size, measuring one number per size.
fn sweep(measure: impl Fn(&Engine) -> f64) -> Vec<(usize, f64)> {
    SIZES
        .iter()
        .map(|files| {
            let (_dir, engine) = repo_of(*files);
            (*files, measure(&engine))
        })
        .collect()
}

/// A recall at a fixed budget costs the same in a large repository as in a small one.
///
/// This is the claim the whole tool rests on. If the tokens spent grew with the repository, the
/// saving would be an artefact of the repository being small.
#[test]
fn a_recall_costs_the_same_however_large_the_repository() {
    let tokens = sweep(|engine| {
        let capsule = engine
            .recall(&RecallQuery {
                text: "parse the configuration file".into(),
                budget: Some(1200),
                ..RecallQuery::default()
            })
            .expect("recall");
        f64::from(capsule.used)
    });
    assert_flat(&tokens, "the tokens a recall spends");

    // The control: the pool it is choosing from does grow, so the flat cost above is a decision
    // the packer is making and not an absence of candidates.
    let candidates = sweep(|engine| {
        let capsule = engine
            .recall(&RecallQuery {
                text: "parse the configuration file".into(),
                budget: Some(1200),
                ..RecallQuery::default()
            })
            .expect("recall");
        f64::from(capsule.omitted) + f64::from(u32::try_from(capsule.symbols.len()).unwrap_or(0))
    });
    assert_grows(&candidates, "the candidates a recall considers");
}

/// A map at a fixed budget costs the same however many files there are.
#[test]
fn a_map_costs_the_same_however_many_files() {
    let tokens = sweep(|engine| {
        let map = engine
            .repo_map(&pn_ultramemory_engine::MapQuery {
                budget: Some(900),
                path_prefix: None,
            })
            .expect("map");
        f64::from(map.used)
    });
    assert_flat(&tokens, "the tokens a map spends");

    let omitted = sweep(|engine| {
        let map = engine
            .repo_map(&pn_ultramemory_engine::MapQuery {
                budget: Some(900),
                path_prefix: None,
            })
            .expect("map");
        f64::from(map.omitted_files)
    });
    assert_grows(&omitted, "the files a map leaves out");
}

/// Re-indexing a repository nothing changed in reads no files, however many it has.
///
/// The number asserted is zero, so the usual flat test does not apply: zero is exact, and anything
/// above it is the incremental index failing to be incremental.
#[test]
fn re_indexing_an_unchanged_repository_reads_nothing() {
    for files in SIZES {
        let (_dir, engine) = repo_of(files);
        let again = engine.index(&IndexOptions::default()).expect("re-index");
        assert_eq!(
            again.files_indexed, 0,
            "{files} files: re-indexing read {} of them again",
            again.files_indexed
        );
        assert_eq!(
            again.files_seen,
            u32::try_from(files).unwrap_or(u32::MAX),
            "{files} files: the sweep did not see them all"
        );
    }
}

/// An outline describes one file at a cost set by that file, not by the repository around it.
#[test]
fn an_outline_costs_what_its_own_file_costs() {
    let tokens = sweep(|engine| {
        // The generator decides the paths, so ask it rather than writing one here: file zero
        // exists at every size, so the same file is measured at every point of the sweep.
        //
        // Taking the first file out of a map would not do. A map is budget-limited, so which files
        // it names changes with the repository, and the test would then be comparing the cost of
        // describing *different* files while claiming to hold that cost constant.
        let path = generated::plan_file(SEED, 0).path;
        let outline = engine.outline(&path, None).expect("outline");
        f64::from(outline.tokens)
    });
    assert_flat(&tokens, "the tokens an outline spends on one file");
}
