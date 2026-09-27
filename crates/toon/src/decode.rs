// SPDX-License-Identifier: Apache-2.0
//! The TOON decoder: text to [`serde_json::Value`].
//!
//! # Role in the architecture
//! [`decode`] is the reading half of the crate. It runs the lexical pre-pass of [`crate::lines`],
//! discovers the root form (section 5), and then walks the lines with a small recursive-descent
//! parser: objects, list-form arrays, tabular arrays and keyed tabular objects, each with the
//! strict-mode checks of section 14. Line classification and header grammar live in
//! [`crate::header`]; delimiter splitting and primitive tokens live in [`crate::token`].
//!
//! # Invariants
//! * Never panics on any input: every failure is a [`DecodeError`] with a line number.
//! * Nesting is limited to [`MAX_DEPTH`] containers; deeper input is an error, not a stack overflow.
//! * Nothing is allocated in proportion to a declared length `[N]`; vectors grow only as values
//!   are actually read (section 15).
//! * A declared length never truncates or terminates a scope; it is only checked (strict mode).
//! * Object keys keep document order, except that tabular rows and keyed entries follow the
//!   header's field order (sections 2, 9.3 and 9.5).
//! * Blank lines inside a header's span are errors in strict mode and ignored otherwise
//!   (section 12), tracked with the `open_spans` counter.

use crate::error::DecodeError;
use crate::header::{Field, Header, Kind, MAX_DEPTH, classify, key_value_fallthrough};
use crate::lines::{Line, split_lines};
use crate::options::{DecodeOptions, effective_indent};
use crate::quote::parse_quoted;
use crate::token::{is_row_line, parse_primitive, split_cells, split_cells_into};
use serde_json::map::Entry;
use serde_json::{Map, Value};

/// Result alias used inside the parser.
type Res<T> = Result<T, DecodeError>;

/// Decodes TOON text into a JSON value.
///
/// The root form is chosen by section 5 of the specification: a root array header, a keyless keyed
/// header, the literal `[]`, a single primitive, or an object. An empty document decodes to `{}`.
///
/// # Numbers
///
/// Numeric tokens decode exactly to `i64` or `u64` when they are integers that fit, and to the
/// nearest `f64` otherwise (integer-valued results that fit `i64`/`u64` become integers). A token
/// that overflows `f64` decodes as a string. See [`crate::to_string`] for the writing side.
///
/// # Limits
///
/// Containers may be nested at most 256 deep (root included); deeper input is an error.
/// A tab counts as one full indentation level in non-strict mode, and `options.indent` is clamped
/// to `1..=1024`.
///
/// # Errors
///
/// Returns a [`DecodeError`] for every violation listed in section 14 of the specification when
/// `options.strict` is `true` (count and width mismatches, indentation errors, blank lines inside
/// arrays, duplicate keys, malformed headers, trailing content after a root array, ...), and in
/// every mode for syntax errors that cannot be interpreted (invalid escapes, unterminated strings,
/// a missing colon, a scalar line outside the root, characters after a closing quote, nesting that
/// is too deep).
///
/// # Examples
///
/// ```
/// use pn_ultramemory_toon::{decode, DecodeOptions};
/// use serde_json::json;
///
/// let text = "users[2]{id,name}:\n  1,Ada\n  2,Bob";
/// let value = decode(text, &DecodeOptions::default()).unwrap();
/// assert_eq!(value, json!({"users": [{"id": 1, "name": "Ada"}, {"id": 2, "name": "Bob"}]}));
///
/// // Strict mode rejects a wrong declared length; lenient mode reads what is there.
/// let short = "tags[3]: a,b";
/// assert!(decode(short, &DecodeOptions::default()).is_err());
/// let lenient = DecodeOptions { strict: false, ..DecodeOptions::default() };
/// assert_eq!(decode(short, &lenient).unwrap(), json!({"tags": ["a", "b"]}));
/// ```
pub fn decode(input: &str, options: &DecodeOptions) -> Result<Value, DecodeError> {
    let lines = split_lines(input, effective_indent(options.indent), options.strict)?;
    Parser {
        lines,
        pos: 0,
        strict: options.strict,
        open_spans: 0,
        depth: 0,
    }
    .document()
}

/// Returns `true` for a list-item line: the bare marker `-` or a line starting with `- `.
fn is_list_item(content: &str) -> bool {
    content == "-" || content.starts_with("- ")
}

