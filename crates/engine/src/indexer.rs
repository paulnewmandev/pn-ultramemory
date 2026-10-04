// SPDX-License-Identifier: Apache-2.0
//! Indexing: turning the files of a repository into symbols, references and edges, incrementally.
//!
//! A run lists the files and then, on several threads, reads each one, hashes it, skips it if the
//! stored hash is the same (before any parsing) and extracts the rest. Results are put back in path
//! order and stored in batches of at most [`BATCH_FILES`] files or [`BATCH_BYTES`] bytes of
//! source, one transaction per batch, and the source text of a file is dropped as soon as it was
//! extracted, so memory stays bounded however large the repository is. References are then resolved
//! into edges: everything on the first run or with `force`, otherwise only what the change can
//! affect. Files that disappeared are removed.
//!
//! # Invariants
//! * **Determinism.** What is stored is identical whatever the number of threads, because results
//!   reach the store in path order (see [`pipeline`]).
//! * **No file is fatal.** A file that cannot be read or parsed, or whose parsing panics, is
//!   reported in [`IndexReport::files_skipped`] and the run goes on. Only a storage failure or a
//!   tree that cannot be listed fails the run.

mod pipeline;

use std::collections::HashSet;
use std::path::Path;
use std::time::Instant;

use pn_ultramemory_core::{
    FileExtract, FileId, FileInput, Language, ResolveScope, SourceFile, hash64,
};
use serde_json::{Value, json};

use crate::engine::Engine;
use crate::error::EngineError;
use crate::metrics::Event;
use crate::pagerank;
use pipeline::{Lookahead, ordered_parallel};

/// The most files stored in one transaction.
const BATCH_FILES: usize = 200;

/// The most bytes of source stored in one transaction.
const BATCH_BYTES: u64 = 16 * 1024 * 1024;

/// How many files threads may run ahead of the one being stored.
const LOOKAHEAD_FILES: usize = 256;

/// How many bytes of source threads may run ahead of the one being stored.
const LOOKAHEAD_BYTES: u64 = 32 * 1024 * 1024;

/// The most threads a run uses, whatever was asked for.
const MAX_THREADS: usize = 64;

/// What an index run should do.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IndexOptions {
    /// Parse every file again, even if its content did not change.
    pub force: bool,
    /// Index only these paths (relative, with forward slashes) instead of the whole repository.
    /// Files missing from the list are then not removed from the index.
    pub only_paths: Vec<String>,
    /// Worker threads for parsing, or `None` for the engine's setting.
    pub threads: Option<usize>,
}

/// What an index run did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IndexReport {
    /// Files considered.
    pub files_seen: u32,
    /// Files parsed and stored.
    pub files_indexed: u32,
    /// Files skipped because their content did not change.
    pub files_unchanged: u32,
    /// Files removed from the index because they no longer exist.
    pub files_removed: u32,
    /// Files that could not be indexed, with the reason.
    pub files_skipped: Vec<(String, String)>,
    /// Symbols that did not exist before.
    pub symbols_added: u32,
    /// Symbols that are gone.
    pub symbols_removed: u32,
    /// Symbols whose signature or body changed.
    pub symbols_modified: u32,
    /// Memories that became stale because their code changed.
    pub memories_marked_stale: u32,
    /// Edges written by the resolution step.
    pub edges_written: u64,
    /// Bytes of source that were parsed.
    pub bytes_indexed: u64,
    /// Duration of the run in milliseconds.
    pub elapsed_ms: u64,
}

impl IndexReport {
    /// The report as a structured value, with the skipped files as a uniform table.
    #[must_use]
    pub fn to_value(&self) -> Value {
        let skipped: Vec<Value> = self
            .files_skipped
            .iter()
            .map(|(path, reason)| json!({ "path": path, "reason": reason }))
            .collect();
        let mut value = json!({
            "files_seen": self.files_seen,
            "files_indexed": self.files_indexed,
            "files_unchanged": self.files_unchanged,
            "files_removed": self.files_removed,
            "symbols_added": self.symbols_added,
            "symbols_removed": self.symbols_removed,
            "symbols_modified": self.symbols_modified,
            "memories_marked_stale": self.memories_marked_stale,
            "edges_written": self.edges_written,
            "bytes_indexed": self.bytes_indexed,
            "elapsed_ms": self.elapsed_ms,
        });
        if !skipped.is_empty() {
            value["skipped"] = Value::Array(skipped);
        }
        value
    }
}

