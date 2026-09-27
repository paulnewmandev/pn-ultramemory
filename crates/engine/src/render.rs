// SPDX-License-Identifier: Apache-2.0
//! Rendering engine results as TOON, JSON or text.
//!
//! Every result of the engine is a [`serde_json::Value`] (see the `to_value` methods). This module
//! turns one into the three output formats of the tool:
//!
//! * [`Format::Toon`]: TOON through `pn-ultramemory-toon`, the default, where uniform arrays become
//!   compact tables.
//! * [`Format::Json`]: pretty-printed JSON for programs.
//! * [`Format::Text`]: a deterministic human-readable layout defined here. Objects are `key: value`
//!   lines indented by two spaces per level, arrays of uniform objects are aligned tables with a
//!   header row and no borders, arrays of primitives are one comma-separated line, and multi-line
//!   strings are indented blocks.
//!
//! # Invariants
//! * **Total.** Rendering never panics, whatever the value: control characters are escaped, cells are
//!   cut at [`MAX_CELL_WIDTH`] characters with an ellipsis, and nesting deeper than [`MAX_DEPTH`]
//!   levels is replaced by an ellipsis.
//! * **Deterministic.** The same value always gives the same text.
//! * **No trailing newline.** The output of every format ends at its last character.

use core::fmt::Write as _;

use pn_ultramemory_codec::Format;
use pn_ultramemory_toon::{Delimiter, EncodeOptions};
use serde_json::{Map, Value};

/// The widest a table column is drawn; longer cells are cut and end with an ellipsis.
pub const MAX_CELL_WIDTH: usize = 60;

/// The deepest nesting the text format draws; anything deeper is replaced by an ellipsis.
pub const MAX_DEPTH: usize = 48;

/// The spaces of one level of indentation.
const INDENT: &str = "  ";

/// Renders `value` in the requested format. The output never ends with a newline.
///
/// `delimiter` is used by TOON only.
///
/// # Examples
/// ```
/// use pn_ultramemory_codec::Format;
/// use pn_ultramemory_engine::render_value;
/// use pn_ultramemory_toon::Delimiter;
/// use serde_json::json;
///
/// let value = json!({"name": "demo", "files": [{"path": "a.rs", "lines": 10}, {"path": "b.rs", "lines": 7}]});
/// assert_eq!(
///     render_value(&value, Format::Toon, Delimiter::Comma),
///     "name: demo\nfiles[2]{path,lines}:\n  a.rs,10\n  b.rs,7"
/// );
/// assert_eq!(
///     render_value(&value, Format::Text, Delimiter::Comma),
///     "name: demo\nfiles:\n  path  lines\n  a.rs  10\n  b.rs  7"
/// );
/// ```
#[must_use]
pub fn render_value(value: &Value, format: Format, delimiter: Delimiter) -> String {
    let mut out = match format {
        Format::Toon => {
            let options = EncodeOptions {
                delimiter,
                ..EncodeOptions::default()
            };
            pn_ultramemory_toon::encode(value, &options)
        }
        Format::Json => serde_json::to_string_pretty(value).unwrap_or_default(),
        Format::Text => render_text(value),
    };
    while out.ends_with('\n') {
        out.pop();
    }
    out
}

/// Renders a value in the text format.
fn render_text(value: &Value) -> String {
    let mut lines = Vec::new();
    match value {
        Value::Object(map) => object_lines(map, 0, 0, &mut lines),
        Value::Array(items) => array_lines(items, 0, 0, &mut lines),
        Value::String(text) if text.contains('\n') => {
            lines.extend(text.split('\n').map(clean_line));
        }
        other => lines.push(scalar_text(other)),
    }
    lines.join("\n")
}

/// Replaces control characters in a piece of text that must stay on one line.
fn escape_control(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{{{:04x}}}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out
}

/// One line of a multi-line string: control characters other than the tab are escaped, and a
/// trailing carriage return (from a CRLF line end) is dropped.
fn clean_line(line: &str) -> String {
    let line = line.strip_suffix('\r').unwrap_or(line);
    let mut out = String::with_capacity(line.len());
    for ch in line.chars() {
        if ch.is_control() && ch != '\t' {
            let _ = write!(out, "\\u{{{:04x}}}", u32::from(ch));
        } else {
            out.push(ch);
        }
    }
    out
}

