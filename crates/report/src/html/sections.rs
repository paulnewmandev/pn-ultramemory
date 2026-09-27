// SPDX-License-Identifier: Apache-2.0
//! The sections of the HTML report, one function each.
//!
//! Every function appends a complete `<section>` to the page. Sections that have nothing to
//! show say so instead of disappearing, except usage and notes, which are optional by design.
//!
//! Invariants: every string that comes from the data is escaped by [`push_escaped`], [`escaped`]
//! or a [`Cell`]; headings follow the order h1 (title), h2 (section), h3 (subsection).

use std::fmt::Write as _;

use super::table::{Cell, Column, language_chart, progress_bar, table};
use crate::content::{
    MAX_DOC_LANGUAGES, MAX_HOTSPOTS, MAX_LANGUAGES, MAX_MEMORIES, MAX_MODULES, MAX_NOTES,
    MAX_UNDOCUMENTED, REPORT_GRAPH_EDGES, REPORT_GRAPH_NODES, Window, doc_percent,
    language_file_total, location, share, sorted_hotspots, sorted_languages, sorted_modules,
};
use crate::graph::layout::layout;
use crate::graph::prepare::{PEdge, prepare};
use crate::graph::svg::{embedded, group_display};
use crate::i18n::{Key, t, tf};
use crate::lang::Lang;
use crate::model::ReportData;
use crate::numfmt::{self, int, percent, percent_int};
use crate::text::{CLIP_NAME, CLIP_TEXT, escaped, push_escaped, single_line};

/// Opens a section with its heading.
fn open(out: &mut String, id: &str, title: &str) {
    let _ = write!(
        out,
        "<section id=\"{id}\" aria-labelledby=\"h-{id}\"><h2 id=\"h-{id}\">"
    );
    push_escaped(out, title);
    out.push_str("</h2>");
}

/// Closes a section.
fn close(out: &mut String) {
    out.push_str("</section>");
}

/// Writes a paragraph of trusted static text or already escaped text.
fn paragraph(out: &mut String, class: &str, text: &str) {
    let _ = write!(out, "<p class=\"{class}\">");
    push_escaped(out, text);
    out.push_str("</p>");
}

/// Writes the "showing N of M" note for a truncated table.
fn cut_note<T>(out: &mut String, window: &Window<'_, T>, lang: Lang) {
    if let Some(note) = window.note(lang) {
        paragraph(out, "note", &note);
    }
}

/// Writes the "no data" paragraph.
fn no_data(out: &mut String, lang: Lang) {
    paragraph(out, "note", t(lang, Key::NoData));
}

/// Writes one metric card; `tone` is `""`, `"warn"` or `"bad"`.
fn card(out: &mut String, label: &str, value: &str, tone: &str) {
    let class = if tone.is_empty() {
        "card".to_owned()
    } else {
        format!("card {tone}")
    };
    let _ = write!(out, "<div class=\"{class}\"><dt>");
    push_escaped(out, label);
    out.push_str("</dt><dd>");
    push_escaped(out, value);
    out.push_str("</dd></div>");
}

/// The executive summary: metric cards.
pub(super) fn summary(out: &mut String, data: &ReportData, lang: Lang) {
    let s = &data.summary;
    open(out, "summary", t(lang, Key::SecSummary));
    out.push_str("<dl class=\"cards\">");
    card(out, t(lang, Key::MetricFiles), &int(s.files, lang), "");
    card(out, t(lang, Key::MetricLines), &int(s.lines, lang), "");
    card(out, t(lang, Key::MetricSymbols), &int(s.symbols, lang), "");
    card(
        out,
        t(lang, Key::MetricRelationships),
        &int(s.edges, lang),
        "",
    );
    card(
        out,
        t(lang, Key::MetricMemories),
        &int(s.memories, lang),
        "",
    );
    card(
        out,
        t(lang, Key::MetricStale),
        &int(s.stale_memories, lang),
        if s.stale_memories > 0 { "warn" } else { "" },
    );
    card(
        out,
        t(lang, Key::MetricParseErrors),
        &int(s.parse_error_files, lang),
        if s.parse_error_files > 0 { "bad" } else { "" },
    );
    out.push_str("</dl>");
    close(out);
}

/// The reading guide.
pub(super) fn how_to_read(out: &mut String, lang: Lang) {
    open(out, "howto", t(lang, Key::SecHowTo));
    paragraph(out, "lead", t(lang, Key::HowToRead));
    close(out);
}

