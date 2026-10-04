// SPDX-License-Identifier: Apache-2.0
//! The banner printed when a person runs the tool in a terminal.
//!
//! # Why it is conditional
//! A banner is for people. Printing it when the output is a pipe would corrupt whatever reads it,
//! and printing it on every command would waste an agent's context on decoration. So it appears
//! only when standard output is a terminal, only on the commands that a person runs to look at
//! something, and never when `NO_COLOR` or a dumb terminal says to keep quiet.
//!
//! # The design
//! Clean, minimal, Claude Code-inspired: a small unicode logo mark, the tool name in bold,
//! version and tagline beneath, then a compact info panel with workspace details and available
//! tools. No large ASCII art — just colour, structure and information.

use std::fmt::Write as _;
use std::io::IsTerminal;

// ── Colours (256-colour palette) ──────────────────────────────────────────────

/// Emerald green — the brand accent.
const GREEN: &str = "\u{1b}[38;5;48m";
/// Bright cyan — section headers.
const CYAN: &str = "\u{1b}[38;5;81m";
/// Soft blue — numbers and paths.
const BLUE: &str = "\u{1b}[38;5;75m";
/// Bold.
const BOLD: &str = "\u{1b}[1m";
/// Dim — structure that should recede.
const DIM: &str = "\u{1b}[2m";
/// Ends any colour.
const RESET: &str = "\u{1b}[0m";

// ── Tool categories ───────────────────────────────────────────────────────────

/// The available tool categories and their commands, matching the MCP server surface.
const TOOL_CATEGORIES: &[(&str, &str)] = &[
    ("code", "brief, recall, expand, impact, outline, map, graph"),
    ("memory", "remember, memories, forget, reanchor, feedback"),
    ("learn", "status, reset, why"),
    ("docs", "gaps, apply, build"),
    ("report", "stats, report, brain, bench"),
    ("setup", "install, uninstall, doctor, mcp-config"),
];

// ── Visibility helpers ────────────────────────────────────────────────────────

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

// ── Rendering ─────────────────────────────────────────────────────────────────

/// Builds the header lines: logo mark + name + version + tagline.
///
/// The result never ends with a newline, so the caller decides the spacing around it.
#[must_use]
pub fn render(version: &str, colour: bool) -> String {
    let (green, bold, dim, reset) = if colour {
        (GREEN, BOLD, DIM, RESET)
    } else {
        ("", "", "", "")
    };
    let mut out = String::new();
    // Line 1: logo mark + tool name
    let _ = writeln!(out, "{green}\u{25c9}{reset} {bold}pn-ultramemory{reset}");
    // Line 2: version + tagline
    let _ = write!(
        out,
        "  {dim}v{version} \u{b7} code-aware memory for coding agents{reset}"
    );
    out
}

