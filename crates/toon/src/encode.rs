// SPDX-License-Identifier: Apache-2.0
//! The TOON encoder: [`serde_json::Value`] to text.
//!
//! # Role in the architecture
//! [`encode`] is the writing half of the crate. The form of every value follows from its shape and
//! position (section 1.4 of the TOON 4.1 specification), never from preference: inline primitive
//! arrays, tabular arrays (with nested field groups), keyed tabular objects, list form, and the
//! objects-as-list-items layout of section 10. Quoting and number rules live in
//! [`crate::quote`] and [`crate::number`].
//!
//! # Invariants
//! * Output is deterministic, uses LF only, has no trailing spaces and no trailing newline.
//! * Object key order is preserved, except that tabular rows and keyed entries are written in the
//!   header's field order (the first element's or first entry's key order).
//! * Every header declares the document delimiter, so the output decodes without out-of-band
//!   information.
//! * A field standing on a list-item hyphen line is at depth `d + 1` where the hyphen is at `d`;
//!   its rows, entries or nested lines are at `d + 2` (section 10).
//! * Recursion is proportional to the nesting depth of the value. `serde_json` itself refuses to
//!   parse documents nested deeper than 128 levels, so values built from parsed input are safe.

use crate::number::push_number;
use crate::options::{Delimiter, EncodeOptions, effective_indent};
use crate::quote::{push_key, push_quoted, push_string};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::fmt::Write as _;

/// A shared `null`, used if a value that the shape check guarantees is somehow absent.
static NULL: Value = Value::Null;

/// Where a value sits, which decides the form of an empty array and whether a tabular header is
/// allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pos {
    /// The document root.
    Root,
    /// An object field, entry or list-item field.
    Field,
    /// An element of an array in list form (tabular form is not available here).
    Item,
}

/// One column of a tabular header: a plain field or a nested field group.
#[derive(Debug)]
enum Column<'a> {
    /// A column of primitives.
    Leaf(&'a str),
    /// A column of uniform objects, expanded in place.
    Group(&'a str, Vec<Column<'a>>),
}

impl<'a> Column<'a> {
    /// Returns the field name of the column.
    fn name(&self) -> &'a str {
        match self {
            Self::Leaf(name) | Self::Group(name, _) => name,
        }
    }
}

/// Encodes a JSON value as TOON text.
///
/// The output has no trailing newline and no trailing spaces, uses LF line endings, and declares the
/// delimiter of `options` in every header. An empty root object encodes to the empty string.
///
/// # Numbers
///
/// Integers are written in decimal; floats in canonical decimal form with the shortest digits that
/// read back to the same value (`1.0` becomes `1`, `-0.0` becomes `0`); integer-valued floats from
/// `2^53` up are written with their exact digits; floats outside `[1e-6, 1e21)` use an exponent
/// (`1e-7`, `1.5e+300`). `NaN` and infinities cannot be stored in a
/// `serde_json::Number` (they become `null` when a `Value` is built), so nothing else is needed.
///
/// # Limits
///
/// `options.indent` is clamped to `1..=1024`. Recursion is proportional to the nesting depth of
/// `value`.
///
/// # Examples
///
/// ```
/// use pn_ultramemory_toon::{encode, EncodeOptions};
/// use serde_json::json;
///
/// let value = json!({
///     "users": [{"id": 1, "name": "Ada"}, {"id": 2, "name": "Bob"}],
///     "tags": ["a", "b,c"],
/// });
/// assert_eq!(
///     encode(&value, &EncodeOptions::default()),
///     "users[2]{id,name}:\n  1,Ada\n  2,Bob\ntags[2]: a,\"b,c\""
/// );
/// ```
#[must_use]
pub fn encode(value: &Value, options: &EncodeOptions) -> String {
    let mut encoder = Encoder {
        out: String::new(),
        indent: effective_indent(options.indent),
        delimiter: options.delimiter,
        dc: options.delimiter.as_char(),
    };
    encoder.root(value);
    encoder.out
}

/// Returns `true` for null, booleans, numbers and strings.
fn is_primitive(value: &Value) -> bool {
    !matches!(value, Value::Array(_) | Value::Object(_))
}