/// The languages table and chart.
pub(super) fn languages(out: &mut String, data: &ReportData, lang: Lang) {
    open(out, "languages", t(lang, Key::SecLanguages));
    let sorted = sorted_languages(&data.languages);
    if sorted.is_empty() {
        no_data(out, lang);
        close(out);
        return;
    }
    out.push_str("<figure>");
    out.push_str(&language_chart(&sorted, lang));
    out.push_str("</figure>");
    let total = language_file_total(&sorted);
    let window = Window::new(&sorted, MAX_LANGUAGES);
    let columns = [
        Column {
            title: t(lang, Key::ColLanguage),
            numeric: false,
        },
        Column {
            title: t(lang, Key::ColFiles),
            numeric: true,
        },
        Column {
            title: t(lang, Key::ColSymbols),
            numeric: true,
        },
        Column {
            title: t(lang, Key::ColShare),
            numeric: true,
        },
    ];
    let rows: Vec<Vec<Cell>> = window
        .rows
        .iter()
        .map(|row| {
            vec![
                Cell::Text(single_line(&row.name, CLIP_NAME)),
                Cell::Num(int(row.files, lang)),
                Cell::Num(int(row.symbols, lang)),
                Cell::Num(percent(share(row.files, total), lang)),
            ]
        })
        .collect();
    table(out, t(lang, Key::SecLanguages), &columns, &rows);
    cut_note(out, &window, lang);
    close(out);
}

/// The modules table.
pub(super) fn modules(out: &mut String, data: &ReportData, lang: Lang) {
    open(out, "modules", t(lang, Key::SecModules));
    let sorted = sorted_modules(&data.modules);
    if sorted.is_empty() {
        no_data(out, lang);
        close(out);
        return;
    }
    paragraph(out, "lead", t(lang, Key::ModulesIntro));
    let window = Window::new(&sorted, MAX_MODULES);
    let columns = [
        Column {
            title: t(lang, Key::ColModule),
            numeric: false,
        },
        Column {
            title: t(lang, Key::ColFiles),
            numeric: true,
        },
        Column {
            title: t(lang, Key::ColSymbols),
            numeric: true,
        },
        Column {
            title: t(lang, Key::ColIncoming),
            numeric: true,
        },
        Column {
            title: t(lang, Key::ColOutgoing),
            numeric: true,
        },
    ];
    let rows: Vec<Vec<Cell>> = window
        .rows
        .iter()
        .map(|row| {
            vec![
                Cell::Mono(single_line(&row.name, CLIP_NAME)),
                Cell::Num(int(row.files, lang)),
                Cell::Num(int(row.symbols, lang)),
                Cell::Num(int(row.incoming, lang)),
                Cell::Num(int(row.outgoing, lang)),
            ]
        })
        .collect();
    table(out, t(lang, Key::SecModules), &columns, &rows);
    cut_note(out, &window, lang);
    close(out);
}

/// The hotspots table.
pub(super) fn hotspots(out: &mut String, data: &ReportData, lang: Lang) {
    open(out, "hotspots", t(lang, Key::SecHotspots));
    let sorted = sorted_hotspots(&data.hotspots);
    if sorted.is_empty() {
        no_data(out, lang);
        close(out);
        return;
    }
    paragraph(out, "lead", t(lang, Key::HotspotsIntro));
    let window = Window::new(&sorted, MAX_HOTSPOTS);
    let columns = [
        Column {
            title: t(lang, Key::ColSymbol),
            numeric: false,
        },
        Column {
            title: t(lang, Key::ColKind),
            numeric: false,
        },
        Column {
            title: t(lang, Key::ColLocation),
            numeric: false,
        },
        Column {
            title: t(lang, Key::ColCallers),
            numeric: true,
        },
        Column {
            title: t(lang, Key::ColCallees),
            numeric: true,
        },
    ];
    let rows: Vec<Vec<Cell>> = window
        .rows
        .iter()
        .map(|row| {
            vec![
                Cell::Mono(single_line(&row.name, CLIP_NAME)),
                Cell::Text(single_line(&row.kind, 40)),
                Cell::Mono(location(&row.path, row.line)),
                Cell::Num(int(u64::from(row.callers), lang)),
                Cell::Num(int(u64::from(row.callees), lang)),
            ]
        })
        .collect();
    table(out, t(lang, Key::SecHotspots), &columns, &rows);
    cut_note(out, &window, lang);
    close(out);
}

