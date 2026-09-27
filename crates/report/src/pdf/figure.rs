// SPDX-License-Identifier: Apache-2.0
//! The code graph drawn as vector graphics inside the PDF.
//!
//! The figure reuses the layout and the edge geometry of the SVG (`graph::layout`), scaled to a
//! fixed frame: edges as lines or curves with filled arrowheads (dashed when uncertain), nodes as
//! circles colored by group, and labels under the heaviest nodes. A legend of group colors
//! follows the frame.
//!
//! Invariants: nothing is drawn outside the frame; the drawing is deterministic because the
//! layout is.

use super::canvas::Color;
use super::doc::{CONTENT_W, Doc, MARGIN_X};
use super::fonts::{Font, encode, fit, text_width};
use crate::graph::layout::{LABEL_FONT, Layout, edge_shapes};
use crate::graph::palette;
use crate::graph::prepare::{Prepared, short_label};

/// Height of the figure frame.
pub(super) const FRAME_HEIGHT: f64 = 330.0;
/// Inner padding of the frame.
const FRAME_PAD: f64 = 8.0;

impl Doc {
    /// Draws the graph figure. `legend` lists `(color index, name)` pairs.
    pub(super) fn graph_figure(
        &mut self,
        prep: &Prepared<'_>,
        placed: &Layout,
        legend: &[(usize, String)],
    ) {
        self.ensure(FRAME_HEIGHT + 30.0);
        let theme = self.theme;
        let top = self.y;
        self.page.fill_rect(
            MARGIN_X,
            top,
            CONTENT_W,
            FRAME_HEIGHT,
            Color::hex("#fbfcfd"),
        );
        self.page
            .stroke_rect(MARGIN_X, top, CONTENT_W, FRAME_HEIGHT, theme.border, 0.6);

        let avail_w = CONTENT_W - 2.0 * FRAME_PAD;
        let avail_h = FRAME_HEIGHT - 2.0 * FRAME_PAD;
        let scale = (avail_w / placed.width.max(1.0)).min(avail_h / placed.height.max(1.0));
        let origin_x = MARGIN_X + FRAME_PAD + (avail_w - placed.width * scale) / 2.0;
        let origin_y = top + FRAME_PAD + (avail_h - placed.height * scale) / 2.0;
        let map = |x: f64, y: f64| (origin_x + x * scale, origin_y + y * scale);
        let edge_color = Color::hex("#8391a4");

        for shape in edge_shapes(prep, placed).into_iter().flatten() {
            let width = (shape.width * scale).max(0.35);
            self.page.set_dashed(shape.dashed);
            // The page is a print of the same drawing, so it follows the same spine: out along the
            // bundle, in along the approach, stopping at the cleft. A print cannot taper a stroke,
            // so the width is the fibre's own and the terminal carries the direction instead.
            let [start, c1, c2, tip] = shape.spine();
            self.page.cubic(
                map(start.x, start.y),
                map(c1.x, c1.y),
                map(c2.x, c2.y),
                map(tip.x, tip.y),
                edge_color,
                width,
            );
            self.page.set_dashed(false);
            self.page
                .circle(map(tip.x, tip.y), width * 1.1, edge_color, edge_color, 0.0);
        }

        let white = Color(1.0, 1.0, 1.0);
        for (node, at) in prep.nodes.iter().zip(&placed.nodes) {
            let center = map(at.at.x, at.at.y);
            let fill = Color::hex(palette::main(node.group_index));
            self.page.circle(center, at.r * scale, fill, white, 0.6);
        }
        let font_size = (LABEL_FONT * scale).clamp(5.5, 8.0);
        for (node, at) in prep.nodes.iter().zip(&placed.nodes) {
            if !at.labeled {
                continue;
            }
            let center = map(at.at.x, at.at.y);
            let label = fit(
                Font::Regular,
                &encode(&short_label(&node.label)),
                font_size,
                110.0,
            );
            let width = text_width(Font::Regular, &label, font_size);
            let baseline = center.1 + at.r * scale + font_size + 0.5;
            let x =
                (center.0 - width / 2.0).clamp(MARGIN_X + 2.0, MARGIN_X + CONTENT_W - width - 2.0);
            self.page.text_halo(
                (x, baseline),
                Font::Regular,
                font_size,
                theme.ink,
                Color(1.0, 1.0, 1.0),
                &label,
            );
        }
        self.y = top + FRAME_HEIGHT + 10.0;
        self.legend(legend);
    }

