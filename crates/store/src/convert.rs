// SPDX-License-Identifier: Apache-2.0
//! Conversions between the domain types and the values SQLite stores.
//!
//! SQLite has one integer type (signed 64 bits), no unsigned integers and no enumerations, so the
//! adapter stores hashes bit for bit, enumerations by their stable lowercase names and lists of
//! names in a small escaped text form. This module holds those encodings so that writing and
//! reading a value can never drift apart. It also holds the few helpers that turn a caller's
//! filter into SQL parameters, so that user input is always bound and never concatenated.
//!
//! Invariants: every encoder has a decoder that returns the original value, and a decoder
//! rejects unknown names instead of guessing.

use pn_ultramemory_core::{
    Confidence, EdgeKind, Language, MemoryKind, Provenance, SymbolKind, Visibility,
};

use crate::error::bad_value;

/// Clamps a caller's `usize` limit into the `i64` SQLite expects.
pub(crate) fn limit_to_sql(limit: usize) -> i64 {
    i64::try_from(limit).unwrap_or(i64::MAX)
}

/// Stores a `u64` hash bit for bit in a signed column.
pub(crate) fn u64_to_sql(value: u64) -> i64 {
    i64::from_ne_bytes(value.to_ne_bytes())
}

/// Reads back a value written by [`u64_to_sql`].
pub(crate) fn u64_from_sql(value: i64) -> u64 {
    u64::from_ne_bytes(value.to_ne_bytes())
}

/// Reads a floating-point column that SQLite may have stored as INTEGER (e.g. a DEFAULT 0.0
/// written by ALTER TABLE ADD COLUMN). Rusqlite 0.40 refuses to coerce INTEGER → f64 on its
/// own, so we inspect the raw value ref and convert manually.
pub(crate) fn f64_from_sql(row: &rusqlite::Row<'_>, idx: usize) -> rusqlite::Result<f64> {
    match row.get_ref(idx)? {
        rusqlite::types::ValueRef::Real(v) => Ok(v),
        #[allow(clippy::cast_precision_loss)]
        rusqlite::types::ValueRef::Integer(v) => Ok(v as f64),
        rusqlite::types::ValueRef::Null => Ok(0.0),
        other => Err(rusqlite::Error::InvalidColumnType(
            idx,
            String::from("f64_column"),
            other.data_type(),
        )),
    }
}

/// Stores a byte count, saturating at the largest representable value.
pub(crate) fn size_to_sql(size: u64) -> i64 {
    i64::try_from(size).unwrap_or(i64::MAX)
}

/// The integer stored for a confidence, ordered like [`Confidence`] itself.
pub(crate) const fn confidence_to_sql(confidence: Confidence) -> i64 {
    match confidence {
        Confidence::Guess => 0,
        Confidence::Heuristic => 1,
        Confidence::Resolved => 2,
        Confidence::Exact => 3,
    }
}

/// Reads back a confidence stored by [`confidence_to_sql`].
pub(crate) fn confidence_from_sql(value: i64, column: usize) -> rusqlite::Result<Confidence> {
    match value {
        0 => Ok(Confidence::Guess),
        1 => Ok(Confidence::Heuristic),
        2 => Ok(Confidence::Resolved),
        3 => Ok(Confidence::Exact),
        other => Err(bad_value(column, format!("unknown confidence {other}"))),
    }
}

/// Reads a stored symbol kind name.
pub(crate) fn symbol_kind(name: &str, column: usize) -> rusqlite::Result<SymbolKind> {
    SymbolKind::from_name(name)
        .ok_or_else(|| bad_value(column, format!("unknown symbol kind `{name}`")))
}

/// Reads a stored visibility name.
pub(crate) fn visibility(name: &str, column: usize) -> rusqlite::Result<Visibility> {
    Visibility::from_name(name)
        .ok_or_else(|| bad_value(column, format!("unknown visibility `{name}`")))
}

/// Reads a stored language name.
pub(crate) fn language(name: &str, column: usize) -> rusqlite::Result<Language> {
    Language::from_name(name).ok_or_else(|| bad_value(column, format!("unknown language `{name}`")))
}

/// Reads a stored edge kind name.
pub(crate) fn edge_kind(name: &str, column: usize) -> rusqlite::Result<EdgeKind> {
    EdgeKind::from_name(name)
        .ok_or_else(|| bad_value(column, format!("unknown edge kind `{name}`")))
}

/// Reads a stored memory kind name.
pub(crate) fn memory_kind(name: &str, column: usize) -> rusqlite::Result<MemoryKind> {
    MemoryKind::from_name(name)
        .ok_or_else(|| bad_value(column, format!("unknown memory kind `{name}`")))
}

/// Reads a stored provenance name.
pub(crate) fn provenance(name: &str, column: usize) -> rusqlite::Result<Provenance> {
    Provenance::from_name(name)
        .ok_or_else(|| bad_value(column, format!("unknown provenance `{name}`")))
}

