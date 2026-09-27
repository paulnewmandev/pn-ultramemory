// SPDX-License-Identifier: Apache-2.0
//! Lays the report out on the pages of the PDF, section by section.
//!
//! This module decides what goes on the pages and in which order; the mechanics (page breaks,
//! tables, bars, the graph figure) live in the sibling modules. Sections mirror the HTML report:
//! title block, executive summary, reading guide, languages, modules, hotspots, documentation,
//! graph, memories, usage and notes.
//!
//! Invariants: all user text is reduced to a single clipped line before layout; empty sections
//! print a "no data" note instead of disappearing (usage and notes are optional).

use super::doc::{Doc, Meta, Tone};
use super::figure::FRAME_HEIGHT;
use super::fonts::Font;
use super::table::{Cell, Col};
use crate::content::{
    MAX_DOC_LANGUAGES, MAX_HOTSPOTS, MAX_LANGUAGE_BARS, MAX_LANGUAGES, MAX_MEMORIES, MAX_MODULES,
    MAX_NOTES, MAX_UNDOCUMENTED, REPORT_GRAPH_EDGES, REPORT_GRAPH_NODES, Window, doc_percent,
    language_file_total, location, project_name, share, sorted_hotspots, sorted_languages,
    sorted_modules, tool_version,
};
use crate::graph::layout::layout;
use crate::graph::prepare::{PEdge, prepare};
use crate::graph::svg::group_display;
use crate::i18n::{Key, t, tf};
use crate::lang::Lang;
use crate::model::ReportData;
use crate::numfmt::{self, int, percent, percent_int, to_f64};
use crate::text::{CLIP_NAME, CLIP_TEXT, single_line};

/// Lays out the whole report and returns the document with the texts around its pages.
pub(super) fn build(data: &ReportData, lang: Lang) -> (Doc, Meta) {
    let project = project_name(data, lang);
    let version = tool_version(data);
    let mut doc = Doc::new();

    let mut meta_lines = Vec::new();
    let generated = single_line(&data.generated_on, 40);
    if !generated.is_empty() {
        meta_lines.push((t(lang, Key::GeneratedOn).to_owned(), generated));
    }
    meta_lines.push((
        t(lang, Key::ToolVersion).to_owned(),
        format!("pn-ultramemory {version}"),
    ));
    doc.title_block(
        "pn-ultramemory",
        t(lang, Key::DocTitle),
        &project,
        &meta_lines,
    );

    summary(&mut doc, data, lang);
    doc.section(t(lang, Key::SecHowTo));
    let muted = doc.theme.muted;
    doc.paragraph(t(lang, Key::HowToRead), Font::Regular, 9.5, muted);
    languages(&mut doc, data, lang);
    modules(&mut doc, data, lang);
    hotspots(&mut doc, data, lang);
    documentation(&mut doc, data, lang);
    code_graph(&mut doc, data, lang);
    memories(&mut doc, data, lang);
    usage(&mut doc, data, lang);
    notes(&mut doc, data, lang);

    let meta = Meta {
        lang,
        title: format!("{project} - {}", t(lang, Key::DocTitle)),
        project,
        kind: t(lang, Key::DocTitle).to_owned(),
        version: version.clone(),
        generated_on: data.generated_on.clone(),
        footer: tf(lang, Key::Footer, &[&version]),
    };
    (doc, meta)
}

/// Writes the "no data" note.
fn no_data(doc: &mut Doc, lang: Lang) {
    doc.note(t(lang, Key::NoData));
}

/// Writes the "showing N of M" note when a table was cut.
fn cut_note<T>(doc: &mut Doc, window: &Window<'_, T>, lang: Lang) {
    if let Some(note) = window.note(lang) {
        doc.note(&note);
    }
}

/// The executive summary: metric cards.
fn summary(doc: &mut Doc, data: &ReportData, lang: Lang) {
    let s = &data.summary;
    doc.section(t(lang, Key::SecSummary));
    let card = |key: Key, value: u64, tone: Tone| (t(lang, key).to_owned(), int(value, lang), tone);
    let cards = vec![
        card(Key::MetricFiles, s.files, Tone::Normal),
        card(Key::MetricLines, s.lines, Tone::Normal),
        card(Key::MetricSymbols, s.symbols, Tone::Normal),
        card(Key::MetricRelationships, s.edges, Tone::Normal),
        card(Key::MetricMemories, s.memories, Tone::Normal),
        card(
            Key::MetricStale,
            s.stale_memories,
            if s.stale_memories > 0 {
                Tone::Warn
            } else {
                Tone::Normal
            },
        ),
        card(
            Key::MetricParseErrors,
            s.parse_error_files,
            if s.parse_error_files > 0 {
                Tone::Bad
            } else {
                Tone::Normal
            },
        ),
    ];
    doc.cards(&cards);
}