/// The content hash used to detect that a file changed: a 64-bit hash and the length in bytes.
///
/// # Examples
/// ```
/// use pn_ultramemory_engine::content_hash;
///
/// assert_eq!(content_hash("a"), content_hash("a"));
/// assert_ne!(content_hash("a"), content_hash("b"));
/// ```
#[must_use]
pub fn content_hash(text: &str) -> String {
    format!("{:016x}-{}", hash64(text.as_bytes()), text.len())
}

/// What became of one file after a thread looked at it.
enum FileOutcome {
    /// Its content hash is the stored one, so nothing was parsed.
    Unchanged,
    /// It could not be indexed, with the reason.
    Skipped(String),
    /// It was extracted and is ready to be stored.
    Parsed(Box<(FileInput, FileExtract)>),
    /// The storage failed while checking it, which fails the whole run.
    Failed(EngineError),
}

/// What the threads of a run need: the files, the mode and the adapters.
struct Workload<'a> {
    /// The files to look at, in path order.
    files: &'a [&'a SourceFile],
    /// Parse every file even if its hash did not change.
    force: bool,
    /// Where the files are read from.
    tree: &'a dyn pn_ultramemory_core::SourceTree,
    /// Where the stored hashes are read from.
    storage: &'a dyn pn_ultramemory_core::Storage,
    /// What turns source into symbols.
    extractor: &'a dyn pn_ultramemory_core::Extractor,
}

impl Workload<'_> {
    /// Reads, hashes and (if it changed) parses file number `index`. The source text is dropped
    /// when this returns.
    fn process(&self, index: usize) -> FileOutcome {
        let file = self.files[index];
        let Some(language) = Language::from_path(Path::new(&file.path)) else {
            return FileOutcome::Skipped("unknown language".into());
        };
        let text = match self.tree.read(&file.path) {
            Ok(text) => text,
            Err(error) => return FileOutcome::Skipped(error.to_string()),
        };
        let hash = content_hash(&text);
        if !self.force {
            match self.storage.file_hash(&file.path) {
                Ok(Some(stored)) if stored == hash => return FileOutcome::Unchanged,
                Ok(_) => {}
                Err(error) => return FileOutcome::Failed(error.into()),
            }
        }
        match self.extractor.extract(language, &text) {
            Ok(extract) => {
                let input = FileInput {
                    path: file.path.clone(),
                    language,
                    hash,
                    size: u64::try_from(text.len()).unwrap_or(u64::MAX),
                    mtime_secs: file.mtime_secs,
                };
                FileOutcome::Parsed(Box::new((input, extract)))
            }
            Err(error) => FileOutcome::Skipped(error.to_string()),
        }
    }
}

/// Collects the outcomes of a run in path order, stores them in batches and builds the report.
struct Collector<'a> {
    /// The engine whose storage receives the batches.
    engine: &'a Engine,
    /// The time stamped on everything stored by this run.
    now: i64,
    /// The report being built.
    report: IndexReport,
    /// The files waiting to be stored.
    batch: Vec<(FileInput, FileExtract)>,
    /// The bytes of source in the batch.
    batch_bytes: u64,
    /// The identities of every file stored, for the resolution step.
    file_ids: Vec<FileId>,
    /// The names whose definitions changed, for the resolution step.
    names: Vec<String>,
}

