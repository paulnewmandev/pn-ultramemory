// SPDX-License-Identifier: Apache-2.0
//! Quoting, escaping and unescaping of TOON strings and keys.
//!
//! # Role in the architecture
//! Sections 7.1 to 7.3 of the TOON 4.1 specification decide when a string value or key must be
//! quoted and which escape sequences exist. The encoder uses [`needs_quotes`], [`push_string`] and
//! [`push_key`]; the decoder uses [`parse_quoted`]. Keeping both directions here means one table of
//! escapes and one definition of "safe unquoted".
//!
//! # Invariants
//! * Only five short escapes exist (`\\`, `\"`, `\n`, `\r`, `\t`) plus `\uXXXX`; the encoder writes
//!   `\uXXXX` (lowercase hex) only for the other control characters U+0000 to U+001F.
//! * The decoder rejects every other escape, a `\u` with fewer than four hex digits, an escape that
//!   names a surrogate, and an unterminated string.
//! * All delimiters and structural characters are ASCII, so byte scanning is valid on UTF-8 text.

use std::fmt::Write as _;

/// Appends `s` to `out` with the escapes of section 7.1, without surrounding quotes.
pub(crate) fn push_escaped(out: &mut String, s: &str) {
    let mut run_start = 0;
    for (i, c) in s.char_indices() {
        let short = match c {
            '\\' => "\\\\",
            '"' => "\\\"",
            '\n' => "\\n",
            '\r' => "\\r",
            '\t' => "\\t",
            c if u32::from(c) < 0x20 => "",
            _ => continue,
        };
        out.push_str(&s[run_start..i]);
        if short.is_empty() {
            let _ = write!(out, "\\u{:04x}", u32::from(c));
        } else {
            out.push_str(short);
        }
        run_start = i + c.len_utf8();
    }
    out.push_str(&s[run_start..]);
}

/// Appends `s` to `out` as a quoted, escaped string.
pub(crate) fn push_quoted(out: &mut String, s: &str) {
    out.push('"');
    push_escaped(out, s);
    out.push('"');
}

/// Returns `true` if `key` may be written without quotes (`^[A-Za-z_][A-Za-z0-9_.]*$`).
pub(crate) fn is_plain_key(key: &str) -> bool {
    let mut bytes = key.bytes();
    match bytes.next() {
        Some(b) if b.is_ascii_alphabetic() || b == b'_' => {}
        _ => return false,
    }
    bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.')
}

/// Appends an object key or field name to `out`, quoting it when section 7.3 requires.
pub(crate) fn push_key(out: &mut String, key: &str) {
    if is_plain_key(key) {
        out.push_str(key);
    } else {
        push_quoted(out, key);
    }
}

