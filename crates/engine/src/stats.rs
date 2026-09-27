// SPDX-License-Identifier: Apache-2.0
//! Index and usage statistics: the counts behind `pn-ultramemory stats`.
//!
//! # Role in the architecture
//! Application layer. [`Engine::stats`] reads three aggregates from
//! [`pn_ultramemory_core::Storage`] (the index counts, the file totals and what has been learned)
//! and one from the local metrics file, and returns them as a plain value. Nothing is written and
//! no source file is read.
//!
//! # The metrics file
//! [`crate::EngineConfig::metrics_path`] names a file to which the metrics sink appends one JSON
//! object per line, holding numbers only. [`Engine::usage_summary`] reads it back and is
//! deliberately forgiving: a line that is not a JSON object, an object with no recognizable event
//! name, and a field of the wrong type are each skipped instead of reported, because a metrics file
//! is a convenience and is never worth failing an operation for. When no path is configured, or the
//! file does not exist, the summary is `None` rather than a row of zeros, so that a reader can tell
//! *off* from *unused*.
//!
//! An event is recognized by a string field named `event`, `kind` or `op`, whose value is one of
//! `recall`, `expand`, `impact`, `remember` or `doc_apply`. A `recall` line may also carry the
//! integer fields `used` and `budget`, which is where the token figures come from.
//!
//! # State
//! The sink's `record` is a stub that writes nothing while the metrics work is being finished, so
//! on a machine that has never written the file the summary is `None`. The reader here is written
//! against the format the sink documents, so it starts reporting the moment recording does.
//!
//! # Determinism
//! Every figure is an integer count or an integer ratio of counts. Nothing depends on iteration
//! order, on the clock or on the file system beyond the bytes of the metrics file.

use std::fs::File;
use std::io::{BufRead as _, BufReader};

use pn_ultramemory_core::{FileTotals, IndexStats, LearningStatus};
use serde_json::{Map, Value, json};

use crate::engine::Engine;
use crate::error::EngineError;

/// How many lines of the metrics file are read before the reader stops, so that a file left
/// growing for years cannot make a `stats` call slow.
const MAX_EVENT_LINES: usize = 2_000_000;

/// What the local metrics file says about how the tool has been used.
///
/// Every field is a count over the whole history of the file. The two averages are integer
/// divisions and are zero while there is nothing to divide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UsageSummary {
    /// Capsules built.
    pub recalls: u64,
    /// Symbols paged through with the expand operation.
    pub expands: u64,
    /// Impact analyses run.
    pub impacts: u64,
    /// Memories stored.
    pub remembers: u64,
    /// Documentation batches applied.
    pub doc_applies: u64,
    /// Tokens carried by every capsule together, as the estimator counted them.
    pub tokens_served_total: u64,
    /// Tokens per capsule on average.
    pub avg_tokens_served: u64,
    /// The share of the budget a capsule uses on average, in whole percent.
    pub avg_budget_percent: u32,
}

impl UsageSummary {
    /// The summary as a structured value, with one field per counter.
    ///
    /// # Examples
    /// ```
    /// use pn_ultramemory_engine::UsageSummary;
    ///
    /// let usage = UsageSummary { recalls: 2, avg_budget_percent: 84, ..UsageSummary::default() };
    /// assert_eq!(usage.to_value()["recalls"], 2);
    /// assert_eq!(usage.to_value()["avg_budget_percent"], 84);
    /// ```
    #[must_use]
    pub fn to_value(&self) -> Value {
        json!({
            "recalls": self.recalls,
            "expands": self.expands,
            "impacts": self.impacts,
            "remembers": self.remembers,
            "doc_applies": self.doc_applies,
            "tokens_served_total": self.tokens_served_total,
            "avg_tokens_served": self.avg_tokens_served,
            "avg_budget_percent": self.avg_budget_percent,
        })
    }
}

/// The running totals of one pass over the metrics file.
#[derive(Debug, Default)]
struct Tally {
    /// The summary being built, apart from the averages.
    summary: UsageSummary,
    /// The budgets of every capsule together, used for the average share.
    budget_total: u64,
}

/// The name of the event an object describes, from the first field that carries one.
fn event_name(object: &Map<String, Value>) -> Option<&str> {
    ["event", "kind", "op"]
        .into_iter()
        .find_map(|key| object.get(key).and_then(Value::as_str))
}

