// SPDX-License-Identifier: Apache-2.0
//! Report data used by the integration tests: a realistic sample, a hostile one and a huge one.

use pn_ultramemory_report::{
    DocCoverage, DocLanguageRow, Graph, GraphEdge, GraphNode, Hotspot, LanguageRow, MemoryRow,
    ModuleRow, ReportData, Summary, UndocumentedRow, Usage,
};

/// The most hostile text the tests know: markup, quotes, ampersands, control characters, right to
/// left text, emoji, combining marks and a bidirectional override.
pub(crate) const NASTY: &str = "<script>alert(1)</script> \"quoted\" 'single' & &amp; &lt; </td></tr> \
    שלום עולם 🙂 e\u{301} \u{202e}reversed\u{202c} tab\there\nnewline \u{0}nul\u{7}bell ]]> <!-- x -->";

/// Returns a unique-looking string of `len` characters without any space, to test wrapping.
pub(crate) fn long_word(len: usize) -> String {
    "Wq9".chars().cycle().take(len).collect()
}

/// Builds a memory row.
fn memory(id: i64, kind: &str, provenance: &str, stale: bool, text: &str) -> MemoryRow {
    MemoryRow {
        id,
        kind: kind.into(),
        provenance: provenance.into(),
        stale,
        text: text.into(),
    }
}

/// Builds a graph edge that calls with exact confidence.
fn call(from: usize, to: usize, weight: u32) -> GraphEdge {
    GraphEdge {
        from,
        to,
        weight,
        kind: "calls".into(),
        confidence: "exact".into(),
    }
}

/// The graph of the realistic sample: four nodes in three groups.
fn sample_graph() -> Graph {
    let node = |index: usize, group: &str, weight| GraphNode {
        id: format!("sym{index}"),
        label: format!("Symbol{index}"),
        group: group.into(),
        weight,
    };
    Graph {
        nodes: vec![
            node(0, "api", 9),
            node(1, "core", 5),
            node(2, "core", 3),
            node(3, "web", 1),
        ],
        edges: vec![call(0, 1, 4), call(1, 2, 1), call(3, 0, 2), call(2, 0, 1)],
    }
}

/// The documentation coverage of the realistic sample.
fn sample_documentation() -> DocCoverage {
    DocCoverage {
        public_symbols: 400,
        documented: 300,
        by_language: vec![DocLanguageRow {
            name: "Rust".into(),
            public_symbols: 400,
            documented: 300,
        }],
        undocumented_examples: vec![UndocumentedRow {
            name: "Thing".into(),
            path: "src/thing.rs".into(),
            line: 7,
        }],
    }
}

/// A modest, realistic report.
pub(crate) fn sample() -> ReportData {
    let language = |name: &str, files, symbols| LanguageRow {
        name: name.into(),
        files,
        symbols,
    };
    let module = |name: &str, files, symbols, incoming, outgoing| ModuleRow {
        name: name.into(),
        files,
        symbols,
        incoming,
        outgoing,
    };
    ReportData {
        project: "Nimbus Ledger".into(),
        generated_on: "2026-09-25".into(),
        tool_version: "1.2.36".into(),
        summary: Summary {
            files: 363,
            lines: 71_240,
            symbols: 5_117,
            edges: 18_930,
            parse_error_files: 3,
            memories: 3,
            stale_memories: 1,
        },
        languages: vec![
            language("Rust", 118, 1_910),
            language("TypeScript", 162, 2_430),
        ],
        modules: vec![
            module("api/http", 48, 612, 90, 410),
            module("core/money", 12, 148, 511, 12),
        ],
        hotspots: (0..30u32)
            .map(|i| Hotspot {
                name: format!("module::function_{i}"),
                kind: "function".into(),
                path: format!("src/module/file_{i}.rs"),
                line: 10 + i,
                callers: 100 - i,
                callees: i,
            })
            .collect(),
        documentation: sample_documentation(),
        graph: sample_graph(),
        memories: vec![
            memory(1, "decision", "user", false, "Use integers for money."),
            memory(2, "fact", "tool", true, "The cache is per request."),
            memory(3, "lesson", "agent", false, "Run migrations first."),
        ],
        usage: Some(Usage {
            recalls: 1_284,
            expands: 402,
            impacts: 96,
            remembers: 51,
            tokens_served_total: 1_902_337,
            avg_tokens_served: 1_053,
            avg_budget_percent: 61,
        }),
        notes: vec!["Three files could not be parsed.".into()],
    }
}

/// Returns `NASTY` followed by `suffix`, so every field has a distinct hostile value.
fn nasty(suffix: &str) -> String {
    format!("{NASTY}{suffix}")
}

/// A graph in which every string is hostile and most edges are invalid or duplicated.
fn hostile_graph() -> Graph {
    let node = |index: usize| GraphNode {
        id: nasty(&format!("#{index}")),
        label: nasty(&format!(" node {index}")),
        group: if index % 3 == 0 {
            String::new()
        } else {
            nasty(&format!(" group {}", index % 3))
        },
        weight: u32::try_from(index * 3).unwrap_or(0),
    };
    let edge = |from, to, weight, kind: &str, confidence: &str| GraphEdge {
        from,
        to,
        weight,
        kind: kind.into(),
        confidence: confidence.into(),
    };
    Graph {
        nodes: (0..8).map(node).collect(),
        edges: vec![
            edge(0, 1, u32::MAX, &nasty(" kind"), &nasty(" conf")),
            edge(0, 1, u32::MAX, "", "guess"),
            edge(3, 3, 1, "", ""),
            edge(99, 0, 1, "", ""),
            edge(2, usize::MAX, 1, "", ""),
            edge(5, 6, 0, "", ""),
        ],
    }
}

