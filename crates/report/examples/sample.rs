// SPDX-License-Identifier: Apache-2.0
//! Writes a realistic sample of every report format into a directory.
//!
//! Usage: `cargo run -p pn-ultramemory-report --example sample -- <output-dir>`
//!
//! The data describes a fictional project (a small invoicing service) with 12 modules, 6
//! languages, 40 hotspots, a 30-node graph in 5 groups, memories (some stale) and usage numbers.
//! The example writes `report-en.html`, `report-es.html`, `report-en.pdf`, `report-es.pdf`,
//! `report-en.md`, `report-es.md`, and the four graph formats in both languages as
//! `graph-en.mmd`, `graph-en.dot`, `graph-en.svg`, `graph-en.json` and the Spanish counterparts.
//! It also prints how long each render took and how big each file is.

use std::path::Path;
use std::time::Instant;

use pn_ultramemory_report::{
    DocCoverage, DocLanguageRow, Graph, GraphEdge, GraphFormat, GraphNode, Hotspot, Lang,
    LanguageRow, MemoryRow, ModuleRow, ReportData, Summary, UndocumentedRow, Usage, render_graph,
    render_html, render_markdown, render_pdf,
};

/// The modules of the sample project: name, files, symbols, incoming and outgoing relationships.
const MODULES: [(&str, u64, u64, u64, u64); 12] = [
    ("api/http", 48, 612, 90, 410),
    ("api/graphql", 22, 274, 31, 188),
    ("billing/invoice", 61, 890, 402, 356),
    ("billing/tax", 19, 233, 187, 64),
    ("billing/payments", 37, 501, 265, 302),
    ("core/money", 12, 148, 511, 12),
    ("core/time", 9, 97, 338, 5),
    ("storage/sql", 33, 420, 298, 120),
    ("storage/cache", 14, 176, 143, 41),
    ("web/dashboard", 74, 1_020, 60, 540),
    ("web/components", 88, 1_310, 720, 210),
    ("tools/migrate", 11, 84, 3, 47),
];

/// The languages of the sample project: name, files, symbols.
const LANGUAGES: [(&str, u64, u64); 6] = [
    ("TypeScript", 162, 2_430),
    ("Rust", 118, 1_910),
    ("Python", 40, 522),
    ("SQL", 27, 96),
    ("Shell", 9, 41),
    ("Go", 7, 118),
];

/// The groups of the sample graph.
const GROUPS: [&str; 5] = ["api", "billing", "core", "storage", "web"];

/// Node labels of the sample graph, six per group in the order of [`GROUPS`].
const NODE_LABELS: [&str; 30] = [
    "Router",
    "Handler",
    "Schema",
    "Resolver",
    "AuthGuard",
    "RateLimit",
    "Invoice",
    "LineItem",
    "TaxRule",
    "Payment",
    "Ledger",
    "Refund",
    "Money",
    "Currency",
    "Clock",
    "Rounding",
    "Uuid",
    "Error",
    "SqlPool",
    "Migration",
    "Cache",
    "Repository",
    "Query",
    "Transaction",
    "Dashboard",
    "Chart",
    "Table",
    "Form",
    "Store",
    "Theme",
];

/// A tiny deterministic generator so the sample is the same on every run.
struct Lcg(u64);

impl Lcg {
    /// Returns the next value below `bound`.
    fn below(&mut self, bound: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) % bound.max(1)
    }
}

/// Returns the module rows of the sample.
fn modules() -> Vec<ModuleRow> {
    MODULES
        .iter()
        .map(|&(name, files, symbols, incoming, outgoing)| ModuleRow {
            name: name.into(),
            files,
            symbols,
            incoming,
            outgoing,
        })
        .collect()
}

/// Returns the language rows of the sample.
fn languages() -> Vec<LanguageRow> {
    LANGUAGES
        .iter()
        .map(|&(name, files, symbols)| LanguageRow {
            name: name.into(),
            files,
            symbols,
        })
        .collect()
}

/// Converts a small number for use as an index or count, falling back to zero.
fn small(value: u64) -> u32 {
    u32::try_from(value).unwrap_or(0)
}

/// Returns 40 hotspots spread over the modules.
fn hotspots(rng: &mut Lcg) -> Vec<Hotspot> {
    let kinds = [
        "function",
        "struct",
        "class",
        "method",
        "trait",
        "interface",
    ];
    (0..40u32)
        .map(|i| {
            let module = MODULES[usize::try_from(i % 12).unwrap_or(0)].0;
            let label = NODE_LABELS[usize::try_from(i % 30).unwrap_or(0)];
            let file = NODE_LABELS[usize::try_from((i * 7) % 30).unwrap_or(0)].to_lowercase();
            Hotspot {
                name: format!("{}::{label}", module.replace('/', "::")),
                kind: kinds[usize::try_from(i % 6).unwrap_or(0)].into(),
                path: format!("src/{module}/{file}.rs"),
                line: 10 + small(rng.below(900)),
                callers: 210 - i * 5 + small(rng.below(4)),
                callees: small(rng.below(40)),
            }
        })
        .collect()
}

/// Returns the documentation coverage of the sample.
fn documentation() -> DocCoverage {
    let by_language = [
        ("Rust", 1_120, 1_040),
        ("TypeScript", 1_610, 1_050),
        ("Python", 310, 290),
        ("Go", 140, 85),
    ]
    .iter()
    .map(|&(name, public_symbols, documented)| DocLanguageRow {
        name: name.into(),
        public_symbols,
        documented,
    })
    .collect();
    let undocumented_examples = (0..8usize)
        .map(|i| {
            let label = NODE_LABELS[24 + i % 6];
            UndocumentedRow {
                name: format!("web::components::{label}"),
                path: format!("src/web/components/{}.tsx", label.to_lowercase()),
                line: 20 + small(u64::try_from(i * 37).unwrap_or(0)),
            }
        })
        .collect();
    DocCoverage {
        public_symbols: 3_180,
        documented: 2_465,
        by_language,
        undocumented_examples,
    }
}

