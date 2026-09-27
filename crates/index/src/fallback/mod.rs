// SPDX-License-Identifier: Apache-2.0
//! Lexical extraction for languages that have no tree-sitter grammar.
//!
//! # Role in the architecture
//! [`extract_lexical`] is what [`crate::TreeSitterExtractor`] falls back to for
//! `Language::Other(_)` (Kotlin, Swift, Scala, Dart, Lua, shell, Perl, Elixir, Haskell,
//! Clojure, SQL, R, Julia, Zig, ...). It reads the file line by line, recognises declaration
//! shapes ([`shapes`]), finds where each body ends by matching brackets or by indentation, and
//! attaches the comment block that precedes a declaration as its documentation.
//!
//! # Invariants
//! * One pass over the lines: time and memory are linear in the size of the file, whatever the
//!   nesting. Declaration stacks are bounded, so hostile input cannot make the scan quadratic.
//! * Output is deterministic and has the same shape as a grammar-backed extraction, with
//!   `Visibility::Unknown` unless a keyword says otherwise, an empty `outline` (the lexical scan
//!   is too coarse to tell control flow), and `parse_errors` always zero.
//! * References are identifiers immediately followed by `(` inside a declaration body.

mod scan;
mod shapes;

use pn_ultramemory_core::{
    FileExtract, Language, RefKind, ReferenceDraft, Span, SymbolDraft, SymbolKind, hash_normalized,
};

use crate::extract::count_lines;
use crate::extract::text::{
    clean_comments, collapse_whitespace, tidy_signature, to_u32, trim_signature_tail,
    truncate_chars,
};
use scan::{Call, Config, LineScan, State, config_for, scan_line};
use shapes::{Shape, detect};

/// Deepest nesting of declarations tracked at once.
const MAX_OPEN: usize = 64;
/// Most references reported for one file.
const MAX_REFERENCES: usize = 20_000;
/// Longest signature, in characters.
const MAX_SIGNATURE: usize = 240;
/// Most extra lines read to complete a multi-line signature.
const MAX_HEADER_LINES: usize = 8;

/// Words that look like calls but are not.
const NOT_CALLS: &[&str] = &[
    "if",
    "elif",
    "elsif",
    "else",
    "unless",
    "while",
    "until",
    "for",
    "foreach",
    "switch",
    "when",
    "catch",
    "except",
    "return",
    "and",
    "or",
    "not",
    "fn",
    "fun",
    "func",
    "function",
    "def",
    "defp",
    "defmodule",
    "class",
    "struct",
    "enum",
    "interface",
    "trait",
    "sizeof",
    "typeof",
    "match",
    "case",
    "do",
    "then",
    "in",
    "of",
    "with",
    "try",
    "using",
    "lambda",
    "await",
    "yield",
    "throw",
    "raise",
    "new",
    "guard",
    "select",
    "let",
    "var",
    "val",
    "where",
];

/// One line of the source.
struct Line<'a> {
    /// Byte offset of the first character.
    start: usize,
    /// Byte offset just after the last character, excluding the line terminator.
    end: usize,
    /// The text without the terminator.
    text: &'a str,
}

/// How the end of a declaration is found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Ends when the bracket depth returns to `close_depth` after having been opened.
    Brace {
        /// The depth outside the body.
        close_depth: i64,
        /// Whether the opening bracket has been seen.
        opened: bool,
    },
    /// Ends at the first later line that is not indented deeper than the header.
    Indent,
    /// Ends at the first line that ends with a semicolon.
    Semicolon,
}

/// A declaration whose end has not been found yet.
struct Open {
    /// Index of the symbol.
    symbol: usize,
    /// How its end is found.
    mode: Mode,
    /// Indentation of the header line, in bytes.
    indent: usize,
    /// Index of the header line.
    header_line: usize,
    /// Bracket depth before the header line.
    depth_before: i64,
    /// Byte offset where the hash of the declaration starts.
    hash_start: usize,
}

/// A run of comment lines directly above the current line.
#[derive(Default)]
struct Run {
    /// The comments, block comments as one text each.
    texts: Vec<String>,
    /// Index of the first line of the run (comments and annotations).
    first_line: usize,
    /// Index of the last line of the run.
    last_line: usize,
    /// Index of the first annotation line in the run.
    attr_line: Option<usize>,
    /// Whether the run holds anything.
    active: bool,
}

