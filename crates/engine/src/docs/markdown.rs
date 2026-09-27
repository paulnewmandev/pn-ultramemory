// SPDX-License-Identifier: Apache-2.0
//! A Markdown API reference built from the index alone.
//!
//! # Role in the architecture
//! Application layer. [`crate::Engine::doc_markdown`] reads the indexed files and their symbols and
//! writes one Markdown document: a heading per file and a table of its public declarations. It works
//! for every language the indexer understands, because it uses nothing but what extraction already
//! recorded — no source file is opened and no language-specific rule is applied.
//!
//! # Invariants
//! * **Deterministic.** Files come from storage in path order and symbols in source order, and
//!   nothing else decides the layout.
//! * **Escaped.** Every cell is escaped so that a pipe, a backtick or an angle bracket inside a
//!   signature cannot break the table or open markup. Newlines and tabs become spaces, because a
//!   table cell is one line.
//! * **Bounded.** At most [`MAX_SYMBOLS`] rows are written; the count of what was left out is
//!   stated at the end rather than silently dropped.

use pn_ultramemory_core::{SymbolKind, Visibility, first_sentence};

use crate::engine::Engine;
use crate::error::EngineError;

/// The heading used when the caller names no title.
const DEFAULT_TITLE: &str = "API reference";

/// The most rows one reference holds.
const MAX_SYMBOLS: usize = 5_000;

/// Escapes one table cell: the three characters that would break a table or open markup, and any
/// line break.
fn escape(cell: &str) -> String {
    let mut out = String::with_capacity(cell.len() + 8);
    for ch in cell.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '|' => out.push_str("\\|"),
            '`' => out.push_str("\\`"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\n' | '\r' | '\t' => out.push(' '),
            other => out.push(other),
        }
    }
    out
}

/// Collapses a title into one line of plain text.
fn clean_title(title: &str) -> String {
    let words: Vec<&str> = title.split_whitespace().collect();
    let joined = words.join(" ");
    if joined.is_empty() {
        DEFAULT_TITLE.to_owned()
    } else {
        joined
    }
}

/// Appends one row of the table: kind, name, signature and the first sentence of the
/// documentation.
fn push_row(out: &mut String, kind: SymbolKind, name: &str, signature: &str, summary: &str) {
    out.push_str("| ");
    out.push_str(kind.as_str());
    out.push_str(" | ");
    out.push_str(&escape(name));
    out.push_str(" | ");
    out.push_str(&escape(signature));
    out.push_str(" | ");
    out.push_str(&escape(summary));
    out.push_str(" |\n");
}

impl Engine {
    /// A Markdown API reference for the repository, or for the part of it under `path_prefix`.
    ///
    /// The document opens with `# title` and then, for each file that declares at least one public
    /// symbol, a `## path` heading and a table of `kind`, `name`, `signature` and the first sentence
    /// of the documentation. Modules are not listed: a module's own documentation is a different
    /// thing from a declaration's. At most 5 000 symbols are written, and a final line says how many
    /// were left out.
    ///
    /// # Errors
    /// Returns a storage error when the files or their symbols cannot be read.
    pub fn doc_markdown(
        &self,
        path_prefix: Option<&str>,
        title: Option<&str>,
    ) -> Result<String, EngineError> {
        let prefix = path_prefix
            .map(|prefix| prefix.trim().trim_start_matches("./"))
            .filter(|prefix| !prefix.is_empty());
        let mut out = String::new();
        out.push_str("# ");
        out.push_str(&escape(&clean_title(title.unwrap_or(DEFAULT_TITLE))));
        out.push('\n');

        let mut written = 0_usize;
        let mut omitted = 0_usize;
        for file in self.storage().list_files()? {
            if prefix.is_some_and(|prefix| !file.path.starts_with(prefix)) {
                continue;
            }
            let symbols = self.storage().symbols_in_file(&file.path)?;
            let public: Vec<_> = symbols
                .into_iter()
                .filter(|symbol| {
                    symbol.visibility == Visibility::Public && symbol.kind != SymbolKind::Module
                })
                .collect();
            if public.is_empty() {
                continue;
            }
            if written >= MAX_SYMBOLS {
                omitted += public.len();
                continue;
            }
            out.push_str("\n## ");
            out.push_str(&escape(&file.path));
            out.push_str("\n\n| kind | name | signature | summary |\n|---|---|---|---|\n");
            for symbol in public {
                if written >= MAX_SYMBOLS {
                    omitted += 1;
                    continue;
                }
                let summary = symbol
                    .doc
                    .as_deref()
                    .map(first_sentence)
                    .unwrap_or_default();
                push_row(
                    &mut out,
                    symbol.kind,
                    &symbol.qualified_name,
                    &symbol.signature,
                    &summary,
                );
                written += 1;
            }
        }
        if omitted > 0 {
            out.push_str("\n> ");
            out.push_str(&omitted.to_string());
            out.push_str(" more public symbols were left out at the limit of ");
            out.push_str(&MAX_SYMBOLS.to_string());
            out.push_str("; narrow the reference with a path prefix.\n");
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_TITLE, clean_title, escape, push_row};
    use pn_ultramemory_core::SymbolKind;

    /// Pipes, backticks, angle brackets and backslashes are escaped, and line breaks become
    /// spaces.
    #[test]
    fn cells_are_escaped() {
        assert_eq!(escape("a|b"), "a\\|b");
        assert_eq!(escape("`code`"), "\\`code\\`");
        assert_eq!(escape("Vec<u8>"), "Vec&lt;u8&gt;");
        assert_eq!(escape("a\\b"), "a\\\\b");
        assert_eq!(escape("one\ntwo\tthree"), "one two three");
        assert_eq!(escape("plain_name"), "plain_name");
    }

    /// A title is collapsed onto one line, and a blank one falls back to the default.
    #[test]
    fn titles_are_cleaned() {
        assert_eq!(clean_title("  My   API\nreference "), "My API reference");
        assert_eq!(clean_title("   "), DEFAULT_TITLE);
    }

    /// A row has four escaped cells and ends the line.
    #[test]
    fn rows_have_four_cells() {
        let mut out = String::new();
        push_row(
            &mut out,
            SymbolKind::Function,
            "f",
            "fn f(a: Vec<u8>) -> bool",
            "Checks | it.",
        );
        assert_eq!(
            out,
            "| function | f | fn f(a: Vec&lt;u8&gt;) -&gt; bool | Checks \\| it. |\n"
        );
    }
}
