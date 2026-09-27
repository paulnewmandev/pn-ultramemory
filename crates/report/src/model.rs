// SPDX-License-Identifier: Apache-2.0
//! The plain data a report is rendered from.
//!
//! Another crate fills these structs from the index and the store; this crate only renders them.
//! Nothing here performs I/O or validation: every renderer clamps or skips bad data (an edge that
//! points outside the node list, a percentage above 100, a `documented` count larger than the
//! number of public symbols) instead of failing, so a report can always be produced.
//!
//! All types derive `Debug`, `Clone`, `PartialEq`, `Eq` and `Default`, and every field is public so
//! callers can build them with struct literals.

/// Everything a report shows about one indexed repository.
///
/// # Examples
///
/// ```
/// use pn_ultramemory_report::ReportData;
///
/// let data = ReportData { project: "demo".into(), ..ReportData::default() };
/// assert_eq!(data.project, "demo");
/// assert!(data.usage.is_none());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReportData {
    /// Project name shown in the title block.
    pub project: String,
    /// Generation date, already formatted (for example `2026-09-25`). It is also the source of the
    /// PDF creation date, so the output never depends on the clock.
    pub generated_on: String,
    /// Version of the tool that produced the data, without a prefix (for example `1.2.36`).
    pub tool_version: String,
    /// Headline counters.
    pub summary: Summary,
    /// One row per programming language found.
    pub languages: Vec<LanguageRow>,
    /// One row per module (directory, package or crate).
    pub modules: Vec<ModuleRow>,
    /// The most referenced symbols.
    pub hotspots: Vec<Hotspot>,
    /// Documentation coverage of public symbols.
    pub documentation: DocCoverage,
    /// The code graph.
    pub graph: Graph,
    /// Stored memories.
    pub memories: Vec<MemoryRow>,
    /// Local usage counters, when the store keeps them.
    pub usage: Option<Usage>,
    /// Free-form remarks, one per entry.
    pub notes: Vec<String>,
}

/// Headline counters of the executive summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Summary {
    /// Indexed files.
    pub files: u64,
    /// Lines of code across the indexed files.
    pub lines: u64,
    /// Symbols (functions, types, constants and so on).
    pub symbols: u64,
    /// Relationships (edges) between symbols.
    pub edges: u64,
    /// Files the parser reported errors for.
    pub parse_error_files: u64,
    /// Stored memories.
    pub memories: u64,
    /// Memories whose anchored code changed after they were saved.
    pub stale_memories: u64,
}

/// One programming language of the repository.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LanguageRow {
    /// Language name (for example `Rust`).
    pub name: String,
    /// Files written in it.
    pub files: u64,
    /// Symbols defined in those files.
    pub symbols: u64,
}

/// One module of the repository.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ModuleRow {
    /// Module name or path.
    pub name: String,
    /// Files inside it.
    pub files: u64,
    /// Symbols defined inside it.
    pub symbols: u64,
    /// Relationships coming in from other modules.
    pub incoming: u64,
    /// Relationships going out to other modules.
    pub outgoing: u64,
}

/// A symbol that many others reference.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Hotspot {
    /// Symbol name.
    pub name: String,
    /// Kind of symbol (`function`, `struct`, ...).
    pub kind: String,
    /// Path of the file that defines it.
    pub path: String,
    /// One-based line where it starts; zero means unknown.
    pub line: u32,
    /// Number of symbols that reference it.
    pub callers: u32,
    /// Number of symbols it references.
    pub callees: u32,
}

/// Documentation coverage of public symbols.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DocCoverage {
    /// Public symbols in total.
    pub public_symbols: u64,
    /// Public symbols that have a documentation comment.
    pub documented: u64,
    /// The same counts per language.
    pub by_language: Vec<DocLanguageRow>,
    /// Some public symbols that lack documentation.
    pub undocumented_examples: Vec<UndocumentedRow>,
}

/// Documentation coverage of one language.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DocLanguageRow {
    /// Language name.
    pub name: String,
    /// Public symbols in that language.
    pub public_symbols: u64,
    /// Documented ones.
    pub documented: u64,
}

/// A public symbol without documentation.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UndocumentedRow {
    /// Symbol name.
    pub name: String,
    /// Path of the file that defines it.
    pub path: String,
    /// One-based line where it starts; zero means unknown.
    pub line: u32,
}

/// The code graph: nodes and weighted, directed edges between them.
///
/// Edge endpoints are indices into [`Graph::nodes`]. Edges whose endpoints are out of range, and
/// self-loops, are ignored by every renderer; duplicate edges are merged by summing their weights.
///
/// # Examples
///
/// ```
/// use pn_ultramemory_report::{Graph, GraphEdge, GraphNode};
///
/// let node = |id: &str| GraphNode { id: id.into(), label: id.into(), group: "core".into(), weight: 1 };
/// let graph = Graph {
///     nodes: vec![node("a"), node("b")],
///     edges: vec![GraphEdge { from: 0, to: 1, weight: 2, ..GraphEdge::default() }],
/// };
/// assert_eq!(graph.edges.len(), 1);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Graph {
    /// The nodes.
    pub nodes: Vec<GraphNode>,
    /// The edges, as indices into `nodes`.
    pub edges: Vec<GraphEdge>,
}

