// SPDX-License-Identifier: Apache-2.0
//! The categorical colors that tell the groups of a graph apart.
//!
//! The ten hues are the Okabe-Ito color-blind-safe set extended with three more, in three
//! variants: a `main` color for fills and strokes on a light background, a `soft` tint for boxes
//! that carry dark text (Mermaid and DOT), and a `dark` variant that keeps contrast on a dark
//! page. A group takes the color at its rank among the sorted group names, wrapping around after
//! ten groups.
//!
//! Invariant: every lookup wraps, so any group index is valid.

/// Number of distinct colors before they repeat.
pub(crate) const COLORS: usize = 10;

/// Colors for fills on a light background.
const MAIN: [&str; COLORS] = [
    "#0072b2", "#e69f00", "#009e73", "#cc79a7", "#56b4e9", "#d55e00", "#7a5195", "#8c8c00",
    "#1b9e77", "#666666",
];

/// Light tints for boxes that carry dark text.
const SOFT: [&str; COLORS] = [
    "#cfe3f5", "#fbe5b6", "#c2ebdc", "#f2d3e4", "#d5ecfa", "#f8d0b5", "#dccbe8", "#e6e6a0",
    "#c5ebde", "#dddddd",
];

/// Colors that keep contrast on a dark background.
const DARK: [&str; COLORS] = [
    "#4da3e0", "#f2b84b", "#3cc79a", "#e39bc4", "#8ccff5", "#f08a4b", "#b48ad6", "#c8c83c",
    "#5ad4b0", "#a0a0a0",
];

/// Returns the main color of group `index`.
pub(crate) fn main(index: usize) -> &'static str {
    MAIN[index % COLORS]
}

/// Returns the soft tint of group `index`.
pub(crate) fn soft(index: usize) -> &'static str {
    SOFT[index % COLORS]
}

/// Returns the dark-theme color of group `index`.
pub(crate) fn dark(index: usize) -> &'static str {
    DARK[index % COLORS]
}

/// Converts a `#rrggbb` color to three components in `0.0..=1.0`, or mid gray when malformed.
pub(crate) fn rgb(hex: &str) -> (f64, f64, f64) {
    let digits = hex.trim_start_matches('#');
    let channel = |start: usize| -> f64 {
        digits
            .get(start..start + 2)
            .and_then(|pair| u8::from_str_radix(pair, 16).ok())
            .map_or(0.5, |value| f64::from(value) / 255.0)
    };
    (channel(0), channel(2), channel(4))
}

#[cfg(test)]
mod tests {
    use super::{COLORS, DARK, MAIN, SOFT, dark, main, rgb, soft};

    /// Indices wrap around, so every group has a color.
    #[test]
    fn lookups_wrap() {
        assert_eq!(main(0), main(COLORS));
        assert_eq!(soft(3), soft(3 + 5 * COLORS));
        assert_eq!(dark(usize::MAX), dark(usize::MAX % COLORS));
    }

    /// All colors are well-formed and distinct within a variant.
    #[test]
    fn colors_are_valid_and_distinct() {
        for table in [MAIN, SOFT, DARK] {
            for (index, color) in table.iter().enumerate() {
                assert_eq!(color.len(), 7, "{color}");
                assert!(color.starts_with('#'));
                assert!(color[1..].chars().all(|c| c.is_ascii_hexdigit()), "{color}");
                assert!(!table[..index].contains(color), "duplicate {color}");
            }
        }
    }

    /// Hex colors convert to unit components, and malformed input falls back to gray.
    #[test]
    fn hex_converts() {
        assert_eq!(rgb("#000000"), (0.0, 0.0, 0.0));
        assert_eq!(rgb("#ffffff"), (1.0, 1.0, 1.0));
        let (r, g, b) = rgb("#ff0000");
        assert!((r - 1.0).abs() < 1e-9 && g.abs() < 1e-9 && b.abs() < 1e-9);
        assert_eq!(rgb("nope"), (0.5, 0.5, 0.5));
        assert_eq!(rgb(""), (0.5, 0.5, 0.5));
    }
}
