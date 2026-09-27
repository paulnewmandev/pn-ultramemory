// SPDX-License-Identifier: Apache-2.0
//! Reading a batch of documentation an agent wrote, in TOON or in JSON.
//!
//! # Role in the architecture
//! Application layer, the entry side of the documentation loop. An agent answers
//! `pn-ultramemory docs gaps` with one row per symbol, and this module turns that answer into
//! [`DocEntry`] values for [`crate::Engine::doc_apply`]. Three shapes are accepted and told apart by
//! their first non-blank character, so a caller never has to declare a format:
//!
//! | Shape | Looks like |
//! |---|---|
//! | TOON | `docs[2]{symbol,text}:` followed by one row per entry |
//! | JSON array | `[{"symbol": "...", "text": "..."}]` |
//! | JSON object | `{"docs": [{"symbol": "...", "text": "..."}]}` |
//!
//! # Invariants
//! * **Nothing is guessed.** A row without both fields, a field of the wrong type, a text over the
//!   length limit and a NUL byte are all refused, and the message names the entry number and what
//!   to change.
//! * **The batch is bounded**: at most [`MAX_ENTRIES`] entries, each text at most
//!   [`MAX_TEXT_CHARS`] characters, so a malformed or hostile input cannot turn into an unbounded
//!   amount of work.
//! * Order is preserved: entry `n` of the output is row `n` of the input.
//!
//! # Quoting in TOON
//! TOON is decoded in strict mode, so a cell that holds a colon or the delimiter must be quoted:
//! `"Config::validate","Checks every field."`. The tool's own output already quotes such cells, so an
//! agent that copies the table it was given needs no special care; one that writes the rows by hand
//! does. JSON has no such rule, which is why both shapes are accepted.

use pn_ultramemory_toon::{DecodeOptions, decode};
use serde_json::Value;

use crate::error::EngineError;

/// The most entries one batch may hold.
const MAX_ENTRIES: usize = 500;

/// The most characters one entry's text may hold.
const MAX_TEXT_CHARS: usize = 2_000;

/// One piece of documentation written for one symbol.
///
/// # Examples
/// ```
/// use pn_ultramemory_engine::DocEntry;
///
/// let entry = DocEntry { symbol: "load_config".into(), text: "Loads the configuration.".into() };
/// assert_eq!(entry.symbol, "load_config");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocEntry {
    /// The symbol the text documents, named the way `resolve_symbol` accepts: an id, a name, a
    /// qualified name or `path:name`.
    pub symbol: String,
    /// The documentation text, in the words a reader of the code should see. Comment markers are
    /// added by the inserter, so the text carries none.
    pub text: String,
}

/// The rows of the input, whatever shape it arrived in.
///
/// # Errors
/// Returns [`EngineError::Invalid`] when the input is neither TOON nor JSON, and when a JSON object
/// carries no `docs` array.
fn rows_of(input: &str) -> Result<Vec<Value>, EngineError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(EngineError::Invalid(
            "the documentation input is empty; write one row per symbol, as printed by \
             `pn-ultramemory docs gaps`"
                .to_owned(),
        ));
    }
    let looks_like_json = trimmed.starts_with('[') || trimmed.starts_with('{');
    let value = if looks_like_json {
        serde_json::from_str::<Value>(trimmed).map_err(|error| {
            EngineError::Invalid(format!(
                "the documentation input is not valid JSON ({error}); list the gaps again with \
                 `pn-ultramemory docs gaps` and answer with one row per symbol"
            ))
        })?
    } else {
        decode(trimmed, &DecodeOptions::default()).map_err(|error| {
            EngineError::Invalid(format!(
                "the documentation input is not valid TOON ({error}); the expected header is \
                 `docs[N]{{symbol,text}}:`, and a cell holding a colon or a comma must be quoted, \
                 as `pn-ultramemory docs gaps` prints it"
            ))
        })?
    };
    match value {
        Value::Array(rows) => Ok(rows),
        Value::Object(map) => match map.get("docs") {
            Some(Value::Array(rows)) => Ok(rows.clone()),
            _ => Err(EngineError::Invalid(
                "the documentation input has no `docs` list; wrap the rows in \
                 `docs[N]{symbol,text}:` or in a JSON array, as `pn-ultramemory docs gaps` shows"
                    .to_owned(),
            )),
        },
        _ => Err(EngineError::Invalid(
            "the documentation input is a single value, not a list of entries; answer \
             `pn-ultramemory docs gaps` with one row per symbol"
                .to_owned(),
        )),
    }
}

