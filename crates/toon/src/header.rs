// SPDX-License-Identifier: Apache-2.0
//! Line classification and header parsing for the TOON decoder.
//!
//! # Role in the architecture
//! Section 5.2 of the TOON 4.1 specification sorts every line into a class, and section 6 defines
//! the header grammar (`key[N]{fields}:`, keyed headers `key[N:]{fields}:`, nested field groups).
//! [`classify`] implements both: it tells the parser whether a line is an array or keyed header, a
//! key-value line, or a scalar line, and hands over the parsed [`Header`].
//!
//! # Invariants
//! * A line whose first unquoted colon comes before its first unquoted `[` is never a header.
//! * A malformed header is an error in strict mode. In non-strict mode the line falls through to a
//!   key-value line whose key is the literal text before the first colon outside brackets and
//!   braces (section 6), except for errors that apply in every mode (invalid escapes, characters
//!   after a closing quote, over-deep field nesting).
//! * Parsing is linear in the length of the line, and field-list nesting is capped at
//!   [`MAX_DEPTH`].

use crate::error::DecodeError;
use crate::options::Delimiter;
use crate::quote::parse_quoted;
use std::collections::HashSet;

/// Maximum nesting depth of containers (and of field groups) the decoder accepts.
pub(crate) const MAX_DEPTH: usize = 256;

/// One entry of a header's field list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Field {
    /// A plain column: one cell per row.
    Leaf(String),
    /// A nested field group: an object column whose sub-fields are expanded in place.
    Group(String, Vec<Field>),
}

/// A parsed array or keyed-object header.
#[derive(Debug, Clone)]
pub(crate) struct Header<'a> {
    /// The key before the bracket segment, or `None` for a keyless (root or list-item) header.
    pub(crate) key: Option<String>,
    /// The declared length (array length, or entry count for a keyed header).
    pub(crate) len: usize,
    /// `true` for a keyed header (`[N:]`).
    pub(crate) keyed: bool,
    /// The active delimiter declared by the bracket segment.
    pub(crate) delim: Delimiter,
    /// The field list, when the header carries one.
    pub(crate) fields: Option<Vec<Field>>,
    /// Number of leaf fields (cells per row), zero without a field list.
    pub(crate) leaf_count: usize,
    /// Deepest nesting of field groups, zero for a flat field list.
    pub(crate) group_depth: usize,
    /// Text after the header's colon, trimmed of spaces.
    pub(crate) rest: &'a str,
    /// The whole line content the header was parsed from.
    pub(crate) raw: &'a str,
    /// 1-based line number.
    pub(crate) line: usize,
}

/// The class of a line (section 5.2), as far as the header logic can tell.
#[derive(Debug, Clone)]
pub(crate) enum Kind<'a> {
    /// An array header or keyed-object header (boxed to keep the recursive parser's frames small).
    Header(Box<Header<'a>>),
    /// A `key: value` line; `rest` is the text after the colon, untrimmed.
    KeyValue {
        /// The decoded key.
        key: String,
        /// Text after the colon.
        rest: &'a str,
    },
    /// A line without an unquoted colon.
    Scalar,
}

/// A header that failed to parse.
struct Bad {
    /// What is wrong.
    message: String,
    /// `true` when the error applies in every mode and no key-value fall-through is allowed.
    fatal: bool,
}

impl Bad {
    /// A malformed header, which non-strict decoding may read as a key-value line.
    fn soft(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            fatal: false,
        }
    }

    /// An error that applies in strict and non-strict mode alike.
    fn fatal(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            fatal: true,
        }
    }
}

