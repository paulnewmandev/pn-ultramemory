// SPDX-License-Identifier: Apache-2.0
//! Serving the source of one symbol, in windows.
//!
//! A capsule shows a symbol at the cheapest level of detail that answers the question. When that is
//! not enough, the agent asks for the code itself, and [`Engine::expand`] hands it over one window
//! at a time so that a four-thousand-line file never arrives in a single reply.
//!
//! # The window
//! Lines are counted **inside the symbol**: line 1 is the first line of its declaration slice (the
//! attached documentation comment included), not line 1 of the file. Both ends are inclusive,
//! `from` defaults to 1 and `to` defaults to the last line. A `to` past the end of the symbol is
//! clamped; a `from` past the end is a refusal, because silently returning the last window would
//! hide the mistake.
//!
//! # The two caps
//! One call returns at most [`MAX_WINDOW_LINES`] lines and at most about [`MAX_WINDOW_TOKENS`]
//! estimated tokens, whichever bites first. The window is shortened, `to` says where it really
//! stopped and `has_more` says that there is something after it, so the caller pages on by asking
//! for `to + 1`. A single line longer than the token cap is still returned whole: a window has to
//! contain at least one line to make progress.
//!
//! # Trust
//! The source is sliced through [`crate::sources::SourceCache`], which refuses a file whose content
//! no longer hashes to what was stored at indexing time. An agent is therefore never shown code
//! that does not match the description it was given, and the refusal names the command that fixes
//! it.

use pn_ultramemory_codec::estimate_tokens;
use pn_ultramemory_core::{SymbolId, SymbolKind};
use serde_json::{Value, json};

use crate::engine::Engine;
use crate::error::EngineError;
use crate::metrics::Event;
use crate::sources::SourceCache;

/// The most lines one call returns, however wide a window the caller asks for.
pub const MAX_WINDOW_LINES: u32 = 400;

/// The most estimated tokens one call returns, unless a single line already costs more.
pub const MAX_WINDOW_TOKENS: u32 = 6000;

/// One window of a symbol's own source, with everything needed to ask for the next one.
///
/// # Examples
/// ```
/// use pn_ultramemory_core::{SymbolId, SymbolKind};
/// use pn_ultramemory_engine::Expansion;
///
/// let window = Expansion {
///     id: SymbolId(7),
///     name: "Config::validate".into(),
///     kind: SymbolKind::Method,
///     path: "src/config.rs".into(),
///     start_line: 10,
///     end_line: 60,
///     from: 1,
///     to: 20,
///     total_lines: 51,
///     has_more: true,
///     text: "pub fn validate(&self) -> bool {".into(),
/// };
/// assert!(window.render_text().starts_with("src/config.rs:10-60 Config::validate"));
/// assert_eq!(window.to_value()["has_more"], true);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expansion {
    /// Identity of the symbol the window belongs to.
    pub id: SymbolId,
    /// The qualified name of that symbol.
    pub name: String,
    /// What kind of element it is.
    pub kind: SymbolKind,
    /// Path of its file, relative to the repository root.
    pub path: String,
    /// First line of the declaration in the file, 1-based.
    pub start_line: u32,
    /// Last line of the declaration in the file, 1-based.
    pub end_line: u32,
    /// First line of the window, counted from the start of the declaration, 1-based.
    pub from: u32,
    /// Last line of the window, counted from the start of the declaration, 1-based and inclusive.
    pub to: u32,
    /// How many lines the whole declaration has.
    pub total_lines: u32,
    /// Whether lines follow the window, so that another call would return more.
    pub has_more: bool,
    /// The lines of the window, exactly as they are in the file.
    pub text: String,
}

impl Expansion {
    /// The window as a structured value, with the code inline as one string.
    ///
    /// The fields are those of the struct, so a program can read the window and compute the next
    /// one without parsing any prose.
    #[must_use]
    pub fn to_value(&self) -> Value {
        json!({
            "id": self.id.0,
            "name": self.name,
            "kind": self.kind.as_str(),
            "path": self.path,
            "start_line": self.start_line,
            "end_line": self.end_line,
            "from": self.from,
            "to": self.to,
            "total_lines": self.total_lines,
            "has_more": self.has_more,
            "text": self.text,
        })
    }