/// A non-negative integer field, or `None` when it is absent, negative or not an integer.
fn count_field(object: &Map<String, Value>, key: &str) -> Option<u64> {
    let value = object.get(key)?;
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|number| u64::try_from(number).ok()))
}

impl Tally {
    /// Counts one decoded line. An unknown event name adds nothing.
    fn add(&mut self, object: &Map<String, Value>) {
        match event_name(object) {
            Some("recall") => {
                self.summary.recalls += 1;
                let used = count_field(object, "used").unwrap_or(0);
                self.summary.tokens_served_total =
                    self.summary.tokens_served_total.saturating_add(used);
                let budget = count_field(object, "budget").unwrap_or(0);
                self.budget_total = self.budget_total.saturating_add(budget);
            }
            Some("expand") => self.summary.expands += 1,
            Some("impact") => self.summary.impacts += 1,
            Some("remember") => self.summary.remembers += 1,
            Some("doc_apply") => self.summary.doc_applies += 1,
            _ => {}
        }
    }

    /// The finished summary, with the two averages worked out.
    fn finish(mut self) -> UsageSummary {
        self.summary.avg_tokens_served = self
            .summary
            .tokens_served_total
            .checked_div(self.summary.recalls)
            .unwrap_or(0);
        let percent = self
            .summary
            .tokens_served_total
            .saturating_mul(100)
            .checked_div(self.budget_total)
            .unwrap_or(0);
        self.summary.avg_budget_percent = u32::try_from(percent).unwrap_or(u32::MAX);
        self.summary
    }
}

/// Everything `stats` reports: what the index holds, what the files total, what has been learned
/// and how the tool has been used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatsReport {
    /// Counts of files, symbols, edges and memories.
    pub index: IndexStats,
    /// Line and parse-error totals over every indexed file.
    pub totals: FileTotals,
    /// What has been learned from the way results were used.
    pub learning: LearningStatus,
    /// What the local metrics file says, or `None` when metrics are off or the file is absent.
    pub usage: Option<UsageSummary>,
}

impl StatsReport {
    /// The report as a structured value, with the languages as a uniform table and `usage` set to
    /// `null` when there is no metrics file to read.
    #[must_use]
    pub fn to_value(&self) -> Value {
        let languages: Vec<Value> = self
            .index
            .languages
            .iter()
            .map(|(name, files)| json!({ "name": name, "files": files }))
            .collect();
        json!({
            "index": {
                "files": self.index.files,
                "symbols": self.index.symbols,
                "edges": self.index.edges,
                "memories": self.index.memories,
                "stale_memories": self.index.stale_memories,
                "languages": languages,
            },
            "totals": {
                "lines": self.totals.lines,
                "parse_error_files": self.totals.parse_error_files,
            },
            "learning": {
                "signals": self.learning.signals,
                "tracked_targets": self.learning.tracked_targets,
                "coaccess_pairs": self.learning.coaccess_pairs,
            },
            "usage": self.usage.as_ref().map_or(Value::Null, UsageSummary::to_value),
        })
    }
}

impl Engine {
    /// What the local metrics file says about how the tool has been used, or `None` when metrics
    /// are off, the file does not exist yet, or it cannot be opened.
    ///
    /// Unreadable lines are skipped: the documentation of this module gives the format and says
    /// why a broken metrics file never fails a call.
    #[must_use]
    pub fn usage_summary(&self) -> Option<UsageSummary> {
        let path = self.config.metrics_path.as_ref()?;
        let file = File::open(path).ok()?;
        let mut tally = Tally::default();
        for line in BufReader::new(file).lines().take(MAX_EVENT_LINES) {
            let Ok(line) = line else { continue };
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let Ok(Value::Object(object)) = serde_json::from_str::<Value>(trimmed) else {
                continue;
            };
            tally.add(&object);
        }
        Some(tally.finish())
    }

    /// The statistics of this repository: the index, the file totals, what has been learned and
    /// the local usage counters.
    ///
    /// # Errors
    /// Returns a storage error when any of the three aggregates cannot be read. A missing or
    /// unreadable metrics file is not an error: `usage` is then `None`.
    pub fn stats(&self) -> Result<StatsReport, EngineError> {
        Ok(StatsReport {
            index: self.storage().stats()?,
            totals: self.storage().file_totals()?,
            learning: self.storage().learning_status()?,
            usage: self.usage_summary(),
        })
    }