/// Classifies `content` (a line without its indentation, or the text after a list-item marker).
///
/// # Errors
///
/// Returns an error for a bad quoted key, for characters after a closing quote, for an empty key
/// in strict mode, and for a malformed header in strict mode.
pub(crate) fn classify(content: &str, line: usize, strict: bool) -> Result<Kind<'_>, DecodeError> {
    if content.starts_with('"') {
        let (key, end) = parse_quoted(content).map_err(|m| DecodeError::new(line, m))?;
        let after = &content[end..];
        if after.starts_with('[') {
            return header_or_key_value(content, Some(key), end, line, strict);
        }
        let trimmed = after.trim_start_matches(' ');
        if let Some(rest) = trimmed.strip_prefix(':') {
            return Ok(Kind::KeyValue { key, rest });
        }
        if trimmed.is_empty() {
            return Ok(Kind::Scalar);
        }
        return Err(DecodeError::new(
            line,
            "unexpected characters after the closing quote",
        ));
    }
    let Some(colon) = content.find(':') else {
        return Ok(Kind::Scalar);
    };
    match content.find('[') {
        Some(bracket) if bracket < colon => {
            let key_text = &content[..bracket];
            if key_text.ends_with([' ', '\t']) {
                let message = "whitespace between a key and its bracket segment";
                return if strict {
                    Err(DecodeError::new(line, message))
                } else {
                    key_value_fallthrough(content, line)
                };
            }
            let key = (!key_text.is_empty()).then(|| key_text.to_string());
            header_or_key_value(content, key, bracket, line, strict)
        }
        _ => {
            let key = content[..colon].trim_end_matches(' ');
            if key.is_empty() && strict {
                return Err(DecodeError::new(line, "missing key before ':'"));
            }
            Ok(Kind::KeyValue {
                key: key.to_string(),
                rest: &content[colon + 1..],
            })
        }
    }
}

/// Parses the header at `bracket`, or (non-strict only) falls through to a key-value line.
fn header_or_key_value(
    content: &str,
    key: Option<String>,
    bracket: usize,
    line: usize,
    strict: bool,
) -> Result<Kind<'_>, DecodeError> {
    match scan_header(content, key, bracket, line, strict) {
        Ok(header) => Ok(Kind::Header(Box::new(header))),
        Err(bad) if bad.fatal || strict => Err(DecodeError::new(line, bad.message)),
        Err(_) => key_value_fallthrough(content, line),
    }
}

/// Reads `content` as a key-value line whose key is the literal text before the first colon that
/// is outside brackets and braces. Used for non-strict decoding of malformed or misplaced headers.
///
/// # Errors
///
/// Returns an error when the content starts with a quoted token that is followed by more
/// characters, or when no such colon exists.
pub(crate) fn key_value_fallthrough(content: &str, line: usize) -> Result<Kind<'_>, DecodeError> {
    if content.starts_with('"') {
        return Err(DecodeError::new(
            line,
            "unexpected characters after the closing quote",
        ));
    }
    let mut depth = 0_usize;
    for (i, b) in content.bytes().enumerate() {
        match b {
            b'[' | b'{' => depth += 1,
            b']' | b'}' => depth = depth.saturating_sub(1),
            b':' if depth == 0 => {
                return Ok(Kind::KeyValue {
                    key: content[..i].trim_end_matches(' ').to_string(),
                    rest: &content[i + 1..],
                });
            }
            _ => {}
        }
    }
    Err(DecodeError::new(line, "missing colon after key"))
}