/// Returns the 30-node graph in 5 groups.
fn graph(rng: &mut Lcg) -> Graph {
    let nodes: Vec<GraphNode> = NODE_LABELS
        .iter()
        .enumerate()
        .map(|(index, label)| GraphNode {
            id: format!("sym{index}"),
            label: (*label).into(),
            group: GROUPS[index / 6].into(),
            weight: 3 + small(u64::try_from((index * 13) % 40).unwrap_or(0)),
        })
        .collect();
    // Groups depend on each other in layers: web -> api -> billing -> core/storage.
    let depends_on: [&[usize]; 5] = [&[1, 2], &[2, 3], &[], &[2], &[0, 2]];
    let mut edges = Vec::new();
    for index in 0..nodes.len() {
        let group = index / 6;
        let mut targets = vec![group * 6 + (index + 1) % 6, group * 6 + (index + 2) % 6];
        for &other in depends_on[group] {
            if rng.below(3) != 0 {
                targets.push(other * 6 + (index * 5 + other) % 6);
            }
        }
        for target in targets.into_iter().filter(|&target| target != index) {
            edges.push(GraphEdge {
                from: index,
                to: target,
                weight: 1 + small(rng.below(9)),
                kind: if target / 6 == group {
                    "calls".into()
                } else {
                    "imports".into()
                },
                confidence: if rng.below(6) == 0 {
                    "heuristic".into()
                } else {
                    "exact".into()
                },
            });
        }
    }
    Graph { nodes, edges }
}

/// Returns eight memories, two of them stale.
fn memories() -> Vec<MemoryRow> {
    [
        (
            "decision",
            "user",
            false,
            "Invoices are immutable once issued; corrections are credit notes.",
        ),
        (
            "fact",
            "tool",
            false,
            "Money is stored as integer minor units, never as floating point.",
        ),
        (
            "error-fix",
            "agent",
            true,
            "Deadlock in Repository::save fixed by taking row locks in id order.",
        ),
        (
            "convention",
            "user",
            false,
            "Handlers return Result<Json<T>, ApiError>; never unwrap in request paths.",
        ),
        (
            "dead-end",
            "agent",
            true,
            "Caching tax rules per request was slower than reading them once at start.",
        ),
        (
            "fact",
            "tool",
            false,
            "Payments retry with exponential backoff, at most 5 attempts.",
        ),
        (
            "decision",
            "user",
            false,
            "Dashboard charts read from a materialized view refreshed every 5 minutes.",
        ),
        (
            "lesson",
            "agent",
            false,
            "Snapshot tests for the PDF export need a fixed clock to stay stable.",
        ),
    ]
    .iter()
    .enumerate()
    .map(|(index, &(kind, provenance, stale, text))| MemoryRow {
        id: i64::try_from(index).unwrap_or(0) + 101,
        kind: kind.into(),
        provenance: provenance.into(),
        stale,
        text: text.into(),
    })
    .collect()
}

/// Builds the sample data.
fn sample() -> ReportData {
    let mut rng = Lcg(2026);
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
            memories: 8,
            stale_memories: 2,
        },
        languages: languages(),
        modules: modules(),
        hotspots: hotspots(&mut rng),
        documentation: documentation(),
        graph: graph(&mut rng),
        memories: memories(),
        usage: Some(Usage {
            recalls: 1_284,
            expands: 402,
            impacts: 96,
            remembers: 51,
            tokens_served_total: 1_902_337,
            avg_tokens_served: 1_053,
            avg_budget_percent: 61,
        }),
        notes: vec![
            "3 files could not be parsed completely; symbols in them may be missing.".into(),
            "Generated files under src/web/generated were skipped.".into(),
        ],
    }
}

/// Writes `bytes` to `dir/name` and prints the size and the time taken to produce it.
fn write(dir: &Path, name: &str, bytes: &[u8], started: Instant) -> std::io::Result<()> {
    std::fs::write(dir.join(name), bytes)?;
    println!(
        "{name}: {} bytes, rendered in {:.1} ms",
        bytes.len(),
        started.elapsed().as_secs_f64() * 1000.0
    );
    Ok(())
}

/// Entry point: renders everything into the directory given as the first argument.
fn main() -> std::io::Result<()> {
    let Some(dir) = std::env::args().nth(1) else {
        eprintln!("usage: sample <output-dir>");
        std::process::exit(2);
    };
    let dir = Path::new(&dir);
    std::fs::create_dir_all(dir)?;
    let data = sample();
    for lang in [Lang::En, Lang::Es] {
        let code = lang.code();
        let started = Instant::now();
        let html = render_html(&data, lang);
        write(
            dir,
            &format!("report-{code}.html"),
            html.as_bytes(),
            started,
        )?;
        let started = Instant::now();
        let pdf = render_pdf(&data, lang);
        write(dir, &format!("report-{code}.pdf"), &pdf, started)?;
        let started = Instant::now();
        let markdown = render_markdown(&data, lang);
        write(
            dir,
            &format!("report-{code}.md"),
            markdown.as_bytes(),
            started,
        )?;
        for format in GraphFormat::ALL {
            let started = Instant::now();
            let text = render_graph(&data.graph, format, lang);
            write(
                dir,
                &format!("graph-{code}.{}", format.extension()),
                text.as_bytes(),
                started,
            )?;
        }
    }
    Ok(())
}
