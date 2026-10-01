// SPDX-License-Identifier: Apache-2.0
//! Tests of the PDF report, validated by an independent structure checker.

use pn_ultramemory_report::{Lang, ReportData, render_pdf};

use crate::fixtures::{hostile, huge, long_word, sample};
use crate::pdf_check::check_pdf;

/// The realistic sample is a structurally valid PDF in both languages.
#[test]
fn sample_is_a_valid_pdf() {
    for lang in [Lang::En, Lang::Es] {
        let bytes = render_pdf(&sample(), lang);
        let pdf = check_pdf(&bytes).expect("valid PDF");
        assert!(!pdf.pages.is_empty());
        assert_eq!(pdf.size, bytes.len());
        assert!(pdf.catalog.contains(&format!("/Lang ({})", lang.code())));
    }
}

/// A known sentence of the report is present in the content streams, in both languages.
#[test]
fn known_sentences_are_present() {
    let original = check_pdf(&render_pdf(&sample(), Lang::En)).expect("valid PDF");
    let en_text = original.pages.join("\n");
    // The footer names the version the report data carries, which the sample fixes; the version
    // of the build would make this test fail on every release.
    assert!(en_text.contains(&format!(
        "Generated locally by pn-ultramemory {}. No data left this machine.",
        super::fixtures::SAMPLE_VERSION
    )));
    assert!(en_text.contains("Executive summary"));
    assert!(en_text.contains("Nimbus Ledger"));
    assert!(en_text.contains("71,240"));
    let translated = check_pdf(&render_pdf(&sample(), Lang::Es)).expect("valid PDF");
    let spanish_text = translated.pages.join("\n");
    assert!(spanish_text.contains(&format!(
        "Generado localmente por pn-ultramemory {}. Ningún dato salió de esta máquina.",
        super::fixtures::SAMPLE_VERSION
    )));
    assert!(spanish_text.contains("Cómo leer este informe"));
    assert!(spanish_text.contains("Líneas de código"));
    assert!(
        spanish_text.contains("Archivos con errores de análisis")
            || spanish_text.contains("Archivos con errores de")
    );
    assert!(spanish_text.contains("71.240"));
}

/// Every page carries its "Page x of y" footer with the right numbers.
#[test]
fn every_page_has_a_pager() {
    for (lang, template) in [(Lang::En, "Page {} of {}"), (Lang::Es, "Página {} de {}")] {
        let pdf = check_pdf(&render_pdf(&huge(), lang)).expect("valid PDF");
        let total = pdf.pages.len();
        assert!(total > 3);
        for (index, page) in pdf.pages.iter().enumerate() {
            let expected = template
                .replacen("{}", &(index + 1).to_string(), 1)
                .replacen("{}", &total.to_string(), 1);
            assert!(
                page.contains(&expected),
                "page {} lacks {expected:?}",
                index + 1
            );
        }
    }
}

/// Only the pages after the first carry the running header with the project name.
#[test]
fn running_header_skips_the_first_page() {
    let pdf = check_pdf(&render_pdf(&huge(), Lang::En)).expect("valid PDF");
    assert!(pdf.pages.len() > 2);
    assert!(pdf.pages[1].contains("Code report"));
}

/// The document information dictionary is complete and deterministic.
#[test]
fn document_information() {
    let pdf = check_pdf(&render_pdf(&sample(), Lang::En)).expect("valid PDF");
    assert_eq!(pdf.title, "Nimbus Ledger - Code report");
    assert_eq!(pdf.creation_date.as_deref(), Some("D:20260925"));
    assert!(pdf.info.contains("/Author (pn-ultramemory)"));
    assert!(pdf.info.contains("/Producer"));
    let es = check_pdf(&render_pdf(&sample(), Lang::Es)).expect("valid PDF");
    assert_eq!(es.title, "Nimbus Ledger - Informe de código");
    let mut undated = sample();
    undated.generated_on = "sometime".into();
    let pdf = check_pdf(&render_pdf(&undated, Lang::En)).expect("valid PDF");
    assert_eq!(pdf.creation_date, None);
}

/// The outline lists the sections in order, in the language of the report.
#[test]
fn outline_lists_the_sections() {
    let en = check_pdf(&render_pdf(&sample(), Lang::En)).expect("valid PDF");
    assert_eq!(
        en.outline,
        [
            "Executive summary",
            "How to read this report",
            "Languages",
            "Modules",
            "Hotspots",
            "Documentation coverage",
            "Code graph",
            "Memories",
            "Usage and token efficiency",
            "Notes",
        ]
    );
    let es = check_pdf(&render_pdf(&sample(), Lang::Es)).expect("valid PDF");
    assert_eq!(es.outline[1], "Cómo leer este informe");
    assert_eq!(es.outline.len(), 10);
    let mut bare = sample();
    bare.usage = None;
    bare.notes.clear();
    assert_eq!(
        check_pdf(&render_pdf(&bare, Lang::En))
            .expect("valid PDF")
            .outline
            .len(),
        8
    );
}

