// SPDX-License-Identifier: Apache-2.0
//! Tests with data at scale: thousands of rows, hundreds of nodes and very long strings.

use std::time::Instant;

use pn_ultramemory_report::{Lang, render_html, render_markdown, render_pdf};

use crate::fixtures::huge;
use crate::html_check::{Mode, check_markup};
use crate::pdf_check::check_pdf;

/// Generous wall-clock bound for a debug build on a busy machine; release builds take a few
/// milliseconds.
const BOUND_MS: u128 = 10_000;

/// The huge report renders to valid, bounded HTML quickly.
#[test]
fn huge_html() {
    let data = huge();
    let started = Instant::now();
    let html = render_html(&data, Lang::En);
    let elapsed = started.elapsed();
    assert!(elapsed.as_millis() < BOUND_MS, "{elapsed:?}");
    println!("huge html: {} bytes in {elapsed:?}", html.len());
    check_markup(&html, Mode::Html).expect("valid HTML");
    assert!(html.len() < 600_000, "{}", html.len());
}

/// The huge report renders to a valid, bounded PDF quickly.
#[test]
fn huge_pdf() {
    let data = huge();
    let started = Instant::now();
    let bytes = render_pdf(&data, Lang::Es);
    let elapsed = started.elapsed();
    assert!(elapsed.as_millis() < BOUND_MS, "{elapsed:?}");
    println!("huge pdf: {} bytes in {elapsed:?}", bytes.len());
    let pdf = check_pdf(&bytes).expect("valid PDF");
    assert!(bytes.len() < 1_000_000, "{}", bytes.len());
    assert!(pdf.pages.len() < 40, "{}", pdf.pages.len());
}

/// The huge report renders to bounded Markdown quickly.
#[test]
fn huge_markdown() {
    let data = huge();
    let started = Instant::now();
    let markdown = render_markdown(&data, Lang::En);
    let elapsed = started.elapsed();
    assert!(elapsed.as_millis() < BOUND_MS, "{elapsed:?}");
    println!("huge markdown: {} bytes in {elapsed:?}", markdown.len());
    assert!(markdown.len() < 600_000, "{}", markdown.len());
}

/// Writes the hostile and huge reports to the directory named by `PN_REPORT_DUMP_DIR`, so they can
/// be checked with external tools (a PDF library, a browser, the Mermaid parser). It does nothing
/// when the variable is unset. Run it with `cargo test -p pn-ultramemory-report -- --ignored`.
#[test]
#[ignore = "writes files for external validation; set PN_REPORT_DUMP_DIR"]
fn dump_outputs_for_external_validation() {
    use pn_ultramemory_report::{GraphFormat, render_graph};

    let Some(dir) = std::env::var_os("PN_REPORT_DUMP_DIR") else {
        return;
    };
    let dir = std::path::Path::new(&dir);
    std::fs::create_dir_all(dir).expect("create the output directory");
    for (name, data) in [("hostile", crate::fixtures::hostile()), ("huge", huge())] {
        for lang in [Lang::En, Lang::Es] {
            let code = lang.code();
            std::fs::write(
                dir.join(format!("{name}-{code}.html")),
                render_html(&data, lang),
            )
            .expect("write html");
            std::fs::write(
                dir.join(format!("{name}-{code}.pdf")),
                render_pdf(&data, lang),
            )
            .expect("write pdf");
            std::fs::write(
                dir.join(format!("{name}-{code}.md")),
                render_markdown(&data, lang),
            )
            .expect("write markdown");
            for format in GraphFormat::ALL {
                let text = render_graph(&data.graph, format, lang);
                std::fs::write(
                    dir.join(format!("{name}-graph-{code}.{}", format.extension())),
                    text,
                )
                .expect("write graph");
            }
        }
    }
}