impl Run {
    /// Forgets the run.
    fn clear(&mut self) {
        self.texts.clear();
        self.attr_line = None;
        self.active = false;
    }

    /// Starts or extends the run with a line.
    fn touch(&mut self, line: usize) {
        if !self.active {
            self.first_line = line;
            self.active = true;
        }
        self.last_line = line;
    }
}

/// The scanner state while extracting one file.
struct Lexer<'a> {
    /// The whole source.
    source: &'a str,
    /// The lines of the source.
    lines: Vec<Line<'a>>,
    /// The language name.
    language: &'static str,
    /// The lexical conventions of the language.
    config: Config,
    /// Scanner state carried between lines.
    state: State,
    /// Current bracket depth.
    depth: i64,
    /// Declarations still open, innermost last.
    open: Vec<Open>,
    /// Symbols found so far.
    symbols: Vec<SymbolDraft>,
    /// References found so far.
    references: Vec<ReferenceDraft>,
    /// The comment run above the current line.
    run: Run,
    /// Index of the last line that holds code (not blank, not only a comment).
    last_code: usize,
    /// Whether an Elixir heredoc documentation block is being read.
    heredoc: bool,
    /// The symbol that an Elixir `@moduledoc` block documents, while it is being read.
    doc_target: Option<usize>,
}

/// Splits a source into lines with their byte offsets.
fn split_lines(source: &str) -> Vec<Line<'_>> {
    let mut lines = Vec::new();
    let mut start = 0;
    for chunk in source.split_inclusive('\n') {
        let text = chunk.strip_suffix('\n').unwrap_or(chunk);
        let text = text.strip_suffix('\r').unwrap_or(text);
        lines.push(Line {
            start,
            end: start + text.len(),
            text,
        });
        start += chunk.len();
    }
    lines
}

/// Extracts symbols and references from a source in a language without a grammar.
///
/// The result has the same shape as a grammar-backed extraction; visibility is `Unknown` unless a
/// keyword such as `pub`, `private` or `defp` says otherwise.
#[must_use]
pub(crate) fn extract_lexical(language: Language, source: &str) -> FileExtract {
    let name = language.name();
    let mut lexer = Lexer {
        source,
        lines: split_lines(source),
        language: name,
        config: config_for(name),
        state: State::Code,
        depth: 0,
        open: Vec::new(),
        symbols: Vec::new(),
        references: Vec::new(),
        run: Run::default(),
        last_code: 0,
        heredoc: false,
        doc_target: None,
    };
    for index in 0..lexer.lines.len() {
        lexer.step(index);
    }
    lexer.finish(language)
}

/// Returns the width of the leading whitespace of a line.
fn indentation(text: &str) -> usize {
    text.len() - text.trim_start().len()
}

/// Returns `true` when `text` starts with the word `end`.
fn starts_with_end(text: &str) -> bool {
    text.strip_prefix("end")
        .is_some_and(|rest| !rest.starts_with(|c: char| c.is_alphanumeric() || c == '_'))
}

