// SPDX-License-Identifier: Apache-2.0
//! A page content stream builder with a top-left origin.
//!
//! [`Canvas`] turns drawing calls (rectangles, lines, circles, polygons, curves and text) into
//! PDF operators. Coordinates are in points with the origin at the top-left corner and `y`
//! growing downwards, like the layout code thinks; the canvas flips them into PDF space. Colors,
//! line width and dash pattern are only emitted when they change, which keeps streams compact.
//!
//! Invariants: the stream is 7-bit ASCII (strings are escaped); numbers are written with at most
//! two decimals and never in exponent form; non-finite coordinates become zero.

use std::fmt::Write as _;

use super::fonts::Font;
use super::writer::literal;
use crate::numfmt::coord;

/// An RGB color with components in `0.0..=1.0`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Color(pub f64, pub f64, pub f64);

impl Color {
    /// Builds a color from a `#rrggbb` string; malformed input gives mid gray.
    pub(super) fn hex(hex: &str) -> Self {
        let (r, g, b) = crate::graph::palette::rgb(hex);
        Self(r, g, b)
    }

    /// Formats the components for an operator.
    fn operands(self) -> String {
        format!("{} {} {}", short(self.0), short(self.1), short(self.2))
    }
}

/// Formats a color component with three decimals and no trailing zeros.
fn short(value: f64) -> String {
    let mut text = format!("{:.3}", value.clamp(0.0, 1.0));
    while text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    text
}

/// A page content stream under construction.
#[derive(Debug, Clone)]
pub(super) struct Canvas {
    /// The operators written so far.
    out: String,
    /// Height of the page, used to flip the vertical axis.
    height: f64,
    /// The current fill color, once set.
    fill: Option<Color>,
    /// The current stroke color, once set.
    stroke: Option<Color>,
    /// The current line width, once set.
    line_width: Option<f64>,
    /// Whether the current dash pattern is dashed.
    dashed: bool,
}

impl Canvas {
    /// Creates an empty canvas for a page of the given height.
    pub(super) fn new(height: f64) -> Self {
        Self {
            out: String::new(),
            height,
            fill: None,
            stroke: None,
            line_width: None,
            dashed: false,
        }
    }

    /// Returns the finished content stream.
    pub(super) fn into_bytes(self) -> Vec<u8> {
        self.out.into_bytes()
    }

    /// Flips a vertical coordinate into PDF space.
    fn flip(&self, y: f64) -> f64 {
        self.height - y
    }

    /// Selects the fill color.
    fn set_fill(&mut self, color: Color) {
        if self.fill != Some(color) {
            let _ = writeln!(self.out, "{} rg", color.operands());
            self.fill = Some(color);
        }
    }

    /// Selects the stroke color.
    fn set_stroke(&mut self, color: Color) {
        if self.stroke != Some(color) {
            let _ = writeln!(self.out, "{} RG", color.operands());
            self.stroke = Some(color);
        }
    }

    /// Selects the line width.
    fn set_width(&mut self, width: f64) {
        if self.line_width != Some(width) {
            let _ = writeln!(self.out, "{} w", coord(width));
            self.line_width = Some(width);
        }
    }

    /// Selects a dashed or solid line.
    pub(super) fn set_dashed(&mut self, dashed: bool) {
        if self.dashed != dashed {
            self.out
                .push_str(if dashed { "[3 2] 0 d\n" } else { "[] 0 d\n" });
            self.dashed = dashed;
        }
    }

    /// Fills a rectangle whose top-left corner is `(x, y)`.
    pub(super) fn fill_rect(&mut self, x: f64, y: f64, w: f64, h: f64, color: Color) {
        self.set_fill(color);
        let _ = writeln!(
            self.out,
            "{} {} {} {} re f",
            coord(x),
            coord(self.flip(y + h)),
            coord(w),
            coord(h)
        );
    }

    /// Strokes the outline of a rectangle whose top-left corner is `(x, y)`.
    pub(super) fn stroke_rect(&mut self, x: f64, y: f64, w: f64, h: f64, color: Color, width: f64) {
        self.set_stroke(color);
        self.set_width(width);
        let _ = writeln!(
            self.out,
            "{} {} {} {} re S",
            coord(x),
            coord(self.flip(y + h)),
            coord(w),
            coord(h)
        );
    }

    /// Draws a straight line.
    pub(super) fn line(&mut self, from: (f64, f64), to: (f64, f64), color: Color, width: f64) {
        self.set_stroke(color);
        self.set_width(width);
        let _ = writeln!(
            self.out,
            "{} {} m {} {} l S",
            coord(from.0),
            coord(self.flip(from.1)),
            coord(to.0),
            coord(self.flip(to.1))
        );
    }

    /// Strokes a cubic curve through two control points.
    ///
    /// PDF's own curve operator is already a cubic, so unlike [`Self::quad`] this needs no
    /// conversion: the four points go straight through.
    pub(super) fn cubic(
        &mut self,
        from: (f64, f64),
        c1: (f64, f64),
        c2: (f64, f64),
        to: (f64, f64),
        color: Color,
        width: f64,
    ) {
        self.set_stroke(color);
        self.set_width(width);
        let _ = writeln!(
            self.out,
            "{} {} m {} {} {} {} {} {} c S",
            coord(from.0),
            coord(self.flip(from.1)),
            coord(c1.0),
            coord(self.flip(c1.1)),
            coord(c2.0),
            coord(self.flip(c2.1)),
            coord(to.0),
            coord(self.flip(to.1))
        );
    }

