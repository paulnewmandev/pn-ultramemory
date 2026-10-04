// SPDX-License-Identifier: Apache-2.0
//! The mark and wordmark printed when a person runs the tool in a terminal.
//!
//! # Why it is conditional
//! A banner is for people. Printing it when the output is a pipe would corrupt whatever reads it,
//! and printing it on every command would waste an agent's context on decoration. So it appears
//! only when standard output is a terminal, only on the commands that a person runs to look at
//! something, and never when `NO_COLOR` or a dumb terminal says to keep quiet.
//!
//! # The drawing
//! Two pieces side by side, three rows tall:
//!
//! * the **mark**, the project's logo as a small graph — three plain nodes and the one accent node
//!   the logo uses, joined by the edges that make its shape;
//! * the **wordmark**, `PN-ULTRAMEMORY` set in a font of box-drawing characters, built here from a
//!   table of glyphs rather than written out as three fixed strings, so the letters cannot drift
//!   out of alignment with each other and a test can check every one of them.
//!
//! The whole banner is **54 columns wide**, which leaves room in an eighty-column terminal for the
//! version and the two lines under it.

use std::fmt::Write as _;
use std::io::IsTerminal;

/// The word set in the wordmark.
const WORDMARK: &str = "PN-ULTRAMEMORY";

/// How many rows every glyph has.
const GLYPH_ROWS: usize = 3;

/// How many columns every glyph has.
///
/// The glyphs are set with no gap between them, the way this style of lettering is meant to be
/// read: the box-drawing characters already carry their own spacing, and a gap would push the
/// wordmark past the width a narrow terminal can show.
const GLYPH_COLS: usize = 3;

/// The letters the wordmark needs, three rows of three columns each.
///
/// Only the letters of [`WORDMARK`] are here. A glyph this table does not have is drawn as a
/// blank, so an unknown letter leaves a hole rather than shifting everything after it.
const GLYPHS: [(char, [&str; GLYPH_ROWS]); 13] = [
    ('P', ["╔═╗", "╠═╝", "╩  "]),
    ('N', ["╔╗╔", "║║║", "╝╚╝"]),
    ('-', ["   ", "═══", "   "]),
    ('U', ["╦ ╦", "║ ║", "╚═╝"]),
    ('L', ["╦  ", "║  ", "╩═╝"]),
    ('T', ["╔╦╗", " ║ ", " ╩ "]),
    ('R', ["╦═╗", "╠╦╝", "╩╚═"]),
    ('A', ["╔═╗", "╠═╣", "╩ ╩"]),
    ('M', ["╔╦╗", "║║║", "╩ ╩"]),
    ('E', ["╔═╗", "║╣ ", "╚═╝"]),
    ('O', ["╔═╗", "║ ║", "╚═╝"]),
    ('Y', ["╦ ╦", "╚╦╝", " ╩ "]),
    (' ', ["   ", "   ", "   "]),
];

/// How many columns the mark occupies. Every row of [`MARK`] is this wide, which a test checks.
const MARK_COLS: usize = 9;

/// The glyph used for a letter the font does not have: a hole of the right size, so the letters
/// after it stay where they belong.
const BLANK: [&str; GLYPH_ROWS] = ["   ", "   ", "   "];

/// The graph mark, one string per row.
const MARK: [&str; GLYPH_ROWS] = ["●───◉───●", " ╲  │  ╱ ", "  ╰─●─╯  "];

/// Spaces between the mark and the wordmark.
const GUTTER: &str = "  ";

/// The accent colour of the mark: the emerald of the logo.
const ACCENT: &str = "\u{1b}[38;5;48m";

/// The colour of the wordmark.
const BOLD: &str = "\u{1b}[1m";

/// The colour of the secondary lines.
const DIM: &str = "\u{1b}[2m";

/// Ends any colour.
const RESET: &str = "\u{1b}[0m";

/// Whether the banner should be shown at all.
///
/// It is shown only when standard output is a terminal, so a pipe or a file never receives it.
#[must_use]
pub fn wanted() -> bool {
    std::io::stdout().is_terminal()
}

/// Whether colour should be used, following the `NO_COLOR` convention and a dumb terminal.
fn coloured() -> bool {
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    !matches!(std::env::var("TERM").as_deref(), Ok("dumb"))
}

/// The three rows of a word, set in the glyph font.
///
/// A letter the font does not have becomes a blank of the same width, so the rows always stay the
/// same length as each other whatever it is asked to set.
fn set(word: &str) -> [String; GLYPH_ROWS] {
    let mut rows = [String::new(), String::new(), String::new()];
    for letter in word.chars() {
        let glyph = GLYPHS
            .iter()
            .find(|(candidate, _)| *candidate == letter)
            .map_or(BLANK, |(_, glyph)| *glyph);
        for (row, cell) in rows.iter_mut().zip(glyph) {
            row.push_str(cell);
        }
    }
    rows
}

