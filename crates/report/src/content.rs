// SPDX-License-Identifier: Apache-2.0
//! Decisions shared by the HTML, PDF and Markdown renderers: what is shown, in which order and
//! how much of it.
//!
//! Keeping these rules in one place is what makes the three formats agree. Ranking tables
//! (languages, modules, hotspots) are sorted with a total order, so the same data always gives
//! the same rows; tables that have a natural order of their own (memories, notes, undocumented
//! examples, per-language coverage) keep the order they were given. Long tables are cut to a
//! fixed number of rows and say so with a "showing N of M" note.
//!
//! Invariants: nothing here panics or allocates proportionally to hostile input beyond one
//! sorted copy of a table; every sort is stable and has a final tie-break on the name.

use crate::i18n::{Key, tf};
use crate::lang::Lang;
use crate::model::{DocCoverage, Hotspot, LanguageRow, ModuleRow, ReportData};
use crate::numfmt;
use crate::text::{CLIP_NAME, single_line};

/// Rows of the hotspots table.
pub(crate) const MAX_HOTSPOTS: usize = 25;
/// Rows of the memories table.
pub(crate) const MAX_MEMORIES: usize = 25;
/// Rows of the modules table.
pub(crate) const MAX_MODULES: usize = 50;
/// Rows of the languages table.
pub(crate) const MAX_LANGUAGES: usize = 30;
/// Bars of the languages chart.
pub(crate) const MAX_LANGUAGE_BARS: usize = 12;
/// Rows of the per-language documentation table.
pub(crate) const MAX_DOC_LANGUAGES: usize = 30;
/// Rows of the undocumented-symbols list.
pub(crate) const MAX_UNDOCUMENTED: usize = 15;
/// Entries of the notes list.
pub(crate) const MAX_NOTES: usize = 100;
/// Nodes drawn in the graph of a report (the standalone SVG export allows more).
pub(crate) const REPORT_GRAPH_NODES: usize = 40;
/// Edges drawn in the graph of a report.
pub(crate) const REPORT_GRAPH_EDGES: usize = 120;

/// A view of the first rows of a table, remembering how many there were in total.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Window<'a, T> {
    /// The rows that will be shown.
    pub rows: &'a [T],
    /// The number of rows before truncation.
    pub total: usize,
}

impl<'a, T> Window<'a, T> {
    /// Takes at most `max` leading rows of `all`.
    pub(crate) fn new(all: &'a [T], max: usize) -> Self {
        Self {
            rows: &all[..all.len().min(max)],
            total: all.len(),
        }
    }

    /// Returns `true` when rows were cut.
    pub(crate) const fn is_cut(&self) -> bool {
        self.rows.len() < self.total
    }

    /// Returns the "Showing N of M." sentence when rows were cut.
    pub(crate) fn note(&self, lang: Lang) -> Option<String> {
        self.is_cut().then(|| {
            let shown = numfmt::int(u64::try_from(self.rows.len()).unwrap_or(u64::MAX), lang);
            let total = numfmt::int(u64::try_from(self.total).unwrap_or(u64::MAX), lang);
            tf(lang, Key::ShowingOf, &[&shown, &total])
        })
    }
}

/// Returns the languages sorted by files, then symbols (both descending), then name.
pub(crate) fn sorted_languages(rows: &[LanguageRow]) -> Vec<&LanguageRow> {
    let mut sorted: Vec<&LanguageRow> = rows.iter().collect();
    sorted.sort_by(|a, b| {
        b.files
            .cmp(&a.files)
            .then(b.symbols.cmp(&a.symbols))
            .then_with(|| a.name.cmp(&b.name))
    });
    sorted
}

/// Returns the modules sorted by symbols, then files (both descending), then name.
pub(crate) fn sorted_modules(rows: &[ModuleRow]) -> Vec<&ModuleRow> {
    let mut sorted: Vec<&ModuleRow> = rows.iter().collect();
    sorted.sort_by(|a, b| {
        b.symbols
            .cmp(&a.symbols)
            .then(b.files.cmp(&a.files))
            .then_with(|| a.name.cmp(&b.name))
    });
    sorted
}

/// Returns the hotspots sorted by callers, then callees (both descending), keeping the given
/// order for ties.
pub(crate) fn sorted_hotspots(rows: &[Hotspot]) -> Vec<&Hotspot> {
    let mut sorted: Vec<&Hotspot> = rows.iter().collect();
    sorted.sort_by(|a, b| b.callers.cmp(&a.callers).then(b.callees.cmp(&a.callees)));
    sorted
}

/// Overall documentation coverage in percent, clamped to `0..=100`.
pub(crate) fn doc_percent(docs: &DocCoverage) -> f64 {
    numfmt::ratio_percent(docs.documented, docs.public_symbols)
}

/// Returns the project name as one clean line, or the localized placeholder when empty.
pub(crate) fn project_name(data: &ReportData, lang: Lang) -> String {
    let name = single_line(&data.project, CLIP_NAME);
    if name.is_empty() {
        crate::i18n::t(lang, Key::UnnamedProject).to_owned()
    } else {
        name
    }
}

/// Returns the tool version to print, falling back to this crate's own version when the data
/// does not carry one.
pub(crate) fn tool_version(data: &ReportData) -> String {
    let version = single_line(&data.tool_version, 40);
    if version.is_empty() {
        env!("CARGO_PKG_VERSION").to_owned()
    } else {
        version
    }
}