    /// The window as a header line followed by the code, which is the cheapest form to read.
    ///
    /// The header is `path:start-end name (lines from-to of total)`, with `, more follow` added
    /// when something follows the window. The output never ends with a newline.
    #[must_use]
    pub fn render_text(&self) -> String {
        let more = if self.has_more { ", more follow" } else { "" };
        let header = format!(
            "{}:{}-{} {} (lines {}-{} of {}{})",
            self.path,
            self.start_line,
            self.end_line,
            self.name,
            self.from,
            self.to,
            self.total_lines,
            more,
        );
        let body = self.text.trim_end_matches('\n');
        if body.is_empty() {
            header
        } else {
            format!("{header}\n{body}")
        }
    }
}

/// The lines of a text, each keeping its own line ending, so that concatenating consecutive
/// windows reproduces the text byte for byte. Empty text counts as one empty line.
fn split_lines(text: &str) -> Vec<&str> {
    let mut lines: Vec<&str> = text.split_inclusive('\n').collect();
    if lines.is_empty() {
        lines.push("");
    }
    lines
}

/// The lines `from..=to` of `lines`, 1-based, joined back into their original text.
fn join_lines(lines: &[&str], from: u32, to: u32) -> String {
    let start = usize::try_from(from.saturating_sub(1)).unwrap_or(usize::MAX);
    let end = usize::try_from(to).unwrap_or(usize::MAX);
    lines.get(start..end).unwrap_or_default().concat()
}

/// The last line a window starting at `from` may reach without breaking either cap.
///
/// It is the largest `end` in `from..=to` whose text fits [`MAX_WINDOW_TOKENS`], found by binary
/// search because the estimate never falls as lines are appended. When even `from` alone costs
/// more, `from` is returned: a window must hold a line to make progress.
fn capped_end(lines: &[&str], from: u32, to: u32) -> u32 {
    let widest = to.min(from.saturating_add(MAX_WINDOW_LINES - 1));
    if estimate_tokens(&join_lines(lines, from, widest)) <= MAX_WINDOW_TOKENS {
        return widest;
    }
    let mut fits = from;
    let mut over = widest;
    while fits + 1 < over {
        let middle = fits + (over - fits) / 2;
        if estimate_tokens(&join_lines(lines, from, middle)) <= MAX_WINDOW_TOKENS {
            fits = middle;
        } else {
            over = middle;
        }
    }
    fits
}

