// SPDX-License-Identifier: Apache-2.0
//! A tiny line scanner for the lexical fallback: it separates code from comments and strings and
//! reports what the declaration finder needs to know about one line.
//!
//! # Role in the architecture
//! Used by [`super`] for languages without a grammar. It tracks block comments and triple-quoted
//! strings across lines, and inside a line it counts structural brackets, finds calls
//! (identifiers immediately followed by `(`) and detects comment-only lines.
//!
//! # Invariants
//! * Linear time: every character of the file is looked at a bounded number of times.
//! * Strings other than triple-quoted ones never continue on the next line, so one unbalanced
//!   quote cannot corrupt the rest of the file.

/// The lexical conventions of one fallback language.
#[derive(Debug, Clone, Copy)]
pub(super) struct Config {
    /// Prefixes that start a comment running to the end of the line.
    pub(super) line_comments: &'static [&'static str],
    /// Opening and closing marks of a block comment, if the language has one.
    pub(super) block: Option<(&'static str, &'static str)>,
    /// The opening and closing bracket that delimit a body.
    pub(super) braces: (char, char),
    /// Characters that start a string literal.
    pub(super) quotes: &'static [char],
    /// Whether `-` is part of identifiers (shell, Clojure).
    pub(super) dash_in_names: bool,
}

/// Returns the conventions of a fallback language by its lowercase name.
pub(super) fn config_for(name: &str) -> Config {
    let base = Config {
        line_comments: &["//"],
        block: Some(("/*", "*/")),
        braces: ('{', '}'),
        quotes: &['"', '\'', '`'],
        dash_in_names: false,
    };
    match name {
        "lua" => Config {
            line_comments: &["--"],
            block: Some(("--[[", "]]")),
            ..base
        },
        "haskell" => Config {
            line_comments: &["--"],
            block: Some(("{-", "-}")),
            quotes: &['"'],
            ..base
        },
        "sql" => Config {
            line_comments: &["--"],
            ..base
        },
        "clojure" => Config {
            line_comments: &[";"],
            block: None,
            braces: ('(', ')'),
            quotes: &['"'],
            dash_in_names: true,
        },
        "shell" => Config {
            line_comments: &["#"],
            block: None,
            dash_in_names: true,
            ..base
        },
        "perl" | "elixir" | "r" => Config {
            line_comments: &["#"],
            block: None,
            ..base
        },
        "julia" => Config {
            line_comments: &["#"],
            block: Some(("#=", "=#")),
            ..base
        },
        _ => base,
    }
}

/// Where the scanner is between two lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum State {
    /// Ordinary code.
    Code,
    /// Inside a block comment, waiting for the closing mark.
    Block,
    /// Inside a triple-quoted string with the given quote character.
    Triple(char),
}

/// A call found in a line: an identifier immediately followed by `(`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Call {
    /// The called name.
    pub(super) name: String,
    /// The identifier written before a `.`, `::`, `->` or `:` separator, if any.
    pub(super) qualifier: Option<String>,
    /// Whether the name follows the keyword `new`.
    pub(super) after_new: bool,
}

/// What the scanner learned about one line.
#[derive(Debug, Default)]
pub(super) struct LineScan {
    /// Opening brackets outside strings and comments.
    pub(super) opens: i64,
    /// Closing brackets outside strings and comments.
    pub(super) closes: i64,
    /// `true` when the line holds no code: it is blank or only a comment.
    pub(super) comment_or_blank: bool,
    /// `true` when the line has comment text (a whole-line comment or the inside of a block).
    pub(super) has_comment: bool,
    /// `true` when the line is empty or whitespace.
    pub(super) blank: bool,
    /// The calls found in code.
    pub(super) calls: Vec<Call>,
}

/// Returns `true` for a character that can be part of an identifier.
fn is_ident(c: char, dash: bool) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$' || (dash && c == '-')
}

