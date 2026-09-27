// SPDX-License-Identifier: Apache-2.0
//! The ratchet: baseline files that may only shrink.
//!
//! # Role in the harness
//! Every guard in `xtask` reports findings as opaque text keys and hands them to this module, which
//! compares them with a baseline file under `.ratchet/`. The comparison, not the analysis, is what
//! makes a guard usable on a codebase that already has findings: the existing ones are recorded
//! once and the guard then blocks anything new.
//!
//! # Invariants
//! * **Keys never contain a line or column.** A finding that moves because somebody edited the line
//!   above it must compare equal, or the guard breaks on every unrelated change and is switched off.
//! * **A new finding fails.** That is the whole point.
//! * **A baseline entry whose anchor was never enumerated fails.** "Somebody fixed it" and "the
//!   analyser stopped seeing it" look identical from the baseline's side, and a guard that cannot
//!   tell them apart is lying. The anchor is the first tab-separated field of the entry, which is
//!   always the file the finding lives in.
//! * **A finding that disappeared while its file was still analysed is only a note.** Here the
//!   guard did look and did not find it, so the evidence is real; the baseline is merely loose and
//!   the developer is told to tighten it.
//! * Entries are sorted by plain byte order, so the file is stable across locales. Shell tools that
//!   post-process it must use `LC_ALL=C`, because `comm` silently misbehaves under other
//!   collations.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// The comment prefix that introduces the explanatory block at the top of a baseline file.
const COMMENT: char = '#';

/// A baseline file: a set of finding keys plus the path it was read from.
pub(crate) struct Baseline {
    /// Path of the baseline file on disk.
    path: PathBuf,
    /// The finding keys recorded in the file, with the comment block stripped.
    entries: BTreeSet<String>,
}

impl Baseline {
    /// Reads the baseline for `name`, treating a missing file as an empty baseline.
    ///
    /// # Errors
    /// Returns a message when the file exists but cannot be read, or holds a malformed entry.
    pub(crate) fn load(root: &Path, name: &str) -> Result<Self, String> {
        let path = root.join(".ratchet").join(name);
        if !path.is_file() {
            return Ok(Self {
                path,
                entries: BTreeSet::new(),
            });
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|err| format!("cannot read {}: {err}", path.display()))?;
        let mut entries = BTreeSet::new();
        for line in text.lines() {
            if line.starts_with(COMMENT) || line.trim().is_empty() {
                continue;
            }
            if !line.contains('\t') {
                return Err(format!(
                    "{}: entry without a tab separator: {line}\n  \
                     every entry is `<anchor>\\t<detail>`; regenerate the file with `--update`",
                    path.display()
                ));
            }
            entries.insert(line.to_owned());
        }
        Ok(Self { path, entries })
    }

    /// Returns how many findings the baseline records.
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    /// Writes `found` as the new baseline, preceded by `header` as a comment block.
    ///
    /// # Errors
    /// Returns a message when the `.ratchet` directory or the file cannot be written.
    pub(crate) fn write(&self, found: &BTreeSet<String>, header: &[&str]) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| format!("cannot create {}: {err}", parent.display()))?;
        }
        let mut text = String::new();
        for line in header {
            if line.is_empty() {
                text.push_str("#\n");
            } else {
                let _ = writeln!(text, "# {line}");
            }
        }
        for entry in found {
            let _ = writeln!(text, "{entry}");
        }
        std::fs::write(&self.path, text)
            .map_err(|err| format!("cannot write {}: {err}", self.path.display()))
    }

    /// Compares `found` with the baseline, given the set of anchors the guard actually enumerated.
    ///
    /// `anchors` is the set of files the guard parsed in this run. It is what separates a finding
    /// that was fixed from a baseline entry that is now unverifiable.
    pub(crate) fn compare(&self, found: &BTreeSet<String>, anchors: &BTreeSet<String>) -> Verdict {
        let mut verdict = Verdict::default();
        for entry in found.difference(&self.entries) {
            verdict.appeared.push(entry.clone());
        }
        for entry in self.entries.difference(found) {
            let anchor = anchor_of(entry);
            if anchors.contains(anchor) {
                verdict.disappeared.push(entry.clone());
            } else {
                verdict.unanchored.push(entry.clone());
            }
        }
        verdict
    }
}

/// The result of comparing this run's findings with the baseline.
#[derive(Default)]
pub(crate) struct Verdict {
    /// Findings that are not in the baseline. These fail the guard.
    pub(crate) appeared: Vec<String>,
    /// Baseline entries whose file was analysed but which were not found. A note: tighten the file.
    pub(crate) disappeared: Vec<String>,
    /// Baseline entries whose file was never analysed. These fail the guard: the entry is stale and
    /// nothing proves whether the finding was fixed or merely became invisible.
    pub(crate) unanchored: Vec<String>,
}

impl Verdict {
    /// Returns true when the verdict blocks the build.
    pub(crate) fn failed(&self) -> bool {
        !self.appeared.is_empty() || !self.unanchored.is_empty()
    }

