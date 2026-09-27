// SPDX-License-Identifier: Apache-2.0
//! The code graph as SVG, both embedded in the HTML report and as a standalone image.
//!
//! The drawing is produced from a [`Prepared`] graph and a [`Layout`]: edges as thin paths with a
//! filled arrowhead (dashed when the indexer was unsure), nodes as circles colored by group, and
//! labels with a halo so they stay readable over lines. All colors come from CSS variables, so
//! the embedded figure follows the light or dark theme of the page and the standalone file
//! follows the theme of the viewer.
//!
//! Invariants: every piece of user text is escaped; the output contains no script, no external
//! reference and no `href`; identical input gives identical bytes.

use std::fmt::Write as _;

use super::layout::{EdgeShape, LABEL_FONT, Layout, edge_shapes};
use super::palette;
use super::prepare::{Prepared, short_label};
use crate::i18n::{Key, t, tf};
use crate::lang::Lang;
use crate::numfmt::{self, coord};
use crate::text::{push_escaped, single_line};

/// Rules that style the graph, shared by the embedded and the standalone drawing.
///
/// An edge is a **filled** shape rather than a stroked line, because it is drawn tapered: wide
/// where it leaves its source and narrowing to a point where it arrives. That taper is what shows
/// the direction of a call, which is why there is no arrowhead to draw, and it is what makes a
/// dense graph readable — hundreds of arrowheads are noise, hundreds of tapers are a texture.
///
/// An edge takes the color of the node it leaves, so a connection can be traced back to its source
/// by color alone, and an uncertain edge is drawn fainter rather than dashed, since a dash pattern
/// has nothing to follow along a shape that is filled.
pub(crate) const GRAPH_CSS: &str = "\
.graph .e{stroke:none;fill-opacity:.5}\
.graph .e.d{fill-opacity:.24}\
.graph .b{fill-opacity:.7;stroke:none}\
.graph .glow{fill-opacity:.05;stroke:none}\
.graph .ring{fill-opacity:.07;stroke:none}\
.graph .core{fill-opacity:.11;stroke:none}\
.graph .n{stroke:var(--node-stroke);stroke-width:1.25}\
.graph .l{font:11px system-ui,-apple-system,\"Segoe UI\",Roboto,Helvetica,Arial,sans-serif;\
fill:var(--graph-text);text-anchor:middle;paint-order:stroke;stroke:var(--halo);\
stroke-width:3px;stroke-linejoin:round}\
.graph .note{font:12px system-ui,-apple-system,\"Segoe UI\",Roboto,Helvetica,Arial,sans-serif;\
fill:var(--graph-text)}\
.graph .bg{fill:var(--halo)}";

/// Returns the CSS variable declarations of the graph for the light theme.
pub(crate) fn vars_light() -> String {
    let mut out =
        String::from("--edge:#64748b;--node-stroke:#ffffff;--halo:#ffffff;--graph-text:#14181f;");
    for index in 0..palette::COLORS {
        let _ = write!(out, "--g{index}:{};", palette::main(index));
    }
    out
}

/// Returns the CSS variable declarations of the graph for the dark theme.
pub(crate) fn vars_dark() -> String {
    let mut out =
        String::from("--edge:#94a3b8;--node-stroke:#0d1117;--halo:#0d1117;--graph-text:#e6edf3;");
    for index in 0..palette::COLORS {
        let _ = write!(out, "--g{index}:{};", palette::dark(index));
    }
    out
}

/// Returns the CSS rules that map the classes `g0`..`g9` to the group colors.
pub(crate) fn group_class_css() -> String {
    let mut out = String::new();
    for index in 0..palette::COLORS {
        let _ = write!(out, ".graph .g{index}{{fill:var(--g{index})}}");
    }
    out
}

/// Returns the accessible name of a graph.
pub(crate) fn alt_text(prep: &Prepared<'_>, lang: Lang) -> String {
    let nodes = numfmt::int(u64::try_from(prep.nodes.len()).unwrap_or(u64::MAX), lang);
    let edges = numfmt::int(u64::try_from(prep.edges.len()).unwrap_or(u64::MAX), lang);
    tf(lang, Key::GraphAlt, &[&nodes, &edges])
}