impl Collector<'_> {
    /// Takes the outcome of one file: counts it, and stores the batch when it is full.
    ///
    /// # Errors
    /// Returns the storage error that made the run fail.
    fn accept(
        &mut self,
        path: &str,
        outcome: Result<FileOutcome, String>,
    ) -> Result<(), EngineError> {
        match outcome {
            Ok(FileOutcome::Unchanged) => self.report.files_unchanged += 1,
            Ok(FileOutcome::Skipped(reason)) => {
                self.report.files_skipped.push((path.to_owned(), reason));
            }
            Ok(FileOutcome::Failed(error)) => return Err(error),
            Ok(FileOutcome::Parsed(parsed)) => {
                let (input, extract) = *parsed;
                self.report.bytes_indexed += input.size;
                self.batch_bytes += input.size;
                self.batch.push((input, extract));
                if self.batch.len() >= BATCH_FILES || self.batch_bytes >= BATCH_BYTES {
                    self.flush()?;
                }
            }
            Err(message) => self
                .report
                .files_skipped
                .push((path.to_owned(), format!("the indexer panicked: {message}"))),
        }
        Ok(())
    }

    /// Stores the waiting files in one transaction and adds what changed to the report.
    ///
    /// # Errors
    /// Returns the storage error.
    fn flush(&mut self) -> Result<(), EngineError> {
        if self.batch.is_empty() {
            return Ok(());
        }
        let outcomes = self.engine.storage().upsert_files(&self.batch, self.now)?;
        self.batch.clear();
        self.batch_bytes = 0;
        for outcome in outcomes {
            self.report.files_indexed += 1;
            self.report.symbols_added += outcome.symbols_added;
            self.report.symbols_removed += outcome.symbols_removed;
            self.report.symbols_modified += outcome.symbols_modified;
            self.report.memories_marked_stale += outcome.memories_marked_stale;
            self.file_ids.extend(outcome.file_id);
            self.names.extend(outcome.changed_names);
        }
        Ok(())
    }
}

impl Engine {
    /// Indexes the repository, or the paths named in the options.
    ///
    /// Files are read, hashed and parsed on `options.threads` threads (the engine's setting when
    /// `None`), unchanged files are skipped before any parsing, and the results are stored in
    /// path order, so the index is the same whatever the number of threads.
    ///
    /// # Errors
    /// Returns a source error if the tree cannot be listed and a storage error if the index cannot
    /// be written. Files that cannot be read or parsed do not fail the run: they are reported in
    /// [`IndexReport::files_skipped`].
    pub fn index(&self, options: &IndexOptions) -> Result<IndexReport, EngineError> {
        self.index_reporting(options, &|_, _| {})
    }

