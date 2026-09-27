// SPDX-License-Identifier: Apache-2.0
//! Parallel indexing: the stored result does not depend on the number of threads, unchanged files
//! are never parsed, files that fail are reported instead of failing the run, and a panic while
//! parsing one file is contained. Also a timing report, run on demand.

// Test helpers outside a `#[test]` function are not covered by the `allow-*-in-tests` settings in
// `clippy.toml`, and a failed setup step in a test should stop that test loudly.
#![allow(
    clippy::expect_used,
    reason = "a failed setup step should stop the test loudly"
)]
// Throughput figures are printed, never asserted, so the rounding a wide integer picks up on its
// way to a floating-point ratio cannot affect a result.
#![allow(
    clippy::cast_precision_loss,
    reason = "printed throughput figures, never asserted"
)]

mod common;

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use common::generated::generate_repo;
use common::{sample_files, write};
use pn_ultramemory_core::{
    Confidence, Direction, ExtractError, Extractor, FileExtract, Language, Storage,
};
use pn_ultramemory_engine::{Deps, Engine, EngineConfig, IndexOptions, SystemClock};
use pn_ultramemory_index::{DocComments, FsSourceTree, TreeSitterExtractor};
use pn_ultramemory_store::SqliteStorage;

/// The text that makes the spy extractor panic, written in two pieces so that this very file (which
/// the timing report indexes as part of the workspace) does not contain it.
fn marker() -> String {
    format!("{}{}", "PANIC", "_MARKER")
}

/// An extractor that counts its calls and panics on sources that contain the marker.
struct Spy {
    /// The real extractor.
    inner: TreeSitterExtractor,
    /// How many times `extract` was called.
    calls: Arc<AtomicUsize>,
}

impl Extractor for Spy {
    /// Delegates to the real extractor.
    fn supports(&self, language: Language) -> bool {
        self.inner.supports(language)
    }

    /// Counts the call, panics on the marker and otherwise delegates.
    fn extract(&self, language: Language, source: &str) -> Result<FileExtract, ExtractError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert!(!source.contains(&marker()), "the parser fell over");
        self.inner.extract(language, source)
    }
}

/// An engine over `root`, the storage behind it and the counter of extractions.
struct Harness {
    /// The engine under test.
    engine: Engine,
    /// The storage, to compare what is stored.
    storage: Arc<SqliteStorage>,
    /// The number of files parsed so far.
    calls: Arc<AtomicUsize>,
}

/// Wires an engine to the real adapters, keeping a handle on the storage.
fn harness(root: &Path) -> Harness {
    let storage = Arc::new(SqliteStorage::open_in_memory().expect("open storage"));
    let calls = Arc::new(AtomicUsize::new(0));
    let deps = Deps {
        storage: storage.clone(),
        extractor: Arc::new(Spy {
            inner: TreeSitterExtractor::new(),
            calls: calls.clone(),
        }),
        tree: Arc::new(FsSourceTree::new(root, 4_000_000).expect("open tree")),
        docs: Arc::new(DocComments),
        clock: Arc::new(SystemClock),
    };
    Harness {
        engine: Engine::new(deps, EngineConfig::default()),
        storage,
        calls,
    }
}

/// Options that index with exactly `threads` threads.
fn with_threads(threads: usize) -> IndexOptions {
    IndexOptions {
        threads: Some(threads),
        ..IndexOptions::default()
    }
}

/// Everything observable about the stored index: files, symbols and both directions of every
/// edge, rendered as text so that two indexes can be compared.
fn snapshot(storage: &SqliteStorage) -> Vec<String> {
    let mut lines = Vec::new();
    for file in storage.list_files().expect("files") {
        lines.push(format!("{file:?}"));
        for symbol in storage.symbols_in_file(&file.path).expect("symbols") {
            lines.push(format!("{symbol:?}"));
            for direction in [Direction::Out, Direction::In] {
                for n in storage
                    .neighbors(symbol.id, direction, Confidence::Guess, 1000)
                    .expect("neighbors")
                {
                    lines.push(format!("{:?} {:?} {:?}", direction, n.edge, n.symbol.id));
                }
            }
        }
    }
    lines.push(format!("{:?}", storage.stats().expect("stats")));
    lines
}

/// Writes the sample repository into `root`.
fn write_sample(root: &Path) {
    for (path, content) in sample_files() {
        write(root, path, content);
    }
}