/// Parses the bracket segment, optional field list and colon of a header. `bracket` is the byte
/// index of the opening `[`.
fn scan_header(
    content: &str,
    key: Option<String>,
    bracket: usize,
    line: usize,
    strict: bool,
) -> Result<Header<'_>, Bad> {
    let bytes = content.as_bytes();
    let mut i = bracket + 1;
    let digits_start = i;
    while bytes.get(i).is_some_and(u8::is_ascii_digit) {
        i += 1;
    }
    let digits = &content[digits_start..i];
    if digits.is_empty() {
        return Err(Bad::soft("bracket segment without a length"));
    }
    if digits.len() > 1 && digits.starts_with('0') {
        return Err(Bad::soft("array length with a leading zero"));
    }
    let len = digits.parse::<usize>().unwrap_or(usize::MAX);
    let keyed = bytes.get(i) == Some(&b':');
    i += usize::from(keyed);
    let delim = match bytes.get(i) {
        Some(b'\t') => Delimiter::Tab,
        Some(b'|') => Delimiter::Pipe,
        _ => Delimiter::Comma,
    };
    i += delim.header_symbol().len();
    if bytes.get(i) != Some(&b']') {
        return Err(Bad::soft("malformed bracket segment"));
    }
    i += 1;
    let mut fields = None;
    if bytes.get(i) == Some(&b'{') {
        let (parsed, next) = parse_fields(content, i + 1, delim, 0, strict)?;
        fields = Some(parsed);
        i = next;
    }
    if bytes.get(i) != Some(&b':') {
        return Err(Bad::soft("expected ':' directly after the header"));
    }
    let rest = content[i + 1..].trim_matches(' ');
    if keyed && fields.is_none() {
        return Err(Bad::soft("a keyed header requires a field list"));
    }
    if fields.is_some() && !rest.is_empty() {
        return Err(Bad::soft(
            "content after the colon of a header with a field list",
        ));
    }
    let (leaf_count, group_depth) = fields.as_deref().map_or((0, 0), measure);
    Ok(Header {
        key,
        len,
        keyed,
        delim,
        fields,
        leaf_count,
        group_depth,
        rest,
        raw: content,
        line,
    })
}

/// Returns the number of leaf fields and the deepest group nesting of a field list.
fn measure(fields: &[Field]) -> (usize, usize) {
    let mut leaves = 0;
    let mut depth = 0;
    for field in fields {
        match field {
            Field::Leaf(_) => leaves += 1,
            Field::Group(_, sub) => {
                let (l, d) = measure(sub);
                leaves += l;
                depth = depth.max(d + 1);
            }
        }
    }
    (leaves, depth)
}

/// Parses a field list whose opening `{` is just before `start`, returning the fields and the byte
/// index just past the matching `}`.
fn parse_fields(
    content: &str,
    start: usize,
    delim: Delimiter,
    level: usize,
    strict: bool,
) -> Result<(Vec<Field>, usize), Bad> {
    if level >= MAX_DEPTH {
        return Err(Bad::fatal("field groups are nested too deeply"));
    }
    let bytes = content.as_bytes();
    let d = delim.as_byte();
    let mut i = start;
    let mut fields = Vec::new();
    loop {
        while bytes.get(i) == Some(&b' ') {
            i += 1;
        }
        if fields.is_empty() && bytes.get(i) == Some(&b'}') {
            return Err(Bad::soft("empty field list"));
        }
        let name = if bytes.get(i) == Some(&b'"') {
            let (name, used) = parse_quoted(&content[i..]).map_err(Bad::fatal)?;
            i += used;
            name
        } else {
            let name_start = i;
            while bytes
                .get(i)
                .is_some_and(|&b| b != d && b != b'{' && b != b'}')
            {
                i += 1;
            }
            let name = content[name_start..i].trim_matches(' ');
            if name.is_empty() {
                return Err(Bad::soft("empty field name"));
            }
            if name
                .bytes()
                .any(|b| matches!(b, b',' | b'\t' | b'|') && b != d)
            {
                return Err(Bad::soft(
                    "field list uses a delimiter other than the declared one",
                ));
            }
            name.to_string()
        };
        while bytes.get(i) == Some(&b' ') {
            i += 1;
        }
        if bytes.get(i) == Some(&b'{') {
            let (sub, next) = parse_fields(content, i + 1, delim, level + 1, strict)?;
            i = next;
            while bytes.get(i) == Some(&b' ') {
                i += 1;
            }
            fields.push(Field::Group(name, sub));
        } else {
            fields.push(Field::Leaf(name));
        }
        match bytes.get(i) {
            Some(&b) if b == d => i += 1,
            Some(b'}') => {
                if strict {
                    reject_duplicates(&fields)?;
                }
                return Ok((fields, i + 1));
            }
            None => return Err(Bad::soft("unmatched '{' in field list")),
            Some(_) => return Err(Bad::soft("unexpected character in field list")),
        }
    }
}