    /// Draws the group legend as flowing colored dots with names.
    fn legend(&mut self, legend: &[(usize, String)]) {
        let theme = self.theme;
        let mut x = MARGIN_X;
        self.ensure(14.0);
        for (index, name) in legend {
            let label = fit(Font::Regular, &encode(name), 8.0, 150.0);
            let width = 12.0 + text_width(Font::Regular, &label, 8.0) + 16.0;
            if x + width > MARGIN_X + CONTENT_W && x > MARGIN_X {
                x = MARGIN_X;
                self.y += 13.0;
                self.ensure(14.0);
            }
            self.page.circle(
                (x + 4.0, self.y + 6.0),
                3.6,
                Color::hex(palette::main(*index)),
                Color(1.0, 1.0, 1.0),
                0.4,
            );
            self.page.text(
                x + 12.0,
                Self::baseline(self.y, 12.0, 8.0),
                Font::Regular,
                8.0,
                theme.ink,
                &label,
            );
            x += width;
        }
        self.y += 18.0;
    }
}

#[cfg(test)]
mod tests {
    use super::FRAME_HEIGHT;
    use crate::graph::layout::layout;
    use crate::graph::prepare::prepare;
    use crate::model::{Graph, GraphEdge, GraphNode};
    use crate::pdf::doc::Doc;

    /// Builds a graph of `count` nodes in two groups, connected in a ring, one edge uncertain.
    fn ring(count: usize) -> Graph {
        let nodes = (0..count)
            .map(|i| GraphNode {
                id: i.to_string(),
                label: format!("Node {i}"),
                group: (i % 2).to_string(),
                weight: u32::try_from(i + 1).unwrap_or(1),
            })
            .collect();
        let edges = (0..count)
            .map(|i| GraphEdge {
                from: i,
                to: (i + 1) % count,
                weight: 1 + u32::try_from(i).unwrap_or(0),
                kind: "calls".into(),
                confidence: if i == 0 {
                    "guess".into()
                } else {
                    "exact".into()
                },
            })
            .collect();
        Graph { nodes, edges }
    }

    /// The figure draws one circle per node plus one per legend entry, labels with halos, edges
    /// with arrowheads, and moves the cursor below the frame.
    #[test]
    fn figure_draws_nodes_edges_labels_and_legend() {
        let graph = ring(6);
        let prep = prepare(&graph, 40, 120);
        let placed = layout(&prep);
        let legend = vec![(0, "zero".to_owned()), (1, "one".to_owned())];
        let mut doc = Doc::new();
        let start = doc.y;
        doc.graph_figure(&prep, &placed, &legend);
        assert!(doc.y - start > FRAME_HEIGHT);
        let page = String::from_utf8_lossy(&doc.page.clone().into_bytes()).into_owned();
        // Six somas, two legend swatches, and the terminal that sits in the cleft of each of the
        // six fibres: every one of them is a filled and stroked circle.
        assert_eq!(page.matches(" c B\n").count(), 6 + 2 + 6);
        assert_eq!(page.matches("1 Tr").count(), 6, "one halo per labeled node");
        assert!(page.contains("(Node 3)"));
        assert!(page.contains("[3 2] 0 d"), "the uncertain edge is dashed");
        // Each fibre is one stroked cubic. There are no arrowheads to draw: the direction is
        // carried by where the fibre stops and by the terminal it stops at.
        assert_eq!(
            page.matches(" c S\n").count(),
            6,
            "one stroked cubic per fibre"
        );
    }

    /// A single node and an empty legend still draw.
    #[test]
    fn single_node_figure() {
        let graph = ring(1);
        let prep = prepare(&graph, 40, 120);
        let placed = layout(&prep);
        let mut doc = Doc::new();
        doc.graph_figure(&prep, &placed, &[]);
        let page = String::from_utf8_lossy(&doc.page.clone().into_bytes()).into_owned();
        assert_eq!(page.matches(" c B\n").count(), 1);
    }
}