/// Renders the graph as an inline `<svg>` element for the HTML report.
pub(crate) fn embedded(prep: &Prepared<'_>, layout: &Layout, lang: Lang) -> String {
    let alt = alt_text(prep, lang);
    let mut out = String::with_capacity(4096 + prep.edges.len() * 200);
    let _ = write!(
        out,
        "<svg class=\"graph\" viewBox=\"0 0 {} {}\" role=\"img\" aria-label=\"",
        coord(layout.width),
        coord(layout.height)
    );
    push_escaped(&mut out, &alt);
    out.push_str("\"><title>");
    push_escaped(&mut out, &alt);
    out.push_str("</title>");
    body(&mut out, prep, layout);
    out.push_str("</svg>");
    out
}

/// Renders the graph as a standalone SVG document.
///
/// The image carries its own style sheet with a dark variant, an `xmlns` declaration and a
/// background, so it displays correctly when opened on its own. When nodes were left out, a
/// "+N more nodes not shown" line is added at the bottom.
pub(crate) fn standalone(prep: &Prepared<'_>, layout: &Layout, lang: Lang) -> String {
    let alt = alt_text(prep, lang);
    let hidden = prep.hidden_nodes();
    let (width, mut height) = (layout.width, layout.height);
    if prep.nodes.is_empty() {
        height = height.max(120.0);
    }
    let width = if prep.nodes.is_empty() {
        width.max(400.0)
    } else {
        width
    };
    let legend = legend_layout(prep, lang, width);
    let legend_top = height;
    height += legend.height;
    let note_height = if hidden > 0 { 26.0 } else { 0.0 };
    height += note_height;

    let mut out = String::with_capacity(8192 + prep.edges.len() * 200);
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    let _ = write!(
        out,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" class=\"graph\" viewBox=\"0 0 {w} {h}\" \
         width=\"{w}\" height=\"{h}\" role=\"img\" aria-label=\"",
        w = coord(width),
        h = coord(height)
    );
    push_escaped(&mut out, &alt);
    out.push_str("\"><title>");
    push_escaped(&mut out, &alt);
    out.push_str("</title><style>");
    let _ = write!(out, ":root{{{}}}", vars_light());
    let _ = write!(
        out,
        "@media (prefers-color-scheme:dark){{:root{{{}}}}}",
        vars_dark()
    );
    out.push_str(GRAPH_CSS);
    out.push_str(&group_class_css());
    out.push_str("</style>");
    let _ = write!(
        out,
        "<rect class=\"bg\" width=\"{}\" height=\"{}\"/>",
        coord(width),
        coord(height)
    );
    if prep.nodes.is_empty() {
        let _ = write!(
            out,
            "<text class=\"note\" x=\"{}\" y=\"{}\" text-anchor=\"middle\">",
            coord(width / 2.0),
            coord(height / 2.0)
        );
        push_escaped(&mut out, t(lang, Key::NoData));
        out.push_str("</text>");
    } else {
        body(&mut out, prep, layout);
    }
    write_legend(&mut out, &legend, legend_top);
    if hidden > 0 {
        let count = numfmt::int(u64::try_from(hidden).unwrap_or(u64::MAX), lang);
        let _ = write!(
            out,
            "<text class=\"note\" x=\"12\" y=\"{}\">",
            coord(height - 10.0)
        );
        push_escaped(&mut out, &tf(lang, Key::GraphMore, &[&count]));
        out.push_str("</text>");
    }
    out.push_str("</svg>\n");
    out
}

/// The legend of a standalone image: one entry per group, flowing over several rows.
struct Legend {
    /// `(group index, name, x, row)` of every entry.
    entries: Vec<(usize, String, f64, usize)>,
    /// Height the legend adds below the drawing.
    height: f64,
}

/// Height of one legend row.
const LEGEND_ROW: f64 = 22.0;

/// Places the legend entries in rows no wider than `width`; there is no legend for a graph
/// without named groups.
fn legend_layout(prep: &Prepared<'_>, lang: Lang, width: f64) -> Legend {
    let named = prep.groups.len() >= 2 || prep.groups.iter().any(|group| !group.is_empty());
    let mut entries = Vec::new();
    let (mut x, mut row) = (14.0, 0usize);
    if named {
        for (index, group) in prep.groups.iter().enumerate().take(20) {
            let name = group_display(group, lang);
            let entry_width = 26.0 + numfmt::usize_to_f64(name.chars().count()) * 6.6 + 18.0;
            if x + entry_width > width - 12.0 && x > 14.0 {
                x = 14.0;
                row += 1;
            }
            entries.push((index, name, x, row));
            x += entry_width;
        }
    }
    let height = if entries.is_empty() {
        0.0
    } else {
        numfmt::usize_to_f64(row + 1) * LEGEND_ROW + 8.0
    };
    Legend { entries, height }
}

