// SPDX-License-Identifier: Apache-2.0
//! Benchmark of the SQLite storage adapter. It prints timings; it asserts nothing.
//!
//! It builds a synthetic repository (default: 1 000 files, 50 000 symbols, 200 000 references),
//! stores it in a file-backed database (once with one transaction per file, once with a single
//! `upsert_files` batch), resolves every reference, and then times 1 000 searches, 1 000 neighbor
//! lookups and the re-storing of a single file followed by the partial resolve it needs.
//!
//! Run it with optimizations, and read the numbers together with the hardware they were measured
//! on:
//!
//! ```text
//! cargo run --release -p pn-ultramemory-store --example bench
//! cargo run --release -p pn-ultramemory-store --example bench -- --quick
//! cargo run --release -p pn-ultramemory-store --example bench -- --no-compare
//! ```
//!
//! `--quick` uses a tenth of the data; `--no-compare` skips the one-transaction-per-file run;
//! `--db PATH` keeps the benchmark database at PATH (replacing any file there) so it can be
//! inspected afterwards.
//!
//! The synthetic names are drawn so that references fall in three groups like real code: a symbol
//! defined in the same file, a name that is unique in the repository, and a common name defined
//! in many places (the ambiguous case that costs the most to resolve). A small share of
//! references name nothing in the index, like calls into a standard library.

use std::time::{Duration, Instant};

use pn_ultramemory_core::{
    Confidence, Direction, FileExtract, FileInput, Language, RefKind, ReferenceDraft, ResolveScope,
    SearchQuery, Span, Storage, SymbolDraft, SymbolId, SymbolKind, Visibility, hash64,
};
use pn_ultramemory_store::SqliteStorage;

/// A small deterministic pseudo-random generator, so every run measures the same data.
struct Rng(u64);

impl Rng {
    /// The next raw value.
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }

    /// A value in `0..bound`.
    fn below(&mut self, bound: usize) -> usize {
        let bound = u64::try_from(bound.max(1)).unwrap_or(1);
        usize::try_from(self.next() % bound).unwrap_or(0)
    }
}

/// Words that identifiers are built from.
const WORDS: [&str; 24] = [
    "parse", "load", "save", "render", "config", "buffer", "token", "query", "index", "cache",
    "stream", "handler", "session", "record", "schema", "widget", "route", "policy", "metric",
    "packet", "socket", "worker", "filter", "mapper",
];

/// Names shared by many symbols across the repository.
const COMMON: [&str; 40] = [
    "new", "get", "set", "run", "init", "start", "stop", "open", "close", "read", "write", "next",
    "build", "apply", "check", "clear", "reset", "update", "insert", "remove", "find", "count",
    "len", "is_empty", "iter", "map", "filter", "merge", "split", "join", "flush", "sync", "drop",
    "clone", "eq", "cmp", "fmt", "hash", "from", "into",
];

/// Names that never resolve, like calls into a standard library.
const EXTERNAL: [&str; 6] = [
    "println",
    "format",
    "unwrap",
    "collect",
    "to_string",
    "push",
];

/// How many files, symbols per file and references per file to generate.
struct Scale {
    /// Number of files.
    files: usize,
    /// Symbols in each file.
    symbols_per_file: usize,
    /// References in each file.
    refs_per_file: usize,
}

/// A generated file: its path, symbols and references.
struct Generated {
    /// The path of the file.
    path: String,
    /// The symbols of the file.
    symbols: Vec<SymbolDraft>,
    /// The references of the file.
    references: Vec<ReferenceDraft>,
}

/// Builds the name of the `n`th symbol of a file: mostly unique names, some common ones.
fn symbol_name(rng: &mut Rng, file: usize, n: usize) -> String {
    if rng.below(100) < 35 {
        COMMON[rng.below(COMMON.len())].to_owned()
    } else {
        let first = WORDS[rng.below(WORDS.len())];
        let second = WORDS[rng.below(WORDS.len())];
        format!("{first}_{second}_{file}_{n}")
    }
}

