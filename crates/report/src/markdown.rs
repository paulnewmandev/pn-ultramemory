// SPDX-License-Identifier: Apache-2.0
//! The report as a Markdown document that renders well on any forge.
//!
//! The document uses GitHub-flavored tables, text bars made of block characters instead of
//! images, and a fenced `mermaid` block for the code graph (GitHub, GitLab and most editors draw
//! it). Names and paths go inside code spans; free text is backslash-escaped, raw HTML is never
//! produced, and `@` is escaped so nobody is mentioned by accident.
//!
//! Invariants: no line of a table can be split by user text (newlines and pipes are neutralized);
//! the output is deterministic; every section of the HTML report has a counterpart here.

use std::fmt::Write as _;

use crate::content::{
    MAX_DOC_LANGUAGES, MAX_HOTSPOTS, MAX_LANGUAGES, MAX_MEMORIES, MAX_MODULES, MAX_NOTES,
    MAX_UNDOCUMENTED, REPORT_GRAPH_EDGES, REPORT_GRAPH_NODES, Window, doc_percent,
    language_file_total, location, project_name, share, sorted_hotspots, sorted_languages,
    sorted_modules, tool_version,
};
use crate::graph::mermaid;
use crate::graph::prepare::{PEdge, prepare};
use crate::graph::svg::group_display;
use crate::i18n::{Key, t, tf};
use crate::lang::Lang;
use crate::model::ReportData;
use crate::numfmt::{self, int, percent, percent_int, usize_to_f64};
use crate::text::{CLIP_NAME, CLIP_TEXT, md_code, md_escape, single_line};

/// Width, in characters, of the text bars.
const BAR_WIDTH: usize = 20;
/// Rows of the "heaviest nodes" table.
const TOP_NODES: usize = 10;

/// Returns a text bar of block characters for `percent`, such as `████░░░░`.
fn text_bar(percent: f64) -> String {
    let filled = (0..BAR_WIDTH)
        .filter(|&i| (usize_to_f64(i) + 0.5) * 100.0 / usize_to_f64(BAR_WIDTH) <= percent)
        .count();
    format!(
        "{}{}",
        "\u{2588}".repeat(filled),
        "\u{2591}".repeat(BAR_WIDTH - filled)
    )
}

/// Appends a Markdown table. `right` marks the columns that are right-aligned.
fn table(out: &mut String, headers: &[(&str, bool)], rows: &[Vec<String>]) {
    out.push('|');
    for (title, _) in headers {
        let _ = write!(out, " {} |", md_escape(title));
    }
    out.push_str("\n|");
    for (_, right) in headers {
        out.push_str(if *right { " ---: |" } else { " --- |" });
    }
    out.push('\n');
    for row in rows {
        out.push('|');
        for cell in row {
            let _ = write!(out, " {cell} |");
        }
        out.push('\n');
    }
    out.push('\n');
}

/// Appends a level-2 heading.
fn heading(out: &mut String, title: &str) {
    let _ = write!(out, "## {}\n\n", md_escape(title));
}

/// Appends a level-3 heading.
fn subheading(out: &mut String, title: &str) {
    let _ = write!(out, "### {}\n\n", md_escape(title));
}

/// Appends an italic note about a truncated table.
fn cut_note<T>(out: &mut String, window: &Window<'_, T>, lang: Lang) {
    if let Some(note) = window.note(lang) {
        let _ = write!(out, "*{}*\n\n", md_escape(&note));
    }
}

/// Appends the "no data" paragraph.
fn no_data(out: &mut String, lang: Lang) {
    let _ = write!(out, "*{}*\n\n", md_escape(t(lang, Key::NoData)));
}