/// Returns `true` if `s` looks like a number (`^[+-]?[0-9]+(\.[0-9]+)?(e[+-]?[0-9]+)?$`, ASCII).
fn is_numeric_like(s: &str) -> bool {
    let bytes = s.as_bytes();
    let digits = |from: usize| {
        bytes[from..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count()
    };
    let mut i = usize::from(matches!(bytes.first(), Some(b'+' | b'-')));
    let n = digits(i);
    if n == 0 {
        return false;
    }
    i += n;
    if bytes.get(i) == Some(&b'.') {
        let n = digits(i + 1);
        if n == 0 {
            return false;
        }
        i += 1 + n;
    }
    if matches!(bytes.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(bytes.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        let n = digits(i);
        if n == 0 {
            return false;
        }
        i += n;
    }
    i == bytes.len()
}

/// Returns `true` if a string value must be quoted (section 7.2) when `delimiter` is the
/// delimiter that governs quoting at that position.
///
/// The rules are: empty; leading or trailing space or tab; `true`, `false` or `null`; numeric-like;
/// containing `:` `"` `\` `[` `]` `{` `}`, a control character or the delimiter; starting with `-`
/// (which includes the bare `-`) or `#`.
pub(crate) fn needs_quotes(s: &str, delimiter: char) -> bool {
    let bytes = s.as_bytes();
    let (Some(&first), Some(&last)) = (bytes.first(), bytes.last()) else {
        return true;
    };
    if matches!(first, b' ' | b'\t' | b'-' | b'#') || matches!(last, b' ' | b'\t') {
        return true;
    }
    if matches!(s, "true" | "false" | "null") {
        return true;
    }
    let delimiter = u8::try_from(delimiter).unwrap_or(b',');
    let special = |b: u8| {
        b < 0x20 || b == delimiter || matches!(b, b':' | b'"' | b'\\' | b'[' | b']' | b'{' | b'}')
    };
    bytes.iter().copied().any(special) || is_numeric_like(s)
}

/// Appends a string value to `out`, quoted only when required for `delimiter`.
pub(crate) fn push_string(out: &mut String, s: &str, delimiter: char) {
    if needs_quotes(s, delimiter) {
        push_quoted(out, s);
    } else {
        out.push_str(s);
    }
}

/// Parses the quoted token that starts at the beginning of `s` (which must begin with `"`).
///
/// Returns the unescaped text and the number of bytes consumed, closing quote included.
///
/// # Errors
///
/// Returns a message for an unterminated string, an unknown escape, a `\u` escape with fewer than
/// four hex digits, or a `\u` escape that names a surrogate code point.
pub(crate) fn parse_quoted(s: &str) -> Result<(String, usize), String> {
    let bytes = s.as_bytes();
    let mut out = String::new();
    let mut run_start = 1;
    let mut i = 1;
    loop {
        match bytes.get(i) {
            None => return Err("unterminated string".to_string()),
            Some(b'"') => {
                out.push_str(&s[run_start..i]);
                return Ok((out, i + 1));
            }
            Some(b'\\') => {
                out.push_str(&s[run_start..i]);
                i += 1;
                match bytes.get(i) {
                    Some(b'\\') => out.push('\\'),
                    Some(b'"') => out.push('"'),
                    Some(b'n') => out.push('\n'),
                    Some(b'r') => out.push('\r'),
                    Some(b't') => out.push('\t'),
                    Some(b'u') => {
                        out.push(parse_unicode_escape(s, i + 1)?);
                        i += 4;
                    }
                    Some(_) => {
                        let bad = s[i..].chars().next().unwrap_or('\\');
                        return Err(format!("invalid escape sequence \\{bad}"));
                    }
                    None => return Err("unterminated string".to_string()),
                }
                i += 1;
                run_start = i;
            }
            Some(_) => i += 1,
        }
    }
}

/// Reads the four hex digits at `s[from..from + 4]` of a `\u` escape.
fn parse_unicode_escape(s: &str, from: usize) -> Result<char, String> {
    let digits = s
        .get(from..from + 4)
        .filter(|d| d.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or_else(|| "invalid escape sequence \\u (four hex digits required)".to_string())?;
    let code = u32::from_str_radix(digits, 16).map_err(|_| "invalid \\u escape".to_string())?;
    char::from_u32(code)
        .ok_or_else(|| format!("invalid escape sequence \\u{digits}: surrogates are not allowed"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Quotes a string with the comma delimiter and returns the written form.
    fn value(s: &str) -> String {
        let mut out = String::new();
        push_string(&mut out, s, ',');
        out
    }

    /// Plain words, inner spaces and non-ASCII text stay unquoted.
    #[test]
    fn safe_strings_stay_unquoted() {
        for s in [
            "hello",
            "Ada_99",
            "hello world",
            "café",
            "你好",
            "🚀",
            "a#b",
            "a-b",
            "\u{a0}x\u{a0}",
        ] {
            assert_eq!(value(s), s, "{s:?}");
        }
    }

    /// Every trigger of section 7.2 forces quotes.
    #[test]
    fn triggers_force_quotes() {
        for s in [
            "", " ", " a", "a ", "\ta", "a\t", "true", "false", "null", "42", "-3.14", "05", "+1",
            "1e-6", "1E5", "a:b", "a\"b", "a\\b", "[x]", "a]", "{k}", "a}", "a,b", "-", "-x",
            "- item", "#", "#x", "a\nb", "a\rb", "a\u{1}b", "\u{0}",
        ] {
            assert!(value(s).starts_with('"'), "{s:?} must be quoted");
        }
    }

    /// Text that is not numeric-like stays unquoted even if it starts with a digit.
    #[test]
    fn near_numbers_are_not_numeric_like() {
        for s in [
            "1.", ".5", "1e", "0x10", "1_000", "Infinity", "NaN", "1.2.3", "12abc", "١٢٣",
        ] {
            assert!(!needs_quotes(s, ','), "{s:?} must not need quotes");
        }
    }

    /// The delimiter that governs a position decides whether it forces quotes.
    #[test]
    fn delimiter_awareness() {
        assert!(needs_quotes("a|b", '|'));
        assert!(!needs_quotes("a|b", ','));
        assert!(needs_quotes("a,b", ','));
        assert!(!needs_quotes("a,b", '|'));
        assert!(needs_quotes("a\tb", '\t'));
    }

    /// Escapes follow the table of section 7.1.
    #[test]
    fn escapes() {
        let mut out = String::new();
        push_quoted(&mut out, "a\\b\"c\nd\re\tf\u{4}g\u{1f}h\u{7f}");
        assert_eq!(out, "\"a\\\\b\\\"c\\nd\\re\\tf\\u0004g\\u001fh\u{7f}\"");
    }

    /// Only keys matching `^[A-Za-z_][A-Za-z0-9_.]*$` are written bare.
    #[test]
    fn key_quoting() {
        let mut out = String::new();
        for key in [
            "id", "_x", "a.b", "A1", "", "1a", "a-b", "a b", "café", "a:b", "-x",
        ] {
            push_key(&mut out, key);
            out.push('|');
        }
        assert_eq!(
            out,
            "id|_x|a.b|A1|\"\"|\"1a\"|\"a-b\"|\"a b\"|\"café\"|\"a:b\"|\"-x\"|"
        );
    }

    /// Valid quoted tokens unescape, including uppercase hex and mixed escapes.
    #[test]
    fn unquotes_valid_tokens() {
        assert_eq!(parse_quoted("\"a\\nb\"").unwrap(), ("a\nb".to_string(), 6));
        assert_eq!(parse_quoted("\"\"").unwrap(), (String::new(), 2));
        assert_eq!(
            parse_quoted("\"x\\u00E9y\" tail").unwrap(),
            ("xéy".to_string(), 10)
        );
        assert_eq!(parse_quoted("\"\\u00Ab\"").unwrap().0, "«");
        assert_eq!(
            parse_quoted("\"\\\\ \\\" \\t \\r\"").unwrap().0,
            "\\ \" \t \r"
        );
        assert_eq!(parse_quoted("\"🚀 é\"").unwrap().0, "🚀 é");
    }

    /// Invalid escapes, surrogates and unterminated strings are rejected.
    #[test]
    fn rejects_bad_tokens() {
        for s in [
            "\"abc",
            "\"abc\\",
            "\"a\\x\"",
            "\"a\\u00b\"",
            "\"a\\u00\"",
            "\"a\\uD800b\"",
            "\"a\\uDFFF\"",
            "\"a\\u12G4\"",
            "\"a\\é\"",
            "\"",
        ] {
            assert!(parse_quoted(s).is_err(), "{s:?} must be rejected");
        }
    }

    /// Whatever the encoder quotes, the decoder reads back unchanged.
    #[test]
    fn escape_round_trip() {
        for s in [
            "",
            "plain",
            "q\"uote",
            "back\\slash",
            "nl\n",
            "\u{0}\u{1f}",
            "🚀\t\r\n\"\\",
        ] {
            let mut out = String::new();
            push_quoted(&mut out, s);
            let (back, used) = parse_quoted(&out).unwrap();
            assert_eq!(back, s);
            assert_eq!(used, out.len());
        }
    }
}