/// Writes the legend below the drawing, starting at `top`.
fn write_legend(out: &mut String, legend: &Legend, top: f64) {
    if legend.entries.is_empty() {
        return;
    }
    out.push_str("<g class=\"legend\">");
    for (index, name, x, row) in &legend.entries {
        let y = top + 4.0 + numfmt::usize_to_f64(*row) * LEGEND_ROW + LEGEND_ROW / 2.0;
        let _ = write!(
            out,
            "<circle class=\"n g{}\" cx=\"{}\" cy=\"{}\" r=\"5\"/><text class=\"note\" x=\"{}\" y=\"{}\">",
            index % palette::COLORS,
            coord(*x + 5.0),
            coord(y),
            coord(*x + 16.0),
            coord(y + 4.0)
        );
        push_escaped(out, name);
        out.push_str("</text>");
    }
    out.push_str("</g>");
}

/// How much wider than its nominal width an edge is where it leaves its source.
///
/// The whole width of the shape is spent at the base and none at the tip, so the base has to be
/// wider than a stroked line of the same weight would be for the edge to carry the same visual
/// weight over its length.
const BASE_WIDTH: f64 = 1.05;

/// The radius of the terminal drawn where an edge meets the node it points at.
const TERMINAL_RADIUS: f64 = 1.2;

/// How much of the terminal's radius scales with the weight of its edge.
const TERMINAL_GROWTH: f64 = 0.45;

/// The three halos under a node, as multiples of its radius, widest first.
///
/// Three flat discs rather than two: with two, the step between them is visible as a hard ring,
/// and a halo that shows its own edges reads as a mistake rather than as light. Three steps at
/// falling opacity are enough for the eye to stop finding the boundaries.
const HALOS: [(f64, &str); 3] = [(2.6, "glow"), (1.9, "ring"), (1.4, "core")];

/// Normalizes a vector; a zero vector gives `(1, 0)` so that a degenerate edge still has a
/// direction to be drawn along instead of collapsing.
fn unit(dx: f64, dy: f64) -> (f64, f64) {
    let length = dx.hypot(dy);
    if length < 1.0e-9 {
        (1.0, 0.0)
    } else {
        (dx / length, dy / length)
    }
}

/// Writes the outline of one tapered fibre.
///
/// The shape is two cubics sharing their control points: one down each side of the fibre from the
/// base to the tip. They bound a form that is `2 * half` wide where it leaves the soma and comes to
/// a point at the cleft, which is the taper the whole drawing depends on.
fn write_edge_path(out: &mut String, shape: &EdgeShape) {
    let [start, c1, c2, tip] = shape.spine();
    let (ux, uy) = unit(c1.x - start.x, c1.y - start.y);
    let (nx, ny) = (-uy, ux);
    let half = shape.width * BASE_WIDTH;
    let _ = write!(
        out,
        "M{} {}C{} {} {} {} {} {}C{} {} {} {} {} {}Z",
        coord(start.x + nx * half),
        coord(start.y + ny * half),
        coord(c1.x + nx * half * 0.45),
        coord(c1.y + ny * half * 0.45),
        coord(c2.x),
        coord(c2.y),
        coord(tip.x),
        coord(tip.y),
        coord(c2.x),
        coord(c2.y),
        coord(c1.x - nx * half * 0.45),
        coord(c1.y - ny * half * 0.45),
        coord(start.x - nx * half),
        coord(start.y - ny * half)
    );
}