/// One node of the code graph.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GraphNode {
    /// Stable identifier, exported as is to JSON and DOT.
    pub id: String,
    /// Text shown next to the node.
    pub label: String,
    /// Group the node belongs to (module, language, ...); nodes of a group share a color.
    pub group: String,
    /// Importance of the node; it decides the radius and which nodes survive a cap.
    pub weight: u32,
}

/// One directed edge of the code graph.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GraphEdge {
    /// Index of the source node.
    pub from: usize,
    /// Index of the target node.
    pub to: usize,
    /// How many times the relationship occurs; zero counts as one.
    pub weight: u32,
    /// Kind of relationship (`calls`, `imports`, ...).
    pub kind: String,
    /// Confidence of the indexer: `exact`, `resolved`, `heuristic` or `guess`. The last two are
    /// drawn dashed.
    pub confidence: String,
}

/// One stored memory.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MemoryRow {
    /// Identifier in the store.
    pub id: i64,
    /// Kind of memory (`decision`, `fact`, ...).
    pub kind: String,
    /// Who produced it (`tool`, `agent` or `user`).
    pub provenance: String,
    /// Whether the code it is anchored to changed since it was saved.
    pub stale: bool,
    /// The memory text.
    pub text: String,
}

/// Local usage counters. They are counted on this machine and never sent anywhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Usage {
    /// Number of `recall` calls.
    pub recalls: u64,
    /// Number of `expand` calls.
    pub expands: u64,
    /// Number of `impact` calls.
    pub impacts: u64,
    /// Number of `remember` calls.
    pub remembers: u64,
    /// Total tokens served across all calls.
    pub tokens_served_total: u64,
    /// Average tokens served per call.
    pub avg_tokens_served: u64,
    /// Average share of the token budget that calls used, in percent.
    pub avg_budget_percent: u32,
}

/// A way to export the code graph.
///
/// # Examples
///
/// ```
/// use pn_ultramemory_report::GraphFormat;
///
/// assert_eq!(GraphFormat::from_name("Mermaid"), Some(GraphFormat::Mermaid));
/// assert_eq!(GraphFormat::Dot.extension(), "dot");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum GraphFormat {
    /// A Mermaid `flowchart LR` diagram.
    #[default]
    Mermaid,
    /// Graphviz DOT.
    Dot,
    /// A standalone SVG image.
    Svg,
    /// A JSON document with `nodes` and `edges`.
    Json,
}

impl GraphFormat {
    /// Every format, in a stable order.
    pub const ALL: [Self; 4] = [Self::Mermaid, Self::Dot, Self::Svg, Self::Json];

    /// Parses a format name, ignoring case and surrounding whitespace.
    ///
    /// Accepted names are `mermaid` (also `mmd`), `dot` (also `graphviz` and `gv`), `svg` and
    /// `json`.
    ///
    /// # Examples
    ///
    /// ```
    /// use pn_ultramemory_report::GraphFormat;
    ///
    /// assert_eq!(GraphFormat::from_name(" GraphViz "), Some(GraphFormat::Dot));
    /// assert_eq!(GraphFormat::from_name("png"), None);
    /// ```
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim().to_lowercase().as_str() {
            "mermaid" | "mmd" => Some(Self::Mermaid),
            "dot" | "graphviz" | "gv" => Some(Self::Dot),
            "svg" => Some(Self::Svg),
            "json" => Some(Self::Json),
            _ => None,
        }
    }

    /// Returns the conventional file extension of the format, without the dot.
    ///
    /// # Examples
    ///
    /// ```
    /// use pn_ultramemory_report::GraphFormat;
    ///
    /// assert_eq!(GraphFormat::Mermaid.extension(), "mmd");
    /// ```
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Mermaid => "mmd",
            Self::Dot => "dot",
            Self::Svg => "svg",
            Self::Json => "json",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{GraphFormat, ReportData};

    /// The default report is empty and has no usage section.
    #[test]
    fn default_report_is_empty() {
        let data = ReportData::default();
        assert!(data.project.is_empty());
        assert!(data.usage.is_none());
        assert!(data.graph.nodes.is_empty());
    }

    /// Format names parse in any case, and the aliases work.
    #[test]
    fn format_names_parse() {
        assert_eq!(
            GraphFormat::from_name("MERMAID"),
            Some(GraphFormat::Mermaid)
        );
        assert_eq!(GraphFormat::from_name("mmd"), Some(GraphFormat::Mermaid));
        assert_eq!(GraphFormat::from_name("gv"), Some(GraphFormat::Dot));
        assert_eq!(GraphFormat::from_name(" svg\n"), Some(GraphFormat::Svg));
        assert_eq!(GraphFormat::from_name("Json"), Some(GraphFormat::Json));
        assert_eq!(GraphFormat::from_name(""), None);
        assert_eq!(GraphFormat::from_name("pdf"), None);
    }

    /// Every format has a distinct extension that parses back to the same format or an alias.
    #[test]
    fn extensions_are_distinct_and_parse_back() {
        let mut seen = Vec::new();
        for format in GraphFormat::ALL {
            let ext = format.extension();
            assert!(!seen.contains(&ext));
            seen.push(ext);
            assert_eq!(GraphFormat::from_name(ext), Some(format));
        }
    }
}
