// SPDX-License-Identifier: Apache-2.0
//! The page-flow engine of the PDF: pages, the cursor, headings, paragraphs, cards and bars.
//!
//! [`Doc`] keeps a cursor that moves down the page. Every block asks for the room it needs; when
//! it does not fit, a new page is started, so a table row is never split and a heading is never
//! left alone at the bottom (headings reserve room for the first lines that follow them). When
//! the document is finished, every page receives its header and its "Page x of y" footer, and the
//! objects are written: catalog, page tree, fonts, bookmarks (outline), and one content stream
//! plus one page object per page.
//!
//! Invariants: all vertical positions are measured from the top of the page; content never enters
//! the header or footer bands; the page count in `/Count` equals the number of `/Page` objects.

use std::fmt::Write as _;

use super::canvas::{Canvas, Color};
use super::fonts::{Font, encode, fit, text_width, wrap};
use super::writer::{Pdf, creation_date, literal, stream_body, text_string};
use crate::i18n::{Key, tf};
use crate::lang::Lang;
use crate::numfmt::usize_to_f64;

/// Page width of A4 in points.
pub(super) const PAGE_W: f64 = 595.28;
/// Page height of A4 in points.
pub(super) const PAGE_H: f64 = 841.89;
/// Left and right margin.
pub(super) const MARGIN_X: f64 = 48.0;
/// Top of the content area on pages after the first.
pub(super) const TOP: f64 = 62.0;
/// Bottom limit of the content area.
pub(super) const BOTTOM: f64 = PAGE_H - 56.0;
/// Width of the content area.
pub(super) const CONTENT_W: f64 = PAGE_W - 2.0 * MARGIN_X;

/// The colors of the document.
#[derive(Debug, Clone, Copy)]
pub(super) struct Theme {
    /// Body text.
    pub ink: Color,
    /// Secondary text.
    pub muted: Color,
    /// Accent for headings and bars.
    pub accent: Color,
    /// Tint behind cards.
    pub accent_soft: Color,
    /// Table header background.
    pub head: Color,
    /// Alternate table row background.
    pub zebra: Color,
    /// Hairlines and frames.
    pub border: Color,
    /// Empty part of a bar.
    pub track: Color,
    /// Warning text.
    pub warn: Color,
    /// Warning background.
    pub warn_bg: Color,
    /// Error text.
    pub bad: Color,
    /// Error background.
    pub bad_bg: Color,
}

impl Theme {
    /// Builds the theme, which matches the light theme of the HTML report.
    pub(super) fn new() -> Self {
        Self {
            ink: Color::hex("#14181f"),
            muted: Color::hex("#4b5563"),
            accent: Color::hex("#0f6b63"),
            accent_soft: Color::hex("#e3f3f1"),
            head: Color::hex("#e3e9f0"),
            zebra: Color::hex("#f6f8fa"),
            border: Color::hex("#cfd6df"),
            track: Color::hex("#dde3ea"),
            warn: Color::hex("#8a4b00"),
            warn_bg: Color::hex("#fff2dc"),
            bad: Color::hex("#a12727"),
            bad_bg: Color::hex("#fde8e8"),
        }
    }
}

/// A section bookmark.
#[derive(Debug, Clone)]
struct Bookmark {
    /// Title shown in the outline.
    title: String,
    /// Zero-based index of the page.
    page: usize,
    /// Distance of the heading from the top of the page.
    y: f64,
}

/// The tone of a metric card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Tone {
    /// A regular metric.
    Normal,
    /// A metric worth a look.
    Warn,
    /// A metric that signals a problem.
    Bad,
}

/// The texts that surround the pages: header, footer and document properties.
#[derive(Debug, Clone)]
pub(super) struct Meta {
    /// Language of the document.
    pub lang: Lang,
    /// Document title.
    pub title: String,
    /// Project name, printed in the page header.
    pub project: String,
    /// Report kind, printed at the right of the page header.
    pub kind: String,
    /// Tool version, for the producer.
    pub version: String,
    /// The `generated_on` text, source of the creation date.
    pub generated_on: String,
    /// The footer sentence.
    pub footer: String,
}