/// One thread and eight threads store exactly the same index for the sample repository.
#[test]
fn sample_repository_is_identical_with_one_and_eight_threads() {
    let dir = tempfile::tempdir().expect("dir");
    write_sample(dir.path());
    let one = harness(dir.path());
    let eight = harness(dir.path());
    let a = one.engine.index(&with_threads(1)).expect("index");
    let b = eight.engine.index(&with_threads(8)).expect("index");
    assert_eq!(a.files_indexed, b.files_indexed);
    assert_eq!(a.symbols_added, b.symbols_added);
    assert_eq!(a.edges_written, b.edges_written);
    assert_eq!(snapshot(&one.storage), snapshot(&eight.storage));
    assert!(snapshot(&one.storage).len() > 20);
}

/// The same holds for a generated repository, including after an incremental change.
#[test]
fn generated_repository_is_identical_for_every_thread_count() {
    let dir = tempfile::tempdir().expect("dir");
    generate_repo(dir.path(), 150, 5);
    let runs: Vec<(usize, Harness)> = [1, 2, 8]
        .into_iter()
        .map(|threads| (threads, harness(dir.path())))
        .collect();
    for (threads, h) in &runs {
        let report = h.engine.index(&with_threads(*threads)).expect("index");
        assert_eq!(report.files_indexed, 150, "threads {threads}");
        assert!(
            report.files_skipped.is_empty(),
            "{:?}",
            report.files_skipped
        );
    }
    let reference = snapshot(&runs[0].1.storage);
    for (threads, h) in &runs[1..] {
        assert_eq!(snapshot(&h.storage), reference, "threads {threads}");
    }

    // Edit two files and index again with the same thread counts.
    let (path, mut text) = common::generated::file_source(5, 150, 3);
    text.push_str("\npub fn brand_new_helper() -> u8 { 1 }\n");
    write(dir.path(), &path, &text);
    write(
        dir.path(),
        "crates/extra/added.rs",
        "pub fn added_later() {}\n",
    );
    for (threads, h) in &runs {
        let report = h.engine.index(&with_threads(*threads)).expect("reindex");
        assert_eq!(report.files_indexed, 2, "threads {threads}");
        assert_eq!(report.files_unchanged, 149, "threads {threads}");
    }
    let reference = snapshot(&runs[0].1.storage);
    for (threads, h) in &runs[1..] {
        assert_eq!(
            snapshot(&h.storage),
            reference,
            "threads {threads} after the change"
        );
    }
}

/// Unchanged files are skipped before parsing, and only edited files are parsed again.
#[test]
fn unchanged_files_are_never_parsed() {
    let dir = tempfile::tempdir().expect("dir");
    generate_repo(dir.path(), 60, 2);
    let h = harness(dir.path());
    h.engine.index(&with_threads(4)).expect("index");
    assert_eq!(h.calls.load(Ordering::SeqCst), 60);
    let again = h.engine.index(&with_threads(4)).expect("index");
    assert_eq!(again.files_unchanged, 60);
    assert_eq!(h.calls.load(Ordering::SeqCst), 60, "nothing parsed again");
    let (path, text) = common::generated::file_source(2, 60, 7);
    write(dir.path(), &path, &format!("{text}\n// touched\n"));
    let edited = h.engine.index(&with_threads(4)).expect("index");
    assert_eq!(edited.files_indexed, 1);
    assert_eq!(h.calls.load(Ordering::SeqCst), 61);
    let forced = h
        .engine
        .index(&IndexOptions {
            force: true,
            threads: Some(4),
            ..IndexOptions::default()
        })
        .expect("index");
    assert_eq!(forced.files_indexed, 60);
    assert_eq!(h.calls.load(Ordering::SeqCst), 121);
}

/// Files that cannot be read as text or whose parsing panics are reported, and everything else
/// is still indexed, for any number of threads.
#[test]
fn failing_files_are_reported_and_never_fatal() {
    for threads in [1, 8] {
        let dir = tempfile::tempdir().expect("dir");
        write_sample(dir.path());
        std::fs::write(dir.path().join("binary.rs"), [0xFF_u8, 0xFE, 0x00, 0x80]).expect("write");
        write(
            dir.path(),
            "bad/boom.rs",
            &format!("// {}\nfn boom() {{}}\n", marker()),
        );
        write(dir.path(), "bad/fine.rs", "fn fine() {}\n");
        let h = harness(dir.path());
        let report = h
            .engine
            .index(&with_threads(threads))
            .expect("the run survives");
        let skipped: Vec<&str> = report
            .files_skipped
            .iter()
            .map(|(p, _)| p.as_str())
            .collect();
        assert_eq!(skipped, ["bad/boom.rs", "binary.rs"], "threads {threads}");
        let reasons: Vec<&str> = report
            .files_skipped
            .iter()
            .map(|(_, r)| r.as_str())
            .collect();
        assert!(reasons[0].contains("panicked"), "{reasons:?}");
        assert!(reasons[1].contains("UTF-8"), "{reasons:?}");
        assert_eq!(report.files_indexed, 5, "the four samples and bad/fine.rs");
        assert_eq!(report.files_seen, 7);
    }
}