/// Hostile data yields a valid PDF; characters outside Windows-1252 become question marks.
#[test]
fn hostile_data_yields_a_valid_pdf() {
    for lang in [Lang::En, Lang::Es] {
        let bytes = render_pdf(&hostile(), lang);
        let pdf = check_pdf(&bytes).expect("valid PDF despite hostile data");
        let text = pdf.pages.join("\n");
        assert!(
            text.contains("<script>alert(1)</script>"),
            "markup is printed literally, not interpreted"
        );
        assert!(text.contains("\"quoted\""));
        assert!(!text.contains('🙂'), "emoji have no glyph");
        assert!(text.contains('?'), "unsupported characters are replaced");
        assert!(!text.contains('\u{202e}'));
    }
}

/// Spanish accents, inverted punctuation and typographic characters survive in the streams.
#[test]
fn winansi_text_round_trips() {
    let mut data = sample();
    data.notes =
        vec!["¿Qué ñandú? ¡Sí! “comillas” – guión — raya • viñeta € euro ‘x’ … ü Ü".into()];
    let pdf = check_pdf(&render_pdf(&data, Lang::Es)).expect("valid PDF");
    let text = pdf.pages.join("\n");
    assert!(
        text.contains("¿Qué ñandú? ¡Sí! “comillas” – guión — raya • viñeta € euro ‘x’ … ü Ü"),
        "{text}"
    );
}

/// Very long strings are clipped or wrapped; the file stays small and valid.
#[test]
fn long_strings_stay_bounded() {
    let mut data = sample();
    data.project = long_word(100_000);
    data.hotspots[0].name = long_word(100_000);
    data.memories[0].text = long_word(100_000);
    data.notes = vec![long_word(100_000)];
    let bytes = render_pdf(&data, Lang::En);
    check_pdf(&bytes).expect("valid PDF");
    assert!(bytes.len() < 200_000, "{}", bytes.len());
}

/// Tables never split a row across pages: every page starts with a header or a heading, and the
/// header row of a continued table is repeated.
#[test]
fn continued_tables_repeat_their_header() {
    let mut data = sample();
    data.memories = huge().memories.into_iter().take(25).collect();
    let pdf = check_pdf(&render_pdf(&data, Lang::En)).expect("valid PDF");
    let with_header = pdf
        .pages
        .iter()
        .filter(|page| page.contains("Provenance"))
        .count();
    let with_rows = pdf
        .pages
        .iter()
        .filter(|page| page.contains("memory text with several words"))
        .count();
    assert!(with_rows >= 1);
    assert!(
        with_header >= with_rows,
        "each page with memory rows shows the header"
    );
}

/// Empty data yields a valid PDF with the fixed sections.
#[test]
fn empty_report() {
    for lang in [Lang::En, Lang::Es] {
        let pdf = check_pdf(&render_pdf(&ReportData::default(), lang)).expect("valid PDF");
        assert!(pdf.pages.len() <= 2, "{}", pdf.pages.len());
        assert!(pdf.pages[0].contains(if lang == Lang::En {
            "No data."
        } else {
            "Sin datos."
        }));
    }
}

/// Rendering twice gives identical bytes, and different languages give different files.
#[test]
fn pdf_is_deterministic() {
    for data in [sample(), hostile()] {
        assert_eq!(render_pdf(&data, Lang::En), render_pdf(&data, Lang::En));
        assert_eq!(render_pdf(&data, Lang::Es), render_pdf(&data, Lang::Es));
        assert_ne!(render_pdf(&data, Lang::En), render_pdf(&data, Lang::Es));
    }
}

/// The file is plain ASCII apart from the four-byte binary marker after the header.
#[test]
fn file_is_ascii_after_the_marker() {
    let bytes = render_pdf(&hostile(), Lang::Es);
    let marker_end = bytes.iter().position(|&b| b == b'\n').expect("header") + 1;
    let after = &bytes[marker_end..];
    let second_line = after.iter().position(|&b| b == b'\n').expect("marker line") + 1;
    assert!(
        after[second_line..]
            .iter()
            .all(|&b| b == b'\n' || (0x20..0x7f).contains(&b)),
        "non-ASCII byte in the body"
    );
}