/// Renders the report as Markdown.
pub(crate) fn render(data: &ReportData, lang: Lang) -> String {
    let project = project_name(data, lang);
    let version = tool_version(data);
    let mut out = String::with_capacity(8192);
    let _ = write!(out, "# {}\n\n", md_escape(t(lang, Key::DocTitle)));
    let _ = write!(
        out,
        "> **{}:** {}",
        md_escape(t(lang, Key::Project)),
        md_escape(&project)
    );
    let generated = single_line(&data.generated_on, 40);
    if !generated.is_empty() {
        let _ = write!(
            out,
            " \u{b7} **{}:** {}",
            md_escape(t(lang, Key::GeneratedOn)),
            md_escape(&generated)
        );
    }
    let _ = write!(
        out,
        " \u{b7} **{}:** pn-ultramemory {}\n\n",
        md_escape(t(lang, Key::ToolVersion)),
        md_escape(&version)
    );

    summary(&mut out, data, lang);
    heading(&mut out, t(lang, Key::SecHowTo));
    let _ = write!(out, "{}\n\n", md_escape(t(lang, Key::HowToRead)));
    languages(&mut out, data, lang);
    modules(&mut out, data, lang);
    hotspots(&mut out, data, lang);
    documentation(&mut out, data, lang);
    code_graph(&mut out, data, lang);
    memories(&mut out, data, lang);
    usage(&mut out, data, lang);
    notes(&mut out, data, lang);
    let _ = write!(
        out,
        "---\n\n*{}*\n",
        md_escape(&tf(lang, Key::Footer, &[&version]))
    );
    out
}

/// The executive summary table.
fn summary(out: &mut String, data: &ReportData, lang: Lang) {
    let s = &data.summary;
    heading(out, t(lang, Key::SecSummary));
    let rows = vec![
        (Key::MetricFiles, s.files),
        (Key::MetricLines, s.lines),
        (Key::MetricSymbols, s.symbols),
        (Key::MetricRelationships, s.edges),
        (Key::MetricMemories, s.memories),
        (Key::MetricStale, s.stale_memories),
        (Key::MetricParseErrors, s.parse_error_files),
    ]
    .into_iter()
    .map(|(key, value)| vec![md_escape(t(lang, key)), int(value, lang)])
    .collect::<Vec<_>>();
    table(
        out,
        &[
            (t(lang, Key::ColMetric), false),
            (t(lang, Key::ColValue), true),
        ],
        &rows,
    );
}

/// The languages table with text bars.
fn languages(out: &mut String, data: &ReportData, lang: Lang) {
    heading(out, t(lang, Key::SecLanguages));
    let sorted = sorted_languages(&data.languages);
    if sorted.is_empty() {
        no_data(out, lang);
        return;
    }
    let total = language_file_total(&sorted);
    let window = Window::new(&sorted, MAX_LANGUAGES);
    let rows: Vec<Vec<String>> = window
        .rows
        .iter()
        .map(|row| {
            let value = share(row.files, total);
            vec![
                md_escape(&single_line(&row.name, CLIP_NAME)),
                int(row.files, lang),
                int(row.symbols, lang),
                format!("`{}` {}", text_bar(value), md_escape(&percent(value, lang))),
            ]
        })
        .collect();
    table(
        out,
        &[
            (t(lang, Key::ColLanguage), false),
            (t(lang, Key::ColFiles), true),
            (t(lang, Key::ColSymbols), true),
            (t(lang, Key::ColShare), false),
        ],
        &rows,
    );
    cut_note(out, &window, lang);
}

/// The modules table.
fn modules(out: &mut String, data: &ReportData, lang: Lang) {
    heading(out, t(lang, Key::SecModules));
    let sorted = sorted_modules(&data.modules);
    if sorted.is_empty() {
        no_data(out, lang);
        return;
    }
    let _ = write!(out, "{}\n\n", md_escape(t(lang, Key::ModulesIntro)));
    let window = Window::new(&sorted, MAX_MODULES);
    let rows: Vec<Vec<String>> = window
        .rows
        .iter()
        .map(|row| {
            vec![
                md_code(&single_line(&row.name, CLIP_NAME)),
                int(row.files, lang),
                int(row.symbols, lang),
                int(row.incoming, lang),
                int(row.outgoing, lang),
            ]
        })
        .collect();
    table(
        out,
        &[
            (t(lang, Key::ColModule), false),
            (t(lang, Key::ColFiles), true),
            (t(lang, Key::ColSymbols), true),
            (t(lang, Key::ColIncoming), true),
            (t(lang, Key::ColOutgoing), true),
        ],
        &rows,
    );
    cut_note(out, &window, lang);
}

