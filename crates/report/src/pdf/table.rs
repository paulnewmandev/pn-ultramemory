// SPDX-License-Identifier: Apache-2.0
//! Tables of the PDF: column widths from content, rows that never split, repeated headers.
//!
//! Column widths start from the widest cell of each column (header included), capped so one
//! column cannot take the page. When the natural widths exceed the page, the flexible columns
//! (names, paths, free text) shrink first and long cells are cut with an ellipsis; when there is
//! room to spare, the flexible columns take it. A column may allow a few lines of wrapped text,
//! and a cell may carry a small progress bar. Rows have the height of their tallest cell and are
//! never split across pages; the header row is repeated at the top of every page the table
//! continues on.
//!
//! Invariants: the widths always add up to the content width; no text is drawn outside its cell.

use super::canvas::Color;
use super::doc::{BOTTOM, CONTENT_W, Doc, MARGIN_X};
use super::fonts::{Font, encode, fit, text_width, wrap};
use crate::numfmt::usize_to_f64;

/// Font size of table text.
const SIZE: f64 = 8.0;
/// Horizontal padding inside a cell.
const PAD_X: f64 = 5.0;
/// Vertical padding inside a cell.
const PAD_Y: f64 = 3.4;
/// Height of one text line.
const LINE: f64 = 10.4;
/// The widest a column may want to be, as a share of the content width.
const MAX_NATURAL_SHARE: f64 = 0.55;
/// The narrowest a shrinking column may become.
const MIN_COLUMN: f64 = 44.0;
/// Width reserved for a progress bar column.
const BAR_COLUMN: f64 = 120.0;

/// A column of a table.
#[derive(Debug, Clone)]
pub(super) struct Col {
    /// Header text.
    pub title: String,
    /// Whether the text is right-aligned.
    pub right: bool,
    /// Whether the column absorbs extra width and shrinks first.
    pub flex: bool,
    /// Maximum lines of text in a cell (one truncates with an ellipsis).
    pub lines: usize,
}

impl Col {
    /// A left-aligned, fixed column.
    pub(super) fn text(title: &str) -> Self {
        Self {
            title: title.to_owned(),
            right: false,
            flex: false,
            lines: 1,
        }
    }

    /// A left-aligned column that takes the spare width.
    pub(super) fn flex(title: &str) -> Self {
        Self {
            title: title.to_owned(),
            right: false,
            flex: true,
            lines: 1,
        }
    }

    /// A right-aligned numeric column.
    pub(super) fn num(title: &str) -> Self {
        Self {
            title: title.to_owned(),
            right: true,
            flex: false,
            lines: 1,
        }
    }

    /// Allows up to `lines` lines of wrapped text.
    pub(super) fn wrapped(mut self, lines: usize) -> Self {
        self.lines = lines.max(1);
        self
    }
}

/// A table cell.
#[derive(Debug, Clone)]
pub(super) struct Cell {
    /// The text.
    pub text: String,
    /// A progress bar, in percent, drawn to the left of the (right-aligned) text.
    pub bar: Option<f64>,
    /// An optional emphasis color for the text.
    pub color: Option<Color>,
}

impl Cell {
    /// A plain text cell.
    pub(super) fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            bar: None,
            color: None,
        }
    }

    /// A cell with a progress bar and a value text.
    pub(super) fn bar(text: impl Into<String>, percent: f64) -> Self {
        Self {
            text: text.into(),
            bar: Some(percent),
            color: None,
        }
    }

    /// A text cell in the given color.
    pub(super) fn colored(text: impl Into<String>, color: Color) -> Self {
        Self {
            text: text.into(),
            bar: None,
            color: Some(color),
        }
    }
}

