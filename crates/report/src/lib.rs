// SPDX-License-Identifier: Apache-2.0
//! HTML, PDF and Markdown reports and code-graph export, in English and Spanish.
//!
//! This crate turns a [`ReportData`] (filled elsewhere from the index and the store) into a
//! report a person can read, share or archive:
//!
//! * [`render_html`] produces one self-contained HTML file (inline CSS and SVG, no script, no
//!   external resource, light and dark themes, print-ready);
//! * [`render_pdf`] writes a PDF 1.4 document by hand with the standard Helvetica fonts;
//! * [`render_markdown`] produces GitHub-flavored Markdown with a Mermaid diagram;
//! * [`render_graph`] exports the code graph as Mermaid, DOT, SVG or JSON;
//! * [`render_brain`] produces the brain view: one HTML page that draws the whole repository as a
//!   brain of particles a person can fly through and search. It is the one output that runs a
//!   script, and its security policy forbids that script any request.
//!
//! # Role in the architecture
//! This is an exit adapter (see `docs/architecture.md`). It depends on no other workspace crate
//! and on no external crate, performs no I/O and opens no network connection: callers pass data
//! in and get text or bytes back, so the promise that no data leaves the machine holds by
//! construction.
//!
//! # Guarantees
//! * **Infallible.** Bad data is clamped or skipped, never a panic and never an error: an edge
//!   that points outside the node list is ignored, a percentage above 100 is clamped, a huge
//!   table is cut to a fixed number of rows with a "showing N of M" note.
//! * **Deterministic.** The same input gives byte-identical output: no clock, no randomness, no
//!   hash-map iteration order, and the graph layout is a seeded force layout whose sine and
//!   cosine are computed without the platform math library.
//! * **Safe.** Text from the repository is untrusted and is escaped for the target format.
//! * **Bilingual.** Every fixed string exists in English and Spanish, with a completeness test.
//!
//! # Examples
//!
//! ```
//! use pn_ultramemory_report::{Lang, ReportData, render_html, render_markdown};
//!
//! let data = ReportData { project: "demo".into(), ..ReportData::default() };
//! let html = render_html(&data, Lang::En);
//! assert!(html.contains("<html lang=\"en\">"));
//! let markdown = render_markdown(&data, Lang::Es);
//! assert!(markdown.starts_with("# Informe de código"));
//! ```

mod brain;
mod content;
mod graph;
mod html;
mod i18n;
mod lang;
mod markdown;
mod model;
mod numfmt;
mod pdf;
mod text;

pub use lang::Lang;
pub use model::{
    DocCoverage, DocLanguageRow, Graph, GraphEdge, GraphFormat, GraphNode, Hotspot, LanguageRow,
    MemoryRow, ModuleRow, ReportData, Summary, UndocumentedRow, Usage,
};

/// Renders the report as one self-contained HTML5 document.
///
/// The page has inline CSS and inline SVG charts, no JavaScript and no external resource, a
/// `Content-Security-Policy` that only allows inline styles and `data:` images, light and dark
/// themes through `prefers-color-scheme`, a print style sheet that starts every section on a new
/// page, and semantic, accessible markup. Every user-provided string is escaped.
///
/// # Examples
///
/// ```
/// use pn_ultramemory_report::{Lang, ReportData, render_html};
///
/// let data = ReportData { project: "<b>x</b>".into(), ..ReportData::default() };
/// let html = render_html(&data, Lang::En);
/// assert!(html.contains("&lt;b&gt;x&lt;/b&gt;"));
/// assert!(!html.contains("<script"));
/// ```
#[must_use]
pub fn render_html(data: &ReportData, lang: Lang) -> String {
    html::render(data, lang)
}