/// Generates one file of the synthetic repository.
fn generate_file(
    rng: &mut Rng,
    index: usize,
    scale: &Scale,
    all_names: &[Vec<String>],
) -> Generated {
    let directory = index % 40;
    let path = format!("src/module{directory}/file{index}.rs");
    let names = &all_names[index];
    let symbols: Vec<SymbolDraft> = names
        .iter()
        .enumerate()
        .map(|(n, name)| {
            let line = u32::try_from(n * 12 + 1).unwrap_or(1);
            let signature = format!("pub fn {name}(input: &Input) -> Output");
            SymbolDraft {
                name: name.clone(),
                qualified_name: name.clone(),
                kind: SymbolKind::Function,
                sig_hash: hash64(signature.as_bytes()),
                body_hash: hash64(format!("{signature} {{ body {index} {n} }}").as_bytes()),
                signature,
                doc: (n % 3 == 0).then(|| format!("Handles the {name} step of the pipeline.")),
                visibility: Visibility::Public,
                span: Span {
                    start_line: line,
                    end_line: line + 9,
                    start_byte: line * 40,
                    end_byte: line * 40 + 400,
                },
                parent: None,
                outline: vec!["helper".to_owned()],
            }
        })
        .collect();
    let references: Vec<ReferenceDraft> = (0..scale.refs_per_file)
        .map(|r| {
            let owner = rng.below(scale.symbols_per_file);
            let roll = rng.below(100);
            let name = if roll < 40 {
                names[rng.below(names.len())].clone()
            } else if roll < 75 {
                let other = &all_names[rng.below(all_names.len())];
                other[rng.below(other.len())].clone()
            } else if roll < 95 {
                COMMON[rng.below(COMMON.len())].to_owned()
            } else {
                EXTERNAL[rng.below(EXTERNAL.len())].to_owned()
            };
            ReferenceDraft {
                name,
                kind: if r % 9 == 0 {
                    RefKind::Type
                } else {
                    RefKind::Call
                },
                line: u32::try_from(owner * 12 + 3).unwrap_or(3),
                owner: Some(owner),
                qualifier: None,
            }
        })
        .collect();
    Generated {
        path,
        symbols,
        references,
    }
}

/// The input of the storage port for a generated file.
fn as_input(file: &Generated, hash: &str) -> (FileInput, FileExtract) {
    (
        FileInput {
            path: file.path.clone(),
            language: Language::Rust,
            hash: hash.to_owned(),
            size: 4_000,
            mtime_secs: 0,
        },
        FileExtract {
            language: Language::Rust,
            symbols: file.symbols.clone(),
            references: file.references.clone(),
            imports: Vec::new(),
            line_count: 700,
            parse_errors: 0,
        },
    )
}

/// The time every write of the benchmark is stamped with.
const NOW: i64 = 1_700_000_000;

/// Milliseconds with two decimals.
fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

/// Converts a count to a float for a rate; the counts here are far below 2^52.
#[allow(clippy::cast_precision_loss)]
fn count_f64(count: usize) -> f64 {
    count as f64
}

/// Converts a byte size to mebibytes.
#[allow(clippy::cast_precision_loss)]
fn mebibytes(bytes: u64) -> f64 {
    bytes as f64 / 1_048_576.0
}

/// Prints one benchmark line.
fn report(label: &str, duration: Duration, detail: &str) {
    println!("{label:<34} {:>10.2} ms   {detail}", ms(duration));
}

/// Turns any displayable error into the message the benchmark reports.
fn failed(error: impl std::fmt::Display) -> String {
    error.to_string()
}

/// Generates every file of the synthetic repository.
fn generate(rng: &mut Rng, scale: &Scale) -> Vec<Generated> {
    // Every symbol name first, so references can point at names defined in other files.
    let all_names: Vec<Vec<String>> = (0..scale.files)
        .map(|file| {
            (0..scale.symbols_per_file)
                .map(|n| symbol_name(rng, file, n))
                .collect()
        })
        .collect();
    (0..scale.files)
        .map(|index| generate_file(rng, index, scale, &all_names))
        .collect()
}

/// Times storing every file with one `upsert_file`, so one transaction, per file.
fn bench_insert_per_file(
    store: &SqliteStorage,
    inputs: &[(FileInput, FileExtract)],
) -> Result<(), String> {
    let symbols: usize = inputs.iter().map(|(_, e)| e.symbols.len()).sum();
    let started = Instant::now();
    for (file, extract) in inputs {
        store.upsert_file(file, extract, NOW).map_err(failed)?;
    }
    let elapsed = started.elapsed();
    report(
        "insert, one transaction per file",
        elapsed,
        &format!(
            "{:.0} symbols/s",
            count_f64(symbols) / elapsed.as_secs_f64()
        ),
    );
    Ok(())
}

/// Times storing every file with one `upsert_files` call: a single transaction.
fn bench_insert_batch(
    store: &SqliteStorage,
    inputs: &[(FileInput, FileExtract)],
) -> Result<(), String> {
    let symbols: usize = inputs.iter().map(|(_, e)| e.symbols.len()).sum();
    let started = Instant::now();
    let outcomes = store.upsert_files(inputs, NOW).map_err(failed)?;
    let elapsed = started.elapsed();
    report(
        "insert, upsert_files (one batch)",
        elapsed,
        &format!(
            "{:.0} symbols/s, {} outcomes",
            count_f64(symbols) / elapsed.as_secs_f64(),
            outcomes.len()
        ),
    );
    Ok(())
}