/// How many columns the banner's drawing occupies.
#[must_use]
pub fn width() -> usize {
    MARK_COLS + GUTTER.len() + WORDMARK.chars().count() * GLYPH_COLS
}

/// Builds the banner text for a given version, with or without colour.
///
/// The result never ends with a newline, so the caller decides the spacing around it.
///
/// # Examples
/// ```text
/// let plain = render("1.0.0", false);
/// assert!(plain.contains("PN-ULTRAMEMORY") || plain.contains('╔'));
/// assert!(!plain.contains('\u{1b}'));   // no escape sequences without colour
/// ```
#[must_use]
pub fn render(version: &str, colour: bool) -> String {
    let (accent, bold, dim, reset) = if colour {
        (ACCENT, BOLD, DIM, RESET)
    } else {
        ("", "", "", "")
    };
    let word = set(WORDMARK);
    let mut out = String::new();
    for (row, (art, letters)) in MARK.iter().zip(&word).enumerate() {
        if row > 0 {
            out.push('\n');
        }
        out.push_str(accent);
        out.push_str(art);
        out.push_str(reset);
        out.push_str(GUTTER);
        out.push_str(bold);
        out.push_str(letters);
        out.push_str(reset);
    }
    // The three lines under the drawing are indented to where the wordmark starts, so the block
    // reads as one thing rather than as a picture with unrelated text beneath it.
    let indent = " ".repeat(MARK_COLS + GUTTER.len());
    let _ = write!(
        out,
        "\n{indent}{dim}v{version} \u{b7} code-aware memory for coding agents{reset}\n\
         {indent}{dim}local only \u{b7} no network \u{b7} no telemetry{reset}"
    );
    out
}

/// The available tool categories and their commands, matching the MCP server surface.
const TOOL_CATEGORIES: &[(&str, &str)] = &[
    ("code", "brief, recall, expand, impact, outline, map, graph"),
    ("memory", "remember, memories, forget, reanchor, feedback"),
    ("learn", "status, reset, why"),
    ("docs", "gaps, apply, build"),
    ("report", "stats, report, brain, bench"),
    ("setup", "install, uninstall, doctor, mcp-config"),
];

/// Renders the dashboard-style info panel below the wordmark.
fn render_panel(version: &str, colour: bool, repo_name: &str, data_dir: &str) -> String {
    let (green, dim, bold, reset, cyan) = if colour {
        (
            "\u{1b}[38;5;48m",
            "\u{1b}[2m",
            "\u{1b}[1m",
            "\u{1b}[0m",
            "\u{1b}[38;5;81m",
        )
    } else {
        ("", "", "", "", "")
    };

    let mut out = String::new();

    // Box top
    let box_width = 72;
    let _ = writeln!(out, "{dim}╭{}╮{reset}", "─".repeat(box_width));

    // Version line centered
    let version_line = format!("pn-ultramemory v{version}");
    let pad_left = (box_width - version_line.len()) / 2;
    let pad_right = box_width - version_line.len() - pad_left;
    let _ = writeln!(
        out,
        "{dim}│{reset}{pad_left}{bold}{version_line}{reset}{pad_right}{dim}│{reset}",
        pad_left = " ".repeat(pad_left),
        pad_right = " ".repeat(pad_right)
    );

    // Separator
    let _ = writeln!(out, "{dim}├{}{reset}", "─".repeat(box_width));

    // Workspace section
    let _ = writeln!(
        out,
        "{dim}│{reset} {green}{bold}workspace{reset}{pad}{dim}│{reset}",
        pad = " ".repeat(box_width - 9 - 1)
    );
    let root_display = if repo_name.len() > 50 {
        format!("...{}", &repo_name[repo_name.len() - 47..])
    } else {
        repo_name.to_owned()
    };
    let _ = writeln!(
        out,
        "{dim}│{reset}   root    {root_display}{pad}{dim}│{reset}",
        pad = " ".repeat(box_width.saturating_sub(13 + root_display.len()))
    );
    let data_display = if data_dir.len() > 50 {
        format!("...{}", &data_dir[data_dir.len() - 47..])
    } else {
        data_dir.to_owned()
    };
    let _ = writeln!(
        out,
        "{dim}│{reset}   data    {data_display}{pad}{dim}│{reset}",
        pad = " ".repeat(box_width.saturating_sub(13 + data_display.len()))
    );

    // Separator
    let _ = writeln!(out, "{dim}├{}┤{reset}", "─".repeat(box_width));

    // Available tools section
    let _ = writeln!(
        out,
        "{dim}│{reset} {cyan}{bold}available tools{reset}{pad}{dim}│{reset}",
        pad = " ".repeat(box_width.saturating_sub(17))
    );
    for (category, tools) in TOOL_CATEGORIES {
        // Visible length: 2 (border+space) + category (padded to 8) + 1 (space) + tools + 1 (border)
        let visible_len = 2 + 8 + 1 + tools.len() + 1;
        let pad = box_width.saturating_sub(visible_len);
        let _ = writeln!(
            out,
            "{dim}│{reset}  {green}{category:<8}{reset} {tools}{pad}{dim}│{reset}",
            pad = " ".repeat(pad)
        );
    }

    // Box bottom
    let _ = write!(out, "{dim}╰{}╯{reset}", "─".repeat(box_width));

    out
}