/// Renders the report as a PDF 1.4 document (A4, portrait).
///
/// The file is written without dependencies and without compression, using the standard fonts
/// Helvetica, Helvetica-Bold and Helvetica-Oblique with `WinAnsiEncoding`, so it needs no
/// embedded fonts. Text that has no glyph in that encoding (anything outside Windows-1252, such
/// as CJK or emoji) is replaced by `?`, and combining marks are dropped. The creation date is
/// derived from [`ReportData::generated_on`], never from the clock, so output is deterministic.
///
/// # Examples
///
/// ```
/// use pn_ultramemory_report::{Lang, ReportData, render_pdf};
///
/// let pdf = render_pdf(&ReportData::default(), Lang::En);
/// assert!(pdf.starts_with(b"%PDF-1.4"));
/// assert!(pdf.ends_with(b"%%EOF\n"));
/// ```
#[must_use]
pub fn render_pdf(data: &ReportData, lang: Lang) -> Vec<u8> {
    pdf::render(data, lang)
}

/// Renders the report as GitHub-flavored Markdown.
///
/// Tables use pipes, bars are made of block characters, and the code graph is a fenced
/// `mermaid` block. Names and paths are code spans, free text is escaped, and raw HTML is never
/// produced.
///
/// # Examples
///
/// ```
/// use pn_ultramemory_report::{Lang, ReportData, render_markdown};
///
/// let md = render_markdown(&ReportData::default(), Lang::En);
/// assert!(md.contains("## Executive summary"));
/// ```
#[must_use]
pub fn render_markdown(data: &ReportData, lang: Lang) -> String {
    markdown::render(data, lang)
}

/// Exports the code graph in the given format.
///
/// * [`GraphFormat::Mermaid`]: a `flowchart LR` with safe ids, neutralized labels, one subgraph
///   per group when there are two or more, and weights as edge labels above one. Capped at 150
///   nodes and 500 edges.
/// * [`GraphFormat::Dot`]: Graphviz DOT with quoted, escaped ids and labels, fill color by group
///   and `penwidth` by weight. Complete.
/// * [`GraphFormat::Svg`]: a standalone image with a deterministic force-directed layout
///   (seeded Fruchterman-Reingold, fixed iteration count, `O(iterations * (N^2 + E))`), capped at
///   150 nodes by weight with a "+N more" note.
/// * [`GraphFormat::Json`]: `{"nodes": [...], "edges": [...]}`, complete.
///
/// Edges with out-of-range endpoints and self-loops are ignored, and duplicate edges are merged
/// by summing their weights, in every format.
///
/// # Examples
///
/// ```
/// use pn_ultramemory_report::{Graph, GraphEdge, GraphFormat, GraphNode, Lang, render_graph};
///
/// let node = |id: &str| GraphNode { id: id.into(), label: id.into(), group: String::new(), weight: 1 };
/// let graph = Graph {
///     nodes: vec![node("a"), node("b")],
///     edges: vec![GraphEdge { from: 0, to: 1, weight: 3, ..GraphEdge::default() }],
/// };
/// let mermaid = render_graph(&graph, GraphFormat::Mermaid, Lang::En);
/// assert!(mermaid.contains("n0 -->|3| n1"));
/// ```
#[must_use]
pub fn render_graph(graph: &Graph, format: GraphFormat, lang: Lang) -> String {
    graph::export(graph, format, lang)
}

/// Renders the brain view: the whole repository as a brain of particles, in one HTML page.
///
/// `data` is the JSON text of the engine's brain (symbols, edges and memories in columns) and
/// `title` names the repository. The page carries its own script and styles and needs nothing
/// else: its `Content-Security-Policy` names no source for connections, images, fonts or frames,
/// so the browser refuses any request it could make, and every string from the repository is
/// escaped before it is embedded.
///
/// # Examples
///
/// ```
/// use pn_ultramemory_report::{Lang, render_brain};
///
/// let page = render_brain(r#"{"repo":"demo","nodes":[]}"#, "demo", Lang::En);
/// assert!(page.contains("default-src 'none'"));
/// assert!(page.contains("<title>demo · brain</title>"));
/// ```
#[must_use]
pub fn render_brain(data: &str, title: &str, lang: Lang) -> String {
    brain::render(data, title, lang)
}
