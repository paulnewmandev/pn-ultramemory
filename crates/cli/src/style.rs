// SPDX-License-Identifier: Apache-2.0
//! Colour, glyphs and a progress bar, for the times a person rather than a program is reading.
//!
//! # When any of this appears
//! Only when the stream it is going to is a terminal. A pipe, a file or a redirect receives the
//! same bytes it always did, which is what lets `pn-ultramemory recall ... | jq` keep working and
//! what stops an agent's context filling with escape sequences. `NO_COLOR` and `TERM=dumb` turn
//! colour off everywhere, following the conventions those two carry.
//!
//! # Where each thing goes
//! Colour is applied to **standard output**, where the result is. Progress and notes go to
//! **standard error**, so a long index can draw a bar over itself without ever touching the result
//! the caller is capturing.
//!
//! # What is coloured
//! The structure of the output, never the content: keys, table headers, punctuation and numbers.
//! A symbol name or a memory's text is printed exactly as it is, because colouring content would
//! mean deciding what part of a person's own words matters.

use std::fmt::Write as _;
use std::io::{IsTerminal, Write};
use std::time::Instant;

/// Ends any colour.
const RESET: &str = "\u{1b}[0m";
/// Dim: structure that should recede.
const DIM: &str = "\u{1b}[2m";
/// Bold.
const BOLD: &str = "\u{1b}[1m";
/// The emerald of the logo, used for what succeeded and for headings.
const GREEN: &str = "\u{1b}[38;5;48m";
/// Amber, used for a warning.
const AMBER: &str = "\u{1b}[38;5;214m";
/// Red, used for a failure.
const RED: &str = "\u{1b}[38;5;203m";
/// Blue, used for a number.
const BLUE: &str = "\u{1b}[38;5;75m";

/// Whether colour was asked to be suppressed, whatever the stream is.
fn suppressed() -> bool {
    if std::env::var_os("NO_COLOR").is_some() {
        return true;
    }
    matches!(std::env::var("TERM").as_deref(), Ok("dumb"))
}

/// Whether to colour what goes to standard output.
#[must_use]
pub fn colour_stdout() -> bool {
    !suppressed() && std::io::stdout().is_terminal()
}

/// Whether to colour and animate what goes to standard error.
#[must_use]
pub fn colour_stderr() -> bool {
    !suppressed() && std::io::stderr().is_terminal()
}

/// Wraps `text` in a colour, or returns it unchanged when `on` is false.
fn paint(on: bool, colour: &str, text: &str) -> String {
    if on {
        format!("{colour}{text}{RESET}")
    } else {
        text.to_owned()
    }
}

/// A short line saying something went well.
#[must_use]
pub fn good(text: &str) -> String {
    let on = colour_stderr();
    format!("{} {}", paint(on, GREEN, "\u{2713}"), text)
}

/// A short line saying something needs attention but did not fail.
#[must_use]
pub fn warn(text: &str) -> String {
    let on = colour_stderr();
    format!("{} {}", paint(on, AMBER, "!"), text)
}

/// A short line saying something failed.
#[must_use]
pub fn bad(text: &str) -> String {
    let on = colour_stderr();
    format!("{} {}", paint(on, RED, "\u{2717}"), text)
}

/// Colours the structure of a TOON or JSON result, leaving every value's text alone.
///
/// It works on the printed form rather than on the data, so it stays correct whatever the renderer
/// decides to emit, and a line it does not recognise is passed through untouched. Nothing here can
/// change what the output *says*: every branch either wraps a span in a colour or returns the line
/// as it was.
#[must_use]
pub fn highlight(text: &str, on: bool) -> String {
    if !on {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len() + text.len() / 4);
    for (index, line) in text.lines().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        out.push_str(&highlight_line(line));
    }
    if text.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Colours one line: its key, its table header, or its numbers.
fn highlight_line(line: &str) -> String {
    let indent: String = line.chars().take_while(|c| *c == ' ').collect();
    let rest = &line[indent.len()..];

    // `name[3]{a,b,c}:` — a table header. The name leads, the shape recedes.
    if let Some(bracket) = rest.find('[') {
        if rest.ends_with(':') && rest[bracket..].contains(']') {
            return format!(
                "{indent}{}{}{}{}{}",
                BOLD,
                &rest[..bracket],
                RESET,
                DIM,
                format_args!("{}{RESET}", &rest[bracket..])
            );
        }
    }
    // `key: value` — the key leads, the value keeps its own colours.
    if let Some(colon) = rest.find(": ") {
        let key = &rest[..colon];
        if !key.contains(',') && !key.contains('"') {
            return format!(
                "{indent}{BOLD}{key}{RESET}: {}",
                numbers(&rest[colon + 2..])
            );
        }
    }
    if rest.ends_with(':') && !rest.contains(',') {
        return format!("{indent}{BOLD}{rest}{RESET}");
    }
    format!("{indent}{}", numbers(rest))
}

/// Colours the runs of digits in a value, so a table of figures can be read down a column.
fn numbers(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut digits = String::new();
    for ch in text.chars() {
        if ch.is_ascii_digit() || (ch == '.' && !digits.is_empty()) {
            digits.push(ch);
            continue;
        }
        if !digits.is_empty() {
            let _ = write!(out, "{BLUE}{digits}{RESET}");
            digits.clear();
        }
        out.push(ch);
    }
    if !digits.is_empty() {
        let _ = write!(out, "{BLUE}{digits}{RESET}");
    }
    out
}

/// How many columns the bar itself occupies.
const BAR_WIDTH: u32 = 28;