/// Computes the column widths for `cols` and `rows`, adding up to `total`.
pub(super) fn column_widths(cols: &[Col], rows: &[Vec<Cell>], total: f64) -> Vec<f64> {
    let cap = total * MAX_NATURAL_SHARE;
    let natural: Vec<f64> = cols
        .iter()
        .enumerate()
        .map(|(index, col)| {
            let header = text_width(Font::Bold, &encode(&col.title), SIZE);
            let mut widest = header;
            for row in rows {
                if let Some(cell) = row.get(index) {
                    let cell_width = text_width(Font::Regular, &encode(&cell.text), SIZE);
                    widest = widest.max(if cell.bar.is_some() {
                        cell_width + 70.0
                    } else {
                        cell_width
                    });
                }
            }
            let has_bar = rows
                .iter()
                .any(|row| row.get(index).is_some_and(|cell| cell.bar.is_some()));
            let width = widest + 2.0 * PAD_X + 0.6;
            if has_bar {
                width.max(BAR_COLUMN)
            } else {
                width.min(cap)
            }
        })
        .collect();
    let minimum: Vec<f64> = natural.iter().map(|w| w.min(MIN_COLUMN)).collect();
    let sum: f64 = natural.iter().sum();
    let mut widths = natural.clone();
    if sum <= total {
        let flex_total: f64 = cols
            .iter()
            .zip(&natural)
            .filter(|(c, _)| c.flex)
            .map(|(_, w)| w)
            .sum();
        let spare = total - sum;
        if flex_total > 0.0 {
            for ((width, col), base) in widths.iter_mut().zip(cols).zip(&natural) {
                if col.flex {
                    *width += spare * base / flex_total;
                }
            }
        } else if sum > 0.0 {
            for (width, base) in widths.iter_mut().zip(&natural) {
                *width += spare * base / sum;
            }
        }
    } else {
        shrink(&mut widths, cols, &minimum, sum - total);
    }
    let scale: f64 = total / widths.iter().sum::<f64>().max(1.0);
    widths.iter().map(|w| w * scale).collect()
}

/// Removes `excess` points from the columns: flexible ones first, then all of them.
fn shrink(widths: &mut [f64], cols: &[Col], minimum: &[f64], excess: f64) {
    let room = |only_flex: bool, widths: &[f64]| -> f64 {
        widths
            .iter()
            .zip(minimum)
            .zip(cols)
            .filter(|(_, col)| !only_flex || col.flex)
            .map(|((w, m), _)| (w - m).max(0.0))
            .sum()
    };
    let mut remaining = excess;
    for only_flex in [true, false] {
        let available = room(only_flex, widths);
        if available <= 0.0 {
            continue;
        }
        let take = remaining.min(available);
        let snapshot: Vec<f64> = widths.to_vec();
        for (((width, base), min), col) in widths.iter_mut().zip(&snapshot).zip(minimum).zip(cols) {
            if !only_flex || col.flex {
                *width -= take * (base - min).max(0.0) / available;
            }
        }
        remaining -= take;
        if remaining <= 1e-9 {
            break;
        }
    }
}

impl Doc {
    /// Draws a table whose header is repeated on every page it spans.
    pub(super) fn table(&mut self, cols: &[Col], rows: &[Vec<Cell>]) {
        let widths = column_widths(cols, rows, CONTENT_W);
        let header_height = LINE + 2.0 * PAD_Y;
        self.ensure(header_height + 2.0 * LINE + 2.0 * PAD_Y);
        self.table_header(cols, &widths);
        for (index, row) in rows.iter().enumerate() {
            let cells: Vec<Vec<Vec<u8>>> = cols
                .iter()
                .enumerate()
                .map(|(column, col)| {
                    let text = row
                        .get(column)
                        .map(|cell| encode(&cell.text))
                        .unwrap_or_default();
                    let width = (widths[column] - 2.0 * PAD_X).max(4.0);
                    if col.lines > 1 {
                        wrap(Font::Regular, &text, SIZE, width, col.lines)
                    } else {
                        vec![fit(
                            Font::Regular,
                            &text,
                            SIZE,
                            width - bar_reserve(row.get(column)),
                        )]
                    }
                })
                .collect();
            let lines = cells.iter().map(Vec::len).max().unwrap_or(1).max(1);
            let height = usize_to_f64(lines) * LINE + 2.0 * PAD_Y;
            if self.y + height > BOTTOM {
                self.new_page();
                self.table_header(cols, &widths);
            }
            self.table_row(cols, row, &cells, &widths, height, index % 2 == 1);
        }
        let border = self.theme.border;
        self.page.line(
            (MARGIN_X, self.y),
            (MARGIN_X + CONTENT_W, self.y),
            border,
            0.6,
        );
        self.y += 10.0;
    }