/// Renders the compact info panel below the header.
fn render_panel(version: &str, colour: bool, repo_name: &str, data_dir: &str) -> String {
    let (green, dim, bold, reset, cyan, blue) = if colour {
        (GREEN, DIM, BOLD, RESET, CYAN, BLUE)
    } else {
        ("", "", "", "", "", "")
    };

    let mut out = String::new();
    let box_width = 68;

    // Top border
    let _ = writeln!(
        out,
        "{dim}\u{256d}{}\u{256e}{reset}",
        "\u{2500}".repeat(box_width)
    );

    // Version centred
    let version_line = format!("pn-ultramemory v{version}");
    let pad_left = (box_width - version_line.len()) / 2;
    let pad_right = box_width - version_line.len() - pad_left;
    let _ = writeln!(
        out,
        "{dim}\u{2502}{reset}{pad_left}{bold}{version_line}{reset}{pad_right}{dim}\u{2502}{reset}",
        pad_left = " ".repeat(pad_left),
        pad_right = " ".repeat(pad_right)
    );

    // Separator
    let _ = writeln!(
        out,
        "{dim}\u{251c}{}\u{2524}{reset}",
        "\u{2500}".repeat(box_width)
    );

    // Workspace section
    let _ = writeln!(
        out,
        "{dim}\u{2502}{reset} {green}{bold}workspace{reset}{pad}{dim}\u{2502}{reset}",
        pad = " ".repeat(box_width.saturating_sub(10))
    );
    let root_display = truncate_path(repo_name, 48);
    let _ = writeln!(
        out,
        "{dim}\u{2502}{reset}   {dim}root{reset}  {blue}{root_display}{reset}{pad}{dim}\u{2502}{reset}",
        pad = " ".repeat(box_width.saturating_sub(12 + root_display.len()))
    );
    let data_display = truncate_path(data_dir, 48);
    let _ = writeln!(
        out,
        "{dim}\u{2502}{reset}   {dim}data{reset}  {blue}{data_display}{reset}{pad}{dim}\u{2502}{reset}",
        pad = " ".repeat(box_width.saturating_sub(12 + data_display.len()))
    );

    // Separator
    let _ = writeln!(
        out,
        "{dim}\u{251c}{}\u{2524}{reset}",
        "\u{2500}".repeat(box_width)
    );

    // Available tools section
    let _ = writeln!(
        out,
        "{dim}\u{2502}{reset} {cyan}{bold}available tools{reset}{pad}{dim}\u{2502}{reset}",
        pad = " ".repeat(box_width.saturating_sub(16))
    );
    for (category, tools) in TOOL_CATEGORIES {
        let visible_len = 2 + 8 + 1 + tools.len() + 1;
        let pad = box_width.saturating_sub(visible_len);
        let _ = writeln!(
            out,
            "{dim}\u{2502}{reset}   {green}{category:<8}{reset}{tools}{pad}{dim}\u{2502}{reset}",
            pad = " ".repeat(pad)
        );
    }

    // Bottom border
    let _ = write!(
        out,
        "{dim}\u{2570}{}\u{256f}{reset}",
        "\u{2500}".repeat(box_width)
    );

    out
}

/// Truncates a path from the left when it exceeds `max` characters, keeping the tail.
fn truncate_path(path: &str, max: usize) -> String {
    if path.len() <= max {
        path.to_owned()
    } else {
        format!("...{}", &path[path.len() - (max - 3)..])
    }
}

/// Prints the full banner (header + panel) when stdout is a terminal.
pub fn print_full(version: &str, repo_name: &str, data_dir: &str) {
    if wanted() {
        let colour = coloured();
        println!("{}\n", render(version, colour));
        println!("{}\n", render_panel(version, colour, repo_name, data_dir));
    }
}

/// Prints just the header when stdout is a terminal.
pub fn print(version: &str) {
    if wanted() {
        println!("{}\n", render(version, coloured()));
    }
}

#[cfg(test)]
mod tests {
    use super::render;

    /// Without colour the banner carries no escape sequence, so it is safe anywhere.
    #[test]
    fn plain_output_has_no_escape_sequences() {
        let plain = render("1.0.0", false);
        assert!(!plain.contains('\u{1b}'), "{plain}");
        assert!(plain.contains("v1.0.0"));
        assert!(plain.contains("pn-ultramemory"));
    }

    /// With colour every sequence that is opened is closed again.
    #[test]
    fn colour_sequences_are_balanced() {
        let coloured = render("1.0.0", true);
        assert!(coloured.contains('\u{1b}'));
        // Every open has a matching close
        let opens = coloured.matches("\u{1b}[").count();
        let closes = coloured.matches("\u{1b}[0m").count();
        assert!(closes >= opens / 2, "unbalanced escapes: {coloured:?}");
    }

    /// The banner fits a narrow terminal.
    #[test]
    fn the_banner_fits_eighty_columns() {
        for line in render("1.0.0", false).lines() {
            assert!(
                line.chars().count() <= 80,
                "{} columns: {line}",
                line.chars().count()
            );
        }
    }
}
