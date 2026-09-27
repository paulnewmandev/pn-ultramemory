// SPDX-License-Identifier: Apache-2.0
//! The offline retrieval benchmark: honest, reproducible numbers about what a capsule costs and
//! whether it contains what was asked for.
//!
//! # Role in the architecture
//! Application layer. [`Engine::bench`] drives the ordinary [`Engine::recall`] path over questions
//! built from the repository's own documentation and compares it with the crudest thing an agent can
//! do without an index: reading whole files chosen by counting query words (see [`baseline`]).
//! **No language model is involved at any point**, so nothing here needs a network, a key or a
//! provider, and anyone can rerun it and get the same numbers.
//!
//! # What a task is
//! A task is one documented symbol (see [`tasks`]). Each one yields two queries, and therefore two
//! task *families*:
//!
//! * **description** — the first sentence of the symbol's documentation with every word that also
//!   appears in the symbol's own name removed, so the query cannot simply repeat the answer;
//! * **name** — the symbol's name split into lowercase words.
//!
//! For every task and every budget the engine answers with a capsule. A **hit** is the wanted symbol
//! appearing in that capsule, and the cost is the capsule's own `used` count. The baseline runs once
//! per task, because reading files has no budget; its hit is the symbol's own file being among the
//! files it read.
//!
//! # Why there is no single score
//! A composite score would be the most requested number here and the least defensible one. Weights
//! chosen once become the thing the code is optimized for, and they hide the trade-off: a score can
//! rise while the retriever returns more plausible dead ends, because a dead end costs the score
//! nothing and costs its reader everything. Each figure is therefore reported on its own, with how it
//! was obtained ([`Derivation`]), and the one figure that would matter most — whether an agent
//! finished the task — is reported as `null` rather than invented, because no agent ran.
//!
//! # Determinism
//! For a fixed seed, a fixed repository and a fixed set of budgets, every count, rate and token
//! figure is byte-identical between runs: the sample comes from a seeded generator, every list is
//! sorted with an explicit tie-break, and the benchmark clears the session record before each recall
//! so that it never teaches the learner anything and so no run is biased by the one before it. The
//! two latency metrics (`p50_ms`, `p95_ms`) are wall-clock measurements and are the only figures that
//! differ between two runs; a caller comparing two reports should compare everything else.
//!
//! What the benchmark reads from storage, it reads as it is: if this repository already carries
//! learned evidence, recall is biased by it exactly as it would be in use. `pn-ultramemory learn
//! reset` clears that evidence when a clean comparison is wanted.

mod baseline;
mod tasks;

use std::collections::BTreeSet;
use std::time::Instant;

use serde_json::{Value, json};

use crate::engine::Engine;
use crate::error::EngineError;
use crate::recall::RecallQuery;
use baseline::Corpus;
use tasks::{Task, sample_tasks};

/// How many tasks a run samples when the caller asks for no particular number.
const DEFAULT_TASKS: usize = 100;

/// The most tasks one run will sample, however many were asked for.
const MAX_TASKS: usize = 2_000;

/// The budgets a run measures when the caller names none.
const DEFAULT_BUDGETS: [u32; 3] = [500, 1000, 2000];

/// How many files the baseline reads when the caller names no number.
const DEFAULT_BASELINE_FILES: usize = 3;

/// The most files the baseline will read for one query.
const MAX_BASELINE_FILES: usize = 50;

/// The names of the metrics every row carries, in the order they are reported.
const METRIC_NAMES: [&str; 8] = [
    "hit_rate",
    "mean_tokens",
    "baseline_hit_rate",
    "baseline_mean_tokens",
    "tokens_saved_ratio",
    "p50_ms",
    "p95_ms",
    "task_success",
];

/// The one figure of a row that no run of this benchmark can observe.
const UNOBSERVABLE_METRIC: &str = "task_success";