/// The least time between redraws, in milliseconds.
///
/// A bar that repaints on every file spends more time writing to the terminal than the indexer
/// spends parsing, and on a fast repository it produces a flicker rather than a bar.
const REDRAW_MS: u128 = 60;

/// A progress bar drawn over one line of standard error.
///
/// It draws nothing at all unless standard error is a terminal, so a log file or a pipe receives
/// only the final line. Dropping it without calling [`Progress::finish`] still clears the line, so
/// an error path cannot leave a half-drawn bar behind.
pub struct Progress {
    /// What is being done, shown to the left of the bar.
    label: String,
    /// Whether anything is drawn at all.
    active: bool,
    /// When it started, for the elapsed time.
    started: Instant,
    /// When it was last drawn, so redraws stay rare.
    drawn: Instant,
    /// Whether a line is currently on screen that has to be cleared.
    dirty: bool,
}

impl Progress {
    /// Starts a bar. Nothing is drawn until the first [`Progress::set`].
    #[must_use]
    pub fn new(label: &str) -> Self {
        Self {
            label: label.to_owned(),
            active: colour_stderr(),
            started: Instant::now(),
            drawn: Instant::now(),
            dirty: false,
        }
    }

    /// Reports that `done` of `total` are finished.
    pub fn set(&mut self, done: u32, total: u32) {
        if !self.active {
            return;
        }
        let elapsed = self.drawn.elapsed().as_millis();
        if self.dirty && elapsed < REDRAW_MS && done < total {
            return;
        }
        self.drawn = Instant::now();
        let fraction = if total == 0 {
            0.0
        } else {
            f64::from(done) / f64::from(total)
        };
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "the product is in 0..=BAR_WIDTH, which is far below usize::MAX"
        )]
        let filled = (fraction * f64::from(BAR_WIDTH)).round() as u32;
        let filled = usize::try_from(filled)
            .unwrap_or(0)
            .min(usize::try_from(BAR_WIDTH).unwrap_or(0));
        let width = usize::try_from(BAR_WIDTH).unwrap_or(0);
        let bar: String = "\u{2501}".repeat(filled);
        let rest: String = "\u{2501}".repeat(width.saturating_sub(filled));
        let mut err = std::io::stderr().lock();
        let _ = write!(
            err,
            "\r\u{1b}[2K{DIM}{}{RESET} {GREEN}{bar}{RESET}{DIM}{rest}{RESET} {BLUE}{done}{RESET}{DIM}/{total}{RESET}",
            self.label
        );
        let _ = err.flush();
        self.dirty = true;
    }

    /// Clears the bar and prints one final line in its place.
    pub fn finish(mut self, summary: &str) {
        self.clear();
        if self.active {
            let seconds = self.started.elapsed().as_secs_f64();
            let mut err = std::io::stderr().lock();
            let _ = writeln!(err, "{} {DIM}in {seconds:.2}s{RESET}", good(summary));
        }
    }

    /// Erases the line the bar is drawn on.
    fn clear(&mut self) {
        if self.active && self.dirty {
            let mut err = std::io::stderr().lock();
            let _ = write!(err, "\r\u{1b}[2K");
            let _ = err.flush();
            self.dirty = false;
        }
    }
}

impl Drop for Progress {
    fn drop(&mut self) {
        self.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::{Progress, highlight, numbers};

    /// Without colour the text comes back byte for byte, which is what a pipe receives.
    #[test]
    fn plain_output_is_untouched() {
        let toon = "capsule:\n  budget: 600\nsymbols[2]{id,name}:\n  7,estimate_tokens\n";
        assert_eq!(highlight(toon, false), toon);
    }

    /// With colour the text still says the same thing once the escapes are removed.
    #[test]
    fn colour_never_changes_what_is_said() {
        let toon = "capsule:\n  budget: 600\nsymbols[2]{id,name}:\n  7,estimate_tokens\n";
        let painted = highlight(toon, true);
        assert!(painted.contains('\u{1b}'));
        let stripped = strip(&painted);
        assert_eq!(stripped, toon, "{painted:?}");
    }

    /// Text with no structure to colour survives, including empty input and odd punctuation.
    #[test]
    fn odd_input_is_handled() {
        for text in [
            "",
            "\n",
            "   ",
            "a: b: c",
            "[]{}:",
            "ñ 日本語 \u{1f600}",
            "no colon here",
        ] {
            let painted = highlight(text, true);
            assert_eq!(strip(&painted), text, "{text:?}");
        }
    }

    /// Runs of digits are wrapped, and nothing else in the text moves.
    #[test]
    fn numbers_are_wrapped() {
        let painted = numbers("used 525 of 600, saved 0.62");
        assert_eq!(strip(&painted), "used 525 of 600, saved 0.62");
        assert_eq!(painted.matches("\u{1b}[0m").count(), 3);
    }

    /// A bar on a stream that is not a terminal draws nothing at all.
    #[test]
    fn a_bar_without_a_terminal_is_silent() {
        let mut bar = Progress::new("indexing");
        bar.active = false;
        bar.set(3, 10);
        assert!(!bar.dirty);
        bar.finish("done");
    }

    /// Removes every escape sequence, so a test can compare what the text says.
    fn strip(text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut chars = text.chars();
        while let Some(ch) = chars.next() {
            if ch != '\u{1b}' {
                out.push(ch);
                continue;
            }
            for escape in chars.by_ref() {
                if escape.is_ascii_alphabetic() {
                    break;
                }
            }
        }
        out
    }
}