/// Splits an entry row at its first unquoted colon into the decoded entry key and the text after
/// the colon. `Ok(None)` means the line has no unquoted colon.
fn split_entry(content: &str) -> Result<Option<(String, &str)>, String> {
    if content.starts_with('"') {
        let (key, end) = parse_quoted(content)?;
        let after = content[end..].trim_start_matches(' ');
        return match after.strip_prefix(':') {
            Some(rest) => Ok(Some((key, rest))),
            None if after.is_empty() => Ok(None),
            None => Err("unexpected characters after the closing quote".to_string()),
        };
    }
    Ok(content.find(':').map(|colon| {
        (
            content[..colon].trim_end_matches(' ').to_string(),
            &content[colon + 1..],
        )
    }))
}

/// Walks a header's field list in depth-first order, taking one cell per leaf. Fields with no
/// cell left are absent from the object, and so are groups that received no cell at all.
fn fill_fields(
    fields: &[Field],
    cells: &[&str],
    next: &mut usize,
    map: &mut Map<String, Value>,
    line: usize,
) -> Res<()> {
    for field in fields {
        let Some(cell) = cells.get(*next) else {
            break;
        };
        match field {
            Field::Leaf(name) => {
                let value = parse_primitive(cell).map_err(|m| DecodeError::new(line, m))?;
                *next += 1;
                map.insert(name.clone(), value);
            }
            Field::Group(name, sub) => {
                let mut inner = Map::new();
                fill_fields(sub, cells, next, &mut inner, line)?;
                map.insert(name.clone(), Value::Object(inner));
            }
        }
    }
    Ok(())
}

/// The recursive-descent parser over the content lines.
struct Parser<'a> {
    /// Content lines, comments and blank lines already removed.
    lines: Vec<Line<'a>>,
    /// Index of the next unread line.
    pos: usize,
    /// Whether strict-mode checks are enforced.
    strict: bool,
    /// Number of enclosing header scopes that have already read at least one content line.
    open_spans: usize,
    /// Current container nesting depth.
    depth: usize,
}