/// The queries come from the symbols' own documentation.
const NOTE_KNOWN_ITEM: &str = "The queries are derived from the documentation of the symbols \
     themselves, so this measures known-item retrieval and not real tasks.";

/// What the baseline arm actually is.
const NOTE_BASELINE: &str = "The baseline is a simple lexical ranking of whole files: it reads \
     the files that contain the most distinct words of the query, in full.";

/// What a hit does and does not mean.
const NOTE_HIT: &str = "A hit means the symbol is present in the capsule, not that a model \
     solved anything.";

/// Where the token counts come from.
const NOTE_TOKENS: &str = "Token counts use this project's estimator, which is calibrated \
     against a real tokenizer to about 4.5 % mean error, not a provider's billed count.";

/// Results are a property of the repository as much as of the tool.
const NOTE_REPOSITORY: &str = "Results depend on the repository: another codebase, another set \
     of numbers.";

/// No model ran, so nothing about task success is observable.
const NOTE_NO_MODEL: &str = "No language model ran, so `task_success` has no value and is \
     reported as null rather than as a zero.";

/// The latency figures cannot be reproduced byte for byte.
const NOTE_LATENCY: &str = "Every figure except `p50_ms` and `p95_ms` is identical between two \
     runs of the same seed; those two are wall-clock measurements.";

/// What a benchmark run should do.
///
/// # Examples
/// ```
/// use pn_ultramemory_engine::BenchOptions;
///
/// let options = BenchOptions::default();
/// assert_eq!(options.tasks, 100);
/// assert_eq!(options.seed, 1);
/// assert_eq!(options.budgets, vec![500, 1000, 2000]);
/// assert_eq!(options.baseline_files, 3);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BenchOptions {
    /// How many documented symbols to ask about. Zero means the default of 100.
    pub tasks: usize,
    /// The seed of the sample. The same seed over the same index draws the same tasks.
    pub seed: u64,
    /// The token budgets to measure. Empty means the default of 500, 1 000 and 2 000.
    pub budgets: Vec<u32>,
    /// How many files the baseline arm reads for each query.
    pub baseline_files: usize,
}

impl Default for BenchOptions {
    fn default() -> Self {
        Self {
            tasks: DEFAULT_TASKS,
            seed: 1,
            budgets: DEFAULT_BUDGETS.to_vec(),
            baseline_files: DEFAULT_BASELINE_FILES,
        }
    }
}

/// How a figure was obtained. It is part of the figure: a number whose provenance is unstated
/// invites being read as something it is not.
///
/// # Examples
/// ```
/// use pn_ultramemory_engine::Derivation;
///
/// assert_eq!(Derivation::Measured.as_str(), "measured");
/// assert_eq!(Derivation::Proxy.as_str(), "proxy");
/// assert_eq!(Derivation::Unobservable.as_str(), "unobservable");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Derivation {
    /// Counted directly from what the run did.
    Measured,
    /// Stands in for something that was not measured, such as an estimate of a real tokenizer.
    Proxy,
    /// Cannot be observed by this benchmark at all. Its value is always `None`.
    Unobservable,
}

impl Derivation {
    /// Stable lowercase name used in output.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Measured => "measured",
            Self::Proxy => "proxy",
            Self::Unobservable => "unobservable",
        }
    }
}

/// One figure of one row.
///
/// `value` is an option on purpose: something this benchmark cannot observe serializes as `null`,
/// never as a fabricated zero, because a zero would be indistinguishable from a real measurement of
/// total failure.
#[derive(Debug, Clone, PartialEq)]
pub struct Metric {
    /// The name of the figure, such as `hit_rate`.
    pub name: String,
    /// The figure, or `None` when it could not be observed.
    pub value: Option<f64>,
    /// How it was obtained.
    pub derivation: Derivation,
}