/// The languages chart and table.
fn languages(doc: &mut Doc, data: &ReportData, lang: Lang) {
    doc.section(t(lang, Key::SecLanguages));
    let sorted = sorted_languages(&data.languages);
    if sorted.is_empty() {
        no_data(doc, lang);
        return;
    }
    let max_files = sorted.iter().map(|row| row.files).max().unwrap_or(1).max(1);
    let bars: Vec<(String, f64, String)> = sorted
        .iter()
        .take(MAX_LANGUAGE_BARS)
        .map(|row| {
            (
                single_line(&row.name, CLIP_NAME),
                to_f64(row.files) / to_f64(max_files),
                int(row.files, lang),
            )
        })
        .collect();
    doc.bars(&bars);
    let total = language_file_total(&sorted);
    let window = Window::new(&sorted, MAX_LANGUAGES);
    let cols = [
        Col::flex(t(lang, Key::ColLanguage)),
        Col::num(t(lang, Key::ColFiles)),
        Col::num(t(lang, Key::ColSymbols)),
        Col::num(t(lang, Key::ColShare)),
    ];
    let rows: Vec<Vec<Cell>> = window
        .rows
        .iter()
        .map(|row| {
            vec![
                Cell::text(single_line(&row.name, CLIP_NAME)),
                Cell::text(int(row.files, lang)),
                Cell::text(int(row.symbols, lang)),
                Cell::text(percent(share(row.files, total), lang)),
            ]
        })
        .collect();
    doc.table(&cols, &rows);
    cut_note(doc, &window, lang);
}

/// The modules table.
fn modules(doc: &mut Doc, data: &ReportData, lang: Lang) {
    doc.section(t(lang, Key::SecModules));
    let sorted = sorted_modules(&data.modules);
    if sorted.is_empty() {
        no_data(doc, lang);
        return;
    }
    doc.note(t(lang, Key::ModulesIntro));
    let window = Window::new(&sorted, MAX_MODULES);
    let cols = [
        Col::flex(t(lang, Key::ColModule)),
        Col::num(t(lang, Key::ColFiles)),
        Col::num(t(lang, Key::ColSymbols)),
        Col::num(t(lang, Key::ColIncoming)),
        Col::num(t(lang, Key::ColOutgoing)),
    ];
    let rows: Vec<Vec<Cell>> = window
        .rows
        .iter()
        .map(|row| {
            vec![
                Cell::text(single_line(&row.name, CLIP_NAME)),
                Cell::text(int(row.files, lang)),
                Cell::text(int(row.symbols, lang)),
                Cell::text(int(row.incoming, lang)),
                Cell::text(int(row.outgoing, lang)),
            ]
        })
        .collect();
    doc.table(&cols, &rows);
    cut_note(doc, &window, lang);
}

/// The hotspots table.
fn hotspots(doc: &mut Doc, data: &ReportData, lang: Lang) {
    doc.section(t(lang, Key::SecHotspots));
    let sorted = sorted_hotspots(&data.hotspots);
    if sorted.is_empty() {
        no_data(doc, lang);
        return;
    }
    doc.note(t(lang, Key::HotspotsIntro));
    let window = Window::new(&sorted, MAX_HOTSPOTS);
    let cols = [
        Col::flex(t(lang, Key::ColSymbol)),
        Col::text(t(lang, Key::ColKind)),
        Col::flex(t(lang, Key::ColLocation)),
        Col::num(t(lang, Key::ColCallers)),
        Col::num(t(lang, Key::ColCallees)),
    ];
    let rows: Vec<Vec<Cell>> = window
        .rows
        .iter()
        .map(|row| {
            vec![
                Cell::text(single_line(&row.name, CLIP_NAME)),
                Cell::text(single_line(&row.kind, 40)),
                Cell::text(location(&row.path, row.line)),
                Cell::text(int(u64::from(row.callers), lang)),
                Cell::text(int(u64::from(row.callees), lang)),
            ]
        })
        .collect();
    doc.table(&cols, &rows);
    cut_note(doc, &window, lang);
}

/// The documentation coverage section.
fn documentation(doc: &mut Doc, data: &ReportData, lang: Lang) {
    let docs = &data.documentation;
    doc.section(t(lang, Key::SecDocs));
    if docs.public_symbols == 0 && docs.by_language.is_empty() {
        no_data(doc, lang);
        return;
    }
    let overall = doc_percent(docs);
    doc.progress(t(lang, Key::DocOverall), overall, &percent(overall, lang));
    let documented = int(docs.documented.min(docs.public_symbols), lang);
    let public = int(docs.public_symbols, lang);
    let ink = doc.theme.ink;
    doc.paragraph(
        &tf(lang, Key::DocSentence, &[&documented, &public]),
        Font::Regular,
        9.5,
        ink,
    );
    if !docs.by_language.is_empty() {
        doc.subsection(t(lang, Key::DocByLanguage));
        let window = Window::new(&docs.by_language, MAX_DOC_LANGUAGES);
        let cols = [
            Col::flex(t(lang, Key::ColLanguage)),
            Col::num(t(lang, Key::ColPublic)),
            Col::num(t(lang, Key::ColDocumented)),
            Col::text(t(lang, Key::ColCoverage)),
        ];
        let rows: Vec<Vec<Cell>> = window
            .rows
            .iter()
            .map(|row| {
                let value = numfmt::ratio_percent(row.documented, row.public_symbols);
                vec![
                    Cell::text(single_line(&row.name, CLIP_NAME)),
                    Cell::text(int(row.public_symbols, lang)),
                    Cell::text(int(row.documented.min(row.public_symbols), lang)),
                    Cell::bar(percent(value, lang), value),
                ]
            })
            .collect();
        doc.table(&cols, &rows);
        cut_note(doc, &window, lang);
    }
    if !docs.undocumented_examples.is_empty() {
        doc.subsection(t(lang, Key::DocUndocumented));
        let window = Window::new(&docs.undocumented_examples, MAX_UNDOCUMENTED);
        let cols = [
            Col::flex(t(lang, Key::ColSymbol)),
            Col::flex(t(lang, Key::ColLocation)),
        ];
        let rows: Vec<Vec<Cell>> = window
            .rows
            .iter()
            .map(|row| {
                vec![
                    Cell::text(single_line(&row.name, CLIP_NAME)),
                    Cell::text(location(&row.path, row.line)),
                ]
            })
            .collect();
        doc.table(&cols, &rows);
        cut_note(doc, &window, lang);
    }
}