/// A document being laid out.
#[derive(Debug)]
pub(super) struct Doc {
    /// Pages already finished.
    pub(super) done: Vec<Canvas>,
    /// The page being drawn.
    pub(super) page: Canvas,
    /// Distance from the top of the page to the next free line.
    pub(super) y: f64,
    /// Colors.
    pub(super) theme: Theme,
    /// Bookmarks collected from section headings.
    bookmarks: Vec<Bookmark>,
}

impl Doc {
    /// Starts a document with one empty page.
    pub(super) fn new() -> Self {
        Self {
            done: Vec::new(),
            page: Canvas::new(PAGE_H),
            y: TOP,
            theme: Theme::new(),
            bookmarks: Vec::new(),
        }
    }

    /// Starts a new page.
    pub(super) fn new_page(&mut self) {
        let finished = std::mem::replace(&mut self.page, Canvas::new(PAGE_H));
        self.done.push(finished);
        self.y = TOP;
    }

    /// Starts a new page unless `needed` points fit below the cursor (or the page is empty).
    pub(super) fn ensure(&mut self, needed: f64) {
        if self.y + needed > BOTTOM && self.y > TOP + 0.5 {
            self.new_page();
        }
    }

    /// Returns the baseline that centers text of `size` in a line box of height `line` at `top`.
    pub(super) fn baseline(top: f64, line: f64, size: f64) -> f64 {
        top + f64::midpoint(line, 0.72 * size)
    }

    /// Draws the title block on the first page.
    pub(super) fn title_block(
        &mut self,
        kicker: &str,
        title: &str,
        project: &str,
        meta: &[(String, String)],
    ) {
        let theme = self.theme;
        self.y = 52.0;
        let width = CONTENT_W;
        self.page
            .fill_rect(MARGIN_X, self.y, width, 3.0, theme.accent);
        self.y += 18.0;
        self.page.text(
            MARGIN_X,
            self.y + 8.0,
            Font::Bold,
            8.5,
            theme.accent,
            &encode(&kicker.to_uppercase()),
        );
        self.y += 16.0;
        let title_bytes = encode(title);
        self.page.text(
            MARGIN_X,
            self.y + 24.0,
            Font::Bold,
            26.0,
            theme.ink,
            &fit(Font::Bold, &title_bytes, 26.0, width),
        );
        self.y += 40.0;
        for line in wrap(Font::Bold, &encode(project), 15.0, width, 2) {
            self.page
                .text(MARGIN_X, self.y + 13.0, Font::Bold, 15.0, theme.ink, &line);
            self.y += 20.0;
        }
        self.y += 4.0;
        for (label, value) in meta {
            let label_bytes = encode(&format!("{label}: "));
            let label_width = text_width(Font::Regular, &label_bytes, 9.0);
            self.page.text(
                MARGIN_X,
                self.y + 9.0,
                Font::Regular,
                9.0,
                theme.muted,
                &label_bytes,
            );
            let rest = (width - label_width).max(20.0);
            self.page.text(
                MARGIN_X + label_width,
                self.y + 9.0,
                Font::Bold,
                9.0,
                theme.ink,
                &fit(Font::Bold, &encode(value), 9.0, rest),
            );
            self.y += 14.0;
        }
        self.y += 8.0;
        self.page.line(
            (MARGIN_X, self.y),
            (MARGIN_X + width, self.y),
            theme.border,
            0.8,
        );
        self.y += 6.0;
    }

    /// Starts a section: a large heading with a rule, remembered as a bookmark.
    pub(super) fn section(&mut self, title: &str) {
        self.section_with_room(title, 90.0);
    }

    /// Starts a section that needs at least `room` points below the cursor, heading included;
    /// otherwise it starts on a new page, so a heading never sits apart from its content.
    pub(super) fn section_with_room(&mut self, title: &str, room: f64) {
        self.ensure(room);
        if self.y > TOP + 1.0 {
            self.y += 14.0;
        }
        self.bookmarks.push(Bookmark {
            title: title.to_owned(),
            page: self.done.len(),
            y: self.y,
        });
        let theme = self.theme;
        let bytes = encode(title);
        self.page.text(
            MARGIN_X,
            self.y + 14.0,
            Font::Bold,
            14.0,
            theme.accent,
            &fit(Font::Bold, &bytes, 14.0, CONTENT_W),
        );
        self.page.line(
            (MARGIN_X, self.y + 21.0),
            (MARGIN_X + CONTENT_W, self.y + 21.0),
            theme.border,
            0.8,
        );
        self.y += 30.0;
    }