/// Everything measured for one family of tasks at one budget.
#[derive(Debug, Clone, PartialEq)]
pub struct BenchRow {
    /// The family of task: `description` or `name`.
    pub family: String,
    /// The token budget the capsules were packed to.
    pub budget: u32,
    /// How many tasks of the family were run. A task whose query came out empty is not run.
    pub tasks: usize,
    /// The figures, in a fixed order: `hit_rate`, `mean_tokens`, `baseline_hit_rate`,
    /// `baseline_mean_tokens`, `tokens_saved_ratio`, `p50_ms`, `p95_ms`, `task_success`.
    pub metrics: Vec<Metric>,
}

impl BenchRow {
    /// The value of one figure of this row, or `None` when it is absent or unobservable.
    #[must_use]
    pub fn value(&self, name: &str) -> Option<f64> {
        self.metrics
            .iter()
            .find(|metric| metric.name == name)
            .and_then(|metric| metric.value)
    }
}

/// What a benchmark run measured, with the limits of the measurement written down beside it.
#[derive(Debug, Clone, PartialEq)]
pub struct BenchReport {
    /// Symbols in the index the run measured.
    pub indexed_symbols: u64,
    /// How many source files those symbols came from, which is also the corpus the baseline arm
    /// could choose to read.
    pub indexed_files: u64,
    /// One row per family and budget, families in order and budgets ascending.
    pub rows: Vec<BenchRow>,
    /// What these numbers do and do not say, in plain sentences.
    pub notes: Vec<String>,
}

/// A `u64` as the nearest `f64`, without a truncating cast.
fn to_f64(value: u64) -> f64 {
    let high = u32::try_from(value >> 32).unwrap_or(u32::MAX);
    let low = u32::try_from(value & 0xFFFF_FFFF).unwrap_or(u32::MAX);
    f64::from(high) * 4_294_967_296.0 + f64::from(low)
}

/// A count as an `f64`.
fn count_f64(value: usize) -> f64 {
    to_f64(u64::try_from(value).unwrap_or(u64::MAX))
}

/// Rounds to four decimals, so that two runs of the same seed print the same digits.
fn round4(value: f64) -> f64 {
    (value * 10_000.0).round() / 10_000.0
}

/// A ratio of two counts, or `None` when there is nothing to divide by.
fn ratio(part: usize, whole: usize) -> Option<f64> {
    if whole == 0 {
        return None;
    }
    Some(round4(count_f64(part) / count_f64(whole)))
}

/// A mean of a total over a count, or `None` when the count is zero.
fn mean(total: u64, count: usize) -> Option<f64> {
    if count == 0 {
        return None;
    }
    Some(round4(to_f64(total) / count_f64(count)))
}

/// The value at a percentile of a sorted list, by nearest rank.
fn percentile(sorted: &[f64], pct: usize) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }
    let rank = sorted.len().saturating_mul(pct).div_ceil(100).max(1);
    sorted.get(rank - 1).copied().map(round4)
}

/// A number with three decimals, for the lines of a summary.
fn decimals(value: Option<f64>) -> String {
    match value {
        Some(number) => format!("{number:.3}"),
        None => "null".to_owned(),
    }
}

/// The two families of task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    /// The query is the symbol's documentation without the words of its name.
    Description,
    /// The query is the symbol's name, split into words.
    Name,
}

impl Family {
    /// Stable lowercase name used in output.
    const fn as_str(self) -> &'static str {
        match self {
            Self::Description => "description",
            Self::Name => "name",
        }
    }

    /// The query this family builds for a task, which may be empty.
    fn query(self, task: &Task) -> &str {
        match self {
            Self::Description => &task.description,
            Self::Name => &task.name,
        }
    }
}

/// What one family at one budget produced, before it becomes a row of figures.
#[derive(Debug, Default)]
struct Arm {
    /// Tasks that were run.
    tasks: usize,
    /// Capsules that held the wanted symbol.
    hits: usize,
    /// Tokens of every capsule together.
    tokens: u64,
    /// Baseline runs whose files held the wanted symbol.
    baseline_hits: usize,
    /// Tokens of every baseline run together.
    baseline_tokens: u64,
    /// The latency of every recall in milliseconds, sorted ascending.
    latencies: Vec<f64>,
}