/// The documentation coverage section.
pub(super) fn documentation(out: &mut String, data: &ReportData, lang: Lang) {
    let docs = &data.documentation;
    open(out, "documentation", t(lang, Key::SecDocs));
    if docs.public_symbols == 0 && docs.by_language.is_empty() {
        no_data(out, lang);
        close(out);
        return;
    }
    let overall = doc_percent(docs);
    let _ = write!(
        out,
        "<p class=\"note\">{}</p><p class=\"big\">",
        escaped(t(lang, Key::DocOverall))
    );
    push_escaped(out, &percent(overall, lang));
    out.push_str("</p>");
    out.push_str(&progress_bar(overall, t(lang, Key::DocOverall), false));
    let documented = int(docs.documented.min(docs.public_symbols), lang);
    let public = int(docs.public_symbols, lang);
    paragraph(
        out,
        "",
        &tf(lang, Key::DocSentence, &[&documented, &public]),
    );

    if !docs.by_language.is_empty() {
        let _ = write!(out, "<h3>{}</h3>", escaped(t(lang, Key::DocByLanguage)));
        let window = Window::new(&docs.by_language, MAX_DOC_LANGUAGES);
        let columns = [
            Column {
                title: t(lang, Key::ColLanguage),
                numeric: false,
            },
            Column {
                title: t(lang, Key::ColPublic),
                numeric: true,
            },
            Column {
                title: t(lang, Key::ColDocumented),
                numeric: true,
            },
            Column {
                title: t(lang, Key::ColCoverage),
                numeric: false,
            },
        ];
        let rows: Vec<Vec<Cell>> = window
            .rows
            .iter()
            .map(|row| {
                let value = numfmt::ratio_percent(row.documented, row.public_symbols);
                let name = single_line(&row.name, CLIP_NAME);
                let cell = format!(
                    "<div class=\"cov\">{}<span class=\"pct\">{}</span></div>",
                    progress_bar(value, &format!("{}: {}", name, percent(value, lang)), true),
                    escaped(&percent(value, lang))
                );
                vec![
                    Cell::Text(name),
                    Cell::Num(int(row.public_symbols, lang)),
                    Cell::Num(int(row.documented.min(row.public_symbols), lang)),
                    Cell::Raw(cell),
                ]
            })
            .collect();
        table(out, t(lang, Key::DocByLanguage), &columns, &rows);
        cut_note(out, &window, lang);
    }

    if !docs.undocumented_examples.is_empty() {
        let _ = write!(out, "<h3>{}</h3>", escaped(t(lang, Key::DocUndocumented)));
        let window = Window::new(&docs.undocumented_examples, MAX_UNDOCUMENTED);
        let columns = [
            Column {
                title: t(lang, Key::ColSymbol),
                numeric: false,
            },
            Column {
                title: t(lang, Key::ColLocation),
                numeric: false,
            },
        ];
        let rows: Vec<Vec<Cell>> = window
            .rows
            .iter()
            .map(|row| {
                vec![
                    Cell::Mono(single_line(&row.name, CLIP_NAME)),
                    Cell::Mono(location(&row.path, row.line)),
                ]
            })
            .collect();
        table(out, t(lang, Key::DocUndocumented), &columns, &rows);
        cut_note(out, &window, lang);
    }
    close(out);
}

/// The code graph section: figure, truncation note and legend.
pub(super) fn code_graph(out: &mut String, data: &ReportData, lang: Lang) {
    open(out, "graph", t(lang, Key::SecGraph));
    let prep = prepare(&data.graph, REPORT_GRAPH_NODES, REPORT_GRAPH_EDGES);
    if prep.nodes.is_empty() {
        no_data(out, lang);
        close(out);
        return;
    }
    paragraph(out, "lead", t(lang, Key::GraphIntro));
    let placed = layout(&prep);
    let count = |n: usize| int(u64::try_from(n).unwrap_or(u64::MAX), lang);
    out.push_str("<figure><div class=\"scroll\" role=\"region\" tabindex=\"0\" aria-label=\"");
    push_escaped(out, t(lang, Key::SecGraph));
    out.push_str("\">");
    out.push_str(&embedded(&prep, &placed, lang));
    out.push_str("</div>");
    if prep.hidden_nodes() > 0 || prep.edges.len() < prep.total_edges {
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
        let _ = write!(out, "<figcaption>{}</figcaption>", escaped(&note));
    }
    out.push_str("</figure>");
    let _ = write!(
        out,
        "<h3>{}</h3><ul class=\"legend\">",
        escaped(t(lang, Key::GraphLegend))
    );
    for (index, group) in prep.groups.iter().enumerate().take(20) {
        let _ = write!(
            out,
            "<li><span class=\"sw g{}\" aria-hidden=\"true\"></span>",
            index % 10
        );
        push_escaped(out, &group_display(group, lang));
        out.push_str("</li>");
    }
    out.push_str("</ul>");
    if prep.edges.iter().any(PEdge::is_uncertain) {
        paragraph(out, "note", t(lang, Key::GraphDashed));
    }
    close(out);
}