/// The code graph section: figure, legend and truncation note.
fn code_graph(doc: &mut Doc, data: &ReportData, lang: Lang) {
    let prep = prepare(&data.graph, REPORT_GRAPH_NODES, REPORT_GRAPH_EDGES);
    let room = if prep.nodes.is_empty() {
        90.0
    } else {
        FRAME_HEIGHT + 130.0
    };
    doc.section_with_room(t(lang, Key::SecGraph), room);
    if prep.nodes.is_empty() {
        no_data(doc, lang);
        return;
    }
    doc.note(t(lang, Key::GraphIntro));
    let placed = layout(&prep);
    let legend: Vec<(usize, String)> = prep
        .groups
        .iter()
        .enumerate()
        .take(20)
        .map(|(index, group)| (index, group_display(group, lang)))
        .collect();
    doc.graph_figure(&prep, &placed, &legend);
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
        doc.note(&note);
    }
    if prep.edges.iter().any(PEdge::is_uncertain) {
        doc.note(t(lang, Key::GraphDashed));
    }
}

/// The memories table.
fn memories(doc: &mut Doc, data: &ReportData, lang: Lang) {
    doc.section(t(lang, Key::SecMemories));
    if data.memories.is_empty() {
        no_data(doc, lang);
        return;
    }
    let window = Window::new(&data.memories, MAX_MEMORIES);
    let cols = [
        Col::num(t(lang, Key::ColId)),
        Col::text(t(lang, Key::ColKind)),
        Col::text(t(lang, Key::ColProvenance)),
        Col::text(t(lang, Key::ColStatus)),
        Col::flex(t(lang, Key::ColText)).wrapped(3),
    ];
    let warn = doc.theme.warn;
    let rows: Vec<Vec<Cell>> = window
        .rows
        .iter()
        .map(|row| {
            let status = if row.stale {
                Cell::colored(t(lang, Key::StatusStale), warn)
            } else {
                Cell::text(t(lang, Key::StatusCurrent))
            };
            vec![
                Cell::text(row.id.to_string()),
                Cell::text(single_line(&row.kind, 40)),
                Cell::text(single_line(&row.provenance, 40)),
                status,
                Cell::text(single_line(&row.text, CLIP_TEXT)),
            ]
        })
        .collect();
    doc.table(&cols, &rows);
    cut_note(doc, &window, lang);
}

/// The usage and token efficiency section, written only when usage data exists.
fn usage(doc: &mut Doc, data: &ReportData, lang: Lang) {
    let Some(usage) = &data.usage else { return };
    doc.section_with_room(t(lang, Key::SecUsage), 230.0);
    let muted = doc.theme.muted;
    doc.paragraph(t(lang, Key::UsageIntro), Font::Regular, 9.5, muted);
    let card = |key: Key, value: String| (t(lang, key).to_owned(), value, Tone::Normal);
    let cards = vec![
        card(Key::UsageRecalls, int(usage.recalls, lang)),
        card(Key::UsageExpands, int(usage.expands, lang)),
        card(Key::UsageImpacts, int(usage.impacts, lang)),
        card(Key::UsageRemembers, int(usage.remembers, lang)),
        card(Key::UsageAvgTokens, int(usage.avg_tokens_served, lang)),
        card(
            Key::UsageAvgBudget,
            percent_int(usage.avg_budget_percent, lang),
        ),
        card(Key::UsageTotalTokens, int(usage.tokens_served_total, lang)),
    ];
    doc.cards(&cards);
    doc.progress(
        t(lang, Key::UsageAvgBudget),
        f64::from(usage.avg_budget_percent),
        &percent_int(usage.avg_budget_percent, lang),
    );
}

/// The notes list, written only when there are notes.
fn notes(doc: &mut Doc, data: &ReportData, lang: Lang) {
    if data.notes.is_empty() {
        return;
    }
    doc.section(t(lang, Key::SecNotes));
    let window = Window::new(&data.notes, MAX_NOTES);
    for note in window.rows {
        doc.bullet(&single_line(note, CLIP_TEXT));
    }
    cut_note(doc, &window, lang);
}