impl Arm {
    /// The eight figures of this arm, named and ordered by `METRIC_NAMES`.
    ///
    /// The last one, `task_success`, is always absent: it is what a reader most wants and what this
    /// benchmark cannot see.
    fn metrics(&self) -> Vec<Metric> {
        let engine_mean = mean(self.tokens, self.tasks);
        let baseline_mean = mean(self.baseline_tokens, self.tasks);
        let saved = match (engine_mean, baseline_mean) {
            (Some(ours), Some(theirs)) if theirs > 0.0 => Some(round4((theirs - ours) / theirs)),
            _ => None,
        };
        let values: [Option<f64>; 8] = [
            ratio(self.hits, self.tasks),
            engine_mean,
            ratio(self.baseline_hits, self.tasks),
            baseline_mean,
            saved,
            percentile(&self.latencies, 50),
            percentile(&self.latencies, 95),
            None,
        ];
        METRIC_NAMES
            .iter()
            .zip(values)
            .map(|(name, value)| Metric {
                name: (*name).to_owned(),
                value,
                derivation: if *name == UNOBSERVABLE_METRIC {
                    Derivation::Unobservable
                } else {
                    Derivation::Measured
                },
            })
            .collect()
    }
}

impl BenchReport {
    /// The report as a structured value: the two index counts, one long uniform table with a row per
    /// figure, and the notes.
    ///
    /// The table is long rather than wide so that every figure carries its own derivation, and so
    /// that TOON declares the shape once for the whole run.
    #[must_use]
    pub fn to_value(&self) -> Value {
        let mut metrics = Vec::new();
        for row in &self.rows {
            for metric in &row.metrics {
                metrics.push(json!({
                    "family": row.family,
                    "budget": row.budget,
                    "tasks": row.tasks,
                    "metric": metric.name,
                    "value": metric.value,
                    "derivation": metric.derivation.as_str(),
                }));
            }
        }
        json!({
            "indexed_symbols": self.indexed_symbols,
            "indexed_files": self.indexed_files,
            "metrics": metrics,
            "notes": self.notes,
        })
    }

    /// The report as short lines, one per row, for reading in a terminal.
    #[must_use]
    pub fn summary(&self) -> String {
        let mut out = String::new();
        out.push_str("indexed ");
        out.push_str(&self.indexed_symbols.to_string());
        out.push_str(" symbols in ");
        out.push_str(&self.indexed_files.to_string());
        out.push_str(" files\n");
        for row in &self.rows {
            out.push_str(&row.family);
            out.push_str(" @");
            out.push_str(&row.budget.to_string());
            out.push_str(" (");
            out.push_str(&row.tasks.to_string());
            out.push_str(" tasks): hit ");
            out.push_str(&decimals(row.value("hit_rate")));
            out.push_str(", tokens ");
            out.push_str(&decimals(row.value("mean_tokens")));
            out.push_str(" | baseline hit ");
            out.push_str(&decimals(row.value("baseline_hit_rate")));
            out.push_str(", tokens ");
            out.push_str(&decimals(row.value("baseline_mean_tokens")));
            out.push_str(" | saved ");
            out.push_str(&decimals(row.value("tokens_saved_ratio")));
            out.push_str(" | p50 ");
            out.push_str(&decimals(row.value("p50_ms")));
            out.push_str(" ms, p95 ");
            out.push_str(&decimals(row.value("p95_ms")));
            out.push_str(" ms\n");
        }
        out.push_str("task_success: not measured, because no language model ran\n");
        out
    }
}