/// Writes the edges, nodes and labels of the drawing.
fn body(out: &mut String, prep: &Prepared<'_>, layout: &Layout) {
    let shapes = edge_shapes(prep, layout);
    out.push_str("<g class=\"edges\">");
    for (edge, shape) in prep.edges.iter().zip(&shapes) {
        let Some(shape) = shape else { continue };
        // `prepare` drops any edge whose endpoints are outside the node list, so both of these
        // always resolve; asking for them rather than indexing keeps that a fact about this loop
        // instead of an assumption about another module.
        let (Some(source), Some(target)) = (prep.nodes.get(edge.from), prep.nodes.get(edge.to))
        else {
            continue;
        };
        let group = source.group_index % palette::COLORS;
        let _ = write!(
            out,
            "<path class=\"e g{group}{}\" d=\"",
            if shape.dashed { " d" } else { "" }
        );
        write_edge_path(out, shape);
        let _ = write!(
            out,
            "\"><title>{} \u{2192} {}</title></path>",
            crate::text::escaped(&short_label(&source.label)),
            crate::text::escaped(&short_label(&target.label))
        );
        // The terminal sits in the cleft the fibre stops short of: the swelling at the end of a
        // process, on the near side of the gap it signals across.
        let [_, _, approach, tip] = shape.spine();
        let radius = TERMINAL_RADIUS + shape.width * TERMINAL_GROWTH;
        let (ox, oy) = unit(approach.x - tip.x, approach.y - tip.y);
        let _ = write!(
            out,
            "<circle class=\"b g{group}\" cx=\"{}\" cy=\"{}\" r=\"{}\"/>",
            coord(tip.x + ox * radius * 0.35),
            coord(tip.y + oy * radius * 0.35),
            coord(radius)
        );
    }
    out.push_str("</g><g class=\"nodes\">");
    // Faint discs under each node. They are plain circles rather than a blur filter: a filter is
    // one more thing a viewer has to support, it is the slowest part of a drawing this size, and
    // flat discs of the node's own colour already read as light coming off it.
    for (node, placed) in prep.nodes.iter().zip(&layout.nodes) {
        let group = node.group_index % palette::COLORS;
        for (scale, class) in HALOS {
            let _ = write!(
                out,
                "<circle class=\"{class} g{group}\" cx=\"{}\" cy=\"{}\" r=\"{}\"/>",
                coord(placed.at.x),
                coord(placed.at.y),
                coord(placed.r * scale)
            );
        }
    }
    for (node, placed) in prep.nodes.iter().zip(&layout.nodes) {
        let _ = write!(
            out,
            "<circle class=\"n g{}\" cx=\"{}\" cy=\"{}\" r=\"{}\"><title>",
            node.group_index % palette::COLORS,
            coord(placed.at.x),
            coord(placed.at.y),
            coord(placed.r)
        );
        push_escaped(out, &node.label);
        if !node.group.is_empty() {
            out.push_str(" (");
            push_escaped(out, &node.group);
            out.push(')');
        }
        let _ = write!(out, ", {}", node.raw.weight);
        out.push_str("</title></circle>");
    }
    out.push_str("</g><g class=\"labels\">");
    for (node, placed) in prep.nodes.iter().zip(&layout.nodes) {
        if !placed.labeled {
            continue;
        }
        let _ = write!(
            out,
            "<text class=\"l\" x=\"{}\" y=\"{}\">",
            coord(placed.at.x),
            coord(placed.at.y + placed.r + LABEL_FONT + 1.0)
        );
        push_escaped(out, &short_label(&node.label));
        out.push_str("</text>");
    }
    out.push_str("</g>");
}

/// Returns the group name to show in a legend: the localized placeholder for the empty group.
pub(crate) fn group_display(group: &str, lang: Lang) -> String {
    if group.is_empty() {
        t(lang, Key::GraphUngrouped).to_owned()
    } else {
        single_line(group, 60)
    }
}

#[cfg(test)]
mod tests {
    use super::{HALOS, alt_text, embedded, group_display, standalone};
    use crate::graph::layout::layout;
    use crate::graph::prepare::prepare;
    use crate::lang::Lang;
    use crate::model::{Graph, GraphEdge, GraphNode};

    /// A small graph with hostile text in labels and groups.
    fn hostile() -> Graph {
        let node = |label: &str, group: &str, weight| GraphNode {
            id: label.into(),
            label: label.into(),
            group: group.into(),
            weight,
        };
        Graph {
            nodes: vec![
                node("<script>alert(1)</script>", "a\"b'c", 5),
                node("x & y", "g2", 3),
                node("ñandú 🙂", "", 1),
            ],
            edges: vec![
                GraphEdge {
                    from: 0,
                    to: 1,
                    weight: 4,
                    ..GraphEdge::default()
                },
                GraphEdge {
                    from: 1,
                    to: 2,
                    weight: 1,
                    confidence: "guess".into(),
                    ..GraphEdge::default()
                },
            ],
        }
    }

