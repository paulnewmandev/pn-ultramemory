// SPDX-License-Identifier: Apache-2.0
//! Decoder-side tokenizing: delimiter splitting and primitive-token parsing.
//!
//! # Role in the architecture
//! The decoder ([`crate::decode`]) reads lines; this module reads what is *inside* a line:
//! it splits inline arrays, tabular rows and entry rows on the active delimiter (section 11.2),
//! decides whether a row-depth line is a row or a key-value line (section 9.3), and turns one
//! token into a JSON primitive (section 4).
//!
//! # Invariants
//! * A quote opens a quoted section only at the start of a token (after optional spaces); a quote
//!   in the middle of an unquoted token is ordinary text. Inside a quoted section the delimiter and
//!   the colon are data.
//! * Tokens are trimmed of U+0020 only, never of tabs or other whitespace (section 12).
//! * Empty tokens are preserved and decode to the empty string.
//! * A token that starts with `"` must be one complete quoted string.

use crate::number::parse_number;
use crate::quote::parse_quoted;
use serde_json::Value;

/// Returns the index just past the closing quote of the quoted section that opens at `start`, or
/// `None` if the string is unterminated. `bytes[start]` must be the opening `"`.
pub(crate) fn skip_quoted(bytes: &[u8], start: usize) -> Option<usize> {
    let mut i = start + 1;
    while let Some(&b) = bytes.get(i) {
        match b {
            b'"' => return Some(i + 1),
            b'\\' => i += 2,
            _ => i += 1,
        }
    }
    None
}

/// Decides whether a line at row depth of a tabular array is a row (`true`) or a key-value line
/// that ends the rows (`false`), by the rule of section 9.3.
///
/// A line with no unquoted colon is a row. Otherwise the first unquoted delimiter and the first
/// unquoted colon are compared: delimiter first means row, colon first means key-value.
///
/// # Errors
///
/// Returns a message when a quoted section that is reached before the decision is unterminated.
pub(crate) fn is_row_line(content: &str, delimiter: u8) -> Result<bool, String> {
    let bytes = content.as_bytes();
    let mut i = 0;
    let mut at_start = true;
    while i < bytes.len() {
        if at_start {
            while bytes.get(i) == Some(&b' ') {
                i += 1;
            }
            at_start = false;
            if bytes.get(i) == Some(&b'"') {
                i = skip_quoted(bytes, i).ok_or_else(|| "unterminated string".to_string())?;
            }
            continue;
        }
        let b = bytes[i];
        if b == delimiter {
            return Ok(true);
        }
        if b == b':' {
            return Ok(false);
        }
        i += 1;
    }
    Ok(true)
}

/// Splits `s` on the unquoted `delimiter` and trims each token of spaces.
///
/// An empty `s` yields zero tokens (not one empty token); a token between two delimiters, or at an
/// end, is the empty string.
///
/// # Errors
///
/// Returns a message when a token opens a quoted section that is never closed.
pub(crate) fn split_cells(s: &str, delimiter: u8) -> Result<Vec<&str>, String> {
    let mut cells = Vec::new();
    split_cells_into(s, delimiter, &mut cells)?;
    Ok(cells)
}

/// Like [`split_cells`], but reuses `cells` (which is cleared first) to avoid an allocation per
/// row.
///
/// # Errors
///
/// Returns a message when a token opens a quoted section that is never closed.
pub(crate) fn split_cells_into<'s>(
    s: &'s str,
    delimiter: u8,
    cells: &mut Vec<&'s str>,
) -> Result<(), String> {
    cells.clear();
    if s.is_empty() {
        return Ok(());
    }
    let bytes = s.as_bytes();
    let mut start = 0;
    let mut i = 0;
    let mut at_start = true;
    while i < bytes.len() {
        if at_start {
            while bytes.get(i) == Some(&b' ') {
                i += 1;
            }
            at_start = false;
            if bytes.get(i) == Some(&b'"') {
                i = skip_quoted(bytes, i).ok_or_else(|| "unterminated string".to_string())?;
            }
            continue;
        }
        if bytes[i] == delimiter {
            cells.push(s[start..i].trim_matches(' '));
            start = i + 1;
            at_start = true;
        }
        i += 1;
    }
    cells.push(s[start..].trim_matches(' '));
    Ok(())
}