    /// Draws the header row at the cursor.
    fn table_header(&mut self, cols: &[Col], widths: &[f64]) {
        let theme = self.theme;
        let height = LINE + 2.0 * PAD_Y;
        self.page
            .fill_rect(MARGIN_X, self.y, CONTENT_W, height, theme.head);
        let mut x = MARGIN_X;
        for (col, width) in cols.iter().zip(widths) {
            let text = fit(
                Font::Bold,
                &encode(&col.title),
                SIZE,
                (width - 2.0 * PAD_X).max(4.0),
            );
            let baseline = Self::baseline(self.y + PAD_Y, LINE, SIZE);
            let start = if col.right {
                x + width - PAD_X - text_width(Font::Bold, &text, SIZE)
            } else {
                x + PAD_X
            };
            self.page
                .text(start, baseline, Font::Bold, SIZE, theme.ink, &text);
            x += width;
        }
        self.page.line(
            (MARGIN_X, self.y + height),
            (MARGIN_X + CONTENT_W, self.y + height),
            theme.border,
            0.8,
        );
        self.y += height;
    }

    /// Draws one body row of the given height.
    fn table_row(
        &mut self,
        cols: &[Col],
        row: &[Cell],
        cells: &[Vec<Vec<u8>>],
        widths: &[f64],
        height: f64,
        shaded: bool,
    ) {
        let theme = self.theme;
        if shaded {
            self.page
                .fill_rect(MARGIN_X, self.y, CONTENT_W, height, theme.zebra);
        }
        let mut x = MARGIN_X;
        for (column, (col, width)) in cols.iter().zip(widths).enumerate() {
            let cell = row.get(column);
            let color = cell.and_then(|c| c.color).unwrap_or(theme.ink);
            for (line_index, line) in cells[column].iter().enumerate() {
                let top = self.y + PAD_Y + usize_to_f64(line_index) * LINE;
                let baseline = Self::baseline(top, LINE, SIZE);
                let line_width = text_width(Font::Regular, line, SIZE);
                let start = if col.right || cell.is_some_and(|c| c.bar.is_some()) {
                    x + width - PAD_X - line_width
                } else {
                    x + PAD_X
                };
                self.page
                    .text(start, baseline, Font::Regular, SIZE, color, line);
            }
            if let Some(percent) = cell.and_then(|c| c.bar) {
                let text_w = cells[column]
                    .first()
                    .map_or(0.0, |line| text_width(Font::Regular, line, SIZE));
                let bar_width = width - 2.0 * PAD_X - text_w - 8.0;
                if bar_width >= 16.0 {
                    let y = self.y + (height - 5.0) / 2.0;
                    self.page
                        .fill_rect(x + PAD_X, y, bar_width, 5.0, theme.track);
                    let fraction = if percent.is_finite() {
                        (percent / 100.0).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    self.page
                        .fill_rect(x + PAD_X, y, bar_width * fraction, 5.0, theme.accent);
                }
            }
            x += width;
        }
        self.y += height;
    }
}

/// Width a bar cell keeps free for its bar, so the text is shortened before the bar is.
fn bar_reserve(cell: Option<&Cell>) -> f64 {
    if cell.is_some_and(|c| c.bar.is_some()) {
        24.0
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::{Cell, Col, column_widths};
    use crate::pdf::doc::{BOTTOM, Doc};

    /// Builds rows of plain cells.
    fn rows(data: &[&[&str]]) -> Vec<Vec<Cell>> {
        data.iter()
            .map(|row| row.iter().map(|text| Cell::text(*text)).collect())
            .collect()
    }

    /// Widths always add up to the available width, whatever the content.
    #[test]
    fn widths_fill_the_page() {
        let cols = vec![Col::flex("Name"), Col::num("Count"), Col::text("Kind")];
        let short = rows(&[&["a", "1", "fn"]]);
        let long = rows(&[&["x".repeat(400).as_str(), "123456789", "structure"]]);
        for data in [&short, &long, &Vec::new()] {
            let widths = column_widths(&cols, data, 500.0);
            assert!(
                (widths.iter().sum::<f64>() - 500.0).abs() < 1e-6,
                "{widths:?}"
            );
            assert!(widths.iter().all(|w| *w > 0.0));
        }
    }

    /// The flexible column takes the spare room and shrinks first.
    #[test]
    fn flexible_columns_absorb_changes() {
        let cols = vec![Col::flex("Name"), Col::num("Count")];
        let short = column_widths(&cols, &rows(&[&["a", "1"]]), 500.0);
        assert!(short[0] > short[1] * 3.0, "{short:?}");
        let long = column_widths(
            &cols,
            &rows(&[&["x".repeat(300).as_str(), "1000000"]]),
            500.0,
        );
        assert!(long[1] < 120.0, "{long:?}");
        assert!(long[0] > 300.0, "{long:?}");
    }

    /// Many wide columns still fit by shrinking everything.
    #[test]
    fn many_columns_shrink_together() {
        let cols: Vec<Col> = (0..12)
            .map(|i| Col::text(&format!("Column number {i}")))
            .collect();
        let data = vec![
            (0..12)
                .map(|i| Cell::text("y".repeat(60 + i)))
                .collect::<Vec<_>>(),
        ];
        let widths = column_widths(&cols, &data, 500.0);
        assert!((widths.iter().sum::<f64>() - 500.0).abs() < 1e-6);
    }

    /// A bar column gets a minimum width for its bar.
    #[test]
    fn bar_columns_get_room() {
        let cols = vec![Col::flex("Language"), Col::text("Coverage")];
        let data = vec![vec![Cell::text("Rust"), Cell::bar("87.5%", 87.5)]];
        let widths = column_widths(&cols, &data, 500.0);
        assert!(widths[1] >= 120.0, "{widths:?}");
    }

    /// A long table spans pages, repeats its header on each and never runs past the bottom.
    #[test]
    fn long_tables_break_between_rows_and_repeat_the_header() {
        let mut doc = Doc::new();
        let cols = vec![Col::flex("Header name"), Col::num("Count")];
        let data: Vec<Vec<Cell>> = (0..200)
            .map(|i| vec![Cell::text(format!("row {i}")), Cell::text(i.to_string())])
            .collect();
        doc.table(&cols, &data);
        assert!(doc.done.len() >= 3);
        assert!(doc.y <= BOTTOM + 12.0);
        let mut pages: Vec<String> = doc
            .done
            .iter()
            .map(|canvas| String::from_utf8_lossy(&canvas.clone().into_bytes()).into_owned())
            .collect();
        pages.push(String::from_utf8_lossy(&doc.page.clone().into_bytes()).into_owned());
        for page in &pages {
            assert!(
                page.contains("(Header name)"),
                "every page repeats the header"
            );
        }
        let rows: usize = pages.iter().map(|page| page.matches("(row ").count()).sum();
        assert_eq!(rows, 200, "every row is drawn exactly once");
    }

    /// Wrapped cells make taller rows, and bar cells draw a bar.
    #[test]
    fn wrapped_and_bar_cells() {
        let mut doc = Doc::new();
        let cols = vec![Col::flex("Text").wrapped(3), Col::text("Coverage")];
        let long = "many words ".repeat(60);
        let data = vec![
            vec![Cell::text(long), Cell::bar("50%", 50.0)],
            vec![Cell::text("short"), Cell::bar("nan", f64::NAN)],
        ];
        let before = doc.y;
        doc.table(&cols, &data);
        assert!(doc.y - before > 60.0, "the wrapped row is three lines tall");
        let page = String::from_utf8_lossy(&doc.page.clone().into_bytes()).into_owned();
        assert!(page.contains("(50%)"));
        assert!(
            page.matches(" re f").count() > 4,
            "shaded header, track and fill rectangles"
        );
    }
}
