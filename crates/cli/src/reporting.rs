// SPDX-License-Identifier: Apache-2.0
//! Turning what the engine measured into what the report renderer draws.
//!
//! # Why this exists
//! `pn-ultramemory-report` has **no dependencies at all**, not even on the rest of this workspace
//! (see `docs/quality.md`). That is deliberate: a renderer that cannot reach a database or a
//! parser cannot leak either into a file a person is about to send to someone. The price is that
//! nothing converts an [`Insights`] into a [`report::ReportData`] by itself, so this module does it, and
//! it is the only place in the program where the two vocabularies meet.
//!
//! # What it decides
//! The engine reports everything it knows; a report shows what fits on a page. The limits here are
//! the editorial part of that, and each one is named as a constant rather than written into a call
//! so that a reader can see the whole policy at once.
//!
//! # Invariants
//! * Nothing is invented. Every number comes from the engine, and a field the engine has no answer
//!   for stays at its default rather than being guessed.
//! * The graph is built from module edges, whose endpoints are looked up by name, so an edge whose
//!   module fell outside the node limit is dropped rather than pointing at the wrong node.

use pn_ultramemory_core::{ModuleEdge, ModuleStats};
use pn_ultramemory_engine::Insights;
use pn_ultramemory_report as report;

use crate::args::{LangArg, ReportAs};

/// The most modules listed in the module table.
const MAX_MODULES: usize = 24;

/// The most nodes drawn in the report's graph.
///
/// A module graph past this size stops being readable on a page long before it stops being
/// correct, and the nodes are already ordered by how connected they are, so the ones dropped are
/// the ones a reader would look at last.
const MAX_GRAPH_NODES: usize = 40;

/// The most memories listed.
const MAX_MEMORIES: usize = 30;

/// The most undocumented symbols shown as examples.
const MAX_UNDOCUMENTED: usize = 15;

/// The report language chosen on the command line.
#[must_use]
pub const fn lang_of(arg: LangArg) -> report::Lang {
    match arg {
        LangArg::En => report::Lang::En,
        LangArg::Es => report::Lang::Es,
    }
}

/// The options that ask the engine for exactly what a report shows.
#[must_use]
pub const fn insight_options(module_depth: usize) -> pn_ultramemory_engine::InsightOptions {
    pn_ultramemory_engine::InsightOptions {
        module_depth,
        max_hotspots: MAX_MODULES,
        max_memories: MAX_MEMORIES,
        max_undocumented: MAX_UNDOCUMENTED,
    }
}

/// The file extension a report kind is written with.
#[must_use]
pub const fn extension_of(kind: ReportAs) -> &'static str {
    match kind {
        ReportAs::Html => "html",
        ReportAs::Pdf => "pdf",
        ReportAs::Md => "md",
    }
}

/// Builds the graph the report draws, from the module statistics and the edges between modules.
///
/// Edges are addressed by position in the node list, so the nodes are placed first and each edge
/// is then resolved by name. An edge naming a module that did not make the node limit is dropped,
/// because keeping it would mean either an index out of range or an edge pointing at the wrong
/// module, and a wrong edge in a picture is worse than a missing one.
fn graph_of(modules: &[ModuleStats], edges: &[ModuleEdge]) -> report::Graph {
    let nodes: Vec<report::GraphNode> = modules
        .iter()
        .take(MAX_GRAPH_NODES)
        .map(|module| report::GraphNode {
            id: module.name.clone(),
            label: module.name.clone(),
            group: module.name.split('/').next().unwrap_or("").to_owned(),
            weight: u32::try_from(module.symbols).unwrap_or(u32::MAX),
        })
        .collect();
    let position = |name: &str| nodes.iter().position(|node| node.id == name);
    let edges = edges
        .iter()
        .filter_map(|edge| {
            let from = position(&edge.from)?;
            let to = position(&edge.to)?;
            (from != to).then_some(report::GraphEdge {
                from,
                to,
                weight: u32::try_from(edge.weight).unwrap_or(u32::MAX),
                kind: "calls".to_owned(),
                confidence: "resolved".to_owned(),
            })
        })
        .collect();
    report::Graph { nodes, edges }
}

/// Converts a graph the engine built into the one the renderer draws.
///
/// The two shapes already match field for field; what this adds is the names of the typed values,
/// because the renderer writes the kind and the confidence into a legend and must not depend on the
/// domain enums to do it. It takes the two lists rather than a graph type so that the symbol graph
/// and the module graph, which have the same shape and differ only in what a node means, both go
/// through one conversion.
#[must_use]
pub fn drawable(
    nodes: &[pn_ultramemory_engine::GraphNodeInfo],
    edges: &[pn_ultramemory_engine::GraphEdgeInfo],
) -> report::Graph {
    struct Both<'a> {
        nodes: &'a [pn_ultramemory_engine::GraphNodeInfo],
        edges: &'a [pn_ultramemory_engine::GraphEdgeInfo],
    }
    let graph = Both { nodes, edges };
    report::Graph {
        nodes: graph
            .nodes
            .iter()
            .map(|node| report::GraphNode {
                id: node.id.clone(),
                label: node.label.clone(),
                group: node.group.clone(),
                weight: node.weight,
            })
            .collect(),
        edges: graph
            .edges
            .iter()
            .map(|edge| report::GraphEdge {
                from: edge.from,
                to: edge.to,
                weight: edge.weight,
                kind: edge.kind.as_str().to_owned(),
                confidence: edge.confidence.as_str().to_owned(),
            })
            .collect(),
    }
}