/// One named string field of a row.
///
/// # Errors
/// Returns [`EngineError::Invalid`] naming the entry number when the field is absent, is not a
/// string or is blank.
fn field(row: &Value, key: &str, number: usize) -> Result<String, EngineError> {
    let Some(value) = row.get(key) else {
        return Err(EngineError::Invalid(format!(
            "entry {number} has no `{key}`; every entry needs a `symbol` and a `text`, as \
             `pn-ultramemory docs gaps` lists them"
        )));
    };
    let Some(text) = value.as_str() else {
        return Err(EngineError::Invalid(format!(
            "entry {number} has a `{key}` that is not text; quote it and rerun \
             `pn-ultramemory docs apply`"
        )));
    };
    if text.trim().is_empty() {
        return Err(EngineError::Invalid(format!(
            "entry {number} has an empty `{key}`; fill it in or drop the entry, then rerun \
             `pn-ultramemory docs apply`"
        )));
    }
    Ok(text.to_owned())
}

/// Checks one entry's two fields against the length and content limits.
///
/// # Errors
/// Returns [`EngineError::Invalid`] naming the entry number for a NUL byte or an over-long text.
fn check(entry: &DocEntry, number: usize) -> Result<(), EngineError> {
    if entry.symbol.contains('\0') || entry.text.contains('\0') {
        return Err(EngineError::Invalid(format!(
            "entry {number} contains a NUL byte, which no source file may hold; remove it and \
             rerun `pn-ultramemory docs apply`"
        )));
    }
    let length = entry.text.chars().count();
    if length > MAX_TEXT_CHARS {
        return Err(EngineError::Invalid(format!(
            "entry {number} is {length} characters long, over the limit of {MAX_TEXT_CHARS}; \
             shorten it and rerun `pn-ultramemory docs apply`"
        )));
    }
    Ok(())
}