/// Returns `path:line`, or just `path` when the line is unknown.
pub(crate) fn location(path: &str, line: u32) -> String {
    let path = single_line(path, crate::text::CLIP_PATH);
    if line == 0 {
        path
    } else {
        format!("{path}:{line}")
    }
}

/// Returns the share of `files` among `total` files as a percentage.
pub(crate) fn share(files: u64, total: u64) -> f64 {
    numfmt::ratio_percent(files, total)
}

/// Returns the file total the language shares are computed against.
pub(crate) fn language_file_total(rows: &[&LanguageRow]) -> u64 {
    rows.iter()
        .fold(0u64, |acc, row| acc.saturating_add(row.files))
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_HOTSPOTS, Window, doc_percent, location, project_name, sorted_hotspots,
        sorted_languages, sorted_modules, tool_version,
    };
    use crate::lang::Lang;
    use crate::model::{DocCoverage, Hotspot, LanguageRow, ModuleRow, ReportData};

    /// A window over a long table reports the cut in both languages.
    #[test]
    fn window_reports_truncation() {
        let rows: Vec<u32> = (0..40).collect();
        let window = Window::new(&rows, MAX_HOTSPOTS);
        assert_eq!(window.rows.len(), 25);
        assert!(window.is_cut());
        assert_eq!(window.note(Lang::En).as_deref(), Some("Showing 25 of 40."));
        assert_eq!(
            window.note(Lang::Es).as_deref(),
            Some("Mostrando 25 de 40.")
        );
    }

    /// A short table is not cut and has no note; numbers in the note are grouped.
    #[test]
    fn window_leaves_short_tables_alone() {
        let rows: Vec<u32> = (0..3).collect();
        let window = Window::new(&rows, 25);
        assert_eq!(window.rows.len(), 3);
        assert!(!window.is_cut());
        assert_eq!(window.note(Lang::En), None);
        let big: Vec<u8> = vec![0; 5000];
        assert_eq!(
            Window::new(&big, 25).note(Lang::En).as_deref(),
            Some("Showing 25 of 5,000.")
        );
        assert_eq!(
            Window::new(&big, 25).note(Lang::Es).as_deref(),
            Some("Mostrando 25 de 5.000.")
        );
        assert_eq!(Window::new(&big[..0], 25).rows.len(), 0);
    }

    /// Languages are ranked by files, then symbols, then name, whatever the input order.
    #[test]
    fn languages_have_a_total_order() {
        let row = |name: &str, files, symbols| LanguageRow {
            name: name.into(),
            files,
            symbols,
        };
        let rows = vec![
            row("b", 5, 1),
            row("a", 5, 1),
            row("c", 9, 0),
            row("d", 5, 7),
        ];
        let names: Vec<&str> = sorted_languages(&rows)
            .iter()
            .map(|r| r.name.as_str())
            .collect();
        assert_eq!(names, ["c", "d", "a", "b"]);
    }

    /// Modules are ranked by symbols, then files, then name.
    #[test]
    fn modules_have_a_total_order() {
        let row = |name: &str, files, symbols| ModuleRow {
            name: name.into(),
            files,
            symbols,
            ..ModuleRow::default()
        };
        let rows = vec![
            row("z", 1, 10),
            row("y", 3, 10),
            row("x", 3, 10),
            row("w", 9, 1),
        ];
        let names: Vec<&str> = sorted_modules(&rows)
            .iter()
            .map(|r| r.name.as_str())
            .collect();
        assert_eq!(names, ["x", "y", "z", "w"]);
    }

    /// Hotspots are ranked by callers then callees, and ties keep the input order.
    #[test]
    fn hotspots_are_stable() {
        let row = |name: &str, callers, callees| Hotspot {
            name: name.into(),
            callers,
            callees,
            ..Hotspot::default()
        };
        let rows = vec![
            row("first", 5, 1),
            row("second", 5, 1),
            row("top", 9, 0),
            row("low", 5, 0),
        ];
        let names: Vec<&str> = sorted_hotspots(&rows)
            .iter()
            .map(|r| r.name.as_str())
            .collect();
        assert_eq!(names, ["top", "first", "second", "low"]);
    }

    /// Coverage is clamped even when the data claims more documented than public symbols.
    #[test]
    fn coverage_percent_is_clamped() {
        let docs = |public_symbols, documented| DocCoverage {
            public_symbols,
            documented,
            ..DocCoverage::default()
        };
        assert!((doc_percent(&docs(200, 50)) - 25.0).abs() < 1e-9);
        assert!((doc_percent(&docs(10, 50)) - 100.0).abs() < 1e-9);
        assert!(doc_percent(&docs(0, 0)).abs() < 1e-9);
    }

    /// Empty names and versions get sensible fallbacks, and locations join path and line.
    #[test]
    fn fallbacks_and_locations() {
        let data = ReportData::default();
        assert_eq!(project_name(&data, Lang::En), "(unnamed project)");
        assert_eq!(project_name(&data, Lang::Es), "(proyecto sin nombre)");
        assert_eq!(tool_version(&data), env!("CARGO_PKG_VERSION"));
        let named = ReportData {
            project: " my\napp ".into(),
            ..ReportData::default()
        };
        assert_eq!(project_name(&named, Lang::En), "my app");
        assert_eq!(location("src/lib.rs", 12), "src/lib.rs:12");
        assert_eq!(location("src/lib.rs", 0), "src/lib.rs");
    }
}
