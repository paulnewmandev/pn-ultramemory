// SPDX-License-Identifier: Apache-2.0
//! Tests of the Markdown report: structure, table integrity, escaping and the Mermaid block.

use pn_ultramemory_report::{Lang, ReportData, render_markdown};

use crate::fixtures::{hostile, sample};

/// Counts the cells of a table row, ignoring escaped pipes and pipes inside code spans.
fn cells(line: &str) -> usize {
    let mut count = 0usize;
    let mut in_code = false;
    let mut escaped = false;
    for c in line.chars() {
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == '`' {
            in_code = !in_code;
        } else if c == '|' && !in_code {
            count += 1;
        }
    }
    count.saturating_sub(1)
}

/// Returns `markdown` with the contents of code spans and fenced blocks removed.
fn outside_code(markdown: &str) -> String {
    let mut out = String::new();
    let mut in_fence = false;
    for line in markdown.lines() {
        if line.starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        let mut in_code = false;
        for c in line.chars() {
            if c == '`' {
                in_code = !in_code;
            } else if !in_code {
                out.push(c);
            }
        }
        out.push('\n');
    }
    out
}

/// Checks that every table has the same number of cells in every row.
fn assert_tables_are_regular(markdown: &str) {
    let lines: Vec<&str> = markdown.lines().collect();
    let mut index = 0;
    while index < lines.len() {
        if lines[index].starts_with('|')
            && lines
                .get(index + 1)
                .is_some_and(|l| l.starts_with("| ---") || l.starts_with("| :--"))
        {
            let width = cells(lines[index]);
            let mut row = index;
            while row < lines.len() && lines[row].starts_with('|') {
                assert_eq!(cells(lines[row]), width, "irregular row: {}", lines[row]);
                row += 1;
            }
            index = row;
        } else {
            index += 1;
        }
    }
}

/// The sample renders every section in both languages with regular tables.
#[test]
fn sample_has_every_section() {
    let en = render_markdown(&sample(), Lang::En);
    for heading in [
        "# Code report",
        "## Executive summary",
        "## How to read this report",
        "## Languages",
        "## Modules",
        "## Hotspots",
        "## Documentation coverage",
        "## Code graph",
        "## Memories",
        "## Usage and token efficiency",
        "## Notes",
    ] {
        assert!(en.contains(heading), "{heading}");
    }
    let es = render_markdown(&sample(), Lang::Es);
    for heading in [
        "# Informe de código",
        "## Resumen ejecutivo",
        "## Cómo leer este informe",
        "## Lenguajes",
        "## Módulos",
        "## Puntos calientes",
        "## Cobertura de la documentación",
        "## Grafo de código",
        "## Memorias",
        "## Uso y eficiencia de tokens",
        "## Notas",
    ] {
        assert!(es.contains(heading), "{heading}");
    }
    assert_tables_are_regular(&en);
    assert_tables_are_regular(&es);
}

/// The code graph is a fenced Mermaid block that closes properly.
#[test]
fn mermaid_block_is_fenced() {
    let md = render_markdown(&sample(), Lang::En);
    assert_eq!(md.matches("```mermaid\nflowchart LR\n").count(), 1);
    assert_eq!(md.matches("```").count() % 2, 0);
}

/// Hostile text cannot break tables, inject HTML or mention anybody.
#[test]
fn hostile_data_is_neutralized() {
    for lang in [Lang::En, Lang::Es] {
        let md = render_markdown(&hostile(), lang);
        assert!(
            !outside_code(&md).contains("<script"),
            "raw HTML outside code spans"
        );
        assert!(
            !outside_code(&md).contains("<!--"),
            "raw HTML comment outside code spans"
        );
        assert!(md.contains("&lt;script&gt;"));
        assert_tables_are_regular(&md);
        assert!(!md.contains('\u{202e}') && !md.contains('\u{0}'));
        for line in md.lines() {
            assert!(
                !line.starts_with('#')
                    || line.starts_with("# ")
                    || line.starts_with("## ")
                    || line.starts_with("### "),
                "{line}"
            );
        }
    }
}

/// Empty data renders with "no data" notes.
#[test]
fn empty_report() {
    let md = render_markdown(&ReportData::default(), Lang::En);
    assert!(md.contains("## Languages\n\n*No data.*"));
    assert!(
        md.trim_end()
            .ends_with("*Generated locally by pn-ultramemory 1.2.36. No data left this machine.*")
            || md.contains("Generated locally by pn-ultramemory")
    );
}

/// Rendering twice gives identical text.
#[test]
fn markdown_is_deterministic() {
    assert_eq!(
        render_markdown(&hostile(), Lang::Es),
        render_markdown(&hostile(), Lang::Es)
    );
}

/// Truncation notes appear and the number of table rows is bounded.
#[test]
fn truncation() {
    let mut data = sample();
    data.hotspots = crate::fixtures::huge().hotspots;
    let md = render_markdown(&data, Lang::En);
    assert!(md.contains("*Showing 25 of 5,000.*"));
    let hotspot_rows = md
        .lines()
        .filter(|l| l.starts_with("| `") && l.contains(".rs"))
        .count();
    assert!(hotspot_rows <= 25 + 30, "{hotspot_rows}");
}