/// Prints the banner to standard output when it is wanted, followed by a blank line.
///
/// When `repo_name` and `data_dir` are provided, also prints the dashboard panel.
pub fn print_full(version: &str, repo_name: &str, data_dir: &str) {
    if wanted() {
        let colour = coloured();
        println!("{}\n", render(version, colour));
        println!("{}\n", render_panel(version, colour, repo_name, data_dir));
    }
}

/// Prints the banner to standard output when it is wanted, followed by a blank line.
pub fn print(version: &str) {
    if wanted() {
        println!("{}\n", render(version, coloured()));
    }
}

#[cfg(test)]
mod tests {
    use super::{GLYPH_COLS, GLYPH_ROWS, GLYPHS, MARK, MARK_COLS, WORDMARK, render, set, width};

    /// Every glyph is exactly the size the font promises, so no letter can shift the ones after it.
    #[test]
    fn every_glyph_is_the_same_size() {
        for (letter, glyph) in GLYPHS {
            assert_eq!(glyph.len(), GLYPH_ROWS, "{letter}");
            for (row, cell) in glyph.iter().enumerate() {
                assert_eq!(
                    cell.chars().count(),
                    GLYPH_COLS,
                    "glyph `{letter}` row {row}: `{cell}`"
                );
            }
        }
    }

    /// The font has a glyph for every letter the wordmark uses.
    #[test]
    fn the_font_covers_the_wordmark() {
        for letter in WORDMARK.chars() {
            assert!(
                GLYPHS.iter().any(|(candidate, _)| *candidate == letter),
                "no glyph for `{letter}`"
            );
        }
    }

    /// Setting a word gives three rows of equal length, one glyph per letter.
    #[test]
    fn a_word_is_set_in_a_rectangle() {
        let rows = set(WORDMARK);
        let expected = WORDMARK.chars().count() * GLYPH_COLS;
        for (index, row) in rows.iter().enumerate() {
            assert_eq!(row.chars().count(), expected, "row {index}: `{row}`");
        }
    }

    /// A letter the font does not have leaves a hole of the right width rather than shifting the
    /// rest of the word.
    #[test]
    fn an_unknown_letter_keeps_the_alignment() {
        let rows = set("PQP");
        for row in &rows {
            assert_eq!(row.chars().count(), 3 * GLYPH_COLS, "`{row}`");
        }
        assert!(rows[0].starts_with("╔═╗   ╔═╗"), "{}", rows[0]);
    }

    /// The mark is rectangular and carries the nodes of the logo.
    #[test]
    fn the_mark_is_rectangular_and_has_the_logo_nodes() {
        for (row, art) in MARK.iter().enumerate() {
            assert_eq!(art.chars().count(), MARK_COLS, "row {row}: {art}");
        }
        let drawing: String = MARK.concat();
        assert_eq!(drawing.matches('●').count(), 3);
        assert_eq!(drawing.matches('◉').count(), 1);
    }

    /// The banner fits a narrow terminal, which is the only hard constraint on its size.
    #[test]
    fn the_banner_fits_eighty_columns() {
        assert!(width() <= 80, "the drawing is {} columns", width());
        for line in render("1.0.0", false).lines() {
            assert!(
                line.chars().count() <= 80,
                "{} columns: {line}",
                line.chars().count()
            );
        }
    }

    /// Without colour the banner carries no escape sequence, so it is safe anywhere.
    #[test]
    fn plain_output_has_no_escape_sequences() {
        let plain = render("1.0.0", false);
        assert!(!plain.contains('\u{1b}'), "{plain}");
        assert!(plain.contains("v1.0.0"));
        assert!(!plain.ends_with('\n'));
    }

    /// With colour every sequence that is opened is closed again, so nothing bleeds into the
    /// output that follows the banner.
    #[test]
    fn colour_is_always_closed() {
        let coloured = render("1.0.0", true);
        let opens = coloured.matches('\u{1b}').count();
        let closes = coloured.matches("\u{1b}[0m").count();
        assert_eq!(opens, closes * 2, "{coloured}");
        assert!(coloured.ends_with("\u{1b}[0m"));
    }
}