/// Times resolving every reference, twice, to show that it is idempotent.
fn bench_resolve(store: &SqliteStorage, references: usize) -> Result<(), String> {
    let started = Instant::now();
    let stats = store.resolve_edges(&ResolveScope::All).map_err(failed)?;
    let elapsed = started.elapsed();
    report(
        "resolve_edges(All)",
        elapsed,
        &format!(
            "{references} references -> {} edges written ({:.0} refs/s)",
            stats.edges_written,
            count_f64(references) / elapsed.as_secs_f64()
        ),
    );
    let started = Instant::now();
    let again = store.resolve_edges(&ResolveScope::All).map_err(failed)?;
    report(
        "resolve_edges(All) again",
        started.elapsed(),
        &format!("{} edges (idempotent)", again.edges_written),
    );
    Ok(())
}

/// Times 1 000 searches over a mix of exact, `camelCase`, `snake_case`, prefix and multi-word
/// queries.
fn bench_search(store: &SqliteStorage) -> Result<(), String> {
    let queries: Vec<String> = (0..1_000)
        .map(|i| match i % 5 {
            0 => COMMON[i % COMMON.len()].to_owned(),
            1 => format!("{}{}", WORDS[i % 24], capitalize(WORDS[(i / 24) % 24])),
            2 => format!("{}_{}", WORDS[i % 24], WORDS[(i / 7) % 24]),
            3 => WORDS[i % 24][..3].to_owned(),
            _ => format!("{} {} pipeline step", WORDS[i % 24], WORDS[(i / 5) % 24]),
        })
        .collect();
    let mut hits_total = 0_usize;
    let mut slowest = Duration::ZERO;
    let started = Instant::now();
    for query in &queries {
        let one = Instant::now();
        let request = SearchQuery {
            text: query.clone(),
            ..SearchQuery::default()
        };
        hits_total += store.search_symbols(&request, 20).map_err(failed)?.len();
        slowest = slowest.max(one.elapsed());
    }
    let elapsed = started.elapsed();
    report(
        "1000 x search_symbols (limit 20)",
        elapsed,
        &format!(
            "{:.3} ms/query, slowest {:.2} ms, {hits_total} hits",
            ms(elapsed) / 1_000.0,
            ms(slowest)
        ),
    );
    Ok(())
}

/// Times 1 000 neighbor lookups on symbols spread over the repository.
fn bench_neighbors(
    store: &SqliteStorage,
    files: &[(FileInput, FileExtract)],
) -> Result<(), String> {
    let mut sample: Vec<SymbolId> = Vec::new();
    for (file, _) in files.iter().step_by((files.len() / 50).max(1)) {
        let stored = store.symbols_in_file(&file.path).map_err(failed)?;
        sample.extend(stored.iter().map(|s| s.id));
    }
    let mut neighbors_total = 0_usize;
    let started = Instant::now();
    for i in 0..1_000 {
        let id = sample[i % sample.len()];
        let direction = if i % 2 == 0 {
            Direction::Out
        } else {
            Direction::In
        };
        neighbors_total += store
            .neighbors(id, direction, Confidence::Guess, 50)
            .map_err(failed)?
            .len();
    }
    let elapsed = started.elapsed();
    report(
        "1000 x neighbors (limit 50)",
        elapsed,
        &format!(
            "{:.3} ms/lookup, {neighbors_total} neighbors returned",
            ms(elapsed) / 1_000.0
        ),
    );
    Ok(())
}

/// Times storing one changed file again (one symbol edited), then resolving what it touches.
fn bench_reupsert(store: &SqliteStorage, files: &[(FileInput, FileExtract)]) -> Result<(), String> {
    let (file, extract) = &files[500 % files.len()];
    let mut changed = extract.clone();
    changed.symbols[0].body_hash ^= 0xff;
    let started = Instant::now();
    let outcome = store.upsert_file(file, &changed, NOW).map_err(failed)?;
    let upsert = started.elapsed();
    let file_id = outcome.file_id.ok_or("the changed file has no id")?;
    let started = Instant::now();
    let touching = store
        .resolve_edges(&ResolveScope::Touching {
            file_ids: vec![file_id],
            names: outcome.changed_names,
        })
        .map_err(failed)?;
    let partial = started.elapsed();
    report(
        "re-upsert of one file",
        upsert,
        &format!(
            "{} symbols, {} references, {} modified",
            changed.symbols.len(),
            changed.references.len(),
            outcome.symbols_modified
        ),
    );
    report(
        "resolve_edges(Touching) after it",
        partial,
        &format!("{} edges rewritten", touching.edges_written),
    );
    Ok(())
}