/// Converts everything the engine measured into the data a report is rendered from.
///
/// `project` names the repository in the heading, and `generated_on` is passed in rather than read
/// from the clock here so that a caller can produce a byte-for-byte reproducible report.
#[must_use]
pub fn report_data(insights: &Insights, project: &str, generated_on: &str) -> report::ReportData {
    let index = &insights.stats.index;
    let totals = &insights.stats.totals;
    report::ReportData {
        project: project.to_owned(),
        generated_on: generated_on.to_owned(),
        tool_version: env!("CARGO_PKG_VERSION").to_owned(),
        summary: report::Summary {
            files: index.files,
            lines: totals.lines,
            symbols: index.symbols,
            edges: index.edges,
            parse_error_files: totals.parse_error_files,
            memories: index.memories,
            stale_memories: index.stale_memories,
        },
        languages: insights
            .languages
            .iter()
            .map(|row| report::LanguageRow {
                name: row.name.clone(),
                files: row.files,
                symbols: row.symbols,
            })
            .collect(),
        modules: insights
            .modules
            .iter()
            .take(MAX_MODULES)
            .map(|module| report::ModuleRow {
                name: module.name.clone(),
                files: module.files,
                symbols: module.symbols,
                incoming: module.incoming,
                outgoing: module.outgoing,
            })
            .collect(),
        hotspots: insights
            .hotspots
            .iter()
            .map(|spot| report::Hotspot {
                name: spot.name.clone(),
                kind: spot.kind.as_str().to_owned(),
                path: spot.path.clone(),
                line: spot.line,
                callers: spot.callers,
                callees: spot.callees,
            })
            .collect(),
        documentation: report::DocCoverage {
            public_symbols: insights
                .documentation
                .iter()
                .map(|r| r.public_symbols)
                .sum(),
            documented: insights.documentation.iter().map(|r| r.documented).sum(),
            by_language: insights
                .documentation
                .iter()
                .map(|row| report::DocLanguageRow {
                    name: row.language.clone(),
                    public_symbols: row.public_symbols,
                    documented: row.documented,
                })
                .collect(),
            undocumented_examples: insights
                .undocumented
                .iter()
                .take(MAX_UNDOCUMENTED)
                .map(|gap| report::UndocumentedRow {
                    name: gap.name.clone(),
                    path: gap.path.clone(),
                    line: gap.line,
                })
                .collect(),
        },
        graph: graph_of(&insights.modules, &insights.module_edges),
        memories: insights
            .memories
            .iter()
            .take(MAX_MEMORIES)
            .map(|record| report::MemoryRow {
                id: record.id.0,
                kind: record.kind.as_str().to_owned(),
                provenance: record.provenance.as_str().to_owned(),
                stale: record.stale_since.is_some(),
                text: record.text.clone(),
            })
            .collect(),
        usage: insights.stats.usage.as_ref().map(|usage| report::Usage {
            recalls: usage.recalls,
            expands: usage.expands,
            impacts: usage.impacts,
            remembers: usage.remembers,
            tokens_served_total: usage.tokens_served_total,
            avg_tokens_served: usage.avg_tokens_served,
            avg_budget_percent: usage.avg_budget_percent,
        }),
        notes: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use pn_ultramemory_core::{ModuleEdge, ModuleStats};

    use super::{MAX_GRAPH_NODES, graph_of};

    /// Builds a module with a name and a symbol count.
    fn module(name: &str, symbols: u64) -> ModuleStats {
        ModuleStats {
            name: name.to_owned(),
            files: 1,
            symbols,
            incoming: 0,
            outgoing: 0,
        }
    }

    /// Builds an edge between two named modules.
    fn edge(from: &str, to: &str) -> ModuleEdge {
        ModuleEdge {
            from: from.to_owned(),
            to: to.to_owned(),
            weight: 1,
        }
    }

    /// Edges are resolved by name to the position of their node, and the group is the first path
    /// segment so that the renderer can colour a whole area at once.
    #[test]
    fn edges_point_at_the_right_nodes() {
        let modules = [module("src/a", 10), module("src/b", 5), module("tests", 2)];
        let graph = graph_of(&modules, &[edge("tests", "src/a")]);
        assert_eq!(graph.nodes.len(), 3);
        assert_eq!(graph.nodes[0].group, "src");
        assert_eq!(graph.edges.len(), 1);
        assert_eq!(graph.edges[0].from, 2);
        assert_eq!(graph.edges[0].to, 0);
    }

    /// An edge naming a module that is not drawn is dropped, never pointed at another node.
    #[test]
    fn an_edge_to_a_missing_node_is_dropped() {
        let modules = [module("src/a", 1)];
        let graph = graph_of(
            &modules,
            &[edge("src/a", "src/gone"), edge("src/x", "src/a")],
        );
        assert!(graph.edges.is_empty());
    }

    /// A module that calls itself is not drawn as a loop.
    #[test]
    fn a_self_edge_is_dropped() {
        let modules = [module("src/a", 1)];
        let graph = graph_of(&modules, &[edge("src/a", "src/a")]);
        assert!(graph.edges.is_empty());
    }

    /// Past the node limit the extra modules are left out, and so is every edge that needed them,
    /// so the picture stays consistent with the list of nodes.
    #[test]
    fn the_node_limit_is_respected() {
        let modules: Vec<ModuleStats> = (0..MAX_GRAPH_NODES + 10)
            .map(|n| module(&format!("m{n}"), 1))
            .collect();
        let edges = [edge("m0", &format!("m{}", MAX_GRAPH_NODES + 5))];
        let graph = graph_of(&modules, &edges);
        assert_eq!(graph.nodes.len(), MAX_GRAPH_NODES);
        assert!(graph.edges.is_empty());
    }

    /// Nothing at all is still a valid, empty graph rather than a failure.
    #[test]
    fn an_empty_repository_yields_an_empty_graph() {
        let graph = graph_of(&[], &[]);
        assert!(graph.nodes.is_empty() && graph.edges.is_empty());
    }
}
