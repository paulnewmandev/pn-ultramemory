// SPDX-License-Identifier: Apache-2.0
//! A deterministic force-directed layout of the code graph, and the geometry of its edges.
//!
//! The layout is a seeded Fruchterman-Reingold: nodes repel each other, edges pull their
//! endpoints together (heavier edges pull harder), nodes of one group are weakly attracted to
//! their group's centroid, and a mild gravity keeps disconnected parts on the page. Nodes start
//! on an ellipse where each group owns an arc proportional to its size, so groups begin apart
//! and stay recognizable. A cooling schedule with a fixed number of iterations makes the result
//! converge, and a final pass pushes overlapping circles apart. The same geometry feeds both the
//! SVG and the PDF drawing.
//!
//! Complexity: `O(I * (N^2 + E))` for `I` iterations (300 up to 60 nodes, 200 above), which is
//! about four million pair evaluations for the 150-node cap and runs in a few milliseconds.
//!
//! Determinism: no clock, no hash map and no platform math library is involved. Randomness comes
//! from a fixed-seed `SplitMix64` generator, and sine and cosine are computed with a Taylor series
//! made only of additions and multiplications, so the output is bit-identical on every platform.

use std::f64::consts::{FRAC_PI_2, PI, TAU};

use super::prepare::{Prepared, short_label};
use crate::numfmt::{to_f64, usize_to_f64};

/// Font size of node labels, in graph units.
pub(crate) const LABEL_FONT: f64 = 11.0;
/// Labels are drawn for at most this many nodes (the heaviest ones).
pub(crate) const MAX_LABELS: usize = 60;
/// Empty margin around the drawing, in graph units.
const PADDING: f64 = 24.0;
/// Fixed seed of the random generator.
const SEED: u64 = 0x5eed_c0de_2026_0925;

/// A point on the drawing.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct Point {
    /// Horizontal position.
    pub x: f64,
    /// Vertical position, growing downwards.
    pub y: f64,
}

/// A node with its final position and radius.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Placed {
    /// Center of the circle.
    pub at: Point,
    /// Radius of the circle.
    pub r: f64,
    /// Whether a label is drawn for the node.
    pub labeled: bool,
}

/// The finished layout: node positions and the size of the drawing.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Layout {
    /// Width of the drawing.
    pub width: f64,
    /// Height of the drawing.
    pub height: f64,
    /// One entry per prepared node, in the same order.
    pub nodes: Vec<Placed>,
}

/// `SplitMix64`, a tiny seeded generator with excellent statistical quality for its size.
struct SplitMix64(u64);