/// The memories table.
pub(super) fn memories(out: &mut String, data: &ReportData, lang: Lang) {
    open(out, "memories", t(lang, Key::SecMemories));
    if data.memories.is_empty() {
        no_data(out, lang);
        close(out);
        return;
    }
    let window = Window::new(&data.memories, MAX_MEMORIES);
    let columns = [
        Column {
            title: t(lang, Key::ColId),
            numeric: true,
        },
        Column {
            title: t(lang, Key::ColKind),
            numeric: false,
        },
        Column {
            title: t(lang, Key::ColProvenance),
            numeric: false,
        },
        Column {
            title: t(lang, Key::ColStatus),
            numeric: false,
        },
        Column {
            title: t(lang, Key::ColText),
            numeric: false,
        },
    ];
    let rows: Vec<Vec<Cell>> = window
        .rows
        .iter()
        .map(|row| {
            let (class, key) = if row.stale {
                ("stale", Key::StatusStale)
            } else {
                ("ok", Key::StatusCurrent)
            };
            vec![
                Cell::Num(row.id.to_string()),
                Cell::Text(single_line(&row.kind, 40)),
                Cell::Text(single_line(&row.provenance, 40)),
                Cell::Raw(format!(
                    "<span class=\"badge {class}\">{}</span>",
                    escaped(t(lang, key))
                )),
                Cell::Text(single_line(&row.text, CLIP_TEXT)),
            ]
        })
        .collect();
    table(out, t(lang, Key::SecMemories), &columns, &rows);
    cut_note(out, &window, lang);
    close(out);
}

/// The usage and token efficiency section, written only when usage data exists.
pub(super) fn usage(out: &mut String, data: &ReportData, lang: Lang) {
    let Some(usage) = &data.usage else { return };
    open(out, "usage", t(lang, Key::SecUsage));
    paragraph(out, "lead", t(lang, Key::UsageIntro));
    out.push_str("<dl class=\"cards\">");
    card(
        out,
        t(lang, Key::UsageRecalls),
        &int(usage.recalls, lang),
        "",
    );
    card(
        out,
        t(lang, Key::UsageExpands),
        &int(usage.expands, lang),
        "",
    );
    card(
        out,
        t(lang, Key::UsageImpacts),
        &int(usage.impacts, lang),
        "",
    );
    card(
        out,
        t(lang, Key::UsageRemembers),
        &int(usage.remembers, lang),
        "",
    );
    card(
        out,
        t(lang, Key::UsageAvgTokens),
        &int(usage.avg_tokens_served, lang),
        "",
    );
    card(
        out,
        t(lang, Key::UsageAvgBudget),
        &percent_int(usage.avg_budget_percent, lang),
        "",
    );
    card(
        out,
        t(lang, Key::UsageTotalTokens),
        &int(usage.tokens_served_total, lang),
        "",
    );
    out.push_str("</dl>");
    let share = f64::from(usage.avg_budget_percent);
    out.push_str(&progress_bar(share, t(lang, Key::UsageAvgBudget), false));
    close(out);
}

/// The notes list, written only when there are notes.
pub(super) fn notes(out: &mut String, data: &ReportData, lang: Lang) {
    if data.notes.is_empty() {
        return;
    }
    open(out, "notes", t(lang, Key::SecNotes));
    let window = Window::new(&data.notes, MAX_NOTES);
    out.push_str("<ul class=\"notes\">");
    for note in window.rows {
        out.push_str("<li><bdi>");
        push_escaped(out, &single_line(note, CLIP_TEXT));
        out.push_str("</bdi></li>");
    }
    out.push_str("</ul>");
    cut_note(out, &window, lang);
    close(out);
}

#[cfg(test)]
mod tests {
    use super::{code_graph, documentation, hotspots, languages, memories, notes, summary, usage};
    use crate::lang::Lang;
    use crate::model::{
        DocCoverage, DocLanguageRow, Graph, GraphEdge, GraphNode, Hotspot, LanguageRow, MemoryRow,
        ReportData, Summary, Usage,
    };

    /// Runs one section writer and returns its HTML.
    fn render(write: fn(&mut String, &ReportData, Lang), data: &ReportData, lang: Lang) -> String {
        let mut out = String::new();
        write(&mut out, data, lang);
        out
    }