/// The hotspots table.
fn hotspots(out: &mut String, data: &ReportData, lang: Lang) {
    heading(out, t(lang, Key::SecHotspots));
    let sorted = sorted_hotspots(&data.hotspots);
    if sorted.is_empty() {
        no_data(out, lang);
        return;
    }
    let _ = write!(out, "{}\n\n", md_escape(t(lang, Key::HotspotsIntro)));
    let window = Window::new(&sorted, MAX_HOTSPOTS);
    let rows: Vec<Vec<String>> = window
        .rows
        .iter()
        .map(|row| {
            vec![
                md_code(&single_line(&row.name, CLIP_NAME)),
                md_escape(&single_line(&row.kind, 40)),
                md_code(&location(&row.path, row.line)),
                int(u64::from(row.callers), lang),
                int(u64::from(row.callees), lang),
            ]
        })
        .collect();
    table(
        out,
        &[
            (t(lang, Key::ColSymbol), false),
            (t(lang, Key::ColKind), false),
            (t(lang, Key::ColLocation), false),
            (t(lang, Key::ColCallers), true),
            (t(lang, Key::ColCallees), true),
        ],
        &rows,
    );
    cut_note(out, &window, lang);
}

/// The documentation coverage section.
fn documentation(out: &mut String, data: &ReportData, lang: Lang) {
    let docs = &data.documentation;
    heading(out, t(lang, Key::SecDocs));
    if docs.public_symbols == 0 && docs.by_language.is_empty() {
        no_data(out, lang);
        return;
    }
    let overall = doc_percent(docs);
    let documented = int(docs.documented.min(docs.public_symbols), lang);
    let public = int(docs.public_symbols, lang);
    let _ = write!(
        out,
        "**{}: {}** `{}`\n\n{}\n\n",
        md_escape(t(lang, Key::DocOverall)),
        md_escape(&percent(overall, lang)),
        text_bar(overall),
        md_escape(&tf(lang, Key::DocSentence, &[&documented, &public]))
    );
    if !docs.by_language.is_empty() {
        subheading(out, t(lang, Key::DocByLanguage));
        let window = Window::new(&docs.by_language, MAX_DOC_LANGUAGES);
        let rows: Vec<Vec<String>> = window
            .rows
            .iter()
            .map(|row| {
                let value = numfmt::ratio_percent(row.documented, row.public_symbols);
                vec![
                    md_escape(&single_line(&row.name, CLIP_NAME)),
                    int(row.public_symbols, lang),
                    int(row.documented.min(row.public_symbols), lang),
                    format!("`{}` {}", text_bar(value), md_escape(&percent(value, lang))),
                ]
            })
            .collect();
        table(
            out,
            &[
                (t(lang, Key::ColLanguage), false),
                (t(lang, Key::ColPublic), true),
                (t(lang, Key::ColDocumented), true),
                (t(lang, Key::ColCoverage), false),
            ],
            &rows,
        );
        cut_note(out, &window, lang);
    }
    if !docs.undocumented_examples.is_empty() {
        subheading(out, t(lang, Key::DocUndocumented));
        let window = Window::new(&docs.undocumented_examples, MAX_UNDOCUMENTED);
        let rows: Vec<Vec<String>> = window
            .rows
            .iter()
            .map(|row| {
                vec![
                    md_code(&single_line(&row.name, CLIP_NAME)),
                    md_code(&location(&row.path, row.line)),
                ]
            })
            .collect();
        table(
            out,
            &[
                (t(lang, Key::ColSymbol), false),
                (t(lang, Key::ColLocation), false),
            ],
            &rows,
        );
        cut_note(out, &window, lang);
    }
}

