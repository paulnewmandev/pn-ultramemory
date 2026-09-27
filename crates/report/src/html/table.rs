// SPDX-License-Identifier: Apache-2.0
//! Building blocks of the HTML report: accessible tables, progress bars and the language chart.
//!
//! Tables sit inside a focusable, labelled scroll region so they scroll horizontally instead of
//! overflowing on small screens, carry a visually hidden caption, and mark the first column as a
//! row header. Every user-provided string is escaped and wrapped in `<bdi>` so right-to-left
//! text cannot disturb its neighbours.
//!
//! Invariants: cell text is escaped exactly once; the only raw HTML accepted is the [`Cell::Raw`]
//! variant, which callers build from escaped parts.

use std::fmt::Write as _;

use crate::content::MAX_LANGUAGE_BARS;
use crate::i18n::{Key, t};
use crate::lang::Lang;
use crate::model::LanguageRow;
use crate::numfmt::{self, coord};
use crate::text::{CLIP_NAME, escaped, push_escaped, single_line};

/// A column header.
pub(super) struct Column {
    /// Header text.
    pub title: &'static str,
    /// Whether the column holds numbers, which are right-aligned.
    pub numeric: bool,
}

/// One table cell.
pub(super) enum Cell {
    /// Plain user text, escaped when written.
    Text(String),
    /// User text in a monospace font (names and paths), escaped when written.
    Mono(String),
    /// A formatted number, right-aligned.
    Num(String),
    /// Trusted HTML built by the caller from escaped parts.
    Raw(String),
}

/// Writes a table with the given columns and rows.
pub(super) fn table(out: &mut String, caption: &str, columns: &[Column], rows: &[Vec<Cell>]) {
    out.push_str("<div class=\"scroll\" role=\"region\" tabindex=\"0\" aria-label=\"");
    push_escaped(out, caption);
    out.push_str("\"><table><caption class=\"sr\">");
    push_escaped(out, caption);
    out.push_str("</caption><thead><tr>");
    for column in columns {
        let class = if column.numeric { " class=\"num\"" } else { "" };
        let _ = write!(out, "<th scope=\"col\"{class}>");
        push_escaped(out, column.title);
        out.push_str("</th>");
    }
    out.push_str("</tr></thead><tbody>");
    for row in rows {
        out.push_str("<tr>");
        for (index, cell) in row.iter().enumerate() {
            let open = if index == 0 {
                "<th scope=\"row\""
            } else {
                "<td"
            };
            let close = if index == 0 { "</th>" } else { "</td>" };
            out.push_str(open);
            match cell {
                Cell::Text(text) => {
                    out.push_str("><bdi>");
                    push_escaped(out, text);
                    out.push_str("</bdi>");
                }
                Cell::Mono(text) => {
                    out.push_str(" class=\"mono\"><bdi>");
                    push_escaped(out, text);
                    out.push_str("</bdi>");
                }
                Cell::Num(text) => {
                    out.push_str(" class=\"num\">");
                    push_escaped(out, text);
                }
                Cell::Raw(html) => {
                    out.push('>');
                    out.push_str(html);
                }
            }
            out.push_str(close);
        }
        out.push_str("</tr>");
    }
    out.push_str("</tbody></table></div>");
}

/// Returns a progress bar for `percent` (clamped to `0..=100`) with an accessible name.
pub(super) fn progress_bar(percent: f64, label: &str, small: bool) -> String {
    let percent = if percent.is_finite() {
        percent.clamp(0.0, 100.0)
    } else {
        0.0
    };
    let width = numfmt::decimal(percent, 1, Lang::En);
    format!(
        "<div class=\"bar{}\" role=\"progressbar\" aria-valuemin=\"0\" aria-valuemax=\"100\" \
         aria-valuenow=\"{width}\" aria-label=\"{}\"><span style=\"width:{width}%\"></span></div>",
        if small { " sm" } else { "" },
        escaped(label)
    )
}