/// What one run actually did, as the notes report it: the values after every default and every
/// limit was applied, not the ones that were asked for.
#[derive(Debug, Clone, Copy)]
struct RunShape {
    /// Tasks that were drawn.
    tasks: usize,
    /// The seed the sample came from.
    seed: u64,
    /// Files the baseline read for each query.
    files_read: usize,
    /// Distinct budgets that were measured.
    budgets: usize,
}

/// The notes of one run: the fixed limits, then what this particular run did.
fn notes_for(run: &RunShape, corpus: &Corpus) -> Vec<String> {
    let mut notes: Vec<String> = [
        NOTE_KNOWN_ITEM,
        NOTE_BASELINE,
        NOTE_HIT,
        NOTE_TOKENS,
        NOTE_REPOSITORY,
        NOTE_NO_MODEL,
        NOTE_LATENCY,
    ]
    .iter()
    .map(|note| (*note).to_owned())
    .collect();

    let mut line = String::new();
    line.push_str("This run drew ");
    line.push_str(&run.tasks.to_string());
    line.push_str(" documented symbols with seed ");
    line.push_str(&run.seed.to_string());
    line.push_str(", read ");
    line.push_str(&run.files_read.to_string());
    line.push_str(" files per baseline query out of ");
    line.push_str(&corpus.len().to_string());
    line.push_str(", and measured ");
    line.push_str(&run.budgets.to_string());
    line.push_str(" budgets.");
    notes.push(line);

    if corpus.left_out > 0 {
        let mut sampled = String::new();
        sampled.push_str(
            "The repository is larger than the read cache, or some files could not be \
             read, so the baseline ranked a deterministic sample and left ",
        );
        sampled.push_str(&corpus.left_out.to_string());
        sampled.push_str(" files out.");
        notes.push(sampled);
    }
    if run.tasks == 0 {
        notes.push(
            "No symbol in this index carries enough documentation to build a question from, so \
             every figure is null. Index a repository with `pn-ultramemory index` first."
                .to_owned(),
        );
    }
    notes
}

impl Engine {
    /// Forgets the last capsule, so the benchmark neither learns from itself nor biases the next
    /// measurement.
    fn clear_session(&self) {
        let mut session = self.session();
        session.shown.clear();
        session.expanded.clear();
        session.shown_at = None;
    }

    /// Measures one family at one budget over the tasks whose query is not empty.
    ///
    /// # Errors
    /// Returns the error `recall` returned, which can only be a storage failure here: every query
    /// handed to it is non-empty.
    fn run_arm(
        &self,
        budget: u32,
        runnable: &[(&Task, &str)],
        baselines: &[baseline::BaselineRun],
    ) -> Result<Arm, EngineError> {
        let mut arm = Arm {
            tasks: runnable.len(),
            ..Arm::default()
        };
        for (index, (task, query)) in runnable.iter().enumerate() {
            self.clear_session();
            let query = RecallQuery {
                text: (*query).to_owned(),
                budget: Some(budget),
                explain: false,
                path_prefix: None,
            };
            let started = Instant::now();
            let capsule = self.recall(&query)?;
            arm.latencies.push(started.elapsed().as_secs_f64() * 1000.0);
            if capsule.symbols.iter().any(|symbol| symbol.id == task.id) {
                arm.hits += 1;
            }
            arm.tokens = arm.tokens.saturating_add(u64::from(capsule.used));
            if let Some(run) = baselines.get(index) {
                if run.hit {
                    arm.baseline_hits += 1;
                }
                arm.baseline_tokens = arm.baseline_tokens.saturating_add(u64::from(run.tokens));
            }
        }
        arm.latencies.sort_by(f64::total_cmp);
        Ok(arm)
    }