/// Fails if two fields of the same list share a name (section 14.2).
fn reject_duplicates(fields: &[Field]) -> Result<(), Bad> {
    let mut seen = HashSet::with_capacity(fields.len());
    for field in fields {
        let (Field::Leaf(name) | Field::Group(name, _)) = field;
        if !seen.insert(name.as_str()) {
            return Err(Bad::soft(format!(
                "duplicate field name {name:?} in field list"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parses a header line in strict mode and returns it, panicking on anything else.
    fn header(content: &str) -> Header<'_> {
        match classify(content, 1, true).unwrap() {
            Kind::Header(h) => *h,
            other => panic!("expected a header, got {other:?}"),
        }
    }

    /// Returns the leaf and group names of a field list as compact text.
    fn shape(fields: &[Field]) -> String {
        fields
            .iter()
            .map(|f| match f {
                Field::Leaf(n) => n.clone(),
                Field::Group(n, sub) => format!("{n}({})", shape(sub)),
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// A plain inline array header carries its inline values in `rest`.
    #[test]
    fn inline_header() {
        let h = header("tags[3]: a,b,c");
        assert_eq!(h.key.as_deref(), Some("tags"));
        assert_eq!((h.len, h.keyed, h.delim), (3, false, Delimiter::Comma));
        assert!(h.fields.is_none());
        assert_eq!(h.rest, "a,b,c");
    }

    /// Tab and pipe delimiters are read from the bracket segment.
    #[test]
    fn delimiter_symbols() {
        assert_eq!(header("a[2|]: x|y").delim, Delimiter::Pipe);
        assert_eq!(header("a[2\t]: x\ty").delim, Delimiter::Tab);
        assert_eq!(header("a[2|]{x|y}:").delim, Delimiter::Pipe);
    }

    /// A tabular header parses fields, nested groups and leaf counts.
    #[test]
    fn tabular_header_with_groups() {
        let h = header("orders[2]{id,customer{name,country},total}:");
        assert_eq!(
            shape(h.fields.as_deref().unwrap()),
            "id customer(name country) total"
        );
        assert_eq!((h.leaf_count, h.group_depth), (4, 1));
        let h = header("items[2]{id,geo{point{lat,lon}}}:");
        assert_eq!((h.leaf_count, h.group_depth), (3, 2));
    }

    /// Quoted names may contain braces, delimiters and colons.
    #[test]
    fn quoted_field_names() {
        let h = header("items[1]{\"a{b}\",c,\"x:y\",\"d,e\"}:");
        assert_eq!(shape(h.fields.as_deref().unwrap()), "a{b} c x:y d,e");
    }

    /// Keyed headers, keyless headers and quoted keys.
    #[test]
    fn keyed_and_keyless() {
        let h = header("m[2:]{v}:");
        assert!(h.keyed);
        assert_eq!(h.len, 2);
        let h = header("[2:|]{v|w}:");
        assert!(h.keyed && h.key.is_none());
        assert_eq!(h.delim, Delimiter::Pipe);
        let h = header("\"a:b\"[2]: 1,2");
        assert_eq!(h.key.as_deref(), Some("a:b"));
        let h = header("[3]: x,y,z");
        assert!(h.key.is_none());
        let h = header("data.meta.items[2]{id,name}:");
        assert_eq!(h.key.as_deref(), Some("data.meta.items"));
    }

    /// A colon before the first bracket makes a key-value line, not a header.
    #[test]
    fn colon_before_bracket_is_key_value() {
        match classify("a:b[2]: x", 1, true).unwrap() {
            Kind::KeyValue { key, rest } => assert_eq!((key.as_str(), rest), ("a", "b[2]: x")),
            other => panic!("unexpected {other:?}"),
        }
        assert!(matches!(
            classify("key: [2]: x", 1, true).unwrap(),
            Kind::KeyValue { .. }
        ));
        assert!(matches!(
            classify("just text", 1, true).unwrap(),
            Kind::Scalar
        ));
        assert!(matches!(
            classify("\"a:b\"", 1, true).unwrap(),
            Kind::Scalar
        ));
    }

    /// Every malformed header of section 6 is a strict-mode error.
    #[test]
    fn malformed_headers_are_strict_errors() {
        for content in [
            "a[]: 1",
            "a[03]: 1",
            "a[-1]: 1",
            "a[+3]: 1",
            "a[3.7]: 1",
            "a[1e1]: 1",
            "a[x]: 1",
            "a[2] : 1",
            "a [2]: 1",
            "a[2]x: 1",
            "a[2] {x}: 1",
            "a[2|:]{x}:",
            "a[2 :]{x}:",
            "a[2:,]{x}:",
            "a[03:]{x}:",
            "a[2:]:",
            "a[2]{}:",
            "a[2]{x,y{}}:",
            "a[2]{x,x}:",
            "a[2]{x,y{z,z}}:",
            "a[2]{x,y:",
            "a[2]{x,y}}:",
            "a[2]{x,,y}:",
            "a[2]{x|y}:",
            "a[2|]{x,y}:",
            "a[2]{x}: 1",
            "a[2:]{x}: 1",
        ] {
            assert!(
                classify(content, 1, true).is_err(),
                "{content:?} must be rejected"
            );
        }
    }

    /// In non-strict mode a malformed header is read as a key-value line with a literal key.
    #[test]
    fn malformed_headers_fall_through_when_lenient() {
        for (content, key) in [
            ("foo[1][bar]: 10", "foo[1][bar]"),
            ("key[]: 1,2", "key[]"),
            ("foo[2]extra: a,b", "foo[2]extra"),
            ("foo [2]: bar,baz", "foo [2]"),
            ("a[2|]{x,y}: 1|2", "a[2|]{x,y}"),
            ("[2]{x}: 1", "[2]{x}"),
        ] {
            match classify(content, 1, false).unwrap() {
                Kind::KeyValue { key: k, .. } => assert_eq!(k, key),
                other => panic!("{content:?}: unexpected {other:?}"),
            }
        }
    }

    /// Duplicate field names are tolerated in non-strict mode (last write wins later).
    #[test]
    fn duplicate_fields_are_lenient_when_not_strict() {
        assert!(matches!(
            classify("a[1]{x,x}:", 1, false).unwrap(),
            Kind::Header(_)
        ));
    }

    /// Errors that apply in every mode are not softened by the lenient fall-through.
    #[test]
    fn fatal_errors_apply_in_every_mode() {
        assert!(classify("a[1]{\"x}:", 1, false).is_err());
        assert!(classify("\"a\" b: 1", 1, false).is_err());
        assert!(classify("\"a\"[bad]: 1", 1, false).is_err());
        assert!(classify("\"a\\q\": 1", 1, false).is_err());
    }

    /// Field groups nested beyond the limit are rejected instead of recursing without bound.
    #[test]
    fn deep_field_groups_are_rejected() {
        let depth = MAX_DEPTH + 10;
        let text = format!("a[1]{{{}x{}}}:", "g{".repeat(depth), "}".repeat(depth));
        assert!(classify(&text, 1, true).is_err());
        assert!(classify(&text, 1, false).is_err());
        let ok = format!("a[1]{{{}x{}}}:", "g{".repeat(50), "}".repeat(50));
        assert!(classify(&ok, 1, true).is_ok());
    }

    /// A huge declared length does not overflow: it saturates.
    #[test]
    fn huge_lengths_saturate() {
        let h = header("a[99999999999999999999999999]: 1");
        assert_eq!(h.len, usize::MAX);
    }
}