/// The text of a primitive value on one line.
fn scalar_text(value: &Value) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::Bool(flag) => flag.to_string(),
        Value::Number(number) => number.to_string(),
        Value::String(text) => escape_control(text),
        Value::Array(_) | Value::Object(_) => String::new(),
    }
}

/// Whether a value is a primitive.
const fn is_primitive(value: &Value) -> bool {
    !matches!(value, Value::Array(_) | Value::Object(_))
}

/// A key as it is written: on one line, and visible even when empty.
fn key_text(key: &str) -> String {
    if key.is_empty() {
        "\"\"".to_owned()
    } else {
        escape_control(key)
    }
}

/// The padding of `level` levels of indentation.
fn pad(level: usize) -> String {
    INDENT.repeat(level)
}

/// Cuts a cell to [`MAX_CELL_WIDTH`] characters, ending with an ellipsis when it was longer.
fn clip(text: &str) -> String {
    if text.chars().count() <= MAX_CELL_WIDTH {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(MAX_CELL_WIDTH - 1).collect();
    out.push('\u{2026}');
    out
}

/// The items of an inline array of primitives, with strings quoted only where a comma or an edge
/// space would make them ambiguous.
fn inline_item(value: &Value) -> String {
    match value {
        Value::String(text) => {
            let flat = escape_control(text);
            let ambiguous = flat.is_empty()
                || flat.contains(',')
                || flat.starts_with(' ')
                || flat.ends_with(' ');
            if ambiguous {
                format!("\"{}\"", flat.replace('"', "\\\""))
            } else {
                flat
            }
        }
        other => scalar_text(other),
    }
}

/// Whether every item is a primitive that fits on one line.
fn is_inline_array(items: &[Value]) -> bool {
    !items.is_empty() && items.iter().all(is_primitive)
}

/// The keys of the objects of an array that is a uniform table, or `None`.
///
/// It is a table when it has items, every item is a non-empty object with the same set of keys as
/// the first, and every value is a primitive.
fn table_keys(items: &[Value]) -> Option<Vec<&str>> {
    let first = items.first()?.as_object()?;
    if first.is_empty() {
        return None;
    }
    let keys: Vec<&str> = first.keys().map(String::as_str).collect();
    for item in items {
        let object = item.as_object()?;
        if object.len() != keys.len() {
            return None;
        }
        if !keys
            .iter()
            .all(|key| object.get(*key).is_some_and(is_primitive))
        {
            return None;
        }
    }
    Some(keys)
}

/// Appends an aligned table: a header row and one row per item, columns separated by two spaces.
fn table_lines(keys: &[&str], items: &[Value], level: usize, out: &mut Vec<String>) {
    let mut rows: Vec<Vec<String>> = Vec::with_capacity(items.len() + 1);
    rows.push(keys.iter().map(|key| clip(&key_text(key))).collect());
    for item in items {
        let row = keys
            .iter()
            .map(|key| clip(&item.get(*key).map(scalar_text).unwrap_or_default()))
            .collect();
        rows.push(row);
    }
    let widths: Vec<usize> = (0..keys.len())
        .map(|column| {
            rows.iter()
                .map(|row| row[column].chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();
    let margin = pad(level);
    for row in rows {
        let mut line = margin.clone();
        for (column, cell) in row.iter().enumerate() {
            line.push_str(cell);
            if column + 1 < row.len() {
                let fill = widths[column] - cell.chars().count() + 2;
                line.push_str(&" ".repeat(fill));
            }
        }
        out.push(line);
    }
}

/// Appends the lines of an array at `level`: a table, one inline line, or a list.
fn array_lines(items: &[Value], level: usize, depth: usize, out: &mut Vec<String>) {
    if depth > MAX_DEPTH {
        out.push(format!("{}\u{2026}", pad(level)));
    } else if items.is_empty() {
        out.push(format!("{}[]", pad(level)));
    } else if let Some(keys) = table_keys(items) {
        table_lines(&keys, items, level, out);
    } else if is_inline_array(items) {
        let joined: Vec<String> = items.iter().map(inline_item).collect();
        out.push(format!("{}{}", pad(level), joined.join(", ")));
    } else {
        list_lines(items, level, depth, out);
    }
}

/// Appends the items of a list as `- ` entries; a nested container starts on the dash line.
fn list_lines(items: &[Value], level: usize, depth: usize, out: &mut Vec<String>) {
    for item in items {
        let margin = pad(level);
        match item {
            Value::Object(map) if !map.is_empty() => {
                let mut nested = Vec::new();
                object_lines(map, level + 1, depth + 1, &mut nested);
                push_with_dash(&margin, nested, out);
            }
            Value::Array(inner) if !inner.is_empty() => {
                let mut nested = Vec::new();
                array_lines(inner, level + 1, depth + 1, &mut nested);
                push_with_dash(&margin, nested, out);
            }
            Value::Object(_) => out.push(format!("{margin}- {{}}")),
            Value::Array(_) => out.push(format!("{margin}- []")),
            Value::String(text) if text.contains('\n') => {
                out.push(format!("{margin}- |"));
                out.extend(block_lines(text, level + 1));
            }
            other => out.push(format!("{margin}- {}", scalar_text(other))),
        }
    }
}

/// Appends nested lines with a dash in place of the first line's indentation of one level.
fn push_with_dash(margin: &str, nested: Vec<String>, out: &mut Vec<String>) {
    for (index, line) in nested.into_iter().enumerate() {
        if index == 0 {
            let rest = line.get(margin.len() + INDENT.len()..).unwrap_or("");
            out.push(format!("{margin}- {rest}"));
        } else {
            out.push(line);
        }
    }
}

/// The lines of a multi-line string block at `level`; empty lines carry no padding.
fn block_lines(text: &str, level: usize) -> Vec<String> {
    let margin = pad(level);
    text.split('\n')
        .map(|line| {
            let line = clean_line(line);
            if line.is_empty() {
                line
            } else {
                format!("{margin}{line}")
            }
        })
        .collect()
}

/// Appends the lines of an object at `level`.
fn object_lines(map: &Map<String, Value>, level: usize, depth: usize, out: &mut Vec<String>) {
    if depth > MAX_DEPTH {
        out.push(format!("{}\u{2026}", pad(level)));
        return;
    }
    let margin = pad(level);
    for (key, value) in map {
        let key = key_text(key);
        match value {
            Value::Object(inner) if inner.is_empty() => out.push(format!("{margin}{key}: {{}}")),
            Value::Object(inner) => {
                out.push(format!("{margin}{key}:"));
                object_lines(inner, level + 1, depth + 1, out);
            }
            Value::Array(items) if items.is_empty() => out.push(format!("{margin}{key}: []")),
            Value::Array(items) if is_inline_array(items) => {
                let joined: Vec<String> = items.iter().map(inline_item).collect();
                out.push(format!("{margin}{key}: {}", joined.join(", ")));
            }
            Value::Array(items) => {
                out.push(format!("{margin}{key}:"));
                array_lines(items, level + 1, depth + 1, out);
            }
            Value::String(text) if text.contains('\n') => {
                out.push(format!("{margin}{key}: |"));
                out.extend(block_lines(text, level + 1));
            }
            other => out.push(format!("{margin}{key}: {}", scalar_text(other))),
        }
    }
}

#[cfg(test)]
mod tests {
    use pn_ultramemory_codec::Format;
    use pn_ultramemory_toon::Delimiter;
    use serde_json::{Value, json};

    use super::{MAX_CELL_WIDTH, MAX_DEPTH, render_value};

    /// Renders as text.
    fn text(value: &Value) -> String {
        render_value(value, Format::Text, Delimiter::Comma)
    }

    /// Objects are `key: value` lines and nested objects are indented by two spaces.
    #[test]
    fn objects_nest_with_two_spaces() {
        let value = json!({"name": "demo", "n": 3, "ok": true, "none": null, "inner": {"a": 1, "deep": {"b": "x"}}});
        assert_eq!(
            text(&value),
            "name: demo\nn: 3\nok: true\nnone: null\ninner:\n  a: 1\n  deep:\n    b: x"
        );
    }

    /// Arrays of uniform objects are aligned tables, arrays of primitives are one line.
    #[test]
    fn arrays_become_tables_or_lines() {
        let value = json!({
            "tags": ["a", "b c", 3, false],
            "rows": [{"path": "src/lib.rs", "lines": 120}, {"path": "a.rs", "lines": 7}],
        });
        assert_eq!(
            text(&value),
            "tags: a, b c, 3, false\nrows:\n  path        lines\n  src/lib.rs  120\n  a.rs        7"
        );
    }

    /// Empty containers are written out, and a table is only used for one uniform shape.
    #[test]
    fn empty_and_mixed_arrays() {
        let value = json!({"e": [], "o": {}, "mixed": [{"a": 1}, {"b": 2}, [1, 2], "s", null]});
        assert_eq!(
            text(&value),
            "e: []\no: {}\nmixed:\n  - a: 1\n  - b: 2\n  - 1, 2\n  - s\n  - null"
        );
    }

    /// A table with nested cells falls back to the list form, so no information is hidden.
    #[test]
    fn nested_cells_use_the_list_form() {
        let value = json!([{"a": 1, "b": [1, 2]}, {"a": 2, "b": []}]);
        assert_eq!(text(&value), "- a: 1\n  b: 1, 2\n- a: 2\n  b: []");
    }

    /// Long cells are cut at the column limit with an ellipsis and short ones are padded.
    #[test]
    fn long_cells_are_clipped() {
        let long = "x".repeat(200);
        let value = json!([{"k": long, "n": 1}, {"k": "short", "n": 22}]);
        let rendered = text(&value);
        let lines: Vec<&str> = rendered.lines().collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(
            lines[1].chars().take(MAX_CELL_WIDTH).count(),
            MAX_CELL_WIDTH
        );
        assert!(lines[1].contains(&format!("{}\u{2026}  1", "x".repeat(MAX_CELL_WIDTH - 1))));
        assert!(lines[2].starts_with(&format!("short{}  22", " ".repeat(MAX_CELL_WIDTH - 5))));
    }

    /// Newlines and control characters never break the layout.
    #[test]
    fn hostile_strings_stay_on_their_lines() {
        let nasty = "a\nb\r\nc\u{0}d\u{7}\te\u{1b}[31m";
        let value = json!({"k\nkey": nasty, "rows": [{"c": nasty}, {"c": "ok"}], "": "empty key"});
        let rendered = text(&value);
        assert!(rendered.contains("k\\nkey: |"), "{rendered}");
        assert!(
            rendered.contains("\\u{0000}d\\u{0007}\te\\u{001b}[31m"),
            "{rendered}"
        );
        assert!(
            rendered.contains("a\\nb\\r\\nc\\u{0000}d\\u{0007}\\te\\u{001b}[31m"),
            "{rendered}"
        );
        assert!(rendered.contains("\"\": empty key"));
        assert!(!rendered.contains('\u{1b}'));
        assert!(!rendered.contains('\u{0}'));
    }

    /// Multi-line strings become indented blocks and blank lines carry no padding.
    #[test]
    fn multiline_strings_are_blocks() {
        let value = json!({"code": "fn f() {\n\n    1\n}\n", "after": 1});
        assert_eq!(
            text(&value),
            "code: |\n  fn f() {\n\n      1\n  }\n\nafter: 1"
        );
    }

    /// Top-level primitives and arrays render too.
    #[test]
    fn top_level_shapes() {
        assert_eq!(text(&json!("plain")), "plain");
        assert_eq!(text(&json!(null)), "null");
        assert_eq!(text(&json!(42)), "42");
        assert_eq!(text(&json!([1, 2, 3])), "1, 2, 3");
        assert_eq!(text(&json!([])), "[]");
        assert_eq!(text(&json!({})), "");
        assert_eq!(text(&json!("a\nb")), "a\nb");
    }

    /// Unicode is kept, and strings with commas are quoted only inside inline arrays.
    #[test]
    fn unicode_and_commas() {
        let value = json!({"list": ["caf\u{e9}", "a, b", "", " pad"], "s": "x, y"});
        assert_eq!(
            text(&value),
            "list: caf\u{e9}, \"a, b\", \"\", \" pad\"\ns: x, y"
        );
    }

    /// Nesting beyond the limit is cut with an ellipsis instead of overflowing the stack.
    #[test]
    fn deep_nesting_is_cut() {
        let mut value = json!("leaf");
        for _ in 0..(MAX_DEPTH * 4) {
            value = json!({"n": value});
        }
        let rendered = text(&value);
        assert!(rendered.contains('\u{2026}'));
        assert_eq!(rendered.lines().count(), MAX_DEPTH + 2);
        let mut arrays = json!(1);
        for _ in 0..(MAX_DEPTH * 4) {
            arrays = json!([arrays, "x"]);
        }
        assert!(text(&arrays).contains('\u{2026}'));
    }

    /// Every format ends without a newline, including for values that end with one.
    #[test]
    fn no_format_ends_with_a_newline() {
        let values = [
            json!({"a": "line\n"}),
            json!("ends\n\n"),
            json!([{"a": "x\n"}]),
            json!({}),
            json!([]),
            json!(null),
        ];
        for value in &values {
            for format in [Format::Toon, Format::Json, Format::Text] {
                for delimiter in [Delimiter::Comma, Delimiter::Tab, Delimiter::Pipe] {
                    let out = render_value(value, format, delimiter);
                    assert!(!out.ends_with('\n'), "{format:?} {value}: {out:?}");
                }
            }
        }
    }

    /// TOON and JSON delegate to their encoders, and TOON honours the delimiter.
    #[test]
    fn toon_and_json_delegate() {
        let value = json!({"rows": [{"a": 1, "b": "x"}, {"a": 2, "b": "y"}]});
        assert_eq!(
            render_value(&value, Format::Toon, Delimiter::Pipe),
            "rows[2|]{a|b}:\n  1|x\n  2|y"
        );
        let json_text = render_value(&value, Format::Json, Delimiter::Comma);
        assert_eq!(
            serde_json::from_str::<Value>(&json_text).expect("json"),
            value
        );
        assert!(json_text.contains("\n  \"rows\""));
    }

    /// The next value of a xorshift generator.
    fn next(state: &mut u64) -> u64 {
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        *state
    }

    /// A number below `bound` from the generator.
    fn below(state: &mut u64, bound: u64) -> usize {
        usize::try_from(next(state) % bound).unwrap_or(0)
    }

    /// A small generator of random values for the property check.
    fn random_value(state: &mut u64, depth: usize) -> Value {
        let alphabet = [
            "a",
            "b c",
            "x,y",
            "",
            "\n",
            "\u{e9}\u{1f389}",
            "\"q\"",
            "k: v",
            "\t",
            "\u{0}",
        ];
        let kinds = if depth > 4 { 4 } else { 7 };
        match below(state, kinds) {
            0 => Value::Null,
            1 => Value::Bool(next(state) % 2 == 0),
            2 => Value::from(i64::try_from(next(state) % 1000).unwrap_or(0) - 500),
            3 => Value::from(alphabet[below(state, 10)]),
            4 | 5 => {
                let count = below(state, 4);
                let uniform = next(state) % 2 == 0;
                let mut items = Vec::new();
                for _ in 0..count {
                    if uniform {
                        let id = next(state) % 100;
                        let name = alphabet[below(state, 10)];
                        items.push(json!({"id": id, "name": name}));
                    } else {
                        items.push(random_value(state, depth + 1));
                    }
                }
                Value::Array(items)
            }
            _ => {
                let count = below(state, 4);
                let mut map = serde_json::Map::new();
                for index in 0..count {
                    let key = format!("{}{index}", alphabet[below(state, 10)]);
                    map.insert(key, random_value(state, depth + 1));
                }
                Value::Object(map)
            }
        }
    }

    /// Random values render in every format without panicking, deterministically and without a
    /// trailing newline, and the text form never contains a raw control character but the tab
    /// and the line feed.
    #[test]
    fn random_values_render_everywhere() {
        let mut state = 0x9E37_79B9_7F4A_7C15_u64;
        for _ in 0..500 {
            let value = random_value(&mut state, 0);
            for format in [Format::Toon, Format::Json, Format::Text] {
                let a = render_value(&value, format, Delimiter::Comma);
                let b = render_value(&value, format, Delimiter::Comma);
                assert_eq!(a, b);
                assert!(!a.ends_with('\n'));
            }
            let rendered = text(&value);
            assert!(
                rendered
                    .chars()
                    .all(|c| !c.is_control() || c == '\n' || c == '\t'),
                "{rendered:?}"
            );
        }
    }
}