    /// Runs the offline retrieval benchmark over this repository.
    ///
    /// See the documentation of this module for what a task is, what the baseline does, why there is
    /// no composite score and exactly which figures are reproducible. An index with no documented
    /// symbol is not an error: the rows then report zero tasks and null figures, and a note says so.
    ///
    /// # Errors
    /// Returns a storage error when the index cannot be read, and a source error when the files the
    /// baseline needs cannot be listed. A file that cannot be read is left out of the baseline
    /// corpus rather than failing the run.
    pub fn bench(&self, options: &BenchOptions) -> Result<BenchReport, EngineError> {
        let stats = self.storage().stats()?;
        let requested = if options.budgets.is_empty() {
            DEFAULT_BUDGETS.to_vec()
        } else {
            options.budgets.clone()
        };
        let mut budgets: Vec<u32> = requested
            .iter()
            .map(|budget| self.config.effective_budget(Some(*budget)))
            .collect();
        budgets.sort_unstable();
        budgets.dedup();

        let wanted = if options.tasks == 0 {
            DEFAULT_TASKS
        } else {
            options.tasks.min(MAX_TASKS)
        };
        let files_read = options.baseline_files.clamp(1, MAX_BASELINE_FILES);
        let drawn = sample_tasks(self, wanted, options.seed)?;
        let required: BTreeSet<String> = drawn.iter().map(|task| task.path.clone()).collect();
        let corpus = Corpus::build(self, &required)?;

        let mut rows = Vec::with_capacity(budgets.len() * 2);
        for family in [Family::Description, Family::Name] {
            let runnable: Vec<(&Task, &str)> = drawn
                .iter()
                .map(|task| (task, family.query(task)))
                .filter(|(_, query)| !query.is_empty())
                .collect();
            let baselines: Vec<baseline::BaselineRun> = runnable
                .iter()
                .map(|(task, query)| corpus.run(query, &task.path, files_read))
                .collect();
            for budget in &budgets {
                let arm = self.run_arm(*budget, &runnable, &baselines)?;
                rows.push(BenchRow {
                    family: family.as_str().to_owned(),
                    budget: *budget,
                    tasks: arm.tasks,
                    metrics: arm.metrics(),
                });
            }
        }
        self.clear_session();
        Ok(BenchReport {
            indexed_symbols: stats.symbols,
            indexed_files: stats.files,
            rows,
            notes: notes_for(
                &RunShape {
                    tasks: drawn.len(),
                    seed: options.seed,
                    files_read,
                    budgets: budgets.len(),
                },
                &corpus,
            ),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Arm, BenchOptions, BenchReport, BenchRow, DEFAULT_BUDGETS, Derivation, METRIC_NAMES,
        Metric, count_f64, decimals, mean, percentile, ratio, to_f64,
    };

    /// Building a one-row report for the shape tests.
    fn report() -> BenchReport {
        let arm = Arm {
            tasks: 4,
            hits: 3,
            tokens: 4000,
            baseline_hits: 1,
            baseline_tokens: 40_000,
            latencies: vec![1.0, 2.0, 3.0, 4.0],
        };
        BenchReport {
            indexed_symbols: 10,
            indexed_files: 2,
            rows: vec![BenchRow {
                family: "description".to_owned(),
                budget: 1000,
                tasks: arm.tasks,
                metrics: arm.metrics(),
            }],
            notes: vec!["a note".to_owned()],
        }
    }

    /// The defaults are the ones the documentation promises.
    #[test]
    fn default_options() {
        let options = BenchOptions::default();
        assert_eq!(options.tasks, 100);
        assert_eq!(options.seed, 1);
        assert_eq!(options.budgets, DEFAULT_BUDGETS.to_vec());
        assert_eq!(options.baseline_files, 3);
    }

    /// The three derivations have stable distinct names.
    #[test]
    fn derivation_names() {
        assert_eq!(Derivation::Measured.as_str(), "measured");
        assert_eq!(Derivation::Proxy.as_str(), "proxy");
        assert_eq!(Derivation::Unobservable.as_str(), "unobservable");
    }

    /// Large integers convert to floating point exactly, without a cast.
    #[test]
    fn integer_conversion_is_exact() {
        assert!((to_f64(0) - 0.0).abs() < f64::EPSILON);
        assert!((to_f64(4_294_967_296) - 4_294_967_296.0).abs() < f64::EPSILON);
        assert!((count_f64(12) - 12.0).abs() < f64::EPSILON);
    }

    /// Ratios and means are rounded, and both refuse to divide by zero.
    #[test]
    fn ratios_and_means() {
        assert_eq!(ratio(1, 3), Some(0.3333));
        assert_eq!(ratio(0, 0), None);
        assert_eq!(mean(1000, 3), Some(333.3333));
        assert_eq!(mean(1000, 0), None);
    }

    /// Percentiles use the nearest rank and handle one and no samples.
    #[test]
    fn percentiles_by_nearest_rank() {
        let sorted = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
        assert_eq!(percentile(&sorted, 50), Some(5.0));
        assert_eq!(percentile(&sorted, 95), Some(10.0));
        assert_eq!(percentile(&[7.5], 50), Some(7.5));
        assert_eq!(percentile(&[], 50), None);
    }

    /// Numbers print with three decimals and an absent one prints as null.
    #[test]
    fn decimal_formatting() {
        assert_eq!(decimals(Some(0.5)), "0.500");
        assert_eq!(decimals(None), "null");
    }

    /// An arm reports the eight figures in order, with task success unobservable.
    #[test]
    fn metrics_are_named_in_order() {
        let row = &report().rows[0];
        let names: Vec<&str> = row.metrics.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, METRIC_NAMES.to_vec());
        assert_eq!(row.value("hit_rate"), Some(0.75));
        assert_eq!(row.value("mean_tokens"), Some(1000.0));
        assert_eq!(row.value("baseline_hit_rate"), Some(0.25));
        assert_eq!(row.value("baseline_mean_tokens"), Some(10_000.0));
        assert_eq!(row.value("tokens_saved_ratio"), Some(0.9));
        assert_eq!(row.value("p50_ms"), Some(2.0));
        assert_eq!(row.value("task_success"), None);
        let unobservable = row
            .metrics
            .iter()
            .filter(|m| m.derivation == Derivation::Unobservable)
            .count();
        assert_eq!(unobservable, 1);
    }

    /// An empty arm reports every figure as null rather than as a zero.
    #[test]
    fn an_empty_arm_reports_null() {
        let metrics: Vec<Metric> = Arm::default().metrics();
        assert!(metrics.iter().all(|metric| metric.value.is_none()));
        assert_eq!(metrics.len(), METRIC_NAMES.len());
    }

    /// The value carries one row per figure, and an unobservable figure serializes as null.
    #[test]
    fn value_is_a_long_table() {
        let value = report().to_value();
        assert_eq!(value["indexed_symbols"], 10);
        let rows = value["metrics"].as_array().expect("metrics");
        assert_eq!(rows.len(), METRIC_NAMES.len());
        assert_eq!(rows[0]["metric"], "hit_rate");
        assert_eq!(rows[0]["derivation"], "measured");
        let last = &rows[METRIC_NAMES.len() - 1];
        assert_eq!(last["metric"], "task_success");
        assert_eq!(last["value"], serde_json::Value::Null);
        assert_eq!(last["derivation"], "unobservable");
        assert_eq!(value["notes"][0], "a note");
    }

    /// The summary names every figure of every row and says that task success was not measured.
    #[test]
    fn summary_lines() {
        let text = report().summary();
        assert!(
            text.starts_with("indexed 10 symbols in 2 files\n"),
            "{text}"
        );
        assert!(
            text.contains("description @1000 (4 tasks): hit 0.750"),
            "{text}"
        );
        assert!(text.contains("baseline hit 0.250"), "{text}");
        assert!(text.contains("saved 0.900"), "{text}");
        assert!(text.ends_with("task_success: not measured, because no language model ran\n"));
    }
}
