// SPDX-License-Identifier: Apache-2.0
//! Deterministic fuzzing: hundreds of randomly generated, mostly nonsensical reports must render
//! in every format, and every output must pass its independent validator.

use pn_ultramemory_report::{
    DocCoverage, DocLanguageRow, Graph, GraphEdge, GraphFormat, GraphNode, Hotspot, Lang,
    LanguageRow, MemoryRow, ModuleRow, ReportData, Summary, UndocumentedRow, Usage, render_graph,
    render_html, render_markdown, render_pdf,
};

use crate::html_check::{Mode, check_markup};
use crate::json::parse;
use crate::pdf_check::check_pdf;

/// Characters the generator draws from: markup, quotes, escapes, separators, controls, accents,
/// right-to-left letters, emoji, combining marks, bidirectional controls and plain letters.
const POOL: &[char] = &[
    '<', '>', '&', '"', '\'', '`', '\\', '/', '|', '[', ']', '(', ')', '{', '}', '#', '%', ';',
    ':', '*', '_', '~', '@', '!', '-', '+', '=', ' ', ' ', ' ', '\t', '\n', '\r', '\u{0}', '\u{7}',
    '\u{1b}', '\u{7f}', '\u{a0}', 'a', 'b', 'Z', '0', '9', 'é', 'ñ', 'ü', '¿', '¡', '€', '“', '”',
    '–', '•', 'ש', 'ל', 'ו', 'م', '日', '本', '🙂', '👩', '\u{200d}', '\u{301}', '\u{202e}',
    '\u{2066}', '\u{2028}', '\u{feff}', '\u{fe0f}',
];

/// A tiny seeded generator.
struct Rng(u64);

impl Rng {
    /// Returns a value in `0..bound` (zero when `bound` is zero).
    fn below(&mut self, bound: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        if bound == 0 {
            0
        } else {
            usize::try_from((self.0 >> 33) % u64::try_from(bound).unwrap_or(1)).unwrap_or(0)
        }
    }

    /// Returns a random string of up to `max` characters, sometimes empty, sometimes long.
    fn text(&mut self, max: usize) -> String {
        let length = match self.below(8) {
            0 => 0,
            1 => self.below(max * 20 + 1),
            _ => self.below(max + 1),
        };
        (0..length).map(|_| POOL[self.below(POOL.len())]).collect()
    }

    /// Returns a number that is often small and sometimes enormous.
    fn number(&mut self) -> u64 {
        match self.below(6) {
            0 => 0,
            1 => u64::MAX,
            2 => u64::MAX - u64::try_from(self.below(10)).unwrap_or(0),
            _ => u64::try_from(self.below(100_000)).unwrap_or(0),
        }
    }

    /// Returns a 32-bit number that is often small and sometimes the maximum.
    fn small(&mut self) -> u32 {
        u32::try_from(self.number().min(u64::from(u32::MAX))).unwrap_or(0)
    }
}

/// Generates the graph of a random report, with invalid, duplicated and reflexive edges.
fn random_graph(rng: &mut Rng) -> Graph {
    let node_count = rng.below(60);
    let nodes: Vec<GraphNode> = (0..node_count)
        .map(|_| GraphNode {
            id: rng.text(12),
            label: rng.text(30),
            group: rng.text(6),
            weight: rng.small(),
        })
        .collect();
    let edges = (0..rng.below(200))
        .map(|_| GraphEdge {
            from: if rng.below(10) == 0 {
                rng.below(1_000_000)
            } else {
                rng.below(node_count + 1)
            },
            to: if rng.below(10) == 0 {
                usize::MAX
            } else {
                rng.below(node_count + 1)
            },
            weight: rng.small(),
            kind: rng.text(8),
            confidence: match rng.below(4) {
                0 => "guess".into(),
                1 => "heuristic".into(),
                2 => "exact".into(),
                _ => rng.text(8),
            },
        })
        .collect();
    Graph { nodes, edges }
}