/// Finds the uniform column structure of `objects` (section 9.3), or `None` when they are not
/// uniform: every object non-empty, the same set of keys, and every column either all primitives
/// or all non-empty objects that are themselves uniform.
///
/// Objects that list their keys in the same order as the first one (the common case) are checked
/// without any hashing.
fn detect<'a>(objects: &[&'a Map<String, Value>]) -> Option<Vec<Column<'a>>> {
    let first: &'a Map<String, Value> = objects.first().copied()?;
    if first.is_empty() || objects.iter().any(|o| o.len() != first.len()) {
        return None;
    }
    let keys: Vec<&'a str> = first.keys().map(String::as_str).collect();
    // Per column: (still all primitive, still all non-empty objects).
    let mut kinds = vec![(true, true); keys.len()];
    let mut positions: Option<HashMap<&str, usize>> = None;
    for object in objects {
        let mut in_order = true;
        for (i, (key, value)) in object.iter().enumerate() {
            let column = if in_order && keys[i] == key.as_str() {
                i
            } else {
                in_order = false;
                let index = positions
                    .get_or_insert_with(|| keys.iter().enumerate().map(|(i, k)| (*k, i)).collect());
                *index.get(key.as_str())?
            };
            match value {
                Value::Object(inner) if !inner.is_empty() => kinds[column].0 = false,
                Value::Object(_) | Value::Array(_) => return None,
                _ => kinds[column].1 = false,
            }
            if !kinds[column].0 && !kinds[column].1 {
                return None;
            }
        }
    }
    let mut columns = Vec::with_capacity(keys.len());
    for (key, (all_primitive, _)) in keys.into_iter().zip(kinds) {
        if all_primitive {
            columns.push(Column::Leaf(key));
        } else {
            let inner: Vec<&'a Map<String, Value>> = objects
                .iter()
                .filter_map(|o| o.get(key)?.as_object())
                .collect();
            columns.push(Column::Group(key, detect(&inner)?));
        }
    }
    Some(columns)
}

/// Returns the objects of `items` if every element is an object.
fn as_objects(items: &[Value]) -> Option<Vec<&Map<String, Value>>> {
    items.iter().map(Value::as_object).collect()
}

/// Detects keyed tabular form for an object (section 9.5): at least two entries whose values are
/// uniform non-empty objects.
fn keyed_columns(map: &Map<String, Value>) -> Option<Vec<Column<'_>>> {
    if map.len() < 2 {
        return None;
    }
    let values: Vec<&Map<String, Value>> =
        map.values().map(Value::as_object).collect::<Option<_>>()?;
    detect(&values)
}

/// The writer state.
struct Encoder {
    /// Output text.
    out: String,
    /// Spaces per indentation level (at least 1).
    indent: usize,
    /// The document delimiter.
    delimiter: Delimiter,
    /// The document delimiter as a character.
    dc: char,
}

impl Encoder {
    /// Starts a new line at `depth`, preceded by `- ` when `hyphen` is set.
    fn open_line(&mut self, depth: usize, hyphen: bool) {
        if !self.out.is_empty() {
            self.out.push('\n');
        }
        self.out
            .extend(std::iter::repeat_n(' ', depth * self.indent));
        if hyphen {
            self.out.push_str("- ");
        }
    }

    /// Writes a primitive value; arrays and objects are never passed here.
    fn primitive(&mut self, value: &Value) {
        match value {
            Value::Null | Value::Array(_) | Value::Object(_) => self.out.push_str("null"),
            Value::Bool(true) => self.out.push_str("true"),
            Value::Bool(false) => self.out.push_str("false"),
            Value::Number(n) => push_number(&mut self.out, n),
            Value::String(s) => push_string(&mut self.out, s, self.dc),
        }
    }

    /// Writes the document.
    fn root(&mut self, value: &Value) {
        match value {
            Value::Object(map) if map.is_empty() => {}
            Value::Object(map) => match keyed_columns(map) {
                Some(columns) => {
                    self.open_line(0, false);
                    self.keyed_object(map, &columns, 1);
                }
                None => self.object_fields(map, 0),
            },
            Value::Array(items) => {
                self.open_line(0, false);
                self.array(items, 1, Pos::Root);
            }
            Value::String(s) if s.starts_with('\u{feff}') => {
                // A leading BOM would be swallowed by a decoder, so it must be quoted.
                push_quoted(&mut self.out, s);
            }
            primitive => self.primitive(primitive),
        }
    }