/// The code graph section: a Mermaid diagram and the heaviest nodes.
fn code_graph(out: &mut String, data: &ReportData, lang: Lang) {
    heading(out, t(lang, Key::SecGraph));
    let prep = prepare(&data.graph, REPORT_GRAPH_NODES, REPORT_GRAPH_EDGES);
    if prep.nodes.is_empty() {
        no_data(out, lang);
        return;
    }
    let _ = write!(out, "{}\n\n", md_escape(t(lang, Key::GraphIntro)));
    out.push_str("```mermaid\n");
    out.push_str(&mermaid::render(&prep, lang));
    out.push_str("```\n\n");
    if prep.hidden_nodes() > 0 || prep.edges.len() < prep.total_edges {
        let count = |n: usize| int(u64::try_from(n).unwrap_or(u64::MAX), lang);
        let note = tf(
            lang,
            Key::GraphShowing,
            &[
                &count(prep.nodes.len()),
                &count(prep.total_nodes),
                &count(prep.edges.len()),
                &count(prep.total_edges),
            ],
        );
        let _ = write!(out, "*{}*\n\n", md_escape(&note));
    }
    if prep.edges.iter().any(PEdge::is_uncertain) {
        let _ = write!(out, "*{}*\n\n", md_escape(t(lang, Key::GraphDashed)));
    }
    let mut heaviest: Vec<&crate::graph::prepare::PNode<'_>> = prep.nodes.iter().collect();
    heaviest.sort_by(|a, b| {
        b.raw
            .weight
            .cmp(&a.raw.weight)
            .then(a.source_index.cmp(&b.source_index))
    });
    let rows: Vec<Vec<String>> = heaviest
        .iter()
        .take(TOP_NODES)
        .map(|node| {
            vec![
                md_code(&node.label),
                md_escape(&group_display(&node.group, lang)),
                int(u64::from(node.raw.weight), lang),
            ]
        })
        .collect();
    table(
        out,
        &[
            (t(lang, Key::ColName), false),
            (t(lang, Key::ColGroup), false),
            (t(lang, Key::ColWeight), true),
        ],
        &rows,
    );
}

/// The memories table.
fn memories(out: &mut String, data: &ReportData, lang: Lang) {
    heading(out, t(lang, Key::SecMemories));
    if data.memories.is_empty() {
        no_data(out, lang);
        return;
    }
    let window = Window::new(&data.memories, MAX_MEMORIES);
    let rows: Vec<Vec<String>> = window
        .rows
        .iter()
        .map(|row| {
            let status = if row.stale {
                format!("**{}**", md_escape(t(lang, Key::StatusStale)))
            } else {
                md_escape(t(lang, Key::StatusCurrent))
            };
            vec![
                row.id.to_string(),
                md_escape(&single_line(&row.kind, 40)),
                md_escape(&single_line(&row.provenance, 40)),
                status,
                md_escape(&single_line(&row.text, CLIP_TEXT)),
            ]
        })
        .collect();
    table(
        out,
        &[
            (t(lang, Key::ColId), true),
            (t(lang, Key::ColKind), false),
            (t(lang, Key::ColProvenance), false),
            (t(lang, Key::ColStatus), false),
            (t(lang, Key::ColText), false),
        ],
        &rows,
    );
    cut_note(out, &window, lang);
}

/// The usage section, written only when usage data exists.
fn usage(out: &mut String, data: &ReportData, lang: Lang) {
    let Some(usage) = &data.usage else { return };
    heading(out, t(lang, Key::SecUsage));
    let _ = write!(out, "{}\n\n", md_escape(t(lang, Key::UsageIntro)));
    let count = |key: Key, value: u64| vec![md_escape(t(lang, key)), int(value, lang)];
    let rows = vec![
        count(Key::UsageRecalls, usage.recalls),
        count(Key::UsageExpands, usage.expands),
        count(Key::UsageImpacts, usage.impacts),
        count(Key::UsageRemembers, usage.remembers),
        count(Key::UsageAvgTokens, usage.avg_tokens_served),
        vec![
            md_escape(t(lang, Key::UsageAvgBudget)),
            percent_int(usage.avg_budget_percent, lang),
        ],
        count(Key::UsageTotalTokens, usage.tokens_served_total),
    ];
    table(
        out,
        &[
            (t(lang, Key::ColMetric), false),
            (t(lang, Key::ColValue), true),
        ],
        &rows,
    );
}

/// The notes list, written only when there are notes.
fn notes(out: &mut String, data: &ReportData, lang: Lang) {
    if data.notes.is_empty() {
        return;
    }
    heading(out, t(lang, Key::SecNotes));
    let window = Window::new(&data.notes, MAX_NOTES);
    for note in window.rows {
        let _ = writeln!(out, "- {}", md_escape(&single_line(note, CLIP_TEXT)));
    }
    out.push('\n');
    cut_note(out, &window, lang);
}