/// Times the four aggregate queries used by reports.
fn bench_reports(store: &SqliteStorage) -> Result<(), String> {
    let started = Instant::now();
    let modules = store.module_stats(2).map_err(failed)?;
    report(
        "module_stats(depth 2)",
        started.elapsed(),
        &format!("{} modules", modules.len()),
    );
    let started = Instant::now();
    let shallow = store.module_stats(1).map_err(failed)?;
    report(
        "module_stats(depth 1)",
        started.elapsed(),
        &format!("{} modules", shallow.len()),
    );
    let started = Instant::now();
    let pairs = store
        .module_edges(2, Confidence::Heuristic, 20)
        .map_err(failed)?;
    report(
        "module_edges(depth 2, top 20)",
        started.elapsed(),
        &format!(
            "{} pairs, heaviest {}",
            pairs.len(),
            pairs.first().map_or(0, |p| p.weight)
        ),
    );
    let started = Instant::now();
    let coverage = store.doc_coverage().map_err(failed)?;
    report(
        "doc_coverage()",
        started.elapsed(),
        &format!(
            "{} languages, {} public symbols",
            coverage.len(),
            coverage.iter().map(|c| c.public_symbols).sum::<u64>()
        ),
    );
    let started = Instant::now();
    let totals = store.file_totals().map_err(failed)?;
    report(
        "file_totals()",
        started.elapsed(),
        &format!(
            "{} lines, {} files with parse errors",
            totals.lines, totals.parse_error_files
        ),
    );
    Ok(())
}

/// Runs the benchmark and returns an error message on failure.
fn run(scale: &Scale, compare: bool, keep: Option<&std::path::Path>) -> Result<(), String> {
    let mut rng = Rng(0x5eed);
    let directory = tempfile::tempdir().map_err(failed)?;
    let files = generate(&mut rng, scale);
    let inputs: Vec<(FileInput, FileExtract)> = files.iter().map(|f| as_input(f, "v1")).collect();
    let symbols: usize = files.iter().map(|f| f.symbols.len()).sum();
    let references: usize = files.iter().map(|f| f.references.len()).sum();
    println!(
        "dataset: {} files, {symbols} symbols, {references} references (file-backed, WAL)",
        files.len()
    );

    // The same data stored one transaction per file, for comparison; that database is discarded.
    if compare {
        let comparison =
            SqliteStorage::open(&directory.path().join("per-file.db")).map_err(failed)?;
        bench_insert_per_file(&comparison, &inputs)?;
    }

    let path = keep.map_or_else(
        || directory.path().join("bench.db"),
        std::path::Path::to_path_buf,
    );
    for suffix in ["", "-wal", "-shm"] {
        let mut leftover = path.clone().into_os_string();
        leftover.push(suffix);
        let _ = std::fs::remove_file(leftover);
    }
    let store = SqliteStorage::open(&path).map_err(failed)?;
    bench_insert_batch(&store, &inputs)?;
    bench_resolve(&store, references)?;
    bench_search(&store)?;
    bench_neighbors(&store, &inputs)?;
    bench_reupsert(&store, &inputs)?;
    bench_reports(&store)?;

    let size = std::fs::metadata(&path).map_or(0, |m| m.len());
    let stats = store.stats().map_err(failed)?;
    println!(
        "database file: {:.1} MiB, {} edges, {} symbols",
        mebibytes(size),
        stats.edges,
        stats.symbols
    );
    Ok(())
}

/// Uppercases the first letter of a word.
fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().collect::<String>() + chars.as_str()
    })
}

/// Entry point: `--quick` runs a tenth of the default size, `--no-compare` skips the per-file run.
fn main() {
    let quick = std::env::args().any(|arg| arg == "--quick");
    let scale = if quick {
        Scale {
            files: 100,
            symbols_per_file: 50,
            refs_per_file: 200,
        }
    } else {
        Scale {
            files: 1_000,
            symbols_per_file: 50,
            refs_per_file: 200,
        }
    };
    let compare = !std::env::args().any(|arg| arg == "--no-compare");
    let mut args = std::env::args();
    let keep = args
        .by_ref()
        .skip_while(|arg| arg != "--db")
        .nth(1)
        .map(std::path::PathBuf::from);
    if let Err(message) = run(&scale, compare, keep.as_deref()) {
        eprintln!("benchmark failed: {message}");
        std::process::exit(1);
    }
}