    /// Prints the verdict, naming what a developer must do about each part of it.
    ///
    /// `guard` is the subcommand name, used to spell out the exact command to regenerate.
    pub(crate) fn report(&self, guard: &str) {
        if !self.appeared.is_empty() {
            println!(
                "\nNEW findings not present in the baseline ({}):",
                self.appeared.len()
            );
            for entry in &self.appeared {
                println!("  + {entry}");
            }
            println!(
                "  Fix them, mark them as intended where the guard allows it, or, if they are \
                 genuinely acceptable, run `cargo run -p xtask -- {guard} --update` and justify \
                 the growth in the pull request."
            );
        }
        if !self.unanchored.is_empty() {
            println!(
                "\nBaseline entries that matched nothing ({}):",
                self.unanchored.len()
            );
            for entry in &self.unanchored {
                println!("  ? {entry}");
            }
            println!(
                "  Their file was not analysed in this run, so the guard cannot tell a fix from a \
                 blind spot. Check the file still exists and still parses, then run \
                 `cargo run -p xtask -- {guard} --update`."
            );
        }
        if !self.disappeared.is_empty() {
            println!(
                "\nNote: baseline entries that no longer occur, though their files were analysed \
                 ({}):",
                self.disappeared.len()
            );
            for entry in &self.disappeared {
                println!("  - {entry}");
            }
            println!(
                "  The ratchet may only shrink: run `cargo run -p xtask -- {guard} --update` to \
                 lock in the improvement."
            );
        }
    }
}

/// Returns the anchor of a baseline entry: everything before the first tab.
fn anchor_of(entry: &str) -> &str {
    entry.split_once('\t').map_or(entry, |(anchor, _)| anchor)
}

/// Builds one baseline entry from an anchor and its detail fields.
///
/// Tabs and newlines inside a field would break the one-entry-per-line format, so they are folded
/// into single spaces before the fields are joined.
pub(crate) fn key(anchor: &str, details: &[&str]) -> String {
    let mut out = crate::source::normalise_whitespace(anchor);
    for detail in details {
        out.push('\t');
        out.push_str(&crate::source::normalise_whitespace(detail));
    }
    out
}

/// Prints the common header of a guard run: what was enumerated and what the baseline holds.
pub(crate) fn print_scope(files: usize, skipped: usize, baseline: usize) {
    println!("  files analysed:    {files}");
    println!("  test files skipped:{skipped:>4}");
    println!("  baseline entries:  {baseline}");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a baseline in memory without touching the filesystem.
    fn baseline_of(entries: &[&str]) -> Baseline {
        Baseline {
            path: PathBuf::from(".ratchet/probe.txt"),
            entries: entries.iter().map(|entry| (*entry).to_owned()).collect(),
        }
    }

    /// Collects strings into the set shape the comparison expects.
    fn set_of(entries: &[&str]) -> BTreeSet<String> {
        entries.iter().map(|entry| (*entry).to_owned()).collect()
    }

    /// A finding absent from the baseline fails the guard.
    #[test]
    fn a_new_finding_fails() {
        let baseline = baseline_of(&["a.rs\told"]);
        let verdict = baseline.compare(&set_of(&["a.rs\told", "a.rs\tnew"]), &set_of(&["a.rs"]));
        assert_eq!(verdict.appeared, vec!["a.rs\tnew"]);
        assert!(verdict.failed());
    }

    /// A baseline entry whose file was analysed but which no longer occurs is only a note.
    #[test]
    fn a_fixed_finding_is_a_note() {
        let baseline = baseline_of(&["a.rs\told"]);
        let verdict = baseline.compare(&BTreeSet::new(), &set_of(&["a.rs"]));
        assert_eq!(verdict.disappeared, vec!["a.rs\told"]);
        assert!(verdict.unanchored.is_empty());
        assert!(!verdict.failed());
    }

    /// A baseline entry whose file was never analysed fails, because the guard cannot tell a fix
    /// from a blind spot.
    #[test]
    fn a_baseline_entry_that_matches_nothing_fails() {
        let baseline = baseline_of(&["gone.rs\told"]);
        let verdict = baseline.compare(&BTreeSet::new(), &set_of(&["a.rs"]));
        assert_eq!(verdict.unanchored, vec!["gone.rs\told"]);
        assert!(verdict.disappeared.is_empty());
        assert!(verdict.failed());
    }

    /// An unchanged run passes and reports nothing.
    #[test]
    fn an_unchanged_run_passes() {
        let baseline = baseline_of(&["a.rs\told"]);
        let verdict = baseline.compare(&set_of(&["a.rs\told"]), &set_of(&["a.rs"]));
        assert!(!verdict.failed());
        assert!(verdict.appeared.is_empty());
        assert!(verdict.disappeared.is_empty());
    }

    /// Keys fold tabs and newlines out of their fields, so one finding is always one line.
    #[test]
    fn keys_are_single_lines() {
        let entry = key("a.rs", &["two\nlines\there", "f"]);
        assert_eq!(entry, "a.rs\ttwo lines here\tf");
        assert_eq!(entry.lines().count(), 1);
        assert_eq!(anchor_of(&entry), "a.rs");
    }

    /// Entries come back in plain byte order, whatever order they were inserted in.
    #[test]
    fn entries_sort_by_byte_order() {
        let baseline = baseline_of(&["b.rs\tz", "a.rs\tz", "B.rs\tz"]);
        let order: Vec<&str> = baseline.entries.iter().map(String::as_str).collect();
        assert_eq!(order, vec!["B.rs\tz", "a.rs\tz", "b.rs\tz"]);
    }

    /// The comment block is ignored when a baseline is read back.
    #[test]
    fn round_trips_through_a_file() {
        let dir = std::env::temp_dir().join(format!("xtask-ratchet-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let baseline = Baseline {
            path: dir.join(".ratchet").join("probe.txt"),
            entries: BTreeSet::new(),
        };
        baseline
            .write(
                &set_of(&["a.rs\tone", "b.rs\ttwo"]),
                &["explanation", "", "more"],
            )
            .expect("writes");
        let reread = Baseline::load(&dir, "probe.txt").expect("reads");
        assert_eq!(reread.len(), 2);
        let text = std::fs::read_to_string(baseline.path).expect("reads");
        assert!(text.starts_with("# explanation\n#\n# more\n"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