#[cfg(test)]
mod tests {
    use super::{render, text_bar};
    use crate::lang::Lang;
    use crate::model::{Hotspot, MemoryRow, ReportData, Summary, Usage};

    /// Text bars fill in proportion to the percentage and always have the same width.
    #[test]
    fn text_bars() {
        assert_eq!(text_bar(0.0), "\u{2591}".repeat(20));
        assert_eq!(text_bar(100.0), "\u{2588}".repeat(20));
        assert_eq!(
            text_bar(50.0),
            format!("{}{}", "\u{2588}".repeat(10), "\u{2591}".repeat(10))
        );
        assert_eq!(text_bar(250.0).chars().count(), 20);
        assert_eq!(text_bar(f64::NAN).chars().count(), 20);
    }

    /// The document has the title, the metadata line, the summary table and the footer.
    #[test]
    fn basic_document() {
        let data = ReportData {
            project: "demo".into(),
            generated_on: "2026-09-25".into(),
            tool_version: "1.2.36".into(),
            summary: Summary {
                files: 1234,
                stale_memories: 2,
                ..Summary::default()
            },
            usage: Some(Usage {
                recalls: 5,
                avg_budget_percent: 61,
                ..Usage::default()
            }),
            notes: vec!["# not a heading".into()],
            ..ReportData::default()
        };
        let en = render(&data, Lang::En);
        assert!(en.starts_with("# Code report\n\n> **Project:** demo \u{b7} **Generated on:** 2026-09-25 \u{b7} **Tool version:** pn-ultramemory 1.2.36\n"));
        assert!(en.contains("| Files | 1,234 |"));
        assert!(en.contains("| Stale memories | 2 |"));
        assert!(en.contains("## Usage and token efficiency"));
        assert!(en.contains("| Average share of the token budget used | 61% |"));
        assert!(en.contains("- \\# not a heading"));
        assert!(
            en.trim_end().ends_with(
                "*Generated locally by pn-ultramemory 1.2.36. No data left this machine.*"
            )
        );
        let es = render(&data, Lang::Es);
        assert!(es.starts_with("# Informe de código"));
        assert!(es.contains("| Archivos | 1.234 |"));
        assert!(es.contains("61\u{a0}%"));
        assert!(es.contains(
            "*Generado localmente por pn-ultramemory 1.2.36. Ningún dato salió de esta máquina.*"
        ));
    }

    /// Long tables are truncated with a note, and cells cannot break the table.
    #[test]
    fn truncation_and_hostile_cells() {
        let hotspots: Vec<Hotspot> = (0..40)
            .map(|i| Hotspot {
                name: format!("fn|{i}\nnew`line"),
                kind: "function".into(),
                path: "src/a|b.rs".into(),
                line: 7,
                callers: 100 - i,
                callees: 1,
            })
            .collect();
        let memories = vec![MemoryRow {
            id: 9,
            kind: "fact".into(),
            provenance: "user".into(),
            stale: true,
            text: "a | b\n<script>alert(1)</script> [x](http://e) @user".into(),
        }];
        let data = ReportData {
            hotspots,
            memories,
            ..ReportData::default()
        };
        let md = render(&data, Lang::En);
        assert!(md.contains("*Showing 25 of 40.*"));
        assert!(!md.contains("<script>"));
        assert!(md.contains("&lt;script&gt;"));
        assert!(md.contains("\\@user"));
        assert!(md.contains("**Stale**"));
        for line in md
            .lines()
            .filter(|line| line.starts_with('|') && line.contains("fn"))
        {
            // 5 columns: name, kind, location, callers, callees.
            assert_eq!(line.matches(" | ").count(), 4, "{line}");
        }
    }

    /// Empty data renders every section with a "no data" note and stays deterministic.
    #[test]
    fn empty_report() {
        let md = render(&ReportData::default(), Lang::Es);
        assert!(md.contains("## Lenguajes\n\n*Sin datos.*"));
        assert!(md.contains("(proyecto sin nombre)"));
        assert_eq!(md, render(&ReportData::default(), Lang::Es));
    }
}
