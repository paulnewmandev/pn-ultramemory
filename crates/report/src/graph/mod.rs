// SPDX-License-Identifier: Apache-2.0
//! Everything about the code graph: cleaning it, laying it out and exporting it.
//!
//! The pipeline is the same for every output: [`prepare`] validates and caps the raw graph,
//! `layout` places the nodes (for SVG, HTML and PDF), and one small module per format writes the
//! text. [`export`] is the entry point behind the public `render_graph` function.
//!
//! Caps: the SVG export draws at most 150 nodes and 600 edges, and the Mermaid export at most 150
//! nodes and 500 edges (the default limit of Mermaid renderers), each noting how many nodes were
//! left out; DOT and JSON are complete.

pub(crate) mod dot;
pub(crate) mod json;
pub(crate) mod layout;
pub(crate) mod mermaid;
pub(crate) mod palette;
pub(crate) mod prepare;
pub(crate) mod svg;

use crate::lang::Lang;
use crate::model::{Graph, GraphFormat};

/// Nodes drawn by the standalone SVG and written by the Mermaid export.
pub(crate) const EXPORT_NODES: usize = 150;
/// Edges drawn by the standalone SVG.
const SVG_EDGES: usize = 600;
/// Edges written by the Mermaid export.
const MERMAID_EDGES: usize = 500;

/// Exports `graph` in `format`.
pub(crate) fn export(graph: &Graph, format: GraphFormat, lang: Lang) -> String {
    match format {
        GraphFormat::Mermaid => {
            mermaid::render(&prepare::prepare(graph, EXPORT_NODES, MERMAID_EDGES), lang)
        }
        GraphFormat::Dot => dot::render(&prepare::prepare(graph, usize::MAX, usize::MAX)),
        GraphFormat::Svg => {
            let prep = prepare::prepare(graph, EXPORT_NODES, SVG_EDGES);
            let placed = layout::layout(&prep);
            svg::standalone(&prep, &placed, lang)
        }
        GraphFormat::Json => json::render(&prepare::prepare(graph, usize::MAX, usize::MAX)),
    }
}
