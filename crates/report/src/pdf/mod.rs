// SPDX-License-Identifier: Apache-2.0
//! The PDF report, written by hand without any dependency.
//!
//! [`render`] lays the report out on A4 portrait pages and serializes a PDF 1.4 file: standard
//! Helvetica fonts in `WinAnsiEncoding` (nothing embedded), uncompressed content streams, a
//! document information dictionary whose creation date comes only from `generated_on`, a
//! bookmark outline of the sections, and a "Page x of y" footer on every page.
//!
//! The pieces: `fonts` (encoding, widths from the Adobe AFM files, wrapping), `writer` (objects,
//! xref, trailer), `canvas` (page content streams), `doc` (page flow, headings, cards, bars,
//! final assembly), `table`, `figure` (the graph) and `report` (which section goes where).
//!
//! Invariants: the output is deterministic; nothing panics on hostile data; characters outside
//! Windows-1252 print as `?`.

mod canvas;
mod doc;
mod figure;
mod fonts;
mod report;
mod table;
mod writer;

use crate::lang::Lang;
use crate::model::ReportData;

/// Renders the report as a PDF file.
pub(crate) fn render(data: &ReportData, lang: Lang) -> Vec<u8> {
    let (document, meta) = report::build(data, lang);
    document.finish(&meta)
}

#[cfg(test)]
mod tests {
    use super::render;
    use crate::lang::Lang;
    use crate::model::ReportData;

    /// An empty report is a valid, non-empty PDF frame in both languages.
    #[test]
    fn empty_report_has_the_pdf_frame() {
        for lang in [Lang::En, Lang::Es] {
            let bytes = render(&ReportData::default(), lang);
            assert!(bytes.starts_with(b"%PDF-1.4\n"));
            assert!(bytes.ends_with(b"%%EOF\n"));
            assert!(bytes.len() > 2000);
        }
    }

    /// The same input gives the same bytes.
    #[test]
    fn output_is_deterministic() {
        let data = ReportData {
            project: "demo".into(),
            generated_on: "2026-09-25".into(),
            ..ReportData::default()
        };
        assert_eq!(render(&data, Lang::En), render(&data, Lang::En));
    }
}