/// `only_paths` indexes just those files and does not remove the others.
#[test]
fn only_paths_indexes_just_those_files() {
    let dir = tempfile::tempdir().expect("dir");
    write_sample(dir.path());
    let h = harness(dir.path());
    let partial = h
        .engine
        .index(&IndexOptions {
            only_paths: vec!["src/main.rs".into(), "not/listed.rs".into()],
            ..IndexOptions::default()
        })
        .expect("partial index");
    assert_eq!(partial.files_seen, 1);
    assert_eq!(partial.files_indexed, 1);
    assert_eq!(h.storage.list_files().expect("files").len(), 1);
    h.engine
        .index(&IndexOptions::default())
        .expect("full index");
    assert_eq!(h.storage.list_files().expect("files").len(), 4);
    let partial = h
        .engine
        .index(&IndexOptions {
            only_paths: vec!["web/api.ts".into()],
            ..IndexOptions::default()
        })
        .expect("partial index");
    assert_eq!(partial.files_unchanged, 1);
    assert_eq!(partial.files_removed, 0);
    assert_eq!(h.storage.list_files().expect("files").len(), 4);
}

/// More threads than files, and an empty repository, both work.
#[test]
fn degenerate_shapes_work() {
    let dir = tempfile::tempdir().expect("dir");
    let h = harness(dir.path());
    let empty = h.engine.index(&with_threads(8)).expect("empty repository");
    assert_eq!(empty.files_seen, 0);
    write(dir.path(), "one.rs", "fn one() {}\n");
    let one = h
        .engine
        .index(&with_threads(64))
        .expect("one file, many threads");
    assert_eq!(one.files_indexed, 1);
    let zero = h
        .engine
        .index(&with_threads(0))
        .expect("zero threads means one");
    assert_eq!(zero.files_unchanged, 1);
}

/// Prints how long indexing takes; run with
/// `cargo test --release -p pn-ultramemory-engine --test index_parallel -- --ignored --nocapture`.
#[test]
#[ignore = "prints timings and takes a while; run it on demand in release mode"]
fn measure_indexing_speed() {
    /// Indexes `root` with `threads` threads on a fresh index and prints the timing.
    fn full_run(label: &str, root: &Path, threads: usize) -> Harness {
        let h = harness(root);
        let started = Instant::now();
        let report = h.engine.index(&with_threads(threads)).expect("index");
        let elapsed = started.elapsed().as_secs_f64();
        let mb = report.bytes_indexed as f64 / 1_048_576.0;
        println!(
            "{label}: {} files, {mb:.1} MB, {} symbols, {} edges, {threads} threads: {:.0} ms, {:.1} MB/s, {:.0} files/s, skipped {}",
            report.files_indexed,
            report.symbols_added,
            report.edges_written,
            elapsed * 1000.0,
            mb / elapsed,
            f64::from(report.files_indexed) / elapsed,
            report.files_skipped.len()
        );
        h
    }

    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for threads in [1, 8] {
        full_run("this repository", &workspace.join("crates"), threads);
    }

    let dir = tempfile::tempdir().expect("dir");
    let generated = generate_repo(dir.path(), 5000, 1);
    println!(
        "generated: {} files, {:.1} MB, {} lines",
        generated.paths.len(),
        generated.bytes as f64 / 1_048_576.0,
        generated.lines
    );
    for threads in [1, 2, 4, 8] {
        full_run("generated 5000", dir.path(), threads);
    }
    let h = full_run("generated 5000", dir.path(), 8);
    for round in 0..3 {
        let started = Instant::now();
        let report = h.engine.index(&with_threads(8)).expect("noop");
        println!(
            "incremental, nothing changed (run {round}): {:.1} ms, unchanged {}",
            started.elapsed().as_secs_f64() * 1000.0,
            report.files_unchanged
        );
    }
    for round in 0..3 {
        let (path, text) = common::generated::file_source(1, 5000, 1234);
        write(
            dir.path(),
            &path,
            &format!("{text}\npub fn edited_{round}() -> u8 {{ {round} }}\n"),
        );
        let started = Instant::now();
        let report = h.engine.index(&with_threads(8)).expect("edit");
        println!(
            "incremental, one file edited (run {round}): {:.1} ms, indexed {} unchanged {}, edges {}",
            started.elapsed().as_secs_f64() * 1000.0,
            report.files_indexed,
            report.files_unchanged,
            report.edges_written
        );
    }
}