impl<'a> Parser<'a> {
    /// Returns the next unread line without consuming it.
    fn peek(&self) -> Option<Line<'a>> {
        self.lines.get(self.pos).copied()
    }

    /// Consumes the next line, applying the strict blank-line rule of section 12.
    fn take(&mut self) -> Res<Line<'a>> {
        let Some(&line) = self.lines.get(self.pos) else {
            let last = self.lines.last().map_or(1, |l| l.number);
            return Err(DecodeError::new(last, "unexpected end of input"));
        };
        self.pos += 1;
        if self.strict && self.open_spans > 0 {
            if let Some(blank) = line.blank_before {
                return Err(DecodeError::new(blank, "blank line inside an array"));
            }
        }
        Ok(line)
    }

    /// Marks a header scope as started after it read its first content line.
    fn start_span(&mut self, started: &mut bool) {
        if !*started {
            *started = true;
            self.open_spans += 1;
        }
    }

    /// Fails if `levels` more levels of container nesting would exceed [`MAX_DEPTH`].
    fn check_depth(&self, levels: usize, line: usize) -> Res<()> {
        if self.depth + levels > MAX_DEPTH {
            return Err(DecodeError::new(
                line,
                format!("nesting is deeper than the limit of {MAX_DEPTH} levels"),
            ));
        }
        Ok(())
    }

    /// Reserves `levels` of container nesting, failing beyond [`MAX_DEPTH`].
    fn enter(&mut self, levels: usize, line: usize) -> Res<()> {
        self.check_depth(levels, line)?;
        self.depth += levels;
        Ok(())
    }

    /// Releases `levels` of container nesting.
    fn leave(&mut self, levels: usize) {
        self.depth -= levels;
    }

    /// Handles a line that is deeper than the scope it appears in and belongs to no scope
    /// (section 8): an error in strict mode; in lenient mode it is skipped unless it is a scalar
    /// line, which is an error in every mode.
    fn orphan(&mut self, line: Line<'a>) -> Res<()> {
        if self.strict {
            return Err(DecodeError::new(
                line.number,
                "line is indented deeper than its enclosing scope",
            ));
        }
        if matches!(classify(line.content, line.number, false)?, Kind::Scalar) {
            return Err(missing_colon(line.number));
        }
        self.pos += 1;
        Ok(())
    }

    /// Returns the depth at which the content of a scope opened at `field_depth` sits, or `None`
    /// when the next line is not deeper. Strict mode requires exactly one level deeper.
    fn child_depth(&self, field_depth: usize) -> Res<Option<usize>> {
        let Some(line) = self.peek() else {
            return Ok(None);
        };
        if line.depth <= field_depth {
            return Ok(None);
        }
        if self.strict {
            if line.depth > field_depth + 1 {
                return Err(DecodeError::new(
                    line.number,
                    "indentation jumps more than one level",
                ));
            }
            return Ok(Some(field_depth + 1));
        }
        Ok(Some(line.depth))
    }

    /// Inserts a key, applying the duplicate-key rule of section 14.3.
    fn insert(
        &self,
        map: &mut Map<String, Value>,
        key: String,
        value: Value,
        line: usize,
    ) -> Res<()> {
        match map.entry(key) {
            Entry::Vacant(slot) => {
                slot.insert(value);
            }
            Entry::Occupied(mut slot) => {
                if self.strict {
                    let message = format!("duplicate key {:?}", slot.key());
                    return Err(DecodeError::new(line, message));
                }
                slot.insert(value);
            }
        }
        Ok(())
    }

    /// Decodes the whole document (section 5, root form discovery).
    fn document(&mut self) -> Res<Value> {
        let Some(first) = self.peek() else {
            return Ok(Value::Object(Map::new()));
        };
        if first.depth == 0 {
            if first.content == "[]" {
                self.pos += 1;
                self.finish_root()?;
                return Ok(Value::Array(Vec::new()));
            }
            match classify(first.content, first.number, self.strict)? {
                Kind::Header(header) if header.key.is_none() => {
                    self.pos += 1;
                    let value = self.header_value(&header, 0)?;
                    self.finish_root()?;
                    return Ok(value);
                }
                Kind::Scalar if self.lines.len() == 1 => {
                    return parse_primitive(first.content)
                        .map_err(|m| DecodeError::new(first.number, m));
                }
                _ => {}
            }
        }
        Ok(Value::Object(self.object_fields(0, None)?))
    }

    /// Rejects content after a completed root array or keyed root object (section 5).
    fn finish_root(&self) -> Res<()> {
        match self.peek() {
            Some(line) if self.strict => Err(DecodeError::new(
                line.number,
                "unexpected content after the root array or keyed object",
            )),
            _ => Ok(()),
        }
    }

    /// Reads the fields of an object whose fields stand at `depth`. `carried` is a first field
    /// already read from a list-item hyphen line (section 10).
    fn object_fields(
        &mut self,
        depth: usize,
        carried: Option<(Kind<'a>, usize)>,
    ) -> Res<Map<String, Value>> {
        let here = self.peek().map_or(1, |l| l.number);
        self.enter(1, here)?;
        let mut map = Map::new();
        if let Some((kind, line)) = carried {
            self.field(kind, line, depth, &mut map)?;
        }
        while let Some(line) = self.peek() {
            if line.depth < depth {
                break;
            }
            if line.depth > depth {
                self.orphan(line)?;
                continue;
            }
            let line = self.take()?;
            let kind = classify(line.content, line.number, self.strict)?;
            self.field(kind, line.number, depth, &mut map)?;
        }
        self.leave(1);
        Ok(map)
    }

    /// Reads one field (a key-value line or a header) that stands at `field_depth`.
    fn field(
        &mut self,
        kind: Kind<'a>,
        line: usize,
        field_depth: usize,
        map: &mut Map<String, Value>,
    ) -> Res<()> {
        match kind {
            Kind::Scalar => Err(missing_colon(line)),
            Kind::KeyValue { key, rest } => {
                let value = self.plain_value(rest.trim_matches(' '), line, field_depth)?;
                self.insert(map, key, value, line)
            }
            Kind::Header(header) => self.header_field(*header, line, field_depth, map),
        }
    }

    /// Reads the value of a `key: rest` line: a nested object, `[]`, or a primitive.
    fn plain_value(&mut self, rest: &str, line: usize, field_depth: usize) -> Res<Value> {
        if rest.is_empty() {
            self.nested_object(field_depth, line)
        } else if rest == "[]" {
            self.check_depth(1, line)?;
            Ok(Value::Array(Vec::new()))
        } else {
            parse_primitive(rest).map_err(|m| DecodeError::new(line, m))
        }
    }

    /// Reads a field introduced by a header, or handles a header without a key.
    fn header_field(
        &mut self,
        mut header: Header<'a>,
        line: usize,
        field_depth: usize,
        map: &mut Map<String, Value>,
    ) -> Res<()> {
        match header.key.take() {
            Some(key) => {
                let value = self.header_value(&header, field_depth)?;
                self.insert(map, key, value, line)
            }
            None if self.strict => Err(DecodeError::new(
                line,
                "a header without a key is only valid at the document root or as a list item",
            )),
            None => {
                let kind = key_value_fallthrough(header.raw, line)?;
                self.field(kind, line, field_depth, map)
            }
        }
    }

    /// Reads the object opened by a bare `key:` line that stands at `field_depth`.
    fn nested_object(&mut self, field_depth: usize, line: usize) -> Res<Value> {
        match self.child_depth(field_depth)? {
            None => {
                self.check_depth(1, line)?;
                Ok(Value::Object(Map::new()))
            }
            Some(depth) => Ok(Value::Object(self.object_fields(depth, None)?)),
        }
    }

    /// Reads the value a header introduces, whose scope content sits below `field_depth`.
    fn header_value(&mut self, header: &Header<'a>, field_depth: usize) -> Res<Value> {
        if header.keyed {
            self.keyed_block(header, field_depth)
        } else if header.fields.is_some() {
            self.tabular_block(header, field_depth)
        } else if header.rest.is_empty() {
            Ok(Value::Array(self.list_block(
                header.len,
                header.line,
                field_depth,
            )?))
        } else {
            self.inline_array(header)
        }
    }

    /// Reads an inline primitive array (`key[N]: a,b,c`).
    #[inline(never)]
    fn inline_array(&self, header: &Header<'_>) -> Res<Value> {
        self.check_depth(1, header.line)?;
        let cells = split_cells(header.rest, header.delim.as_byte())
            .map_err(|m| DecodeError::new(header.line, m))?;
        if self.strict && cells.len() != header.len {
            return Err(count_error(header.line, "values", header.len, cells.len()));
        }
        let mut values = Vec::new();
        for cell in cells {
            values.push(parse_primitive(cell).map_err(|m| DecodeError::new(header.line, m))?);
        }
        Ok(Value::Array(values))
    }

    /// Reads the list items of an array in list form whose header stands at `field_depth`.
    fn list_block(
        &mut self,
        len: usize,
        header_line: usize,
        field_depth: usize,
    ) -> Res<Vec<Value>> {
        self.enter(1, header_line)?;
        let mut items = Vec::new();
        let mut started = false;
        if let Some(child) = self.child_depth(field_depth)? {
            while let Some(line) = self.peek() {
                if line.depth <= field_depth {
                    break;
                }
                if line.depth != child {
                    self.orphan(line)?;
                    continue;
                }
                if !is_list_item(line.content) {
                    break;
                }
                let line = self.take()?;
                self.start_span(&mut started);
                items.push(self.list_item(line, child)?);
            }
        }
        if started {
            self.open_spans -= 1;
        }
        self.leave(1);
        if self.strict && items.len() != len {
            return Err(count_error(header_line, "list items", len, items.len()));
        }
        Ok(items)
    }

    /// Reads one list item whose hyphen line stands at `item_depth` (section 9.4 and 10).
    fn list_item(&mut self, line: Line<'a>, item_depth: usize) -> Res<Value> {
        if line.content == "-" {
            self.check_depth(1, line.number)?;
            return Ok(Value::Object(Map::new()));
        }
        let rest = line.content[2..].trim_start_matches(' ');
        if rest == "[]" {
            self.check_depth(1, line.number)?;
            return Ok(Value::Array(Vec::new()));
        }
        let kind = match classify(rest, line.number, self.strict)? {
            Kind::Scalar => {
                return parse_primitive(rest).map_err(|m| DecodeError::new(line.number, m));
            }
            Kind::Header(header) if header.key.is_none() => {
                if header.fields.is_none() {
                    if header.rest.is_empty() {
                        let items = self.list_block(header.len, header.line, item_depth)?;
                        return Ok(Value::Array(items));
                    }
                    return self.inline_array(&header);
                }
                if self.strict {
                    return Err(DecodeError::new(
                        line.number,
                        "a header with a field list and no key is only valid at the document root",
                    ));
                }
                key_value_fallthrough(header.raw, line.number)?
            }
            other => other,
        };
        let fields = self.object_fields(item_depth + 1, Some((kind, line.number)))?;
        Ok(Value::Object(fields))
    }

    /// Reads the rows of a tabular array whose header stands at `field_depth` (section 9.3).
    #[inline(never)]
    fn tabular_block(&mut self, header: &Header<'a>, field_depth: usize) -> Res<Value> {
        self.enter(2 + header.group_depth, header.line)?;
        let fields = header.fields.as_deref().unwrap_or_default();
        let delimiter = header.delim.as_byte();
        let mut rows = Vec::new();
        let mut cells: Vec<&'a str> = Vec::new();
        let mut started = false;
        if let Some(child) = self.child_depth(field_depth)? {
            while let Some(line) = self.peek() {
                if line.depth <= field_depth {
                    break;
                }
                if line.depth != child {
                    self.orphan(line)?;
                    continue;
                }
                let is_row = is_row_line(line.content, delimiter)
                    .map_err(|m| DecodeError::new(line.number, m))?;
                if !is_row {
                    break;
                }
                let line = self.take()?;
                self.start_span(&mut started);
                split_cells_into(line.content, delimiter, &mut cells)
                    .map_err(|m| DecodeError::new(line.number, m))?;
                if self.strict && cells.len() != header.leaf_count {
                    return Err(width_error(line.number, header.leaf_count, cells.len()));
                }
                let mut map = Map::with_capacity(cells.len().min(header.leaf_count));
                fill_fields(fields, &cells, &mut 0, &mut map, line.number)?;
                rows.push(Value::Object(map));
            }
        }
        if started {
            self.open_spans -= 1;
        }
        self.leave(2 + header.group_depth);
        if self.strict && rows.len() != header.len {
            return Err(count_error(header.line, "rows", header.len, rows.len()));
        }
        Ok(Value::Array(rows))
    }

    /// Reads the entry rows of a keyed tabular object whose header stands at `field_depth`
    /// (section 9.5).
    #[inline(never)]
    fn keyed_block(&mut self, header: &Header<'a>, field_depth: usize) -> Res<Value> {
        self.enter(2 + header.group_depth, header.line)?;
        let fields = header.fields.as_deref().unwrap_or_default();
        let delimiter = header.delim.as_byte();
        let mut entries = Map::new();
        let mut cells: Vec<&'a str> = Vec::new();
        let mut count = 0_usize;
        let mut started = false;
        if let Some(child) = self.child_depth(field_depth)? {
            while let Some(line) = self.peek() {
                if line.depth <= field_depth {
                    break;
                }
                if line.depth != child {
                    self.orphan(line)?;
                    continue;
                }
                let split =
                    split_entry(line.content).map_err(|m| DecodeError::new(line.number, m))?;
                let Some((key, rest)) = split else {
                    if self.strict {
                        return Err(DecodeError::new(line.number, "entry row without a colon"));
                    }
                    self.pos += 1;
                    continue;
                };
                let line = self.take()?;
                self.start_span(&mut started);
                split_cells_into(rest.trim_matches(' '), delimiter, &mut cells)
                    .map_err(|m| DecodeError::new(line.number, m))?;
                if self.strict && cells.len() != header.leaf_count {
                    return Err(width_error(line.number, header.leaf_count, cells.len()));
                }
                let mut value = Map::with_capacity(cells.len().min(header.leaf_count));
                fill_fields(fields, &cells, &mut 0, &mut value, line.number)?;
                self.insert(&mut entries, key, Value::Object(value), line.number)?;
                count += 1;
            }
        }
        if started {
            self.open_spans -= 1;
        }
        self.leave(2 + header.group_depth);
        if self.strict && count != header.len {
            return Err(count_error(header.line, "entry rows", header.len, count));
        }
        Ok(Value::Object(entries))
    }
}

/// Builds the error for a scalar line where a `key: value` line is required.
fn missing_colon(line: usize) -> DecodeError {
    DecodeError::new(line, "expected `key: value`, found no colon")
}

/// Builds the error for a declared length that does not match what was read.
fn count_error(line: usize, what: &str, declared: usize, found: usize) -> DecodeError {
    DecodeError::new(line, format!("expected {declared} {what}, found {found}"))
}

/// Builds the error for a row whose cell count does not match the header.
fn width_error(line: usize, expected: usize, found: usize) -> DecodeError {
    DecodeError::new(
        line,
        format!("row has {found} cells, the header declares {expected}"),
    )
}

#[cfg(test)]
mod tests;