    /// Draws a circle, filled and outlined.
    pub(super) fn circle(
        &mut self,
        center: (f64, f64),
        radius: f64,
        fill: Color,
        outline: Color,
        width: f64,
    ) {
        const KAPPA: f64 = 0.552_284_75;
        self.set_fill(fill);
        self.set_stroke(outline);
        self.set_width(width);
        let (cx, cy, r) = (center.0, self.flip(center.1), radius);
        let k = r * KAPPA;
        let _ = writeln!(
            self.out,
            "{} {} m {} {} {} {} {} {} c {} {} {} {} {} {} c {} {} {} {} {} {} c {} {} {} {} {} {} c B",
            coord(cx + r),
            coord(cy),
            coord(cx + r),
            coord(cy + k),
            coord(cx + k),
            coord(cy + r),
            coord(cx),
            coord(cy + r),
            coord(cx - k),
            coord(cy + r),
            coord(cx - r),
            coord(cy + k),
            coord(cx - r),
            coord(cy),
            coord(cx - r),
            coord(cy - k),
            coord(cx - k),
            coord(cy - r),
            coord(cx),
            coord(cy - r),
            coord(cx + k),
            coord(cy - r),
            coord(cx + r),
            coord(cy - k),
            coord(cx + r),
            coord(cy),
        );
    }

    /// Writes one line of text with its baseline at `y`, starting at `x`.
    pub(super) fn text(
        &mut self,
        x: f64,
        y: f64,
        font: Font,
        size: f64,
        color: Color,
        text: &[u8],
    ) {
        if text.is_empty() {
            return;
        }
        self.set_fill(color);
        let _ = writeln!(
            self.out,
            "BT /{} {} Tf {} {} Td {} Tj ET",
            font.resource(),
            coord(size),
            coord(x),
            coord(self.flip(y)),
            literal(text)
        );
    }

    /// Writes one line of text with a halo of `halo` color around the glyphs, which keeps the
    /// text readable over lines and shapes.
    pub(super) fn text_halo(
        &mut self,
        at: (f64, f64),
        font: Font,
        size: f64,
        color: Color,
        halo: Color,
        text: &[u8],
    ) {
        if text.is_empty() {
            return;
        }
        self.set_stroke(halo);
        self.set_width(size * 0.3);
        let _ = writeln!(
            self.out,
            "BT /{} {} Tf {} {} Td 1 Tr {} Tj 0 Tr ET",
            font.resource(),
            coord(size),
            coord(at.0),
            coord(self.flip(at.1)),
            literal(text)
        );
        self.text(at.0, at.1, font, size, color, text);
    }
}

#[cfg(test)]
mod tests {
    use super::{Canvas, Color};
    use crate::pdf::fonts::Font;

    /// Returns the stream of a canvas as text.
    fn stream(canvas: Canvas) -> String {
        String::from_utf8(canvas.into_bytes()).expect("streams are ASCII")
    }

    /// Coordinates are flipped from top-left to PDF space, and rectangles start at their bottom.
    #[test]
    fn rectangles_are_flipped() {
        let mut canvas = Canvas::new(800.0);
        canvas.fill_rect(10.0, 100.0, 50.0, 20.0, Color(1.0, 0.0, 0.0));
        assert_eq!(stream(canvas), "1 0 0 rg\n10 680 50 20 re f\n");
    }

    /// Repeated colors and widths are written once.
    #[test]
    fn state_changes_are_deduplicated() {
        let mut canvas = Canvas::new(100.0);
        let gray = Color(0.5, 0.5, 0.5);
        canvas.line((0.0, 0.0), (10.0, 0.0), gray, 1.0);
        canvas.line((0.0, 5.0), (10.0, 5.0), gray, 1.0);
        canvas.set_dashed(true);
        canvas.set_dashed(true);
        let text = stream(canvas);
        assert_eq!(text.matches("RG").count(), 1);
        assert_eq!(text.matches(" w\n").count(), 1);
        assert_eq!(text.matches(" d\n").count(), 1);
        assert_eq!(text.matches(" S\n").count(), 2);
    }

    /// Text uses the font resource, escapes its string and skips empty text.
    #[test]
    fn text_is_escaped() {
        let mut canvas = Canvas::new(100.0);
        canvas.text(
            5.0,
            20.0,
            Font::Bold,
            9.0,
            Color(0.0, 0.0, 0.0),
            b"a(b)\xe1",
        );
        canvas.text(5.0, 20.0, Font::Bold, 9.0, Color(0.0, 0.0, 0.0), b"");
        let text = stream(canvas);
        assert!(text.contains("BT /F2 9 Tf 5 80 Td (a\\(b\\)\\341) Tj ET"));
        assert_eq!(text.matches("BT").count(), 1);
        assert!(text.is_ascii());
    }

    /// Circles and curves produce closed, well-formed paths and tolerate bad numbers.
    #[test]
    fn shapes_and_bad_numbers() {
        let mut canvas = Canvas::new(100.0);
        let black = Color(0.0, 0.0, 0.0);
        canvas.circle((50.0, 50.0), 10.0, black, black, 1.0);
        canvas.cubic((0.0, 0.0), (3.0, 6.0), (7.0, 6.0), (10.0, 0.0), black, 0.5);
        canvas.line((f64::NAN, f64::INFINITY), (1.0, 1.0), black, 1.0);
        let text = stream(canvas);
        assert!(text.contains("60 50 m"));
        assert!(text.contains(" c B"));
        assert!(!text.contains("NaN") && !text.contains("inf") && !text.contains("e+"));
        assert!(text.is_ascii());
    }

    /// Hex colors convert to components.
    #[test]
    fn hex_colors() {
        assert_eq!(Color::hex("#ffffff"), Color(1.0, 1.0, 1.0));
        assert_eq!(Color::hex("nope"), Color(0.5, 0.5, 0.5));
    }
}