/// Reads a batch of documentation entries from TOON or JSON text.
///
/// The shape is detected from the first non-blank character: `[` or `{` mean JSON, anything else
/// means TOON. The three accepted shapes and the limits are described below.
///
/// # Errors
/// Returns [`EngineError::Invalid`] for text that is neither format, for a batch of more than 500
/// entries, and for any entry that lacks a field, has a field of the wrong type, holds a NUL byte or
/// carries more than 2 000 characters of text. Every message names the entry number.
///
/// # Examples
/// ```
/// use pn_ultramemory_engine::parse_doc_entries;
///
/// let toon = "docs[1]{symbol,text}:\n  load_config,Loads the configuration.";
/// let entries = parse_doc_entries(toon).unwrap();
/// assert_eq!(entries[0].symbol, "load_config");
///
/// let json = r#"[{"symbol": "load_config", "text": "Loads the configuration."}]"#;
/// assert_eq!(parse_doc_entries(json).unwrap(), entries);
///
/// assert!(parse_doc_entries("docs[1]{symbol}:\n  a").is_err());
/// ```
pub fn parse_doc_entries(input: &str) -> Result<Vec<DocEntry>, EngineError> {
    let rows = rows_of(input)?;
    if rows.len() > MAX_ENTRIES {
        return Err(EngineError::Invalid(format!(
            "{} entries is over the limit of {MAX_ENTRIES}; apply them in smaller batches with \
             `pn-ultramemory docs apply`",
            rows.len()
        )));
    }
    let mut entries = Vec::with_capacity(rows.len());
    for (index, row) in rows.iter().enumerate() {
        let number = index + 1;
        let entry = DocEntry {
            symbol: field(row, "symbol", number)?.trim().to_owned(),
            text: field(row, "text", number)?,
        };
        check(&entry, number)?;
        entries.push(entry);
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::{DocEntry, MAX_ENTRIES, MAX_TEXT_CHARS, parse_doc_entries};

    /// The three accepted shapes all decode to the same entries.
    #[test]
    fn every_shape_decodes_the_same() {
        let expected = vec![
            DocEntry {
                symbol: "a".to_owned(),
                text: "First.".to_owned(),
            },
            DocEntry {
                symbol: "B::c".to_owned(),
                text: "Second one.".to_owned(),
            },
        ];
        let toon = "docs[2]{symbol,text}:\n  a,First.\n  \"B::c\",Second one.";
        assert_eq!(parse_doc_entries(toon).unwrap(), expected);
        let array = r#"[{"symbol":"a","text":"First."},{"symbol":"B::c","text":"Second one."}]"#;
        assert_eq!(parse_doc_entries(array).unwrap(), expected);
        let object =
            r#"{"docs":[{"symbol":"a","text":"First."},{"symbol":"B::c","text":"Second one."}]}"#;
        assert_eq!(parse_doc_entries(object).unwrap(), expected);
    }

    /// Leading blank lines do not change which format is detected.
    #[test]
    fn detection_ignores_leading_space() {
        let entries = parse_doc_entries("\n\n  [{\"symbol\":\"a\",\"text\":\"T.\"}]  \n").unwrap();
        assert_eq!(entries.len(), 1);
        let toon = parse_doc_entries("\ndocs[1]{symbol,text}:\n  a,T.\n").unwrap();
        assert_eq!(toon, entries);
    }

    /// A symbol is trimmed, and a text keeps its own spacing.
    #[test]
    fn fields_are_trimmed_where_it_is_safe() {
        let entries = parse_doc_entries(r#"[{"symbol":"  a  ","text":" Keeps  spacing. "}]"#)
            .expect("parses");
        assert_eq!(entries[0].symbol, "a");
        assert_eq!(entries[0].text, " Keeps  spacing. ");
    }

    /// Empty input, a broken document and a value instead of a list are each refused by name.
    #[test]
    fn broken_input_is_refused() {
        let empty = parse_doc_entries("   \n").unwrap_err().to_string();
        assert!(empty.contains("is empty"), "{empty}");
        let bad_json = parse_doc_entries("[{\"symbol\":}]")
            .unwrap_err()
            .to_string();
        assert!(bad_json.contains("not valid JSON"), "{bad_json}");
        let bad_toon = parse_doc_entries("docs[3]{symbol,text}:\n  a,b")
            .unwrap_err()
            .to_string();
        assert!(bad_toon.contains("not valid TOON"), "{bad_toon}");
        let no_list = parse_doc_entries("{\"other\": 1}").unwrap_err().to_string();
        assert!(no_list.contains("`docs`"), "{no_list}");
        let scalar = parse_doc_entries("plain text").unwrap_err().to_string();
        assert!(!scalar.is_empty());
    }

    /// A missing, mistyped or blank field names the entry number that is wrong.
    #[test]
    fn field_problems_name_the_entry() {
        let missing = parse_doc_entries(r#"[{"symbol":"a","text":"T."},{"symbol":"b"}]"#)
            .unwrap_err()
            .to_string();
        assert!(missing.contains("entry 2"), "{missing}");
        assert!(missing.contains("`text`"), "{missing}");
        let mistyped = parse_doc_entries(r#"[{"symbol":"a","text":7}]"#)
            .unwrap_err()
            .to_string();
        assert!(mistyped.contains("entry 1"), "{mistyped}");
        let blank = parse_doc_entries(r#"[{"symbol":"a","text":"   "}]"#)
            .unwrap_err()
            .to_string();
        assert!(blank.contains("empty"), "{blank}");
    }

    /// A NUL byte and an over-long text are refused, and both messages say what to do.
    #[test]
    fn content_limits_are_enforced() {
        let nul = parse_doc_entries("[{\"symbol\":\"a\",\"text\":\"a\\u0000b\"}]")
            .unwrap_err()
            .to_string();
        assert!(nul.contains("NUL"), "{nul}");
        assert!(nul.contains("pn-ultramemory docs apply"), "{nul}");
        let long = "x".repeat(MAX_TEXT_CHARS + 1);
        let input = format!("[{{\"symbol\":\"a\",\"text\":\"{long}\"}}]");
        let error = parse_doc_entries(&input).unwrap_err().to_string();
        assert!(error.contains("over the limit"), "{error}");
        let edge = "y".repeat(MAX_TEXT_CHARS);
        let ok = format!("[{{\"symbol\":\"a\",\"text\":\"{edge}\"}}]");
        assert_eq!(parse_doc_entries(&ok).unwrap().len(), 1);
    }

    /// A batch larger than the limit is refused before any entry is read.
    #[test]
    fn batch_size_is_capped() {
        let mut rows = Vec::new();
        for index in 0..=MAX_ENTRIES {
            rows.push(format!("{{\"symbol\":\"s{index}\",\"text\":\"T.\"}}"));
        }
        let input = format!("[{}]", rows.join(","));
        let error = parse_doc_entries(&input).unwrap_err().to_string();
        assert!(error.contains("over the limit of 500"), "{error}");
    }
}