/// Turns one trimmed token into a JSON primitive (section 4).
///
/// A token starting with `"` is a quoted string. Otherwise `true`, `false` and `null` are
/// literals, a token matching the number grammar is a number, and anything else (including the
/// empty token) is a string.
///
/// # Errors
///
/// Returns a message for an invalid quoted string or for characters after a closing quote.
pub(crate) fn parse_primitive(token: &str) -> Result<Value, String> {
    if token.starts_with('"') {
        let (text, used) = parse_quoted(token)?;
        if used != token.len() {
            return Err("unexpected characters after the closing quote".to_string());
        }
        return Ok(Value::String(text));
    }
    Ok(match token {
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        "null" => Value::Null,
        _ => parse_number(token).map_or_else(|| Value::String(token.to_string()), Value::Number),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Splitting preserves empty tokens, trims spaces only and honours quotes.
    #[test]
    fn splits_on_the_active_delimiter() {
        assert_eq!(split_cells("a,b,c", b',').unwrap(), ["a", "b", "c"]);
        assert_eq!(split_cells("a, b ,c ", b',').unwrap(), ["a", "b", "c"]);
        assert_eq!(split_cells("a,,c", b',').unwrap(), ["a", "", "c"]);
        assert_eq!(split_cells(",", b',').unwrap(), ["", ""]);
        assert_eq!(split_cells("\"a,b\",c", b',').unwrap(), ["\"a,b\"", "c"]);
        assert_eq!(split_cells("a|b,c|d", b'|').unwrap(), ["a", "b,c", "d"]);
        assert_eq!(split_cells("a \t b\tc", b'\t').unwrap(), ["a", "b", "c"]);
        assert_eq!(
            split_cells("\"a\\\"b,c\",d", b',').unwrap(),
            ["\"a\\\"b,c\"", "d"]
        );
    }

    /// An empty input is zero cells, and a lone space is one empty cell.
    #[test]
    fn empty_input_is_zero_cells() {
        assert!(split_cells("", b',').unwrap().is_empty());
        assert_eq!(split_cells(" ", b',').unwrap(), [""]);
    }

    /// A quote in the middle of a token does not open a quoted section.
    #[test]
    fn mid_token_quote_is_literal() {
        assert_eq!(split_cells("a\"b,c", b',').unwrap(), ["a\"b", "c"]);
    }

    /// An unterminated quoted token is an error.
    #[test]
    fn unterminated_quote_is_an_error() {
        assert!(split_cells("a,\"b", b',').is_err());
        assert!(is_row_line("\"abc", b',').is_err());
    }

    /// Row versus key-value classification follows section 9.3.
    #[test]
    fn row_disambiguation() {
        assert!(is_row_line("1,Ada", b',').unwrap());
        assert!(is_row_line("Ada", b',').unwrap());
        assert!(is_row_line("1,a:b", b',').unwrap());
        assert!(!is_row_line("x: 3,4", b',').unwrap());
        assert!(!is_row_line("count: 2", b',').unwrap());
        assert!(is_row_line("1,\"a:b\"", b',').unwrap());
        assert!(is_row_line("\"a:b\"", b',').unwrap());
        assert!(!is_row_line("k: 1|2", b'|').unwrap());
        assert!(is_row_line("1|k:v", b'|').unwrap());
    }

    /// Primitive tokens follow section 4.
    #[test]
    fn primitive_tokens() {
        assert_eq!(parse_primitive("true").unwrap(), Value::Bool(true));
        assert_eq!(parse_primitive("null").unwrap(), Value::Null);
        assert_eq!(parse_primitive("42").unwrap(), serde_json::json!(42));
        assert_eq!(
            parse_primitive("hello").unwrap(),
            serde_json::json!("hello")
        );
        assert_eq!(parse_primitive("").unwrap(), serde_json::json!(""));
        assert_eq!(
            parse_primitive("\"true\"").unwrap(),
            serde_json::json!("true")
        );
        assert_eq!(parse_primitive("05").unwrap(), serde_json::json!("05"));
        assert_eq!(parse_primitive("[]").unwrap(), serde_json::json!("[]"));
        assert_eq!(parse_primitive("True").unwrap(), serde_json::json!("True"));
        assert!(parse_primitive("\"a\" b").is_err());
        assert!(parse_primitive("\"a").is_err());
        assert!(parse_primitive("\"a\\q\"").is_err());
    }
}