    /// Starts a subsection with a smaller heading.
    pub(super) fn subsection(&mut self, title: &str) {
        self.ensure(56.0);
        self.y += 6.0;
        let theme = self.theme;
        let bytes = encode(title);
        self.page.text(
            MARGIN_X,
            self.y + 10.0,
            Font::Bold,
            10.5,
            theme.ink,
            &fit(Font::Bold, &bytes, 10.5, CONTENT_W),
        );
        self.y += 20.0;
    }

    /// Writes a wrapped paragraph; it may continue on the next page.
    pub(super) fn paragraph(&mut self, text: &str, font: Font, size: f64, color: Color) {
        let line_height = size * 1.4;
        for line in wrap(font, &encode(text), size, CONTENT_W, usize::MAX) {
            self.ensure(line_height);
            let baseline = Self::baseline(self.y, line_height, size);
            self.page.text(MARGIN_X, baseline, font, size, color, &line);
            self.y += line_height;
        }
        self.y += 4.0;
    }

    /// Writes one bullet point with a hanging indent.
    pub(super) fn bullet(&mut self, text: &str) {
        const INDENT: f64 = 12.0;
        let (size, line_height) = (9.5, 13.3);
        let theme = self.theme;
        for (index, line) in wrap(
            Font::Regular,
            &encode(text),
            size,
            CONTENT_W - INDENT,
            usize::MAX,
        )
        .iter()
        .enumerate()
        {
            self.ensure(line_height);
            let baseline = Self::baseline(self.y, line_height, size);
            if index == 0 {
                self.page.text(
                    MARGIN_X + 2.0,
                    baseline,
                    Font::Bold,
                    size,
                    theme.accent,
                    &encode("\u{2022}"),
                );
            }
            self.page.text(
                MARGIN_X + INDENT,
                baseline,
                Font::Regular,
                size,
                theme.ink,
                line,
            );
            self.y += line_height;
        }
        self.y += 2.0;
    }

    /// Writes a small muted note.
    pub(super) fn note(&mut self, text: &str) {
        let muted = self.theme.muted;
        self.paragraph(text, Font::Oblique, 8.0, muted);
    }

    /// Draws metric cards, four per row.
    pub(super) fn cards(&mut self, items: &[(String, String, Tone)]) {
        const PER_ROW: usize = 4;
        const GAP: f64 = 8.0;
        const HEIGHT: f64 = 50.0;
        let width = (CONTENT_W - GAP * 3.0) / 4.0;
        let theme = self.theme;
        for row in items.chunks(PER_ROW) {
            self.ensure(HEIGHT + GAP);
            for (index, (label, value, tone)) in row.iter().enumerate() {
                let x = MARGIN_X + (width + GAP) * usize_to_f64(index);
                let (background, stripe, ink) = match tone {
                    Tone::Normal => (theme.accent_soft, theme.accent, theme.ink),
                    Tone::Warn => (theme.warn_bg, theme.warn, theme.warn),
                    Tone::Bad => (theme.bad_bg, theme.bad, theme.bad),
                };
                self.page.fill_rect(x, self.y, width, HEIGHT, background);
                self.page.fill_rect(x, self.y, 3.0, HEIGHT, stripe);
                let inner = width - 18.0;
                self.page.text(
                    x + 11.0,
                    self.y + 23.0,
                    Font::Bold,
                    16.0,
                    ink,
                    &fit(Font::Bold, &encode(value), 16.0, inner),
                );
                for (line_index, line) in wrap(Font::Regular, &encode(label), 7.5, inner, 2)
                    .iter()
                    .enumerate()
                {
                    self.page.text(
                        x + 11.0,
                        self.y + 33.0 + 8.5 * usize_to_f64(line_index),
                        Font::Regular,
                        7.5,
                        theme.muted,
                        line,
                    );
                }
            }
            self.y += HEIGHT + GAP;
        }
        self.y += 4.0;
    }