    /// The statistics as a structured value, ready to print in any of the output formats.
    ///
    /// # Errors
    /// Same as [`Engine::stats`].
    pub fn storage_stats(&self) -> Result<Value, EngineError> {
        Ok(self.stats()?.to_value())
    }
}

#[cfg(test)]
mod tests {
    use super::{StatsReport, Tally, UsageSummary, count_field, event_name};
    use pn_ultramemory_core::{FileTotals, IndexStats, LearningStatus};
    use serde_json::{Value, json};

    /// Builds the JSON object of one metrics line.
    fn line(text: &str) -> serde_json::Map<String, Value> {
        match serde_json::from_str::<Value>(text) {
            Ok(Value::Object(object)) => object,
            _ => serde_json::Map::new(),
        }
    }

    /// An event name is read from any of the three accepted field names.
    #[test]
    fn event_names_are_read_from_three_fields() {
        assert_eq!(event_name(&line(r#"{"event":"recall"}"#)), Some("recall"));
        assert_eq!(event_name(&line(r#"{"kind":"expand"}"#)), Some("expand"));
        assert_eq!(event_name(&line(r#"{"op":"impact"}"#)), Some("impact"));
        assert_eq!(event_name(&line(r#"{"n":1}"#)), None);
    }

    /// Integer fields are read, and negative or fractional ones are ignored.
    #[test]
    fn only_non_negative_integers_count() {
        let object = line(r#"{"a":7,"b":-1,"c":1.5,"d":"8"}"#);
        assert_eq!(count_field(&object, "a"), Some(7));
        assert_eq!(count_field(&object, "b"), None);
        assert_eq!(count_field(&object, "c"), None);
        assert_eq!(count_field(&object, "d"), None);
        assert_eq!(count_field(&object, "missing"), None);
    }

    /// The tally counts every kind of event and works out both averages.
    #[test]
    fn tally_counts_and_averages() {
        let mut tally = Tally::default();
        tally.add(&line(r#"{"event":"recall","used":400,"budget":1000}"#));
        tally.add(&line(r#"{"event":"recall","used":600,"budget":1000}"#));
        tally.add(&line(r#"{"event":"expand","lines":12}"#));
        tally.add(&line(r#"{"event":"impact"}"#));
        tally.add(&line(r#"{"event":"remember"}"#));
        tally.add(&line(r#"{"event":"doc_apply"}"#));
        tally.add(&line(r#"{"event":"nonsense"}"#));
        let summary = tally.finish();
        assert_eq!(summary.recalls, 2);
        assert_eq!(summary.expands, 1);
        assert_eq!(summary.impacts, 1);
        assert_eq!(summary.remembers, 1);
        assert_eq!(summary.doc_applies, 1);
        assert_eq!(summary.tokens_served_total, 1000);
        assert_eq!(summary.avg_tokens_served, 500);
        assert_eq!(summary.avg_budget_percent, 50);
    }

    /// With nothing to divide, both averages are zero rather than an error.
    #[test]
    fn empty_tally_has_zero_averages() {
        let summary = Tally::default().finish();
        assert_eq!(summary, UsageSummary::default());
    }

    /// The report keeps the languages as a uniform table and writes `null` for absent usage.
    #[test]
    fn report_shape() {
        let report = StatsReport {
            index: IndexStats {
                files: 3,
                symbols: 20,
                edges: 8,
                memories: 1,
                stale_memories: 0,
                languages: vec![("rust".to_owned(), 2), ("python".to_owned(), 1)],
            },
            totals: FileTotals {
                lines: 400,
                parse_error_files: 0,
            },
            learning: LearningStatus::default(),
            usage: None,
        };
        let value = report.to_value();
        assert_eq!(value["index"]["symbols"], 20);
        assert_eq!(
            value["index"]["languages"][0],
            json!({"name":"rust","files":2})
        );
        assert_eq!(value["totals"]["lines"], 400);
        assert_eq!(value["learning"]["signals"], 0);
        assert_eq!(value["usage"], Value::Null);
    }
}