    /// Cards flag stale memories and parse errors with a tone.
    #[test]
    fn summary_cards_have_tones() {
        let plain = render(summary, &ReportData::default(), Lang::En);
        assert!(!plain.contains("card warn") && !plain.contains("card bad"));
        let data = ReportData {
            summary: Summary {
                stale_memories: 1,
                parse_error_files: 2,
                ..Summary::default()
            },
            ..ReportData::default()
        };
        let flagged = render(summary, &data, Lang::En);
        assert!(flagged.contains("card warn") && flagged.contains("card bad"));
    }

    /// The languages section is sorted by files and shows shares that add up.
    #[test]
    fn languages_are_sorted_with_shares() {
        let row = |name: &str, files| LanguageRow {
            name: name.into(),
            files,
            symbols: 1,
        };
        let data = ReportData {
            languages: vec![row("B", 1), row("A", 3)],
            ..ReportData::default()
        };
        let html = render(languages, &data, Lang::En);
        assert!(html.find("<bdi>A</bdi>") < html.find("<bdi>B</bdi>"));
        assert!(html.contains("75%") && html.contains("25%"));
    }

    /// Hotspots are cut to 25 rows with the note, and empty data says so.
    #[test]
    fn hotspots_are_cut() {
        let rows = (0..30)
            .map(|i| Hotspot {
                name: format!("f{i}"),
                callers: i,
                ..Hotspot::default()
            })
            .collect();
        let html = render(
            hotspots,
            &ReportData {
                hotspots: rows,
                ..ReportData::default()
            },
            Lang::En,
        );
        assert_eq!(html.matches("<tr>").count(), 26);
        assert!(html.contains("Showing 25 of 30."));
        assert!(render(hotspots, &ReportData::default(), Lang::Es).contains("Sin datos."));
    }

    /// Documentation coverage clamps impossible numbers and lists per-language bars.
    #[test]
    fn documentation_clamps() {
        let data = ReportData {
            documentation: DocCoverage {
                public_symbols: 10,
                documented: 50,
                by_language: vec![DocLanguageRow {
                    name: "Rust".into(),
                    public_symbols: 0,
                    documented: 5,
                }],
                undocumented_examples: vec![],
            },
            ..ReportData::default()
        };
        let html = render(documentation, &data, Lang::En);
        assert!(html.contains("100%"));
        assert!(html.contains("10 of 10 public symbols"));
        assert!(html.contains("role=\"progressbar\""));
    }

    /// The graph section has a legend and notes truncation and dashed edges.
    #[test]
    fn graph_section_has_legend_and_notes() {
        let nodes = (0..50)
            .map(|i| GraphNode {
                id: i.to_string(),
                label: i.to_string(),
                group: (i % 3).to_string(),
                weight: i,
            })
            .collect();
        let edges = vec![GraphEdge {
            from: 49,
            to: 48,
            weight: 1,
            confidence: "guess".into(),
            ..GraphEdge::default()
        }];
        let data = ReportData {
            graph: Graph { nodes, edges },
            ..ReportData::default()
        };
        let html = render(code_graph, &data, Lang::En);
        assert_eq!(html.matches("class=\"sw g").count(), 3);
        assert!(html.contains("Showing the 40 heaviest nodes of 50 and 1 of 1 relationships."));
        assert!(html.contains("Dashed lines mark"));
    }

    /// Memories mark stale entries and are cut to 25.
    #[test]
    fn memories_mark_stale_ones() {
        let rows = (0..26)
            .map(|i| MemoryRow {
                id: i,
                stale: i == 0,
                text: format!("m{i}"),
                ..MemoryRow::default()
            })
            .collect();
        let html = render(
            memories,
            &ReportData {
                memories: rows,
                ..ReportData::default()
            },
            Lang::Es,
        );
        assert_eq!(html.matches("badge stale").count(), 1);
        assert!(html.contains("Obsoleta") && html.contains("Vigente"));
        assert!(html.contains("Mostrando 25 de 26."));
    }

    /// Usage and notes appear only with data.
    #[test]
    fn optional_sections_need_data() {
        assert_eq!(render(usage, &ReportData::default(), Lang::En), "");
        assert_eq!(render(notes, &ReportData::default(), Lang::En), "");
        let data = ReportData {
            usage: Some(Usage {
                avg_budget_percent: 250,
                ..Usage::default()
            }),
            notes: (0..120).map(|i| format!("n{i}")).collect(),
            ..ReportData::default()
        };
        assert!(render(usage, &data, Lang::En).contains("aria-valuenow=\"100\""));
        assert!(render(notes, &data, Lang::En).contains("Showing 100 of 120."));
    }
}