    /// Draws horizontal bars: label, bar scaled to `0.0..=1.0`, and a value text.
    pub(super) fn bars(&mut self, rows: &[(String, f64, String)]) {
        const LABEL: f64 = 120.0;
        const ROW: f64 = 16.0;
        let theme = self.theme;
        let bar_width = CONTENT_W - LABEL - 70.0;
        for (label, fraction, value) in rows {
            self.ensure(ROW);
            let baseline = Self::baseline(self.y, ROW, 8.0);
            self.page.text(
                MARGIN_X,
                baseline,
                Font::Regular,
                8.0,
                theme.ink,
                &fit(Font::Regular, &encode(label), 8.0, LABEL - 8.0),
            );
            let x = MARGIN_X + LABEL;
            self.page
                .fill_rect(x, self.y + 3.5, bar_width, 9.0, theme.track);
            let filled = bar_width * fraction.clamp(0.0, 1.0);
            self.page
                .fill_rect(x, self.y + 3.5, filled, 9.0, theme.accent);
            let value_x = x + filled + 5.0;
            let room = (MARGIN_X + CONTENT_W - value_x).max(0.0);
            self.page.text(
                value_x,
                baseline,
                Font::Bold,
                8.0,
                theme.ink,
                &fit(Font::Bold, &encode(value), 8.0, room),
            );
            self.y += ROW;
        }
        self.y += 6.0;
    }