/// Encodes a list of names as text: every name is followed by a newline, and backslash and
/// newline inside a name are escaped, so any list round-trips exactly.
///
/// The empty list is the empty string, and a list holding one empty name is a single newline.
pub(crate) fn encode_names(names: &[String]) -> String {
    let mut out = String::with_capacity(names.iter().map(|n| n.len() + 1).sum());
    for name in names {
        for ch in name.chars() {
            match ch {
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                other => out.push(other),
            }
        }
        out.push('\n');
    }
    out
}

/// Decodes text written by [`encode_names`].
pub(crate) fn decode_names(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut current = String::new();
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '\n' => names.push(std::mem::take(&mut current)),
            '\\' => match chars.next() {
                Some('n') => current.push('\n'),
                Some(other) => current.push(other),
                None => current.push('\\'),
            },
            other => current.push(other),
        }
    }
    names
}

/// The smallest string that sorts after every string starting with `prefix`, or `None` when no
/// such string exists (an empty prefix, or one made only of the largest code point).
///
/// Together with the prefix itself it gives the index-friendly range
/// `path >= prefix AND path < bound`, which selects exactly the paths that start with `prefix`
/// without any pattern syntax to escape.
pub(crate) fn prefix_upper_bound(prefix: &str) -> Option<String> {
    let mut chars: Vec<char> = prefix.chars().collect();
    while let Some(last) = chars.pop() {
        let next = match u32::from(last) + 1 {
            0xD800 => 0xE000,
            other => other,
        };
        if let Some(successor) = char::from_u32(next) {
            chars.push(successor);
            return Some(chars.into_iter().collect());
        }
    }
    None
}

/// Cuts `text` to at most `max_bytes` bytes without splitting a character.
pub(crate) fn truncate_utf8(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }
    let mut end = max_bytes;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

#[cfg(test)]
mod tests {
    use super::{
        confidence_from_sql, confidence_to_sql, decode_names, encode_names, limit_to_sql,
        prefix_upper_bound, truncate_utf8, u64_from_sql, u64_to_sql,
    };
    use pn_ultramemory_core::Confidence;

    /// Hashes survive the trip through a signed column, including the extremes.
    #[test]
    fn u64_round_trips_bit_for_bit() {
        for value in [
            0,
            1,
            u64::MAX,
            u64::MAX / 2,
            u64::MAX / 2 + 1,
            0xdead_beef_cafe,
        ] {
            assert_eq!(u64_from_sql(u64_to_sql(value)), value);
        }
    }

    /// Every confidence round-trips and unknown integers are rejected.
    #[test]
    fn confidence_round_trips() {
        for confidence in Confidence::ALL {
            let stored = confidence_to_sql(confidence);
            assert_eq!(confidence_from_sql(stored, 0).ok(), Some(confidence));
        }
        assert!(confidence_from_sql(9, 0).is_err());
        assert!(confidence_from_sql(-1, 0).is_err());
    }

    /// Lists of names round-trip, including empty names and awkward characters.
    #[test]
    fn names_round_trip() {
        let cases: Vec<Vec<String>> = vec![
            vec![],
            vec![String::new()],
            vec![String::new(), String::new()],
            vec!["a".into(), "b c".into()],
            vec!["line\nbreak".into(), "back\\slash".into(), "\\n".into()],
            vec!["trailing\\".into(), "\n".into(), "\\\n\\".into()],
            vec!["ünï".into(), "日本".into()],
        ];
        for names in cases {
            assert_eq!(decode_names(&encode_names(&names)), names);
        }
    }

    /// The upper bound sorts right after every string with the prefix.
    #[test]
    fn prefix_bounds() {
        assert_eq!(prefix_upper_bound(""), None);
        assert_eq!(prefix_upper_bound("src/"), Some("src0".to_owned()));
        assert_eq!(prefix_upper_bound("a\u{10FFFF}"), Some("b".to_owned()));
        assert_eq!(prefix_upper_bound("\u{10FFFF}"), None);
        assert_eq!(prefix_upper_bound("\u{D7FF}"), Some("\u{E000}".to_owned()));
        let bound = prefix_upper_bound("src/").unwrap_or_default();
        assert!("src/zzz" < bound.as_str());
        assert!("src0" >= bound.as_str());
    }

    /// Truncation never splits a character and limits clamp instead of overflowing.
    #[test]
    fn truncation_and_limits() {
        assert_eq!(truncate_utf8("héllo", 2), "h");
        assert_eq!(truncate_utf8("héllo", 3), "hé");
        assert_eq!(truncate_utf8("abc", 10), "abc");
        assert_eq!(limit_to_sql(usize::MAX), i64::MAX);
        assert_eq!(limit_to_sql(7), 7);
    }
}
