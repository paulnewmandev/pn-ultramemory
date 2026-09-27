// SPDX-License-Identifier: Apache-2.0
//! Writing a batch of documentation into the source files it belongs to.
//!
//! # Role in the architecture
//! Application layer. It owns the order of the steps and the safety rules; the language-specific
//! work of building a comment belongs to the [`pn_ultramemory_core::DocInserter`] port, and reading
//! and writing files to [`pn_ultramemory_core::SourceTree`].
//!
//! One call does, in this order: resolve every entry's symbol, pass every text through the secret
//! redactor, group the entries by file, refuse whole any file that changed since it was indexed,
//! insert from the bottom of each file upwards, write each file once, and re-index what was written
//! so that the index and the staleness of memories are current again.
//!
//! # Invariants
//! * **Bottom-up.** Insertions move the lines below them, never the lines above, so applying them
//!   in descending line order keeps every remaining recorded line valid.
//! * **All or nothing per file.** A file whose content hash no longer matches the index is skipped
//!   entirely, with a reason naming `pn-ultramemory index`.
//! * **A refusal is never fatal.** Only writing a file or reading the index can fail the call.
//! * **Determinism.** Files are visited in path order and insertions in source order, so the three
//!   lists of a report are the same on every run.

use std::collections::BTreeMap;

use pn_ultramemory_core::{DocTarget, SymbolRecord};
use serde_json::{Value, json};

use crate::docs::DocEntry;
use crate::engine::Engine;
use crate::error::EngineError;
use crate::guard::redact;
use crate::indexer::content_hash;
use crate::metrics::Event;

/// Why an entry whose text says nothing after redaction is skipped.
const EMPTY_AFTER_REDACTION: &str = "the text holds nothing but a redacted secret; write a description and rerun \
     `pn-ultramemory docs apply`";

/// Why every entry of a file that changed since it was indexed is skipped.
const FILE_CHANGED: &str = "the file changed since it was indexed, so the recorded lines are unreliable; run \
     `pn-ultramemory index` and apply again";

/// What one entry became once its symbol was found.
#[derive(Debug)]
struct Insertion {
    /// The symbol the documentation belongs to.
    symbol: SymbolRecord,
    /// The text to write, already redacted.
    text: String,
    /// The symbol as the caller named it, used when reporting a refusal.
    given: String,
}

/// What applying a batch of documentation did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocApplyReport {
    /// The qualified names of the symbols that were documented, in file and source order.
    pub applied: Vec<String>,
    /// The entries that were not applied, as the symbol the caller named and the reason.
    pub skipped: Vec<(String, String)>,
    /// The files that were written, or that would have been written in a dry run, in path order.
    pub files_changed: Vec<String>,
    /// Whether this was a rehearsal that wrote nothing.
    pub dry_run: bool,
}

impl DocApplyReport {
    /// The report as a structured value, with the refusals as a uniform table.
    #[must_use]
    pub fn to_value(&self) -> Value {
        let skipped: Vec<Value> = self
            .skipped
            .iter()
            .map(|(symbol, reason)| json!({ "symbol": symbol, "reason": reason }))
            .collect();
        json!({
            "dry_run": self.dry_run,
            "applied": self.applied,
            "skipped": skipped,
            "files_changed": self.files_changed,
        })
    }

    /// One line saying how many entries were applied, how many were refused and how many files
    /// were touched.
    ///
    /// # Examples
    /// ```
    /// use pn_ultramemory_engine::DocApplyReport;
    ///
    /// let report = DocApplyReport {
    ///     applied: vec!["load_config".into()],
    ///     skipped: Vec::new(),
    ///     files_changed: vec!["src/config.rs".into()],
    ///     dry_run: true,
    /// };
    /// assert_eq!(report.summary(), "dry run: 1 applied, 0 skipped, 1 file would change");
    /// ```
    #[must_use]
    pub fn summary(&self) -> String {
        let mut out = String::new();
        if self.dry_run {
            out.push_str("dry run: ");
        }
        out.push_str(&self.applied.len().to_string());
        out.push_str(" applied, ");
        out.push_str(&self.skipped.len().to_string());
        out.push_str(" skipped, ");
        out.push_str(&self.files_changed.len().to_string());
        out.push_str(" file");
        if self.files_changed.len() != 1 {
            out.push('s');
        }
        out.push_str(if self.dry_run {
            " would change"
        } else {
            " changed"
        });
        out
    }
}

impl Engine {
    /// Finds the symbol of every entry, redacting its text, and groups what is left by file.
    ///
    /// Entries whose symbol does not resolve, and entries left empty by redaction, go straight into
    /// `skipped` in the order they were given.
    ///
    /// # Errors
    /// Returns a storage error when the index cannot be read for a reason other than the symbol
    /// simply not being there.
    fn plan_insertions(
        &self,
        entries: &[DocEntry],
        skipped: &mut Vec<(String, String)>,
    ) -> Result<BTreeMap<String, Vec<Insertion>>, EngineError> {
        let mut planned: BTreeMap<String, Vec<Insertion>> = BTreeMap::new();
        for entry in entries {
            let symbol = match self.resolve_symbol(&entry.symbol) {
                Ok(symbol) => symbol,
                Err(EngineError::Storage(error)) => return Err(EngineError::Storage(error)),
                Err(error) => {
                    skipped.push((entry.symbol.clone(), error.to_string()));
                    continue;
                }
            };
            let (text, _redactions) = redact(&entry.text);
            if text.trim().is_empty() {
                skipped.push((entry.symbol.clone(), EMPTY_AFTER_REDACTION.to_owned()));
                continue;
            }
            planned
                .entry(symbol.path.clone())
                .or_default()
                .push(Insertion {
                    symbol,
                    text,
                    given: entry.symbol.clone(),
                });
        }
        Ok(planned)
    }