    /// Indexes the repository, calling `progress` as each file is finished with how many are
    /// done and how many there are.
    ///
    /// The callback runs on the thread that collects results, never in parallel and never
    /// while a lock is held, so it is free to write to a terminal. It is called once per file
    /// and once more when every file is done.
    ///
    /// # Errors
    /// The same failures as [`Engine::index`].
    #[allow(clippy::too_many_lines)]
    pub fn index_reporting(
        &self,
        options: &IndexOptions,
        progress: &(dyn Fn(u32, u32) + Sync),
    ) -> Result<IndexReport, EngineError> {
        let started = Instant::now();
        let listed = self.deps.tree.list()?;
        let now = self.now();
        let partial = !options.only_paths.is_empty();
        let wanted: HashSet<&str> = options.only_paths.iter().map(String::as_str).collect();
        let work: Vec<&SourceFile> = listed
            .iter()
            .filter(|file| !partial || wanted.contains(file.path.as_str()))
            .collect();

        let first_index = !partial && self.storage().stats()?.files == 0;
        let mut collector = Collector {
            engine: self,
            now,
            report: IndexReport {
                files_seen: u32::try_from(work.len()).unwrap_or(u32::MAX),
                ..IndexReport::default()
            },
            batch: Vec::new(),
            batch_bytes: 0,
            file_ids: Vec::new(),
            names: Vec::new(),
        };
        let workload = Workload {
            files: &work,
            force: options.force,
            tree: self.deps.tree.as_ref(),
            storage: self.deps.storage.as_ref(),
            extractor: self.deps.extractor.as_ref(),
        };
        let threads = options
            .threads
            .unwrap_or_else(|| self.config.threads())
            .clamp(1, MAX_THREADS);
        let weights: Vec<u64> = work.iter().map(|file| file.size).collect();
        ordered_parallel(
            &weights,
            threads,
            Lookahead {
                items: LOOKAHEAD_FILES,
                weight: LOOKAHEAD_BYTES,
            },
            &|index| workload.process(index),
            |index, outcome| {
                let result = collector.accept(&work[index].path, outcome);
                progress(
                    u32::try_from(index + 1).unwrap_or(u32::MAX),
                    u32::try_from(work.len()).unwrap_or(u32::MAX),
                );
                result
            },
        )?;
        collector.flush()?;

        let Collector {
            mut report,
            mut file_ids,
            mut names,
            ..
        } = collector;
        if !partial {
            let present: Vec<String> = listed.iter().map(|file| file.path.clone()).collect();
            report.files_removed = self.storage().remove_files_not_in(&present, now)?;
        }

        if first_index || options.force {
            report.edges_written = self
                .storage()
                .resolve_edges(&ResolveScope::All)?
                .edges_written;
        } else if !file_ids.is_empty() || report.files_removed > 0 {
            file_ids.sort_unstable();
            names.sort_unstable();
            names.dedup();
            let scope = ResolveScope::Touching { file_ids, names };
            report.edges_written = self.storage().resolve_edges(&scope)?.edges_written;
        }

        // Structural scoring: recompute PageRank on full index or when edges changed
        // significantly. Incremental updates keep the previous scores; the local signals
        // (neighbor, coaccess) compensate for mild staleness between full recomputes.
        if first_index || options.force || report.edges_written > 50 {
            let edges = self.storage().all_edges()?;
            let stats = self.storage().stats()?;
            #[allow(clippy::cast_possible_truncation)]
            let node_count = stats.symbols as usize;
            if node_count > 0 && !edges.is_empty() {
                // Build a compact index: SymbolId -> 0..node_count. The symbol ids in the
                // store are dense after indexing, but we map explicitly to be safe.
                let mut id_to_idx: std::collections::HashMap<i64, usize> =
                    std::collections::HashMap::with_capacity(node_count);
                let mut next_idx = 0_usize;
                let mut compact_edges: Vec<(usize, usize)> = Vec::with_capacity(edges.len());
                for (src, dst) in &edges {
                    let src_idx = *id_to_idx.entry(src.0).or_insert_with(|| {
                        let i = next_idx;
                        next_idx += 1;
                        i
                    });
                    let dst_idx = *id_to_idx.entry(dst.0).or_insert_with(|| {
                        let i = next_idx;
                        next_idx += 1;
                        i
                    });
                    compact_edges.push((src_idx, dst_idx));
                }
                let pr_scores = pagerank::compute(&compact_edges, next_idx);
                let scored: Vec<(pn_ultramemory_core::SymbolId, f64)> = id_to_idx
                    .iter()
                    .filter_map(|(&sym_id, &node_idx)| {
                        pr_scores.get(node_idx).and_then(|&s| {
                            if s > 0.0 {
                                Some((pn_ultramemory_core::SymbolId(sym_id), s))
                            } else {
                                None
                            }
                        })
                    })
                    .collect();
                if !scored.is_empty() {
                    self.storage().update_pageranks(&scored)?;
                }
            }
        }

        self.storage().set_meta("last_index_at", &now.to_string())?;

        report.elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        self.record(Event::Index {
            files: report.files_indexed,
            elapsed_ms: report.elapsed_ms,
        });
        Ok(report)
    }

    /// Indexes exactly these paths, for example after documentation was written into them.
    ///
    /// Called by the documentation operation, which is being written.
    #[allow(
        dead_code,
        reason = "called by the documentation operation, which is being written"
    )]
    ///
    /// # Errors
    /// Same as [`Engine::index`].
    pub(crate) fn reindex_paths(&self, paths: &[String]) -> Result<IndexReport, EngineError> {
        self.index(&IndexOptions {
            only_paths: paths.to_vec(),
            ..IndexOptions::default()
        })
    }
}