/// Documentation coverage with impossible numbers and hostile names.
fn hostile_documentation() -> DocCoverage {
    DocCoverage {
        public_symbols: 3,
        documented: 300,
        by_language: vec![DocLanguageRow {
            name: nasty(" doc"),
            public_symbols: 0,
            documented: 9,
        }],
        undocumented_examples: vec![UndocumentedRow {
            name: nasty(" undocumented"),
            path: long_word(3_000),
            line: 0,
        }],
    }
}

/// A report in which every user-provided string is hostile.
pub(crate) fn hostile() -> ReportData {
    ReportData {
        project: nasty(" project"),
        generated_on: nasty(""),
        tool_version: nasty(" 1.0"),
        summary: Summary {
            files: u64::MAX,
            lines: u64::MAX,
            symbols: 0,
            edges: u64::MAX,
            parse_error_files: 1,
            memories: 2,
            stale_memories: 5,
        },
        languages: vec![
            LanguageRow {
                name: nasty(" lang"),
                files: u64::MAX,
                symbols: 1,
            },
            LanguageRow {
                name: long_word(5_000),
                files: 0,
                symbols: 0,
            },
            LanguageRow {
                name: String::new(),
                files: 3,
                symbols: 3,
            },
        ],
        modules: vec![ModuleRow {
            name: nasty(" module"),
            files: 1,
            symbols: 1,
            incoming: u64::MAX,
            outgoing: 0,
        }],
        hotspots: vec![Hotspot {
            name: nasty(" hotspot"),
            kind: nasty(" kind"),
            path: nasty(" path"),
            line: u32::MAX,
            callers: u32::MAX,
            callees: 0,
        }],
        documentation: hostile_documentation(),
        graph: hostile_graph(),
        memories: vec![memory(
            i64::MIN,
            &nasty(" kind"),
            &nasty(" prov"),
            true,
            &long_word(100_000),
        )],
        usage: Some(Usage {
            recalls: u64::MAX,
            expands: 0,
            impacts: 0,
            remembers: 0,
            tokens_served_total: u64::MAX,
            avg_tokens_served: u64::MAX,
            avg_budget_percent: u32::MAX,
        }),
        notes: vec![nasty(" note"), long_word(50_000), String::new()],
    }
}

/// A graph with 500 nodes in 12 groups and 5 000 edges, many of them uncertain.
fn huge_graph() -> Graph {
    let node = |index: usize| GraphNode {
        id: format!("node-{index}"),
        label: format!("{} {index}", long_word(300)),
        group: format!("group {}", index % 12),
        weight: u32::try_from((index * 37) % 1000).unwrap_or(0),
    };
    Graph {
        nodes: (0..500).map(node).collect(),
        edges: (0..5_000)
            .map(|i| GraphEdge {
                from: (i * 7 + i / 500) % 500,
                to: (i * 13 + 1 + i / 250) % 500,
                weight: u32::try_from(1 + i % 9).unwrap_or(1),
                kind: "calls".into(),
                confidence: if i % 5 == 0 {
                    "guess".into()
                } else {
                    "exact".into()
                },
            })
            .collect(),
    }
}

/// A report with thousands of rows and very long strings.
pub(crate) fn huge() -> ReportData {
    ReportData {
        project: long_word(10_000),
        generated_on: "2026-09-25".into(),
        tool_version: "1.2.36".into(),
        summary: Summary {
            files: 99_999,
            lines: 12_345_678,
            symbols: 500_000,
            edges: 2_000_000,
            parse_error_files: 0,
            memories: 2_000,
            stale_memories: 400,
        },
        languages: (0..200)
            .map(|i| LanguageRow {
                name: format!("Language {i}"),
                files: 1_000 - i,
                symbols: i * 3,
            })
            .collect(),
        modules: (0..3_000)
            .map(|i| ModuleRow {
                name: format!("{}/{i}", long_word(200)),
                files: i % 90,
                symbols: i * 7 % 5_000,
                incoming: i,
                outgoing: i / 2,
            })
            .collect(),
        hotspots: (0..5_000u32)
            .map(|i| Hotspot {
                name: format!("{}{i}", long_word(400)),
                kind: "function".into(),
                path: format!("{}/{i}.rs", long_word(500)),
                line: i,
                callers: 5_000 - i,
                callees: i % 50,
            })
            .collect(),
        documentation: DocCoverage {
            public_symbols: 10_000,
            documented: 7_000,
            by_language: (0..100)
                .map(|i| DocLanguageRow {
                    name: format!("L{i}"),
                    public_symbols: 100,
                    documented: i,
                })
                .collect(),
            undocumented_examples: (0..500)
                .map(|i| UndocumentedRow {
                    name: format!("sym{i}"),
                    path: "a.rs".into(),
                    line: i,
                })
                .collect(),
        },
        graph: huge_graph(),
        memories: (0..2_000)
            .map(|i| {
                let text = format!("{} {i}", "memory text with several words ".repeat(400));
                memory(i64::from(i), "fact", "tool", i % 5 == 0, &text)
            })
            .collect(),
        usage: Some(Usage {
            recalls: 1,
            expands: 2,
            impacts: 3,
            remembers: 4,
            tokens_served_total: 5,
            avg_tokens_served: 6,
            avg_budget_percent: 7,
        }),
        notes: (0..1_000)
            .map(|i| format!("note {i} {}", long_word(2_000)))
            .collect(),
    }
}