    /// Writes the fields of an object, one line each, at `depth`.
    fn object_fields(&mut self, map: &Map<String, Value>, depth: usize) {
        for (key, value) in map {
            self.open_line(depth, false);
            self.field(key, value, depth + 1);
        }
    }

    /// Writes `key` and its value on the line that is already open. Nested content goes at
    /// `content_depth`.
    fn field(&mut self, key: &str, value: &Value, content_depth: usize) {
        push_key(&mut self.out, key);
        match value {
            Value::Object(map) => {
                if let Some(columns) = keyed_columns(map) {
                    self.keyed_object(map, &columns, content_depth);
                } else {
                    self.out.push(':');
                    self.object_fields(map, content_depth);
                }
            }
            Value::Array(items) => self.array(items, content_depth, Pos::Field),
            primitive => {
                self.out.push_str(": ");
                self.primitive(primitive);
            }
        }
    }

    /// Writes `[N<delim>]`, an optional `{fields}` and the colon of a header. `keyed` adds the
    /// entry-count marker.
    fn header_tail(&mut self, len: usize, keyed: bool, columns: Option<&[Column<'_>]>) {
        let _ = write!(self.out, "[{len}");
        if keyed {
            self.out.push(':');
        }
        self.out.push_str(self.delimiter.header_symbol());
        self.out.push(']');
        if let Some(columns) = columns {
            self.out.push('{');
            self.columns(columns);
            self.out.push('}');
        }
        self.out.push(':');
    }

    /// Writes a field list, expanding nested groups in place.
    fn columns(&mut self, columns: &[Column<'_>]) {
        for (i, column) in columns.iter().enumerate() {
            if i > 0 {
                self.out.push(self.dc);
            }
            match column {
                Column::Leaf(name) => push_key(&mut self.out, name),
                Column::Group(name, sub) => {
                    push_key(&mut self.out, name);
                    self.out.push('{');
                    self.columns(sub);
                    self.out.push('}');
                }
            }
        }
    }

    /// Writes the cells of one row or entry row, depth-first over `columns`. Objects whose keys are
    /// in header order are read by position; others fall back to lookups by name.
    fn cells(&mut self, columns: &[Column<'_>], object: &Map<String, Value>, first: &mut bool) {
        let mut entries = object.iter();
        let mut in_sync = true;
        for column in columns {
            let name = column.name();
            let value = if in_sync {
                match entries.next() {
                    Some((key, value)) if key == name => Some(value),
                    _ => {
                        in_sync = false;
                        object.get(name)
                    }
                }
            } else {
                object.get(name)
            };
            match column {
                Column::Leaf(_) => {
                    if !std::mem::take(first) {
                        self.out.push(self.dc);
                    }
                    self.primitive(value.unwrap_or(&NULL));
                }
                Column::Group(_, sub) => {
                    if let Some(inner) = value.and_then(Value::as_object) {
                        self.cells(sub, inner, first);
                    }
                }
            }
        }
    }

    /// Writes an array whose key (if any) is already on the open line. List items and rows go at
    /// `content_depth`.
    fn array(&mut self, items: &[Value], content_depth: usize, pos: Pos) {
        if items.is_empty() {
            match pos {
                Pos::Root => self.out.push_str("[]"),
                Pos::Field => self.out.push_str(": []"),
                Pos::Item => {
                    self.out.push_str("[0");
                    self.out.push_str(self.delimiter.header_symbol());
                    self.out.push_str("]:");
                }
            }
            return;
        }
        if items.iter().all(is_primitive) {
            self.header_tail(items.len(), false, None);
            self.out.push(' ');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    self.out.push(self.dc);
                }
                self.primitive(item);
            }
            return;
        }
        if pos != Pos::Item {
            if let Some(objects) = as_objects(items) {
                if let Some(columns) = detect(&objects) {
                    self.header_tail(items.len(), false, Some(&columns));
                    for object in objects {
                        self.open_line(content_depth, false);
                        self.cells(&columns, object, &mut true);
                    }
                    return;
                }
            }
        }
        self.header_tail(items.len(), false, None);
        for item in items {
            self.list_item(item, content_depth);
        }
    }

    /// Writes a keyed tabular object: its header on the open line, then one entry row per entry at
    /// `content_depth` (section 9.5).
    fn keyed_object(
        &mut self,
        map: &Map<String, Value>,
        columns: &[Column<'_>],
        content_depth: usize,
    ) {
        self.header_tail(map.len(), true, Some(columns));
        for (key, value) in map {
            self.open_line(content_depth, false);
            push_key(&mut self.out, key);
            self.out.push_str(": ");
            if let Some(entry) = value.as_object() {
                self.cells(columns, entry, &mut true);
            }
        }
    }

    /// Writes one element of an array in list form, with its hyphen at `depth` (section 10).
    fn list_item(&mut self, item: &Value, depth: usize) {
        match item {
            Value::Object(map) if map.is_empty() => {
                self.open_line(depth, false);
                self.out.push('-');
            }
            Value::Object(map) => {
                for (i, (key, value)) in map.iter().enumerate() {
                    if i == 0 {
                        self.open_line(depth, true);
                    } else {
                        self.open_line(depth + 1, false);
                    }
                    self.field(key, value, depth + 2);
                }
            }
            Value::Array(items) => {
                self.open_line(depth, true);
                self.array(items, depth + 1, Pos::Item);
            }
            primitive => {
                self.open_line(depth, true);
                self.primitive(primitive);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Encodes with the default options.
    fn enc(value: &Value) -> String {
        encode(value, &EncodeOptions::default())
    }

    /// Primitives at the root are written bare or quoted as needed.
    #[test]
    fn root_primitives() {
        assert_eq!(enc(&json!("hello")), "hello");
        assert_eq!(enc(&json!("")), "\"\"");
        assert_eq!(enc(&json!(42)), "42");
        assert_eq!(enc(&json!(true)), "true");
        assert_eq!(enc(&json!(null)), "null");
        assert_eq!(enc(&json!("a\u{feff}")), "a\u{feff}");
        assert_eq!(enc(&json!("\u{feff}a")), "\"\u{feff}a\"");
    }

    /// Empty containers use the explicit forms of section 9.1 and section 8.
    #[test]
    fn empty_containers() {
        assert_eq!(enc(&json!({})), "");
        assert_eq!(enc(&json!([])), "[]");
        assert_eq!(enc(&json!({"a": [], "b": {}})), "a: []\nb:");
        assert_eq!(enc(&json!([[], {}])), "[2]:\n  - [0]:\n  -");
    }

    /// The form of an array follows from its shape.
    #[test]
    fn array_forms() {
        assert_eq!(enc(&json!({"a": [1, "x", null]})), "a[3]: 1,x,null");
        assert_eq!(
            enc(&json!({"a": [{"x": 1}, {"x": 2}]})),
            "a[2]{x}:\n  1\n  2"
        );
        assert_eq!(
            enc(&json!({"a": [{"x": 1}, {"y": 2}]})),
            "a[2]:\n  - x: 1\n  - y: 2"
        );
        assert_eq!(
            enc(&json!({"a": [[1], [2, 3]]})),
            "a[2]:\n  - [1]: 1\n  - [2]: 2,3"
        );
        assert_eq!(enc(&json!({"a": [1, {"x": 1}]})), "a[2]:\n  - 1\n  - x: 1");
    }

    /// Columns that are not uniform disqualify tabular form.
    #[test]
    fn tabular_detection_edge_cases() {
        assert_eq!(
            enc(&json!([{"a": null}, {"a": {"b": 1}}])),
            "[2]:\n  - a: null\n  - a:\n      b: 1"
        );
        assert_eq!(
            enc(&json!([{"a": []}, {"a": []}])),
            "[2]:\n  - a: []\n  - a: []"
        );
        assert_eq!(enc(&json!([{"a": {}}, {"a": {}}])), "[2]:\n  - a:\n  - a:");
        assert_eq!(enc(&json!([{}, {}])), "[2]:\n  -\n  -");
        assert_eq!(
            enc(&json!([{"a": 1, "b": 2}, {"b": 4, "a": 3}])),
            "[2]{a,b}:\n  1,2\n  3,4"
        );
        assert_eq!(
            enc(&json!([{"a": 1}, {"b": 1}])),
            "[2]:\n  - a: 1\n  - b: 1"
        );
        assert_eq!(
            enc(&json!([{"g": {"x": 1, "y": 2}}, {"g": {"y": 4, "x": 3}}])),
            "[2]{g{x,y}}:\n  1,2\n  3,4"
        );
        assert_eq!(
            enc(&json!([{"g": {"x": 1}}, {"g": {"y": 3}}])),
            "[2]:\n  - g:\n      x: 1\n  - g:\n      y: 3"
        );
    }

    /// Keyed tabular form needs two or more uniform entries; array elements are never keyed.
    #[test]
    fn keyed_forms() {
        assert_eq!(
            enc(&json!({"m": {"a": {"x": 1}, "b": {"x": 2}}})),
            "m[2:]{x}:\n  a: 1\n  b: 2"
        );
        assert_eq!(
            enc(&json!({"a": {"x": 1}, "b": {"x": 2}})),
            "[2:]{x}:\n  a: 1\n  b: 2"
        );
        assert_eq!(enc(&json!({"m": {"a": {"x": 1}}})), "m:\n  a:\n    x: 1");
        assert_eq!(
            enc(&json!([{"a": {"x": 1}, "b": {"x": 2}}, 5])),
            "[2]:\n  - a:\n      x: 1\n    b:\n      x: 2\n  - 5"
        );
    }

    /// Objects as list items carry their first field on the hyphen line (section 10).
    #[test]
    fn list_item_layout() {
        let value = json!({"items": [
            {"users": [{"id": 1}, {"id": 2}], "status": "ok"},
            {"m": {"a": {"x": 1}, "b": {"x": 2}}, "k": 1},
            {"nested": {"p": 1}, "q": [1, 2]},
        ]});
        assert_eq!(
            enc(&value),
            "items[3]:\n  - users[2]{id}:\n      1\n      2\n    status: ok\n  - m[2:]{x}:\n      a: 1\n      b: 2\n    k: 1\n  - nested:\n      p: 1\n    q[2]: 1,2"
        );
    }

    /// The delimiter is declared in every header and used in rows.
    #[test]
    fn delimiters() {
        let value = json!({"a": [{"x": "p|q", "y": "r,s"}], "b": ["u|v"]});
        let pipe = EncodeOptions {
            delimiter: Delimiter::Pipe,
            ..EncodeOptions::default()
        };
        assert_eq!(
            encode(&value, &pipe),
            "a[1|]{x|y}:\n  \"p|q\"|r,s\nb[1|]: \"u|v\""
        );
        let tab = EncodeOptions {
            delimiter: Delimiter::Tab,
            ..EncodeOptions::default()
        };
        assert_eq!(
            encode(&value, &tab),
            "a[1\t]{x\ty}:\n  p|q\tr,s\nb[1\t]: u|v"
        );
    }

    /// A custom indent is applied per level, and zero is clamped to one.
    #[test]
    fn indentation() {
        let value = json!({"a": {"b": 1}});
        let four = EncodeOptions {
            indent: 4,
            ..EncodeOptions::default()
        };
        assert_eq!(encode(&value, &four), "a:\n    b: 1");
        let zero = EncodeOptions {
            indent: 0,
            ..EncodeOptions::default()
        };
        assert_eq!(encode(&value, &zero), "a:\n b: 1");
        let huge = EncodeOptions {
            indent: usize::MAX,
            ..EncodeOptions::default()
        };
        let text = encode(&value, &huge);
        assert_eq!(text, format!("a:\n{}b: 1", " ".repeat(1024)));
        let decoded = crate::decode(
            &text,
            &crate::DecodeOptions {
                indent: usize::MAX,
                strict: true,
            },
        );
        assert_eq!(decoded.unwrap(), value);
    }

    /// Keys that do not match the unquoted pattern are quoted everywhere, including headers.
    #[test]
    fn key_quoting_everywhere() {
        let value = json!({"my-key": [1, 2], "x y": [{"a b": 1}], "": {"q:": 1}});
        assert_eq!(
            enc(&value),
            "\"my-key\"[2]: 1,2\n\"x y\"[1]{\"a b\"}:\n  1\n\"\":\n  \"q:\": 1"
        );
    }

    /// Strings that could be misread are quoted, and numbers are canonical.
    #[test]
    fn scalars_are_safe_and_canonical() {
        let value =
            json!({"a": "true", "b": "1e5", "c": "-x", "d": " p ", "e": 1.0, "f": -0.0, "g": 1e21});
        assert_eq!(
            enc(&value),
            "a: \"true\"\nb: \"1e5\"\nc: \"-x\"\nd: \" p \"\ne: 1\nf: 0\ng: 1e+21"
        );
    }
}
