// SPDX-License-Identifier: Apache-2.0
//! Lexical pre-pass of the TOON decoder: lines, comments, blank lines and indentation.
//!
//! # Role in the architecture
//! Before any structure is interpreted, sections 5.1 and 12 of the TOON 4.1 specification require a
//! pre-pass: remove a leading byte-order mark, split on LF while excluding one CR at the end of a
//! line, strip trailing spaces, drop full-line comments, and measure indentation. [`split_lines`]
//! does exactly that and hands the parser only content lines, each annotated with its depth and
//! with whether blank lines preceded it (the parser needs that for the blank-line rule).
//!
//! # Invariants
//! * Line numbers refer to the original input, so errors point at the right place.
//! * Comment lines and blank lines never appear in the output.
//! * Strict mode requires the leading spaces to be an exact multiple of the indent size and
//!   forbids tabs in indentation. Non-strict mode computes `floor(width / indent)` and accepts
//!   leading tabs, each counting as one full indent level (documented deviation point of section
//!   12: the depth of tabs is implementation-defined).

use crate::error::DecodeError;

/// A content line of the document, ready for the parser.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Line<'a> {
    /// 1-based line number in the original input.
    pub(crate) number: usize,
    /// Indentation depth in levels.
    pub(crate) depth: usize,
    /// The line without indentation and trailing spaces.
    pub(crate) content: &'a str,
    /// Line number of the first blank line between the previous content line and this one.
    pub(crate) blank_before: Option<usize>,
}

/// Splits `input` into content lines.
///
/// `indent` is the size of one indentation level (at least 1).
///
/// # Errors
///
/// In strict mode, returns an error for a line indented with a tab or with a number of spaces that
/// is not a multiple of `indent`.
pub(crate) fn split_lines(
    input: &str,
    indent: usize,
    strict: bool,
) -> Result<Vec<Line<'_>>, DecodeError> {
    let text = input.strip_prefix('\u{feff}').unwrap_or(input);
    let mut lines = Vec::new();
    let mut pending_blank = None;
    for (index, raw) in text.split('\n').enumerate() {
        let number = index + 1;
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        if raw.trim_start_matches(' ').starts_with('#') {
            continue;
        }
        let body = raw.trim_end_matches(' ');
        let after_spaces = body.trim_start_matches(' ');
        let spaces = body.len() - after_spaces.len();
        let (width, content) = if after_spaces.starts_with('\t') {
            if strict {
                return Err(DecodeError::new(number, "tab used for indentation"));
            }
            lenient_indentation(body, indent)
        } else {
            (spaces, after_spaces)
        };
        if content.is_empty() {
            pending_blank.get_or_insert(number);
            continue;
        }
        if strict && width % indent != 0 {
            return Err(DecodeError::new(
                number,
                format!(
                    "indentation of {width} spaces is not a multiple of the indent size {indent}"
                ),
            ));
        }
        lines.push(Line {
            number,
            depth: width / indent,
            content,
            blank_before: pending_blank.take(),
        });
    }
    Ok(lines)
}

/// Measures leading spaces and tabs (non-strict mode): a space is one column, a tab is one full
/// indent level. Returns the width in columns and the content that follows.
fn lenient_indentation(body: &str, indent: usize) -> (usize, &str) {
    let mut width = 0;
    let mut rest = body;
    while let Some(c) = rest.bytes().next() {
        match c {
            b' ' => width += 1,
            b'\t' => width += indent,
            _ => break,
        }
        rest = &rest[1..];
    }
    (width, rest)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Renders lines as `number:depth:content` for compact assertions.
    fn render(input: &str, indent: usize, strict: bool) -> Vec<String> {
        split_lines(input, indent, strict)
            .unwrap()
            .iter()
            .map(|l| format!("{}:{}:{}", l.number, l.depth, l.content))
            .collect()
    }

    /// Depth is the number of leading spaces divided by the indent size.
    #[test]
    fn measures_depth() {
        assert_eq!(
            render("a:\n  b: 1\n    c: 2", 2, true),
            ["1:0:a:", "2:1:b: 1", "3:2:c: 2"]
        );
        assert_eq!(render("a:\n    b: 1", 4, true), ["1:0:a:", "2:1:b: 1"]);
    }

    /// CRLF, a lone trailing CR, a BOM and trailing spaces are not content.
    #[test]
    fn line_terminators_bom_and_trailing_spaces() {
        assert_eq!(
            render("\u{feff}a: 1  \r\nb: 2\r\n", 2, true),
            ["1:0:a: 1", "2:0:b: 2"]
        );
        assert_eq!(render("a: 1\r", 2, true), ["1:0:a: 1"]);
        assert_eq!(render("x\u{feff}: 1", 2, true), ["1:0:x\u{feff}: 1"]);
        assert_eq!(render("  -  ", 2, true), ["1:1:-"]);
    }

    /// Comments are removed without leaving a trace, but only when only spaces precede the hash.
    #[test]
    fn comments_are_removed() {
        assert_eq!(
            render("# c\na: 1\n   # c\n  # c\nb: 2 # not a comment", 2, true),
            ["2:0:a: 1", "5:0:b: 2 # not a comment"]
        );
        assert!(split_lines("a: 1\n\t# c", 2, true).is_err());
    }

    /// Blank lines are dropped but remembered on the next content line.
    #[test]
    fn blank_lines_are_remembered() {
        let lines = split_lines("a: 1\n\n   \n# c\nb: 2\nc: 3\n\n", 2, true).unwrap();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].blank_before, None);
        assert_eq!(lines[1].blank_before, Some(2));
        assert_eq!(lines[2].blank_before, None);
    }

    /// Strict mode rejects tabs and misaligned indentation, but not on blank lines.
    #[test]
    fn strict_indentation_errors() {
        assert!(split_lines("a:\n   b: 1", 2, true).is_err());
        assert!(split_lines("a:\n\tb: 1", 2, true).is_err());
        assert!(split_lines("a:\n \tb: 1", 2, true).is_err());
        assert!(split_lines("\ta: 1", 2, true).is_err());
        assert!(split_lines("a: 1\n\t\nb: 2", 2, true).is_err());
        assert!(split_lines("a: 1\n   \nb: 2", 2, true).is_ok());
        let err = split_lines("a:\n  b:\n   c: 1", 2, true).unwrap_err();
        assert_eq!(err.line(), 3);
    }

    /// Non-strict mode floors the depth and accepts tabs as one level each.
    #[test]
    fn lenient_indentation_rules() {
        assert_eq!(render("a:\n   b: 1", 2, false), ["1:0:a:", "2:1:b: 1"]);
        assert_eq!(
            render("a:\n\tb: 1\n\t\tc: 2\n \tx", 2, false),
            ["1:0:a:", "2:1:b: 1", "3:2:c: 2", "4:1:x"]
        );
        assert_eq!(render("a: 1\n\t\nb: 2", 2, false), ["1:0:a: 1", "3:0:b: 2"]);
        assert_eq!(render("\t# x", 2, false), ["1:1:# x"]);
    }
}