/// Generates a random report.
fn random_report(seed: u64) -> ReportData {
    let mut rng = Rng(seed);
    let graph = random_graph(&mut rng);
    ReportData {
        project: rng.text(40),
        generated_on: rng.text(12),
        tool_version: rng.text(8),
        summary: Summary {
            files: rng.number(),
            lines: rng.number(),
            symbols: rng.number(),
            edges: rng.number(),
            parse_error_files: rng.number(),
            memories: rng.number(),
            stale_memories: rng.number(),
        },
        languages: (0..rng.below(20))
            .map(|_| LanguageRow {
                name: rng.text(12),
                files: rng.number(),
                symbols: rng.number(),
            })
            .collect(),
        modules: (0..rng.below(30))
            .map(|_| ModuleRow {
                name: rng.text(20),
                files: rng.number(),
                symbols: rng.number(),
                incoming: rng.number(),
                outgoing: rng.number(),
            })
            .collect(),
        hotspots: (0..rng.below(60))
            .map(|_| Hotspot {
                name: rng.text(20),
                kind: rng.text(8),
                path: rng.text(30),
                line: rng.small(),
                callers: rng.small(),
                callees: rng.small(),
            })
            .collect(),
        documentation: DocCoverage {
            public_symbols: rng.number(),
            documented: rng.number(),
            by_language: (0..rng.below(10))
                .map(|_| DocLanguageRow {
                    name: rng.text(10),
                    public_symbols: rng.number(),
                    documented: rng.number(),
                })
                .collect(),
            undocumented_examples: (0..rng.below(20))
                .map(|_| UndocumentedRow {
                    name: rng.text(20),
                    path: rng.text(30),
                    line: rng.small(),
                })
                .collect(),
        },
        graph,
        memories: (0..rng.below(40))
            .map(|_| MemoryRow {
                id: i64::try_from(rng.number()).unwrap_or(i64::MIN),
                kind: rng.text(8),
                provenance: rng.text(8),
                stale: rng.below(2) == 0,
                text: rng.text(80),
            })
            .collect(),
        usage: (rng.below(3) != 0).then(|| Usage {
            recalls: rng.number(),
            expands: rng.number(),
            impacts: rng.number(),
            remembers: rng.number(),
            tokens_served_total: rng.number(),
            avg_tokens_served: rng.number(),
            avg_budget_percent: rng.small(),
        }),
        notes: (0..rng.below(8)).map(|_| rng.text(60)).collect(),
    }
}

/// Every output of 60 random reports, in both languages, passes its validator.
#[test]
fn random_reports_render_valid_outputs() {
    for seed in 0..60u64 {
        let data = random_report(seed);
        for lang in [Lang::En, Lang::Es] {
            let html = render_html(&data, lang);
            check_markup(&html, Mode::Html)
                .unwrap_or_else(|e| panic!("seed {seed}: invalid HTML: {e}"));
            let pdf = render_pdf(&data, lang);
            check_pdf(&pdf).unwrap_or_else(|e| panic!("seed {seed}: invalid PDF: {e}"));
            let markdown = render_markdown(&data, lang);
            assert!(markdown.starts_with("# "), "seed {seed}");
            assert!(!markdown.contains('\u{0}'), "seed {seed}");
            for format in GraphFormat::ALL {
                let text = render_graph(&data.graph, format, lang);
                match format {
                    GraphFormat::Json => {
                        parse(&text).unwrap_or_else(|e| panic!("seed {seed}: invalid JSON: {e}"));
                    }
                    GraphFormat::Svg => {
                        check_markup(&text, Mode::Xml)
                            .unwrap_or_else(|e| panic!("seed {seed}: invalid SVG: {e}"));
                    }
                    GraphFormat::Mermaid => {
                        assert!(text.starts_with("flowchart LR\n"), "seed {seed}");
                    }
                    GraphFormat::Dot => assert!(text.starts_with("digraph "), "seed {seed}"),
                }
            }
        }
    }
}

/// The same seed always gives the same outputs, byte for byte.
#[test]
fn random_reports_are_deterministic() {
    for seed in [1u64, 7, 42, 99] {
        let data = random_report(seed);
        assert_eq!(
            render_pdf(&data, Lang::Es),
            render_pdf(&random_report(seed), Lang::Es)
        );
        assert_eq!(
            render_html(&data, Lang::En),
            render_html(&random_report(seed), Lang::En)
        );
    }
}