/// Scans one line, updating the state that carries over to the next one.
pub(super) fn scan_line(config: &Config, state: &mut State, line: &str) -> LineScan {
    let mut scan = LineScan {
        blank: line.trim().is_empty(),
        ..LineScan::default()
    };
    let mut has_code = false;
    let mut i = 0;
    let mut ident: Option<usize> = None;
    while i < line.len() {
        let rest = &line[i..];
        let Some(c) = rest.chars().next() else {
            break;
        };
        match *state {
            State::Block => {
                scan.has_comment = true;
                let close = config.block.map_or("", |b| b.1);
                match rest.find(close).filter(|_| !close.is_empty()) {
                    Some(at) => {
                        i += at + close.len();
                        *state = State::Code;
                    }
                    None => i = line.len(),
                }
                continue;
            }
            State::Triple(q) => {
                has_code = true;
                let triple: String = std::iter::repeat_n(q, 3).collect();
                match rest.find(&triple) {
                    Some(at) => {
                        i += at + 3;
                        *state = State::Code;
                    }
                    None => i = line.len(),
                }
                continue;
            }
            State::Code => {}
        }
        if is_ident(c, config.dash_in_names) && !(c == '-' && ident.is_none()) {
            if ident.is_none() {
                ident = Some(i);
            }
            has_code = true;
            i += c.len_utf8();
            continue;
        }
        if let Some(start) = ident.take() {
            if c == '(' {
                scan.calls.push(make_call(line, start, i));
            }
        }
        if let Some((open, _)) = config.block {
            if rest.starts_with(open) {
                scan.has_comment = true;
                *state = State::Block;
                i += open.len();
                continue;
            }
        }
        if config.line_comments.iter().any(|p| rest.starts_with(p)) {
            scan.has_comment = true;
            break;
        }
        if config.quotes.contains(&c) {
            has_code = true;
            let triple: String = std::iter::repeat_n(c, 3).collect();
            if rest.starts_with(&triple) && c != '\'' {
                *state = State::Triple(c);
                i += 3;
                continue;
            }
            i += skip_string(rest, c);
            continue;
        }
        if !c.is_whitespace() {
            has_code = true;
        }
        if c == config.braces.0 {
            scan.opens += 1;
        } else if c == config.braces.1 {
            scan.closes += 1;
        }
        i += c.len_utf8();
    }
    scan.comment_or_blank = !has_code;
    scan
}

/// Returns the byte length of a string literal that starts at the beginning of `rest`, up to
/// and including its closing quote, or up to the end of the line if it is not closed.
fn skip_string(rest: &str, quote: char) -> usize {
    let mut escaped = false;
    for (offset, c) in rest.char_indices().skip(1) {
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == quote {
            return offset + c.len_utf8();
        }
    }
    rest.len()
}

/// Builds a [`Call`] for the identifier `line[start..end]`.
fn make_call(line: &str, start: usize, end: usize) -> Call {
    let name = line[start..end].to_owned();
    let before = &line[..start];
    let trimmed = before.trim_end();
    let after_new = trimmed.ends_with("new") && before.len() > trimmed.len();
    let separator = ["::", "->", ".", ":"]
        .iter()
        .find(|s| before.ends_with(**s))
        .map_or(0, |s| s.len());
    let qualifier = (separator > 0)
        .then(|| {
            let head = &before[..before.len() - separator];
            let begin = head
                .char_indices()
                .rev()
                .take_while(|(_, c)| is_ident(*c, false))
                .last()
                .map_or(head.len(), |(i, _)| i);
            let word = &head[begin..];
            (!word.is_empty()).then(|| word.to_owned())
        })
        .flatten();
    Call {
        name,
        qualifier,
        after_new,
    }
}

#[cfg(test)]
mod tests {
    use super::{State, config_for, scan_line};

    /// Brackets inside strings and comments are ignored.
    #[test]
    fn brackets_in_strings_and_comments_do_not_count() {
        let config = config_for("kotlin");
        let mut state = State::Code;
        let scan = scan_line(&config, &mut state, r#"val s = "{" // }"#);
        assert_eq!((scan.opens, scan.closes), (0, 0));
        let scan = scan_line(&config, &mut state, "fun f() {");
        assert_eq!((scan.opens, scan.closes), (1, 0));
    }

    /// Block comments and triple-quoted strings carry across lines.
    #[test]
    fn multi_line_constructs_carry_state() {
        let config = config_for("kotlin");
        let mut state = State::Code;
        let first = scan_line(&config, &mut state, "/* start {");
        assert!(first.comment_or_blank && state == State::Block);
        let second = scan_line(&config, &mut state, "still } inside */ val x = 1");
        assert_eq!(state, State::Code);
        assert!(!second.comment_or_blank);
        assert_eq!((second.opens, second.closes), (0, 0));
        let third = scan_line(&config, &mut state, "val t = \"\"\"{");
        assert_eq!(state, State::Triple('"'));
        assert_eq!(third.opens, 0);
        let fourth = scan_line(&config, &mut state, "}\"\"\"");
        assert_eq!(state, State::Code);
        assert_eq!(fourth.closes, 0);
    }

    /// Calls are identifiers followed by an opening parenthesis, with their qualifier.
    #[test]
    fn calls_are_found_with_qualifiers() {
        let config = config_for("lua");
        let mut state = State::Code;
        let scan = scan_line(&config, &mut state, "local x = string.format(a) + run(b)");
        let names: Vec<_> = scan
            .calls
            .iter()
            .map(|c| (c.name.as_str(), c.qualifier.as_deref()))
            .collect();
        assert_eq!(names, [("format", Some("string")), ("run", None)]);
    }

    /// A comment-only line and a blank line hold no code.
    #[test]
    fn comment_and_blank_lines() {
        let config = config_for("shell");
        let mut state = State::Code;
        assert!(scan_line(&config, &mut state, "# note").comment_or_blank);
        assert!(scan_line(&config, &mut state, "   ").blank);
        assert!(!scan_line(&config, &mut state, "echo hi # note").comment_or_blank);
    }
}