impl Lexer<'_> {
    /// Processes one line.
    fn step(&mut self, index: usize) {
        if self.step_line(index) {
            self.last_code = index;
        }
    }

    /// Processes one line and returns `true` when it holds code.
    fn step_line(&mut self, index: usize) -> bool {
        let text = self.lines[index].text;
        let state_before = self.state;
        let depth_before = self.depth;
        let scan = scan_line(&self.config, &mut self.state, text);
        self.depth += scan.opens - scan.closes;
        if self.heredoc {
            self.continue_heredoc(index, text, state_before);
            return false;
        }
        self.close_finished(index, text, &scan);
        if state_before == State::Block || (scan.comment_or_blank && !scan.blank) {
            self.add_comment(index, text, state_before == State::Block);
            return false;
        }
        if scan.blank {
            self.run.clear();
            return false;
        }
        let head = text.trim_start();
        if self.language == "elixir" && (head.starts_with("@doc") || head.starts_with("@moduledoc"))
        {
            self.start_elixir_doc(index, head);
            return false;
        }
        if self.is_attribute_line(head) {
            self.run.touch(index);
            if self.run.attr_line.is_none() {
                self.run.attr_line = Some(index);
            }
            return true;
        }
        let declared = match detect(self.language, head) {
            Some(shape) if self.open.len() < MAX_OPEN => {
                self.push_declaration(index, &shape, &scan, depth_before);
                true
            }
            _ => false,
        };
        if !declared {
            self.add_calls(&scan.calls, index);
        }
        self.run.clear();
        true
    }

    /// Returns `true` for a line that only carries annotations or attributes.
    fn is_attribute_line(&self, head: &str) -> bool {
        if !head.starts_with('@') {
            return false;
        }
        if self.language == "elixir" {
            return true;
        }
        head.split_whitespace()
            .all(|word| word.starts_with('@') || word.starts_with('(') || word.ends_with(')'))
    }

    /// Adds a comment line to the current run.
    fn add_comment(&mut self, index: usize, text: &str, continuation: bool) {
        if self.run.active && index > self.run.last_line + 1 {
            self.run.clear();
        }
        if index == 0 && text.starts_with("#!") {
            return;
        }
        self.run.touch(index);
        if continuation {
            match self.run.texts.last_mut() {
                Some(last) => {
                    last.push('\n');
                    last.push_str(text);
                }
                None => self.run.texts.push(text.to_owned()),
            }
        } else {
            self.run.texts.push(text.trim_start().to_owned());
        }
    }

    /// Starts reading an Elixir `@doc` or `@moduledoc` attribute as documentation.
    ///
    /// `@doc` documents the declaration below it; `@moduledoc` documents the module whose header
    /// is the innermost open declaration.
    fn start_elixir_doc(&mut self, index: usize, head: &str) {
        let value = head
            .split_once(char::is_whitespace)
            .map_or("", |(_, rest)| rest.trim());
        if self.run.active && index > self.run.last_line + 1 {
            self.run.clear();
        }
        if head.starts_with("@moduledoc") {
            self.doc_target = self.open.last().map(|o| o.symbol);
        } else {
            self.doc_target = None;
        }
        self.run.touch(index);
        if let Some(rest) = value.strip_prefix("\"\"\"") {
            self.heredoc = self.state != State::Code;
            let rest = rest.trim();
            if !rest.is_empty() {
                self.run.texts.push(format!("# {rest}"));
            }
            if !self.heredoc {
                self.finish_doc_target();
            }
        } else {
            if value != "false" {
                self.run
                    .texts
                    .push(format!("# {}", value.trim_matches('"')));
            }
            self.finish_doc_target();
        }
    }

    /// Reads one more line of an Elixir heredoc documentation block.
    fn continue_heredoc(&mut self, index: usize, text: &str, state_before: State) {
        self.run.touch(index);
        if state_before == State::Code || self.state == State::Code {
            self.heredoc = false;
            let before = text.split("\"\"\"").next().unwrap_or("").trim();
            if !before.is_empty() {
                self.run.texts.push(format!("# {before}"));
            }
            self.finish_doc_target();
            return;
        }
        self.run.texts.push(format!("# {}", text.trim()));
    }

    /// Moves a finished `@moduledoc` block from the run to the module it documents.
    fn finish_doc_target(&mut self) {
        let Some(target) = self.doc_target.take() else {
            return;
        };
        if let Some(doc) = self.run_doc() {
            if let Some(symbol) = self.symbols.get_mut(target) {
                symbol.doc.get_or_insert(doc);
            }
        }
        self.run.clear();
    }

    /// Closes the declarations that end at the current line.
    fn close_finished(&mut self, index: usize, text: &str, scan: &LineScan) {
        let depth = self.depth;
        for open in &mut self.open {
            if let Mode::Brace {
                close_depth,
                opened: false,
            } = open.mode
            {
                if depth > close_depth {
                    open.mode = Mode::Brace {
                        close_depth,
                        opened: true,
                    };
                } else if index > open.header_line + MAX_HEADER_LINES + 1 {
                    open.mode = Mode::Indent;
                }
            }
        }
        let head = text.trim_start();
        let indent = indentation(text);
        loop {
            let Some(top) = self.open.last() else {
                return;
            };
            let end_line = match top.mode {
                Mode::Brace {
                    close_depth,
                    opened: true,
                } if depth <= close_depth => Some(index),
                Mode::Semicolon if index > top.header_line && text.trim_end().ends_with(';') => {
                    Some(index)
                }
                Mode::Indent
                    if index > top.header_line
                        && !scan.comment_or_blank
                        && indent <= top.indent =>
                {
                    let inclusive = (starts_with_end(head) && indent == top.indent)
                        || (head.starts_with(['}', ')', ']']) && depth >= top.depth_before);
                    Some(if inclusive {
                        index
                    } else {
                        self.previous_content(index)
                    })
                }
                _ => None,
            };
            match end_line {
                Some(end) => self.close_top(end),
                None => return,
            }
        }
    }

    /// Returns the index of the last line of code before the line being processed.
    fn previous_content(&self, index: usize) -> usize {
        self.last_code.min(index)
    }

    /// Closes the innermost open declaration at `end_line`.
    fn close_top(&mut self, end_line: usize) {
        let Some(open) = self.open.pop() else {
            return;
        };
        let end_line = end_line.min(self.lines.len().saturating_sub(1));
        let end_byte = self.lines[end_line].end;
        let symbol = &mut self.symbols[open.symbol];
        symbol.span.end_line = to_u32(end_line) + 1;
        symbol.span.end_byte = to_u32(end_byte);
        let body = self.source.get(open.hash_start..end_byte).unwrap_or("");
        symbol.body_hash = hash_normalized(body);
    }

    /// Turns the comments of the current run into documentation text.
    ///
    /// Block comments in the language's own syntax (`--[[ ]]`, `{- -}`, `#= =#`) are rewritten to
    /// the common `/* */` form first, so their markers are removed like any other.
    fn run_doc(&self) -> Option<String> {
        let normalized: Vec<String> = self
            .run
            .texts
            .iter()
            .map(|text| match self.config.block {
                Some((open, close)) if text.starts_with(open) => {
                    let inner = text[open.len()..].trim_end();
                    let inner = inner.strip_suffix(close).unwrap_or(inner);
                    format!("/*{inner}*/")
                }
                _ => text.clone(),
            })
            .collect();
        let texts: Vec<&str> = normalized.iter().map(String::as_str).collect();
        let doc = clean_comments(&texts)?;
        if self.language == "haskell" {
            // Haddock markers: `-- | text` documents what follows, `-- ^ text` what precedes.
            let stripped = doc
                .strip_prefix("| ")
                .or_else(|| doc.strip_prefix("^ "))
                .unwrap_or(&doc);
            return Some(stripped.to_owned());
        }
        Some(doc)
    }

    /// Reports a declaration found on the header line `index`.
    fn push_declaration(
        &mut self,
        index: usize,
        shape: &Shape,
        scan: &LineScan,
        depth_before: i64,
    ) {
        let text = self.lines[index].text;
        let parent = self.open.last().map(|o| o.symbol);
        let parent_symbol = parent.and_then(|p| self.symbols.get(p));
        let kind = match (shape.kind, parent_symbol.map(|p| p.kind)) {
            (
                SymbolKind::Function,
                Some(
                    SymbolKind::Class
                    | SymbolKind::Struct
                    | SymbolKind::Interface
                    | SymbolKind::Enum
                    | SymbolKind::Other,
                ),
            ) => SymbolKind::Method,
            (kind, _) => kind,
        };
        let qualified = match parent_symbol {
            Some(p) => format!("{}.{}", p.qualified_name, shape.written),
            None => shape.written.clone(),
        };
        let signature = self.signature(index, shape);
        let attached = self.run.active && self.run.last_line + 1 == index;
        let first_line = if attached { self.run.first_line } else { index };
        let hash_line = match (attached, self.run.attr_line) {
            (true, Some(attr)) => attr,
            _ => index,
        };
        let doc = if attached && !self.run.texts.is_empty() {
            self.run_doc()
        } else {
            None
        };
        let hash_start = self.lines[hash_line].start;
        let end_line = index;
        let symbol_index = self.symbols.len();
        self.symbols.push(SymbolDraft {
            name: shape.name.clone(),
            qualified_name: qualified,
            kind,
            sig_hash: hash_normalized(&signature),
            signature,
            doc,
            visibility: shape.visibility,
            span: Span {
                start_line: to_u32(first_line) + 1,
                end_line: to_u32(end_line) + 1,
                start_byte: to_u32(
                    self.lines[first_line].start + indentation(self.lines[first_line].text),
                ),
                end_byte: to_u32(self.lines[index].end),
            },
            parent,
            outline: Vec::new(),
            body_hash: 0,
        });
        let mode = self.mode_for(index, scan, depth_before);
        self.open.push(Open {
            symbol: symbol_index,
            mode,
            indent: indentation(text),
            header_line: index,
            depth_before,
            hash_start,
        });
        let single_line = matches!(mode, Mode::Semicolon) && text.trim_end().ends_with(';')
            || (scan.opens > 0 && scan.opens <= scan.closes && self.depth <= depth_before);
        if single_line {
            self.close_top(index);
        }
    }

    /// Chooses how the end of the declaration on the header line is found.
    fn mode_for(&self, index: usize, scan: &LineScan, depth_before: i64) -> Mode {
        if self.language == "sql" {
            return Mode::Semicolon;
        }
        if scan.opens > 0 && self.depth > depth_before {
            return Mode::Brace {
                close_depth: depth_before,
                opened: true,
            };
        }
        let unfinished_header = bracket_balance(self.lines[index].text) > 0;
        if scan.opens == 0 && (unfinished_header || self.next_code_starts_with_open(index)) {
            return Mode::Brace {
                close_depth: depth_before,
                opened: false,
            };
        }
        Mode::Indent
    }

    /// Returns `true` when the next line with code begins with the opening bracket.
    fn next_code_starts_with_open(&self, index: usize) -> bool {
        self.lines
            .iter()
            .skip(index + 1)
            .take(8)
            .map(|l| l.text.trim())
            .find(|t| !t.is_empty())
            .is_some_and(|t| t.starts_with(self.config.braces.0))
    }

    /// Builds the signature of a declaration whose header starts on line `index`.
    fn signature(&self, index: usize, shape: &Shape) -> String {
        let head = self.lines[index].text.trim_start();
        let mut buffer = String::from(head.get(shape.header_start..).unwrap_or(head));
        let mut parens = bracket_balance(&buffer);
        let mut next = index + 1;
        while parens > 0 && next < self.lines.len() && next <= index + MAX_HEADER_LINES {
            buffer.push(' ');
            buffer.push_str(self.lines[next].text.trim());
            parens = bracket_balance(&buffer);
            next += 1;
        }
        let opener = self.config.braces.0;
        let cut = if opener == '{' {
            buffer.find('{').unwrap_or(buffer.len())
        } else {
            buffer.len()
        };
        let collapsed = tidy_signature(&collapse_whitespace(&buffer[..cut]));
        let collapsed = collapsed
            .strip_suffix(" do")
            .or_else(|| collapsed.strip_suffix(" then"))
            .unwrap_or(&collapsed);
        truncate_chars(trim_signature_tail(collapsed).to_owned(), MAX_SIGNATURE)
    }

    /// Reports the calls found on a line that is inside a declaration body.
    fn add_calls(&mut self, calls: &[Call], index: usize) {
        if self.open.is_empty() {
            return;
        }
        for call in calls {
            if NOT_CALLS.contains(&call.name.as_str()) || call.name.len() > 128 {
                continue;
            }
            let kind = if call.after_new {
                RefKind::Type
            } else {
                RefKind::Call
            };
            if self.references.len() < MAX_REFERENCES {
                self.references.push(ReferenceDraft {
                    name: call.name.clone(),
                    kind,
                    line: to_u32(index) + 1,
                    owner: self.open.last().map(|o| o.symbol),
                    qualifier: call.qualifier.clone(),
                });
            }
        }
    }

    /// Closes everything still open at the end of the file and builds the result.
    fn finish(mut self, language: Language) -> FileExtract {
        let end = self.last_code;
        while !self.open.is_empty() {
            self.close_top(end);
        }
        FileExtract {
            language,
            symbols: self.symbols,
            references: self.references,
            imports: Vec::new(),
            line_count: count_lines(self.source),
            parse_errors: 0,
        }
    }
}

/// Returns how many `(` are still open in `text`.
fn bracket_balance(text: &str) -> i32 {
    text.chars().fold(0, |depth, c| match c {
        '(' => depth + 1,
        ')' => depth - 1,
        _ => depth,
    })
}