    /// Applies the insertions of one file, bottom upwards, and returns the new text together with
    /// what was applied and what was refused, both in source order.
    fn apply_to_file(
        &self,
        source: &str,
        insertions: &mut [Insertion],
    ) -> (String, Vec<String>, Vec<(String, String)>) {
        insertions.sort_by(|left, right| {
            right
                .symbol
                .span
                .start_line
                .cmp(&left.symbol.span.start_line)
                .then(right.symbol.id.cmp(&left.symbol.id))
        });
        let mut text = source.to_owned();
        let mut applied = Vec::new();
        let mut refused = Vec::new();
        for insertion in insertions.iter() {
            let target = DocTarget {
                name: &insertion.symbol.name,
                kind: insertion.symbol.kind,
                line: insertion.symbol.span.start_line,
            };
            match self.deps.docs.insert_doc(
                insertion.symbol.language,
                &text,
                &target,
                &insertion.text,
            ) {
                Ok(updated) => {
                    text = updated;
                    applied.push(insertion.symbol.qualified_name.clone());
                }
                Err(error) => refused.push((insertion.given.clone(), error.to_string())),
            }
        }
        applied.reverse();
        refused.reverse();
        (text, applied, refused)
    }

    /// Writes a batch of documentation into the source files of the repository.
    ///
    /// Each entry's symbol is resolved, its text is passed through the secret redactor, and the
    /// entries are grouped by file. A file whose content no longer matches the index is skipped
    /// whole; otherwise its insertions are applied from the bottom upwards and the file is written
    /// once. With `dry_run` nothing is written, and the report says which files would have changed.
    /// After a real run the files that were written are indexed again, so that the recorded lines
    /// and the staleness of memories are current.
    ///
    /// # Errors
    /// Returns a source error when a file cannot be written, and a storage error when the index
    /// cannot be read or written. An entry that cannot be resolved, a text the inserter refuses and
    /// a file that changed are reported in [`DocApplyReport::skipped`], never as an error.
    pub fn doc_apply(
        &self,
        entries: &[DocEntry],
        dry_run: bool,
    ) -> Result<DocApplyReport, EngineError> {
        let mut report = DocApplyReport {
            applied: Vec::new(),
            skipped: Vec::new(),
            files_changed: Vec::new(),
            dry_run,
        };
        let mut planned = self.plan_insertions(entries, &mut report.skipped)?;
        for (path, insertions) in &mut planned {
            let stored = self.storage().file_hash(path)?;
            let current = match self.deps.tree.read(path) {
                Ok(text) => text,
                Err(error) => {
                    let reason = error.to_string();
                    for insertion in insertions.iter() {
                        report
                            .skipped
                            .push((insertion.given.clone(), reason.clone()));
                    }
                    continue;
                }
            };
            if stored.as_deref() != Some(content_hash(&current).as_str()) {
                for insertion in insertions.iter() {
                    report
                        .skipped
                        .push((insertion.given.clone(), FILE_CHANGED.to_owned()));
                }
                continue;
            }
            let (text, applied, refused) = self.apply_to_file(&current, insertions);
            report.skipped.extend(refused);
            if applied.is_empty() {
                continue;
            }
            if !dry_run {
                self.deps.tree.write(path, &text)?;
            }
            report.files_changed.push(path.clone());
            report.applied.extend(applied);
        }
        if !dry_run && !report.files_changed.is_empty() {
            self.reindex_paths(&report.files_changed)?;
        }
        self.record(Event::DocApply {
            applied: u32::try_from(report.applied.len()).unwrap_or(u32::MAX),
            skipped: u32::try_from(report.skipped.len()).unwrap_or(u32::MAX),
        });
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::DocApplyReport;

    /// The summary counts the three lists and says whether anything was written.
    #[test]
    fn summary_counts_and_mood() {
        let mut report = DocApplyReport {
            applied: vec!["a".to_owned(), "b".to_owned()],
            skipped: vec![("c".to_owned(), "no".to_owned())],
            files_changed: vec!["x.rs".to_owned(), "y.rs".to_owned()],
            dry_run: false,
        };
        assert_eq!(report.summary(), "2 applied, 1 skipped, 2 files changed");
        report.dry_run = true;
        assert_eq!(
            report.summary(),
            "dry run: 2 applied, 1 skipped, 2 files would change"
        );
        report.files_changed.pop();
        assert!(report.summary().ends_with("1 file would change"));
    }

    /// The value carries the refusals as a uniform table and the dry-run flag.
    #[test]
    fn value_shape() {
        let report = DocApplyReport {
            applied: vec!["a".to_owned()],
            skipped: vec![("c".to_owned(), "because".to_owned())],
            files_changed: vec!["x.rs".to_owned()],
            dry_run: true,
        };
        let value = report.to_value();
        assert_eq!(value["dry_run"], true);
        assert_eq!(value["applied"][0], "a");
        assert_eq!(value["skipped"][0]["symbol"], "c");
        assert_eq!(value["skipped"][0]["reason"], "because");
        assert_eq!(value["files_changed"][0], "x.rs");
    }
}