impl Engine {
    /// Returns one window of the source of the symbol `target` names.
    ///
    /// `window` is `(from, to)` in lines of the symbol's own declaration, 1-based and inclusive on
    /// both ends; `to` defaults to the last line and is clamped to it, and a `from` of zero is
    /// raised to one. The window is shortened to honour [`MAX_WINDOW_LINES`] and
    /// [`MAX_WINDOW_TOKENS`], in which case [`Expansion::has_more`] is set and
    /// [`Expansion::to`] says where to continue.
    ///
    /// Side effects: the expansion is recorded for learning (it is the strongest signal that a
    /// symbol was useful) and a metrics event is written. Nothing is stored in the index.
    ///
    /// # Errors
    /// Returns [`EngineError::NotFound`] or [`EngineError::Ambiguous`] when `target` does not name
    /// exactly one symbol, [`EngineError::Invalid`] when the file changed since it was indexed (so
    /// the stored position cannot be trusted), when `from` is past the end of the symbol, or when
    /// the window ends before it starts, and a storage error when the index cannot be read.
    ///
    /// # Examples
    /// ```no_run
    /// # fn demo(engine: &pn_ultramemory_engine::Engine) -> Result<(), pn_ultramemory_engine::EngineError> {
    /// let whole = engine.expand("parse_config", None)?;
    /// let head = engine.expand("parse_config", Some((1, Some(20))))?;
    /// assert!(head.to <= whole.to);
    /// # Ok(())
    /// # }
    /// ```
    pub fn expand(
        &self,
        target: &str,
        window: Option<(u32, Option<u32>)>,
    ) -> Result<Expansion, EngineError> {
        let record = self.resolve_symbol(target)?;
        let Some(source) = SourceCache::new(self).slice(&record)? else {
            return Err(EngineError::Invalid(format!(
                "`{}` changed since it was indexed, so the stored position of `{}` would show the \
                 wrong lines; run `pn-ultramemory index` and ask again",
                record.path, record.qualified_name
            )));
        };
        let lines = split_lines(&source);
        let total_lines = u32::try_from(lines.len()).unwrap_or(u32::MAX);
        let (asked_from, asked_to) = window.unwrap_or((1, None));
        let from = asked_from.max(1);
        if from > total_lines {
            return Err(EngineError::Invalid(format!(
                "`{}` is {total_lines} lines long, so it has no line {from}; run \
                 `pn-ultramemory expand {} --from 1`",
                record.qualified_name, record.id
            )));
        }
        let asked_to = asked_to.unwrap_or(total_lines).clamp(1, total_lines);
        if asked_to < from {
            return Err(EngineError::Invalid(format!(
                "the window {from}-{asked_to} ends before it starts; run \
                 `pn-ultramemory expand {} --from {asked_to} --to {from}`",
                record.id
            )));
        }
        let to = capped_end(&lines, from, asked_to);
        let text = join_lines(&lines, from, to);

        self.note_expanded(record.id);
        self.record(Event::Expand {
            lines: to.saturating_sub(from).saturating_add(1),
        });
        Ok(Expansion {
            id: record.id,
            name: record.qualified_name,
            kind: record.kind,
            path: record.path,
            start_line: record.span.start_line,
            end_line: record.span.end_line,
            from,
            to,
            total_lines,
            has_more: to < total_lines,
            text,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_WINDOW_LINES, MAX_WINDOW_TOKENS, capped_end, join_lines, split_lines};
    use pn_ultramemory_codec::estimate_tokens;

    /// Splitting keeps every line ending, so the pieces concatenate back to the original text.
    #[test]
    fn splitting_is_lossless() {
        for text in ["", "one", "one\n", "a\nb", "a\nb\n", "a\r\nb\r\n", "\n\n"] {
            let lines = split_lines(text);
            assert_eq!(lines.concat(), text, "{text:?}");
            assert!(!lines.is_empty(), "{text:?}");
        }
        assert_eq!(split_lines("").len(), 1);
        assert_eq!(split_lines("a\nb\n").len(), 2);
        assert_eq!(split_lines("a\nb").len(), 2);
    }

    /// Consecutive windows of a text reproduce it exactly, whatever the step.
    #[test]
    fn windows_tile_the_text() {
        let mut text = String::new();
        for index in 0..17 {
            text.push_str("line ");
            text.push_str(&index.to_string());
            text.push('\n');
        }
        let lines = split_lines(&text);
        for step in 1_u32..=6 {
            let mut joined = String::new();
            let mut from = 1_u32;
            while from <= 17 {
                let to = (from + step - 1).min(17);
                joined.push_str(&join_lines(&lines, from, to));
                from = to + 1;
            }
            assert_eq!(joined, text, "step {step}");
        }
    }

    /// The line cap is honoured, and the token cap shortens a window of expensive lines.
    #[test]
    fn caps_shorten_the_window() {
        let cheap: Vec<&str> = vec!["x\n"; 900];
        assert_eq!(capped_end(&cheap, 1, 900), MAX_WINDOW_LINES);
        assert_eq!(capped_end(&cheap, 700, 900), 900);

        let heavy_line = format!(
            "{}\n",
            "let value_of_something = compute(a, b, c); ".repeat(20)
        );
        let per_line = estimate_tokens(&heavy_line);
        assert!(per_line > 100, "{per_line}");
        let heavy: Vec<&str> = vec![heavy_line.as_str(); 300];
        let end = capped_end(&heavy, 1, 300);
        assert!(end < MAX_WINDOW_LINES, "{end}");
        assert!(estimate_tokens(&join_lines(&heavy, 1, end)) <= MAX_WINDOW_TOKENS);
        assert!(estimate_tokens(&join_lines(&heavy, 1, end + 1)) > MAX_WINDOW_TOKENS);
    }

    /// A single line that already costs more than the cap is still returned whole.
    #[test]
    fn one_huge_line_still_comes_back() {
        let huge = format!("{}\n", "identifier_number_one, ".repeat(4000));
        assert!(estimate_tokens(&huge) > MAX_WINDOW_TOKENS);
        let lines = vec![huge.as_str(), "after\n"];
        assert_eq!(capped_end(&lines, 1, 2), 1);
    }
}