    /// Draws one full-width progress bar with a label and a value.
    pub(super) fn progress(&mut self, label: &str, percent: f64, value: &str) {
        self.ensure(30.0);
        let theme = self.theme;
        let value_bytes = encode(value);
        let value_width = text_width(Font::Bold, &value_bytes, 11.0);
        self.page.text(
            MARGIN_X,
            self.y + 10.0,
            Font::Regular,
            9.0,
            theme.muted,
            &fit(
                Font::Regular,
                &encode(label),
                9.0,
                CONTENT_W - value_width - 10.0,
            ),
        );
        self.page.text(
            MARGIN_X + CONTENT_W - value_width,
            self.y + 11.0,
            Font::Bold,
            11.0,
            theme.ink,
            &value_bytes,
        );
        self.page
            .fill_rect(MARGIN_X, self.y + 16.0, CONTENT_W, 8.0, theme.track);
        let fraction = if percent.is_finite() {
            (percent / 100.0).clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.page.fill_rect(
            MARGIN_X,
            self.y + 16.0,
            CONTENT_W * fraction,
            8.0,
            theme.accent,
        );
        self.y += 32.0;
    }

    /// Finishes the document and returns the PDF file.
    pub(super) fn finish(mut self, meta: &Meta) -> Vec<u8> {
        let last = std::mem::replace(&mut self.page, Canvas::new(PAGE_H));
        self.done.push(last);
        let total = self.done.len();
        let theme = self.theme;
        let total_text = crate::numfmt::int(u64::try_from(total).unwrap_or(u64::MAX), meta.lang);
        for (index, canvas) in self.done.iter_mut().enumerate() {
            if index > 0 {
                canvas.text(
                    MARGIN_X,
                    36.0,
                    Font::Regular,
                    8.0,
                    theme.muted,
                    &fit(Font::Regular, &encode(&meta.project), 8.0, CONTENT_W * 0.6),
                );
                let kind = encode(&meta.kind);
                let width = text_width(Font::Regular, &kind, 8.0);
                canvas.text(
                    MARGIN_X + CONTENT_W - width,
                    36.0,
                    Font::Regular,
                    8.0,
                    theme.muted,
                    &kind,
                );
                canvas.line(
                    (MARGIN_X, 44.0),
                    (MARGIN_X + CONTENT_W, 44.0),
                    theme.border,
                    0.5,
                );
            }
            canvas.line(
                (MARGIN_X, PAGE_H - 44.0),
                (MARGIN_X + CONTENT_W, PAGE_H - 44.0),
                theme.border,
                0.5,
            );
            let number =
                crate::numfmt::int(u64::try_from(index + 1).unwrap_or(u64::MAX), meta.lang);
            let pager = encode(&tf(meta.lang, Key::PageOf, &[&number, &total_text]));
            let pager_width = text_width(Font::Regular, &pager, 8.0);
            canvas.text(
                MARGIN_X + CONTENT_W - pager_width,
                PAGE_H - 30.0,
                Font::Regular,
                8.0,
                theme.muted,
                &pager,
            );
            canvas.text(
                MARGIN_X,
                PAGE_H - 30.0,
                Font::Regular,
                7.5,
                theme.muted,
                &fit(
                    Font::Regular,
                    &encode(&meta.footer),
                    7.5,
                    CONTENT_W - pager_width - 12.0,
                ),
            );
        }
        assemble(self.done, &self.bookmarks, meta)
    }
}

/// Writes all objects of the file and serializes it.
fn assemble(pages: Vec<Canvas>, bookmarks: &[Bookmark], meta: &Meta) -> Vec<u8> {
    let mut pdf = Pdf::new();
    let catalog = pdf.reserve();
    let tree = pdf.reserve();
    let info = pdf.reserve();
    let font_ids: Vec<usize> = Font::ALL.iter().map(|_| pdf.reserve()).collect();
    let outlines = pdf.reserve();
    let item_ids: Vec<usize> = bookmarks.iter().map(|_| pdf.reserve()).collect();
    let mut page_ids = Vec::with_capacity(pages.len());
    let mut streams = Vec::with_capacity(pages.len());
    for _ in &pages {
        streams.push(pdf.reserve());
        page_ids.push(pdf.reserve());
    }

    let mut resources = String::from("<< /Font << ");
    for (font, id) in Font::ALL.iter().zip(&font_ids) {
        let _ = write!(resources, "/{} {id} 0 R ", font.resource());
        pdf.set(
            *id,
            format!(
                "<< /Type /Font /Subtype /Type1 /BaseFont /{} /Encoding /WinAnsiEncoding >>",
                font.base_font()
            )
            .into_bytes(),
        );
    }
    resources.push_str(">> >>");

    let mut kids = String::new();
    for (index, canvas) in pages.into_iter().enumerate() {
        pdf.set(streams[index], stream_body("", &canvas.into_bytes()));
        let _ = write!(kids, "{} 0 R ", page_ids[index]);
        pdf.set(
            page_ids[index],
            format!(
                "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595.28 841.89] /Resources {resources} /Contents {} 0 R >>",
                streams[index]
            )
            .into_bytes(),
        );
    }
    pdf.set(
        tree,
        format!(
            "<< /Type /Pages /Kids [ {kids}] /Count {} >>",
            page_ids.len()
        )
        .into_bytes(),
    );

    write_outline(&mut pdf, outlines, &item_ids, bookmarks, &page_ids);

    let mut catalog_body = format!(
        "<< /Type /Catalog /Pages {tree} 0 R /Lang {} /ViewerPreferences << /DisplayDocTitle true >>",
        literal(meta.lang.code().as_bytes())
    );
    if !bookmarks.is_empty() {
        let _ = write!(catalog_body, " /Outlines {outlines} 0 R");
    }
    catalog_body.push_str(" >>");
    pdf.set(catalog, catalog_body.into_bytes());

    let producer = if meta.version.is_empty() {
        "pn-ultramemory".to_owned()
    } else {
        format!("pn-ultramemory {}", meta.version)
    };
    let mut info_body = format!(
        "<< /Title {} /Author (pn-ultramemory) /Creator (pn-ultramemory) /Producer {}",
        text_string(&meta.title),
        text_string(&producer)
    );
    if let Some(date) = creation_date(&meta.generated_on) {
        let _ = write!(info_body, " /CreationDate ({date})");
    }
    info_body.push_str(" >>");
    pdf.set(info, info_body.into_bytes());
    pdf.finish(catalog, info)
}

/// Writes the outline root and one item per bookmark.
fn write_outline(
    pdf: &mut Pdf,
    root: usize,
    items: &[usize],
    bookmarks: &[Bookmark],
    page_ids: &[usize],
) {
    if items.is_empty() {
        pdf.set(root, b"<< /Type /Outlines /Count 0 >>".to_vec());
        return;
    }
    let (first, last) = (items[0], items[items.len() - 1]);
    pdf.set(
        root,
        format!(
            "<< /Type /Outlines /First {first} 0 R /Last {last} 0 R /Count {} >>",
            items.len()
        )
        .into_bytes(),
    );
    for (index, (id, bookmark)) in items.iter().zip(bookmarks).enumerate() {
        let page = page_ids
            .get(bookmark.page)
            .or_else(|| page_ids.last())
            .copied()
            .unwrap_or(1);
        let mut body = format!(
            "<< /Title {} /Parent {root} 0 R /Dest [{page} 0 R /XYZ 0 {} null]",
            text_string(&bookmark.title),
            crate::numfmt::coord(PAGE_H - bookmark.y + 8.0)
        );
        if index > 0 {
            let _ = write!(body, " /Prev {} 0 R", items[index - 1]);
        }
        if let Some(next) = items.get(index + 1) {
            let _ = write!(body, " /Next {next} 0 R");
        }
        body.push_str(" >>");
        pdf.set(*id, body.into_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::{BOTTOM, Doc, Meta, TOP};
    use crate::lang::Lang;
    use crate::pdf::fonts::Font;

    /// Builds the texts around the pages for a test document.
    fn meta() -> Meta {
        Meta {
            lang: Lang::En,
            title: "Title".into(),
            project: "Project".into(),
            kind: "Report".into(),
            version: "1.0".into(),
            generated_on: "2026-01-02".into(),
            footer: "Footer text".into(),
        }
    }

    /// A block that does not fit starts a new page; one that fits, or an oversized block on an
    /// empty page, does not.
    #[test]
    fn ensure_starts_pages_only_when_needed() {
        let mut doc = Doc::new();
        doc.ensure(100.0);
        assert_eq!(doc.done.len(), 0);
        doc.y = BOTTOM - 10.0;
        doc.ensure(50.0);
        assert_eq!(doc.done.len(), 1);
        assert!((doc.y - TOP).abs() < 1e-9);
        doc.ensure(5_000.0);
        assert_eq!(
            doc.done.len(),
            1,
            "an oversized block on an empty page must not loop"
        );
    }

    /// A long paragraph flows onto further pages and never passes the bottom limit.
    #[test]
    fn paragraphs_flow_across_pages() {
        let mut doc = Doc::new();
        let ink = doc.theme.ink;
        doc.paragraph(&"word ".repeat(4_000), Font::Regular, 10.0, ink);
        assert!(doc.done.len() >= 3);
        assert!(doc.y <= BOTTOM + 1e-9);
    }

    /// Sections become bookmarks in the finished file, and the page count is exact.
    #[test]
    fn finished_file_has_outline_and_page_count() {
        let mut doc = Doc::new();
        doc.section("First");
        doc.new_page();
        doc.section("Second");
        doc.new_page();
        let bytes = doc.finish(&meta());
        let text = String::from_utf8_lossy(&bytes).into_owned();
        assert!(text.contains("/Count 3 >>"), "three pages");
        assert!(text.contains("/Type /Outlines /First"));
        assert_eq!(text.matches("/Type /Page /Parent").count(), 3);
        assert!(text.contains("/CreationDate (D:20260102)"));
        assert!(text.contains("Page 3 of 3"));
        assert!(text.contains("(Footer text)"));
    }

    /// Cards, bars and progress bars advance the cursor and stay on the page.
    #[test]
    fn blocks_advance_the_cursor() {
        let mut doc = Doc::new();
        let start = doc.y;
        let cards: Vec<(String, String, super::Tone)> = (0..9)
            .map(|i| (format!("Label {i}"), i.to_string(), super::Tone::Normal))
            .collect();
        doc.cards(&cards);
        assert!(doc.y > start + 3.0 * 50.0);
        let before = doc.y;
        doc.bars(&[
            ("a".into(), 0.5, "50".into()),
            ("b".into(), f64::NAN, "0".into()),
        ]);
        doc.progress("Coverage", 250.0, "100%");
        doc.progress("Coverage", f64::NAN, "0%");
        assert!(doc.y > before);
        assert!(doc.y <= BOTTOM + 60.0);
    }

    /// A value too wide for the room after a full bar is cut with an ellipsis instead of leaving
    /// the page.
    #[test]
    fn bar_values_stay_inside_the_margin() {
        let mut doc = Doc::new();
        doc.bars(&[("wide".into(), 1.0, "9".repeat(60))]);
        let page = String::from_utf8_lossy(&doc.page.clone().into_bytes()).into_owned();
        assert!(
            page.contains("\\205) Tj"),
            "the value ends with an ellipsis: {page}"
        );
        assert!(!page.contains(&"9".repeat(30)));
    }

    /// The baseline centers text inside its line box.
    #[test]
    fn baseline_is_inside_the_line_box() {
        let baseline = Doc::baseline(100.0, 14.0, 10.0);
        assert!(baseline > 100.0 && baseline < 114.0);
    }
}