/// Returns the bar chart of files per language as an inline SVG.
///
/// At most [`MAX_LANGUAGE_BARS`] languages are drawn; bars are scaled to the largest file count.
/// The chart is named through `aria-label` and `<title>`, and `<desc>` lists the values so the
/// figure is fully readable without seeing it.
pub(super) fn language_chart(rows: &[&LanguageRow], lang: Lang) -> String {
    const WIDTH: f64 = 640.0;
    const LABEL: f64 = 150.0;
    const BAR_MAX: f64 = 400.0;
    const ROW: f64 = 28.0;
    let shown = &rows[..rows.len().min(MAX_LANGUAGE_BARS)];
    let max_files = shown.iter().map(|row| row.files).max().unwrap_or(0).max(1);
    let height = numfmt::usize_to_f64(shown.len()) * ROW + 8.0;
    let name = t(lang, Key::ChartFiles);

    let mut out = String::new();
    let _ = write!(
        out,
        "<svg class=\"chart\" viewBox=\"0 0 {} {}\" role=\"img\" aria-label=\"{}\"><title>{}</title><desc>",
        coord(WIDTH),
        coord(height),
        escaped(name),
        escaped(name)
    );
    for (index, row) in shown.iter().enumerate() {
        if index > 0 {
            out.push_str("; ");
        }
        push_escaped(&mut out, &single_line(&row.name, CLIP_NAME));
        out.push_str(": ");
        out.push_str(&numfmt::int(row.files, lang));
    }
    out.push_str("</desc>");
    for (index, row) in shown.iter().enumerate() {
        let top = numfmt::usize_to_f64(index) * ROW + 4.0;
        let bar = (numfmt::to_f64(row.files) / numfmt::to_f64(max_files) * BAR_MAX).max(0.0);
        let bar = if row.files > 0 { bar.max(2.0) } else { 0.0 };
        let _ = write!(
            out,
            "<text class=\"lbl\" x=\"{}\" y=\"{}\" text-anchor=\"end\">",
            coord(LABEL - 10.0),
            coord(top + 17.0)
        );
        push_escaped(&mut out, &single_line(&row.name, 20));
        let _ = write!(
            out,
            "</text><rect class=\"bx\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"18\" rx=\"3\"/>\
             <text class=\"val\" x=\"{}\" y=\"{}\">{}</text>",
            coord(LABEL),
            coord(top + 3.0),
            coord(bar),
            coord(LABEL + bar + 8.0),
            coord(top + 17.0),
            numfmt::int(row.files, lang)
        );
    }
    out.push_str("</svg>");
    out
}

#[cfg(test)]
mod tests {
    use super::{Cell, Column, language_chart, progress_bar, table};
    use crate::lang::Lang;
    use crate::model::LanguageRow;

    /// Builds a language row.
    fn row(name: &str, files: u64) -> LanguageRow {
        LanguageRow {
            name: name.into(),
            files,
            symbols: 0,
        }
    }

    /// Tables are labelled, scrollable and mark headers and escape every cell.
    #[test]
    fn table_is_accessible_and_escaped() {
        let mut out = String::new();
        let columns = [
            Column {
                title: "Name",
                numeric: false,
            },
            Column {
                title: "Count",
                numeric: true,
            },
        ];
        let rows = vec![vec![
            Cell::Mono("<b>x</b>".into()),
            Cell::Num("1,234".into()),
        ]];
        table(&mut out, "Cap\"tion", &columns, &rows);
        assert!(out.contains("role=\"region\" tabindex=\"0\" aria-label=\"Cap&quot;tion\""));
        assert!(out.contains("<caption class=\"sr\">Cap&quot;tion</caption>"));
        assert!(out.contains("<th scope=\"col\">Name</th>"));
        assert!(out.contains("<th scope=\"col\" class=\"num\">Count</th>"));
        assert!(
            out.contains("<th scope=\"row\" class=\"mono\"><bdi>&lt;b&gt;x&lt;/b&gt;</bdi></th>")
        );
        assert!(out.contains("<td class=\"num\">1,234</td>"));
        assert!(!out.contains("<b>"));
    }

    /// Progress bars clamp their value and expose it to assistive technology.
    #[test]
    fn progress_bar_clamps() {
        let bar = progress_bar(133.0, "Docs \"x\"", false);
        assert!(bar.contains("aria-valuenow=\"100\""));
        assert!(bar.contains("style=\"width:100%\""));
        assert!(bar.contains("aria-label=\"Docs &quot;x&quot;\""));
        assert!(progress_bar(f64::NAN, "a", true).contains("aria-valuenow=\"0\""));
        assert!(progress_bar(-4.0, "a", true).contains("class=\"bar sm\""));
        assert!(progress_bar(87.54, "a", false).contains("aria-valuenow=\"87.5\""));
    }

    /// The chart is an accessible image, scales bars to the maximum and caps the number of bars.
    #[test]
    fn chart_scales_and_caps() {
        let rows: Vec<LanguageRow> = (0..20).map(|i| row(&format!("L{i}"), 100 - i)).collect();
        let refs: Vec<&LanguageRow> = rows.iter().collect();
        let svg = language_chart(&refs, Lang::En);
        assert!(svg.contains("role=\"img\" aria-label=\"Files per language\""));
        assert!(svg.contains("<title>Files per language</title>"));
        assert_eq!(svg.matches("<rect").count(), 12);
        assert!(svg.contains("width=\"400\""));
        assert!(svg.contains("<desc>L0: 100; L1: 99;"));
        let es = language_chart(&refs, Lang::Es);
        assert!(es.contains("aria-label=\"Archivos por lenguaje\""));
    }

    /// Hostile names and empty data do not break the chart.
    #[test]
    fn chart_handles_hostile_and_empty_input() {
        let rows = [
            row("<script>alert(1)</script>", 0),
            row(&"w".repeat(500), u64::MAX),
        ];
        let refs: Vec<&LanguageRow> = rows.iter().collect();
        let svg = language_chart(&refs, Lang::En);
        assert!(!svg.contains("<script"));
        assert!(svg.contains("&lt;script&gt;"));
        assert!(svg.contains("width=\"400\""));
        assert!(language_chart(&[], Lang::En).contains("viewBox=\"0 0 640 8\""));
    }
}
