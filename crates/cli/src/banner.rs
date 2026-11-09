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

/// How many columns the panel occupies between its two borders.
const PANEL_INNER: usize = 68;

/// One row of the panel: `plain` is what the row says, `painted` the same text with colour.
///
/// The padding is computed from `plain`, counted in characters rather than bytes, so the right
/// border stays in its column whatever the row holds: accents, `ñ` or escape sequences.
fn panel_row(out: &mut String, plain: &str, painted: &str, dim: &str, reset: &str) {
    let pad = PANEL_INNER.saturating_sub(plain.chars().count());
    let _ = writeln!(
        out,
        "{dim}\u{2502}{reset}{painted}{}{dim}\u{2502}{reset}",
        " ".repeat(pad)
    );
}

/// Renders the compact info panel below the header, its title set into the top border.
fn render_panel(version: &str, colour: bool, repo_name: &str, data_dir: &str) -> String {
    let (green, dim, bold, reset, cyan, blue) = if colour {
        (GREEN, DIM, BOLD, RESET, CYAN, BLUE)
    } else {
        ("", "", "", "", "", "")
    };
    let mut out = String::new();
    let title = format!(" pn-ultramemory v{version} ");
    let rule = PANEL_INNER.saturating_sub(title.chars().count() + 1);
    let _ = writeln!(
        out,
        "{dim}\u{256d}\u{2500}{reset}{bold}{title}{reset}{dim}{}\u{256e}{reset}",
        "\u{2500}".repeat(rule)
    );
    panel_row(
        &mut out,
        " workspace",
        &format!(" {green}{bold}workspace{reset}"),
        dim,
        reset,
    );
    for (label, value) in [("root", repo_name), ("data", data_dir)] {
        let shown = truncate_path(value, PANEL_INNER - 11);
        panel_row(
            &mut out,
            &format!("   {label}  {shown}"),
            &format!("   {dim}{label}{reset}  {blue}{shown}{reset}"),
            dim,
            reset,
        );
    }
    panel_row(&mut out, "", "", dim, reset);
    panel_row(
        &mut out,
        " commands",
        &format!(" {cyan}{bold}commands{reset}"),
        dim,
        reset,
    );
    for (category, tools) in TOOL_CATEGORIES {
        panel_row(
            &mut out,
            &format!("   {category:<8}{tools}"),
            &format!("   {green}{category:<8}{reset}{tools}"),
            dim,
            reset,
        );
    }
    let _ = write!(
        out,
        "{dim}\u{2570}{}\u{256f}{reset}",
        "\u{2500}".repeat(PANEL_INNER)
    );
    out
}

/// Shortens a path to at most `max` characters by dropping its start, which is the part a person
/// already knows. It counts characters, not bytes, so a path with accents is never cut mid-letter.
fn truncate_path(path: &str, max: usize) -> String {
    let count = path.chars().count();
    if count <= max {
        return path.to_owned();
    }
    let tail: String = path.chars().skip(count - (max - 3)).collect();
    format!("...{tail}")
}

/// Prints the full banner (header + panel) when stdout is a terminal.
pub fn print(version: &str, repo_name: &str, data_dir: &str) {
    if wanted() {
        let colour = coloured();
        println!("{}\n", render(version, colour));
        println!("{}\n", render_panel(version, colour, repo_name, data_dir));
    }
}

#[cfg(test)]
mod tests {
    use super::{render, render_panel, truncate_path};

    /// Every row of the panel is the same width, so its right border is one straight line.
    #[test]
    fn the_panel_is_rectangular() {
        let panel = render_panel(
            "1.3.0",
            false,
            "repo",
            "/Users/ñandú/Library/Application Support/pn-ultramemory/x",
        );
        let widths: Vec<usize> = panel.lines().map(|line| line.chars().count()).collect();
        assert!(
            widths.windows(2).all(|w| w[0] == w[1]),
            "{widths:?}\n{panel}"
        );
        assert!(widths[0] <= 80, "{}", widths[0]);
    }

    /// A long path with letters outside ASCII is shortened without cutting a letter in half.
    #[test]
    fn long_paths_with_accents_are_shortened_safely() {
        let path = "/Users/maría/Documentos/proyectos/código/año/señal/pn-ultramemory-0123456789";
        let short = truncate_path(path, 30);
        assert_eq!(short.chars().count(), 30);
        assert!(short.starts_with("...") && short.ends_with("0123456789"));
        assert_eq!(truncate_path("corto", 30), "corto");
    }

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