    /// The embedded drawing is accessible, escaped and free of scripts and external references.
    #[test]
    fn embedded_svg_is_safe_and_accessible() {
        let graph = hostile();
        let prep = prepare(&graph, 40, 120);
        let svg = embedded(&prep, &layout(&prep), Lang::En);
        assert!(svg.starts_with("<svg class=\"graph\" viewBox=\"0 0 "));
        assert!(svg.contains("role=\"img\""));
        assert!(svg.contains("aria-label=\"Code graph with 3 nodes and 2 relationships\""));
        assert!(svg.contains("<title>Code graph with 3 nodes and 2 relationships</title>"));
        assert!(!svg.contains("<script"));
        assert!(svg.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
        assert!(svg.contains("x &amp; y"));
        assert!(svg.contains("a&quot;b&#39;c"));
        assert!(!svg.contains("http"));
        assert!(!svg.contains("href"));
        assert_eq!(
            svg.matches(" d\" d=\"").count(),
            1,
            "exactly the uncertain edge carries the faint class"
        );
        // Three nodes, each with its three halos and its own circle, plus one terminal for each
        // edge that could be drawn.
        let terminals = svg.matches("class=\"b g").count();
        assert_eq!(
            svg.matches("<circle").count(),
            3 * HALOS.len() + 3 + terminals
        );
        assert_eq!(svg.matches("<text").count(), 3);
    }

    /// The Spanish drawing has a Spanish accessible name.
    #[test]
    fn embedded_svg_is_localized() {
        let graph = hostile();
        let prep = prepare(&graph, 40, 120);
        assert_eq!(
            alt_text(&prep, Lang::Es),
            "Grafo de código con 3 nodos y 2 relaciones"
        );
    }

    /// The standalone file has an XML declaration, a namespace, a style sheet and a background.
    #[test]
    fn standalone_svg_is_self_contained() {
        let graph = hostile();
        let prep = prepare(&graph, 150, 600);
        let svg = standalone(&prep, &layout(&prep), Lang::En);
        assert!(svg.starts_with(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\""
        ));
        assert!(svg.contains("prefers-color-scheme:dark"));
        assert!(svg.contains("<rect class=\"bg\""));
        assert!(svg.trim_end().ends_with("</svg>"));
        assert!(!svg.contains("<script"));
    }

    /// A capped graph says how many nodes were left out, in both languages.
    #[test]
    fn standalone_svg_reports_hidden_nodes() {
        let nodes: Vec<GraphNode> = (0..20)
            .map(|i| GraphNode {
                id: i.to_string(),
                label: i.to_string(),
                group: String::new(),
                weight: i,
            })
            .collect();
        let graph = Graph {
            nodes,
            edges: vec![],
        };
        let prep = prepare(&graph, 8, 10);
        let placed = layout(&prep);
        assert!(standalone(&prep, &placed, Lang::En).contains("+12 more nodes not shown"));
        assert!(standalone(&prep, &placed, Lang::Es).contains("+12 nodos más sin mostrar"));
    }

    /// A standalone image with named groups carries a legend with one entry per group.
    #[test]
    fn standalone_svg_has_a_legend() {
        let graph = hostile();
        let prep = prepare(&graph, 150, 600);
        let svg = standalone(&prep, &layout(&prep), Lang::Es);
        assert!(svg.contains("<g class=\"legend\">"));
        assert!(svg.contains("Sin grupo"));
        assert!(svg.contains("g2"));
        let ungrouped = Graph {
            nodes: vec![GraphNode {
                id: "a".into(),
                label: "a".into(),
                ..GraphNode::default()
            }],
            edges: vec![],
        };
        let prep = prepare(&ungrouped, 150, 600);
        assert!(!standalone(&prep, &layout(&prep), Lang::En).contains("legend"));
    }

    /// An empty graph still yields a valid image with a localized message.
    #[test]
    fn empty_graph_svg() {
        let graph = Graph::default();
        let prep = prepare(&graph, 150, 600);
        let svg = standalone(&prep, &layout(&prep), Lang::Es);
        assert!(svg.contains("Sin datos."));
        assert!(svg.contains("viewBox=\"0 0 400 120\""));
    }

    /// The same input gives the same bytes.
    #[test]
    fn svg_is_deterministic() {
        let graph = hostile();
        let prep = prepare(&graph, 40, 120);
        let placed = layout(&prep);
        assert_eq!(
            embedded(&prep, &placed, Lang::En),
            embedded(&prep, &placed, Lang::En)
        );
        assert_eq!(
            standalone(&prep, &placed, Lang::Es),
            standalone(&prep, &placed, Lang::Es)
        );
    }

    /// The empty group displays as a localized placeholder and long names are clipped.
    #[test]
    fn group_names_display() {
        assert_eq!(group_display("", Lang::En), "Ungrouped");
        assert_eq!(group_display("", Lang::Es), "Sin grupo");
        assert_eq!(group_display("core", Lang::En), "core");
        assert_eq!(
            group_display(&"x".repeat(200), Lang::En).chars().count(),
            61
        );
    }
}