impl SplitMix64 {
    /// Returns the next 64 random bits.
    fn next_bits(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Returns a number in `[0, 1)` built from the upper 32 bits.
    fn unit(&mut self) -> f64 {
        f64::from(u32::try_from(self.next_bits() >> 32).unwrap_or(u32::MAX)) / 4_294_967_296.0
    }
}

/// Computes the sine of `angle` (radians) with a Taylor series, identically on every platform.
pub(crate) fn det_sin(angle: f64) -> f64 {
    let reduced = angle - TAU * (angle / TAU).round();
    // Fold into [-pi/2, pi/2], where the series converges fastest.
    let x = if reduced > FRAC_PI_2 {
        PI - reduced
    } else if reduced < -FRAC_PI_2 {
        -PI - reduced
    } else {
        reduced
    };
    let square = x * x;
    let mut term = x;
    let mut sum = x;
    for k in 1..=9u32 {
        let k = f64::from(k);
        term *= -square / ((2.0 * k) * (2.0 * k + 1.0));
        sum += term;
    }
    sum
}

/// Computes the cosine of `angle` (radians), see [`det_sin`].
pub(crate) fn det_cos(angle: f64) -> f64 {
    det_sin(angle + FRAC_PI_2)
}

/// Returns the radius of a node of the given weight.
fn node_radius(weight: u32, max_weight: u32) -> f64 {
    5.0 + 9.0 * (f64::from(weight) / f64::from(max_weight.max(1))).sqrt()
}

/// Estimates the width of a label of `chars` characters.
fn label_width(chars: usize) -> f64 {
    usize_to_f64(chars) * LABEL_FONT * 0.56
}

/// Places the nodes of `prep` and returns the layout. An empty graph gives an empty layout.
pub(crate) fn layout(prep: &Prepared<'_>) -> Layout {
    let count = prep.nodes.len();
    if count == 0 {
        return Layout {
            width: 0.0,
            height: 0.0,
            nodes: Vec::new(),
        };
    }
    let max_weight = prep.max_node_weight();
    let radii: Vec<f64> = prep
        .nodes
        .iter()
        .map(|node| node_radius(node.raw.weight, max_weight))
        .collect();
    let labeled = label_flags(prep);

    let side = (usize_to_f64(count).sqrt() * 140.0).clamp(560.0, 1800.0);
    let (frame_w, frame_h) = (side * 1.3, side);
    let mut pos = initial_positions(prep, frame_w, frame_h);
    if count > 1 {
        run_forces(prep, &mut pos, frame_w, frame_h);
        separate(&mut pos, &radii);
    }
    fit(&pos, &radii, &labeled, prep)
}

/// Marks the heaviest `MAX_LABELS` nodes, which are the ones that get a label.
fn label_flags(prep: &Prepared<'_>) -> Vec<bool> {
    let mut order: Vec<usize> = (0..prep.nodes.len()).collect();
    order.sort_by(|&a, &b| {
        prep.nodes[b]
            .raw
            .weight
            .cmp(&prep.nodes[a].raw.weight)
            .then(a.cmp(&b))
    });
    let mut flags = vec![false; prep.nodes.len()];
    for &index in order.iter().take(MAX_LABELS) {
        flags[index] = true;
    }
    flags
}

/// Puts each group on its own arc of an ellipse, with a seeded radial jitter.
fn initial_positions(prep: &Prepared<'_>, frame_w: f64, frame_h: f64) -> Vec<Point> {
    let count = prep.nodes.len();
    let center = Point {
        x: frame_w / 2.0,
        y: frame_h / 2.0,
    };
    if count == 1 {
        return vec![center];
    }
    let mut members: Vec<Vec<usize>> = vec![Vec::new(); prep.groups.len()];
    for (index, node) in prep.nodes.iter().enumerate() {
        members[node.group_index].push(index);
    }
    let mut rng = SplitMix64(SEED);
    let mut pos = vec![Point::default(); count];
    let mut start = 0.0;
    for group in &members {
        if group.is_empty() {
            continue;
        }
        let span = TAU * usize_to_f64(group.len()) / usize_to_f64(count);
        for (slot, &node) in group.iter().enumerate() {
            let angle = start + span * (usize_to_f64(slot) + 0.5) / usize_to_f64(group.len());
            let radial = 0.7 + 0.3 * rng.unit();
            pos[node] = Point {
                x: center.x + 0.36 * frame_w * radial * det_cos(angle),
                y: center.y + 0.36 * frame_h * radial * det_sin(angle),
            };
        }
        start += span;
    }
    pos
}

/// Runs the Fruchterman-Reingold iterations in place.
fn run_forces(prep: &Prepared<'_>, pos: &mut [Point], frame_w: f64, frame_h: f64) {
    let count = pos.len();
    let count_f = usize_to_f64(count);
    let k = 0.85 * (frame_w * frame_h / count_f).sqrt();
    let k_squared = k * k;
    let iterations = if count <= 60 { 300u32 } else { 200u32 };
    let start_temperature = frame_w / 10.0;
    let max_edge = to_f64(prep.max_edge_weight());
    let pulls: Vec<(usize, usize, f64)> = prep
        .edges
        .iter()
        .map(|edge| {
            (
                edge.from,
                edge.to,
                1.0 + 0.6 * (to_f64(edge.weight) / max_edge).sqrt(),
            )
        })
        .collect();
    let center = Point {
        x: frame_w / 2.0,
        y: frame_h / 2.0,
    };
    let margin = 20.0;
    let mut forces = vec![Point::default(); count];

    for step in 0..iterations {
        let progress = f64::from(step) / f64::from(iterations);
        let temperature = start_temperature * (1.0 - progress) + 0.5;
        for force in &mut forces {
            *force = Point::default();
        }
        repel(pos, &mut forces, k_squared);
        for &(a, b, pull) in &pulls {
            let dx = pos[a].x - pos[b].x;
            let dy = pos[a].y - pos[b].y;
            let dist = (dx * dx + dy * dy).sqrt().max(0.01);
            let factor = dist / k * pull;
            forces[a].x -= dx * factor;
            forces[a].y -= dy * factor;
            forces[b].x += dx * factor;
            forces[b].y += dy * factor;
        }
        cohesion(prep, pos, &mut forces, center);
        for (point, force) in pos.iter_mut().zip(&forces) {
            let length = (force.x * force.x + force.y * force.y).sqrt();
            if length > 0.0 {
                let scale = length.min(temperature) / length;
                point.x = (point.x + force.x * scale).clamp(margin, frame_w - margin);
                point.y = (point.y + force.y * scale).clamp(margin, frame_h - margin);
            }
        }
    }
}

/// Adds the pairwise repulsive forces.
fn repel(pos: &[Point], forces: &mut [Point], k_squared: f64) {
    for i in 0..pos.len() {
        for j in (i + 1)..pos.len() {
            let mut dx = pos[i].x - pos[j].x;
            let mut dy = pos[i].y - pos[j].y;
            let mut dist_squared = dx * dx + dy * dy;
            if dist_squared < 1.0e-6 {
                dx = 0.01 * (usize_to_f64(i) + 1.0);
                dy = 0.01 * (usize_to_f64(j) + 1.0);
                dist_squared = dx * dx + dy * dy;
            }
            let factor = k_squared / dist_squared;
            forces[i].x += dx * factor;
            forces[i].y += dy * factor;
            forces[j].x -= dx * factor;
            forces[j].y -= dy * factor;
        }
    }
}

/// Adds the pull towards the group centroid and the gravity towards the center.
fn cohesion(prep: &Prepared<'_>, pos: &[Point], forces: &mut [Point], center: Point) {
    let groups = prep.groups.len();
    let mut sum = vec![Point::default(); groups];
    let mut size = vec![0.0f64; groups];
    for (node, point) in prep.nodes.iter().zip(pos) {
        sum[node.group_index].x += point.x;
        sum[node.group_index].y += point.y;
        size[node.group_index] += 1.0;
    }
    for ((node, point), force) in prep.nodes.iter().zip(pos).zip(forces.iter_mut()) {
        let g = node.group_index;
        if size[g] > 1.0 {
            force.x += (sum[g].x / size[g] - point.x) * 0.05;
            force.y += (sum[g].y / size[g] - point.y) * 0.05;
        }
        force.x += (center.x - point.x) * 0.02;
        force.y += (center.y - point.y) * 0.02;
    }
}

/// Pushes overlapping circles apart, keeping a small gap between them.
fn separate(pos: &mut [Point], radii: &[f64]) {
    const GAP: f64 = 6.0;
    for _ in 0..40 {
        let mut moved = false;
        for i in 0..pos.len() {
            for j in (i + 1)..pos.len() {
                let mut dx = pos[j].x - pos[i].x;
                let mut dy = pos[j].y - pos[i].y;
                let mut dist = (dx * dx + dy * dy).sqrt();
                let wanted = radii[i] + radii[j] + GAP;
                if dist >= wanted {
                    continue;
                }
                if dist < 1.0e-6 {
                    dx = 1.0;
                    dy = 0.0;
                    dist = 1.0;
                }
                let push = (wanted - dist) / 2.0 / dist;
                pos[i].x -= dx * push;
                pos[i].y -= dy * push;
                pos[j].x += dx * push;
                pos[j].y += dy * push;
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }
}

/// Translates the drawing so it starts at the padding, and computes its size.
fn fit(pos: &[Point], radii: &[f64], labeled: &[bool], prep: &Prepared<'_>) -> Layout {
    let (mut min_x, mut min_y) = (f64::MAX, f64::MAX);
    let (mut max_x, mut max_y) = (f64::MIN, f64::MIN);
    for (index, point) in pos.iter().enumerate() {
        let mut half = radii[index];
        let mut below = radii[index];
        if labeled[index] {
            half =
                half.max(label_width(short_label(&prep.nodes[index].label).chars().count()) / 2.0);
            below += LABEL_FONT + 6.0;
        }
        min_x = min_x.min(point.x - half);
        max_x = max_x.max(point.x + half);
        min_y = min_y.min(point.y - radii[index]);
        max_y = max_y.max(point.y + below);
    }
    let width = (max_x - min_x + 2.0 * PADDING).max(240.0);
    let height = (max_y - min_y + 2.0 * PADDING).max(160.0);
    let shift_x = PADDING - min_x + (width - (max_x - min_x + 2.0 * PADDING)) / 2.0;
    let shift_y = PADDING - min_y + (height - (max_y - min_y + 2.0 * PADDING)) / 2.0;
    let nodes = pos
        .iter()
        .enumerate()
        .map(|(index, point)| Placed {
            at: Point {
                x: point.x + shift_x,
                y: point.y + shift_y,
            },
            r: radii[index],
            labeled: labeled[index],
        })
        .collect();
    Layout {
        width,
        height,
        nodes,
    }
}

/// The drawable shape of one edge, with the arrowhead already positioned.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct EdgeShape {
    /// Where the fibre leaves the soma: on its border, in the direction its bundle emerges.
    pub start: Point,
    /// The control point that holds the fibre on its bundle's course as it leaves.
    pub out: Point,
    /// The control point that sets the direction the fibre arrives from.
    pub approach: Point,
    /// Where the fibre ends: short of the target's border by the synaptic cleft.
    pub tip: Point,
    /// Half the width of the fibre where it leaves. It tapers to nothing at the tip.
    pub width: f64,
    /// Whether the indexer was unsure of this edge.
    pub dashed: bool,
}

impl EdgeShape {
    /// The four points of the fibre's centre line, from the soma to the cleft.
    pub(crate) const fn spine(&self) -> [Point; 4] {
        [self.start, self.out, self.approach, self.tip]
    }
}

/// Returns the stroke width for an edge of `weight` among edges of at most `max_weight`.
pub(crate) fn edge_width(weight: u64, max_weight: u64) -> f64 {
    0.8 + 2.4 * (to_f64(weight) / to_f64(max_weight.max(1))).min(1.0).sqrt()
}

/// Normalizes `(dx, dy)`; a zero vector gives `(1, 0)`.
fn unit(dx: f64, dy: f64) -> (f64, f64) {
    let length = (dx * dx + dy * dy).sqrt();
    if length < 1.0e-9 {
        (1.0, 0.0)
    } else {
        (dx / length, dy / length)
    }
}

/// How far apart, in radians, two fibres may leave a soma and still share a trunk.
///
/// About thirty-two degrees. Wider and unrelated fibres are welded together, which reads as an
/// error; narrower and nothing bundles, which is the star-burst every other code graph draws.
const BUNDLE_ARC: f64 = 0.56;

/// How far a bundle runs before its fibres separate, as a fraction of the distance to the target.
const TRUNK: f64 = 0.34;

/// How far the fibre stops short of the soma it points at: the synaptic cleft.
///
/// A fibre that touches its target reads as a wire soldered on. The gap is what a synapse is, and
/// it is what makes the terminal on the far side of it read as a terminal.
const CLEFT: f64 = 3.2;

/// The direction each edge leaves its source in and the direction it arrives at its target from.
///
/// Fibres are bundled at **both** ends, and the second half is the one that matters: a code graph
/// is not a tree of things calling outwards, it is a small number of symbols that everything calls
/// *into*. Bundling only by source does nothing at all for such a hub, because each of its callers
/// has one edge and nothing to share a trunk with. Bundling by target is what makes the fibres
/// converge on a soma the way processes do.
///
/// Within one end, edges are grouped by angle: sorted, then swept once, closing a bundle where the
/// gap to the previous fibre or the spread of the whole bundle exceeds [`BUNDLE_ARC`]. The sweep
/// wraps, so a bundle straddling the twelve o'clock direction is one bundle and not two. Every
/// fibre in a bundle then uses the bundle's mean angle instead of its own.
fn bundled(prep: &Prepared<'_>, layout: &Layout) -> (Vec<f64>, Vec<f64>) {
    let mut leaving: Vec<f64> = Vec::with_capacity(prep.edges.len());
    let mut arriving: Vec<f64> = Vec::with_capacity(prep.edges.len());
    for edge in &prep.edges {
        let (from, to) = (layout.nodes[edge.from], layout.nodes[edge.to]);
        leaving.push((to.at.y - from.at.y).atan2(to.at.x - from.at.x));
        arriving.push((from.at.y - to.at.y).atan2(from.at.x - to.at.x));
    }
    let by_source: Vec<usize> = prep.edges.iter().map(|edge| edge.from).collect();
    let by_target: Vec<usize> = prep.edges.iter().map(|edge| edge.to).collect();
    (
        share_within_bundles(&leaving, &by_source),
        share_within_bundles(&arriving, &by_target),
    )
}

/// Replaces each angle with the mean angle of the bundle it falls in, grouped by `owner`.
fn share_within_bundles(angles: &[f64], owner: &[usize]) -> Vec<f64> {
    let mut out = angles.to_vec();
    let mut groups: std::collections::BTreeMap<usize, Vec<usize>> =
        std::collections::BTreeMap::new();
    for (index, node) in owner.iter().enumerate() {
        groups.entry(*node).or_default().push(index);
    }
    for indices in groups.values() {
        let mut sorted: Vec<usize> = indices.clone();
        sorted.sort_by(|a, b| angles[*a].total_cmp(&angles[*b]));
        let mut bundles: Vec<Vec<usize>> = Vec::new();
        for index in sorted {
            // A bundle closes on either of two conditions: a gap to the previous fibre wider than
            // the arc, or a total spread wider than it. Without the second, fibres spread evenly
            // around a soma chain into one bundle however far apart the ends are, and the whole
            // soma then sprouts in a single direction.
            let start_new = bundles.last().is_none_or(|bundle| {
                let gap = bundle
                    .last()
                    .is_some_and(|previous| angles[index] - angles[*previous] > BUNDLE_ARC);
                let spread = bundle
                    .first()
                    .is_some_and(|first| angles[index] - angles[*first] > BUNDLE_ARC);
                gap || spread
            });
            if start_new {
                bundles.push(vec![index]);
            } else if let Some(bundle) = bundles.last_mut() {
                bundle.push(index);
            }
        }
        // The sweep runs from -pi to pi, so a bundle sitting across that seam arrives as two.
        if bundles.len() > 1 {
            let first = bundles.first().and_then(|b| b.first()).map(|i| angles[*i]);
            let last = bundles.last().and_then(|b| b.last()).map(|i| angles[*i]);
            if let (Some(first), Some(last)) = (first, last) {
                if first + std::f64::consts::TAU - last <= BUNDLE_ARC {
                    if let Some(tail) = bundles.pop() {
                        if let Some(head) = bundles.first_mut() {
                            head.extend(tail);
                        }
                    }
                }
            }
        }
        for bundle in bundles {
            let mean = mean_angle(&bundle, angles);
            for index in bundle {
                out[index] = mean;
            }
        }
    }
    out
}

/// The mean of a set of angles, taken as unit vectors so that the wrap at pi does not skew it.
fn mean_angle(indices: &[usize], angles: &[f64]) -> f64 {
    let (mut x, mut y) = (0.0, 0.0);
    for index in indices {
        x += angles[*index].cos();
        y += angles[*index].sin();
    }
    if x.abs() < 1.0e-9 && y.abs() < 1.0e-9 {
        return indices.first().map_or(0.0, |index| angles[*index]);
    }
    y.atan2(x)
}

/// Computes the shape of every edge of `prep` over `layout`; `None` marks an edge that cannot be
/// drawn because its nodes overlap.
pub(crate) fn edge_shapes(prep: &Prepared<'_>, layout: &Layout) -> Vec<Option<EdgeShape>> {
    let max_weight = prep.max_edge_weight();
    let pairs: std::collections::BTreeSet<(usize, usize)> =
        prep.edges.iter().map(|edge| (edge.from, edge.to)).collect();
    let (emerge, arrive) = bundled(prep, layout);
    prep.edges
        .iter()
        .enumerate()
        .map(|(index, edge)| {
            let reciprocal = pairs.contains(&(edge.to, edge.from));
            let width = edge_width(edge.weight, max_weight);
            shape(
                layout.nodes[edge.from],
                layout.nodes[edge.to],
                bend_of(edge.from, edge.to, reciprocal),
                width,
                edge.is_uncertain(),
                emerge[index],
                arrive[index],
            )
        })
        .collect()
}

/// The smallest bend any edge is drawn with.
const MIN_BEND: f64 = 0.07;

/// How much the bend of an ordinary edge may vary above [`MIN_BEND`].
const BEND_SPREAD: f64 = 0.06;

/// How many distinct bends ordinary edges are drawn with.
const BEND_STEPS: usize = 5;

/// The bend of one edge: how far its curve bows away from the straight line between its nodes.
///
/// Every edge curves. A straight line reads as a wire, and the drawing is meant to read as a
/// branching structure, where nothing runs straight. Two nodes that call each other bow further,
/// and in opposite directions, because the bend is measured against each edge's own direction:
/// that is what keeps a mutual pair from drawing one line on top of another.
///
/// Ordinary edges take one of [`BEND_STEPS`] bends chosen from the pair of node positions. The
/// variation is what stops a fan of edges leaving one node from looking like a machine part, and
/// deriving it from the endpoints rather than from a counter keeps the drawing identical whenever
/// the input is, which the whole renderer promises.
fn bend_of(from: usize, to: usize, reciprocal: bool) -> f64 {
    if reciprocal {
        return 0.16;
    }
    let step = (from.wrapping_mul(31).wrapping_add(to)) % BEND_STEPS;
    #[allow(clippy::cast_precision_loss, reason = "step is below BEND_STEPS")]
    let fraction = step as f64 / (BEND_STEPS - 1) as f64;
    MIN_BEND + BEND_SPREAD * fraction
}

/// Builds the shape of a single fibre between two placed nodes.
///
/// It is a cubic, not an arc: the first control point holds the fibre on the course its bundle
/// leaves along, the second sets the direction it arrives from. That is what lets several fibres
/// leave a soma together and still reach targets in quite different places, which a single arc
/// cannot do.
fn shape(
    from: Placed,
    to: Placed,
    bend: f64,
    width: f64,
    dashed: bool,
    emerge: f64,
    arrive: f64,
) -> Option<EdgeShape> {
    let (dx, dy) = (to.at.x - from.at.x, to.at.y - from.at.y);
    let distance = (dx * dx + dy * dy).sqrt();
    if distance < from.r + to.r + CLEFT + 6.0 {
        return None;
    }
    let (ux, uy) = unit(dx, dy);
    // The fibre arrives from one side of the straight line, so two nodes that call each other are
    // never drawn one on top of the other.
    let sway = bend * distance;
    let (ex, ey) = (emerge.cos(), emerge.sin());
    let start = Point {
        x: from.at.x + ex * from.r,
        y: from.at.y + ey * from.r,
    };
    let trunk = distance * TRUNK;
    let out = Point {
        x: start.x + ex * trunk,
        y: start.y + ey * trunk,
    };
    let (bx, by) = (arrive.cos(), arrive.sin());
    let tip = Point {
        x: to.at.x + bx * (to.r + CLEFT),
        y: to.at.y + by * (to.r + CLEFT),
    };
    // The approach runs back along the bundle's arrival direction, pushed a little to one side so
    // that two nodes calling each other do not draw one fibre over the other.
    let approach = Point {
        x: tip.x + bx * distance * 0.30 - uy * sway * 0.6,
        y: tip.y + by * distance * 0.30 + ux * sway * 0.6,
    };
    Some(EdgeShape {
        start,
        out,
        approach,
        tip,
        width,
        dashed,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        BUNDLE_ARC, CLEFT, Layout, det_cos, det_sin, edge_shapes, edge_width, layout,
        share_within_bundles,
    };
    use crate::graph::prepare::prepare;
    use crate::model::{Graph, GraphEdge, GraphNode};
    use std::f64::consts::PI;

    /// Builds a graph with `n` nodes in `groups` groups and a ring of edges plus chords.
    fn sample(n: usize, groups: usize) -> Graph {
        let nodes = (0..n)
            .map(|i| GraphNode {
                id: format!("n{i}"),
                label: format!("node number {i}"),
                group: format!("group{}", i % groups),
                weight: u32::try_from(1 + (i * 7) % 20).unwrap_or(1),
            })
            .collect();
        let mut edges = Vec::new();
        for i in 0..n {
            edges.push(GraphEdge {
                from: i,
                to: (i + 1) % n,
                weight: 2,
                ..GraphEdge::default()
            });
            edges.push(GraphEdge {
                from: i,
                to: (i * 3 + 1) % n,
                weight: 1,
                ..GraphEdge::default()
            });
        }
        Graph { nodes, edges }
    }

    /// The Taylor sine and cosine agree with the platform functions to a tiny tolerance.
    #[test]
    fn deterministic_trig_is_accurate() {
        let mut angle = -20.0;
        while angle < 20.0 {
            assert!((det_sin(angle) - angle.sin()).abs() < 1e-8, "sin {angle}");
            assert!((det_cos(angle) - angle.cos()).abs() < 1e-8, "cos {angle}");
            angle += 0.137;
        }
        assert!(det_sin(0.0).abs() < 1e-12);
        assert!((det_cos(PI) + 1.0).abs() < 1e-8);
    }

    /// An empty graph has an empty layout, and one node sits inside the drawing.
    #[test]
    fn trivial_layouts() {
        let empty = layout(&prepare(&Graph::default(), 10, 10));
        assert!(empty.nodes.is_empty());
        let graph = sample(1, 1);
        let one = layout(&prepare(&graph, 10, 10));
        assert_eq!(one.nodes.len(), 1);
        assert!(one.width >= 240.0 && one.height >= 160.0);
        assert!(one.nodes[0].at.x > 0.0 && one.nodes[0].at.x < one.width);
    }

    /// Every node lies inside the drawing, no circles overlap, and coordinates are finite.
    #[test]
    fn nodes_fit_and_do_not_overlap() {
        for (n, g) in [(2, 1), (12, 3), (40, 5), (150, 8)] {
            let graph = sample(n, g);
            let prep = prepare(&graph, 150, 600);
            let placed: Layout = layout(&prep);
            for node in &placed.nodes {
                assert!(node.at.x.is_finite() && node.at.y.is_finite());
                assert!(
                    node.at.x - node.r >= 0.0 && node.at.x + node.r <= placed.width,
                    "n={n}"
                );
                assert!(
                    node.at.y - node.r >= 0.0 && node.at.y + node.r <= placed.height,
                    "n={n}"
                );
            }
            for i in 0..placed.nodes.len() {
                for j in (i + 1)..placed.nodes.len() {
                    let (a, b) = (placed.nodes[i], placed.nodes[j]);
                    let dist = ((a.at.x - b.at.x).powi(2) + (a.at.y - b.at.y).powi(2)).sqrt();
                    assert!(dist >= a.r + b.r - 0.5, "n={n} nodes {i} and {j} overlap");
                }
            }
        }
    }

    /// The layout is identical on every run.
    #[test]
    fn layout_is_deterministic() {
        let graph = sample(60, 4);
        let prep = prepare(&graph, 150, 600);
        assert_eq!(layout(&prep), layout(&prep));
    }

    /// Nodes of the same group end up closer to each other than to the rest, on average.
    #[test]
    fn groups_cluster() {
        let graph = Graph {
            nodes: (0..24)
                .map(|i| GraphNode {
                    id: i.to_string(),
                    label: i.to_string(),
                    group: (i / 12).to_string(),
                    weight: 3,
                })
                .collect(),
            edges: (0..24)
                .map(|i| GraphEdge {
                    from: i,
                    to: (i / 12) * 12 + (i + 1) % 12,
                    weight: 1,
                    ..GraphEdge::default()
                })
                .collect(),
        };
        let prep = prepare(&graph, 150, 600);
        let placed = layout(&prep);
        let (mut within, mut across, mut nw, mut na) = (0.0, 0.0, 0.0, 0.0);
        for i in 0..24usize {
            for j in (i + 1)..24usize {
                let d = ((placed.nodes[i].at.x - placed.nodes[j].at.x).powi(2)
                    + (placed.nodes[i].at.y - placed.nodes[j].at.y).powi(2))
                .sqrt();
                if i / 12 == j / 12 {
                    within += d;
                    nw += 1.0;
                } else {
                    across += d;
                    na += 1.0;
                }
            }
        }
        assert!(
            within / nw < across / na,
            "groups should be tighter than the whole"
        );
    }

    /// Only the heaviest nodes are labeled when there are many.
    #[test]
    fn label_count_is_capped() {
        let graph = sample(150, 5);
        let placed = layout(&prepare(&graph, 150, 600));
        assert_eq!(placed.nodes.iter().filter(|n| n.labeled).count(), 60);
        let small = layout(&prepare(&sample(10, 2), 150, 600));
        assert!(small.nodes.iter().all(|n| n.labeled));
    }

    /// Fibres heading the same way share a direction, fibres far apart do not, and no fibre is
    /// ever moved further than the arc allows. This bundling is the whole reason the drawing reads
    /// as tissue rather than as a star, so the rule is pinned directly rather than through a
    /// layout that may or may not happen to place two nodes near each other.
    #[test]
    fn fibres_of_a_bundle_share_a_direction() {
        // Three fibres within the arc of each other, then one far away.
        let angles = [0.0, 0.2, 0.4, 2.5];
        let owner = [7, 7, 7, 7];
        let shared = share_within_bundles(&angles, &owner);

        assert!(
            (shared[0] - shared[1]).abs() < 1.0e-12 && (shared[1] - shared[2]).abs() < 1.0e-12,
            "the close three did not converge: {shared:?}"
        );
        assert!(
            (shared[3] - shared[0]).abs() > 1.0,
            "the far one was pulled into the bundle: {shared:?}"
        );
        for (index, angle) in angles.iter().enumerate() {
            let moved = (shared[index] - angle).abs();
            assert!(moved <= BUNDLE_ARC, "fibre {index} moved {moved} radians");
        }

        // A bundle stops growing once its spread reaches the arc, so a fan of fibres each just
        // inside the gap does not chain into one bundle pointing somewhere none of them go.
        let fan: Vec<f64> = (0..8).map(|i| f64::from(i) * 0.3).collect();
        let owners = vec![1_usize; fan.len()];
        let fanned = share_within_bundles(&fan, &owners);
        let mut distinct = fanned.clone();
        distinct.sort_by(f64::total_cmp);
        distinct.dedup_by(|a, b| (*a - *b).abs() < 1.0e-12);
        let distinct = distinct.len();
        assert!(distinct > 1, "a whole fan became one bundle: {fanned:?}");
        for (index, angle) in fan.iter().enumerate() {
            assert!((fanned[index] - angle).abs() <= BUNDLE_ARC, "fibre {index}");
        }

        // Fibres owned by different nodes never bundle with each other.
        let split = share_within_bundles(&[0.0, 0.1], &[1, 2]);
        assert!((split[0] - 0.0).abs() < 1.0e-12 && (split[1] - 0.1).abs() < 1.0e-12);
    }

    /// Edge widths grow with weight inside a fixed range.
    #[test]
    fn edge_width_scales() {
        assert!((edge_width(1, 1) - 3.2).abs() < 1e-9);
        assert!(edge_width(1, 100) < edge_width(50, 100));
        assert!(edge_width(0, 0) >= 0.8);
        assert!(edge_width(500, 100) <= 3.2 + 1e-9);
    }

    /// Edge shapes start at the source border, end before the target, and reciprocal pairs bend.
    #[test]
    fn edge_shapes_are_sound() {
        let graph = Graph {
            nodes: vec![
                GraphNode {
                    id: "a".into(),
                    label: "a".into(),
                    group: String::new(),
                    weight: 5,
                },
                GraphNode {
                    id: "b".into(),
                    label: "b".into(),
                    group: String::new(),
                    weight: 5,
                },
                GraphNode {
                    id: "c".into(),
                    label: "c".into(),
                    group: String::new(),
                    weight: 5,
                },
            ],
            edges: vec![
                GraphEdge {
                    from: 0,
                    to: 1,
                    weight: 1,
                    ..GraphEdge::default()
                },
                GraphEdge {
                    from: 1,
                    to: 0,
                    weight: 1,
                    ..GraphEdge::default()
                },
                GraphEdge {
                    from: 0,
                    to: 2,
                    weight: 1,
                    confidence: "guess".into(),
                    ..GraphEdge::default()
                },
            ],
        };
        let prep = prepare(&graph, 10, 10);
        let placed = layout(&prep);
        let shapes = edge_shapes(&prep, &placed);
        assert_eq!(shapes.len(), 3);
        let draw = |index: usize| shapes[index].expect("the nodes are far enough apart to draw");
        // Prepared edges are sorted by (from, to): 0->1, 0->2, 1->0.
        let (first, third, second) = (draw(0), draw(1), draw(2));
        assert!(third.dashed && !first.dashed);

        // Every fibre stops short of the soma it points at. The gap is the synaptic cleft, and it
        // is what the terminal on the near side of it sits in.
        for (shape, target) in [(first, 1), (third, 2), (second, 0)] {
            let node = placed.nodes[target];
            let reach = (shape.tip.x - node.at.x).hypot(shape.tip.y - node.at.y);
            assert!(
                (reach - (node.r + CLEFT)).abs() < 1.0e-6,
                "fibre ends {reach} from a soma of radius {}",
                node.r
            );
            for point in shape.spine() {
                assert!(point.x.is_finite() && point.y.is_finite());
            }
        }

        // A fibre leaves along its bundle and arrives along its target's bundle, so the two
        // control points are pushed out from the ends rather than sitting on the straight line.
        for shape in [first, second, third] {
            let [begin, out, approach, tip] = shape.spine();
            assert!(
                (out.x - begin.x).hypot(out.y - begin.y) > 1.0,
                "a fibre leaves along its bundle"
            );
            assert!(
                (approach.x - tip.x).hypot(approach.y - tip.y) > 1.0,
                "a fibre arrives along its target's bundle"
            );
        }

        let source = placed.nodes[0];
        let border =
            ((third.start.x - source.at.x).powi(2) + (third.start.y - source.at.y).powi(2)).sqrt();
        assert!((border - source.r).abs() < 1e-6);
    }
}
