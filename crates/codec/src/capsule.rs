// SPDX-License-Identifier: Apache-2.0
//! The capsule: the code and memories that answer one query, packed to a token budget, and how
//! it is written out.
//!
//! A [`Capsule`] is a plain value. The engine builds it, and [`render`] turns it into one of three
//! formats (see `docs/formats.md`):
//!
//! * [`Format::Toon`]: TOON for the structure, with a file table so every path is written once,
//!   and raw source blocks after the document for symbols shown in full (a source listing does
//!   not fit a one-line table cell without escaping every newline, which costs tokens).
//! * [`Format::Json`]: one JSON object with the source inline, for programs.
//! * [`Format::Text`]: short human-readable lines.
//!
//! [`symbol_text`] and [`symbol_cost`] are the single definition of what a symbol looks like at each
//! [`Detail`] and what that costs, so the packer prices exactly what the renderer prints.

use core::fmt::Write as _;

use pn_ultramemory_core::{
    Confidence, Detail, EdgeKind, MemoryId, MemoryKind, Provenance, SymbolId, SymbolKind,
};
use pn_ultramemory_toon::{Delimiter, EncodeOptions};
use serde_json::{Map, Value, json};

use crate::estimate_tokens;

/// One symbol in a capsule, at the level of detail the packer chose for it.
#[derive(Debug, Clone, PartialEq)]
pub struct CapsuleSymbol {
    /// Identity of the symbol, usable with `expand`.
    pub id: SymbolId,
    /// Index of the symbol's file in [`Capsule::files`].
    pub file: usize,
    /// First line of the declaration, 1-based.
    pub start_line: u32,
    /// Last line of the declaration, 1-based.
    pub end_line: u32,
    /// What kind of element it is.
    pub kind: SymbolKind,
    /// The qualified name.
    pub name: String,
    /// The level of detail it is shown at.
    pub detail: Detail,
    /// The content for that level, as returned by [`symbol_text`]: empty for [`Detail::Name`], one
    /// line for levels 1 to 3, and the raw source (possibly many lines) for [`Detail::Source`].
    pub text: String,
    /// Why the symbol was chosen, when the caller asked for an explanation.
    pub why: Option<String>,
}

/// One memory in a capsule.
#[derive(Debug, Clone, PartialEq)]
pub struct CapsuleMemory {
    /// Identity of the memory.
    pub id: MemoryId,
    /// What kind of memory it is.
    pub kind: MemoryKind,
    /// Who produced it.
    pub provenance: Provenance,
    /// Whether the code it is about changed since it was written.
    pub stale: bool,
    /// The text of the memory.
    pub text: String,
}

/// A relationship between two symbols of the capsule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapsuleRelation {
    /// The qualified name of the source symbol.
    pub from: String,
    /// The qualified name of the target symbol.
    pub to: String,
    /// What the relationship means.
    pub kind: EdgeKind,
    /// How sure the indexer is about it.
    pub confidence: Confidence,
}

/// The context handed to an agent for one question.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Capsule {
    /// The query that produced it.
    pub query: String,
    /// The token budget it was packed to.
    pub budget: u32,
    /// Tokens the capsule uses, as counted by the caller.
    pub used: u32,
    /// Candidates that did not fit.
    pub omitted: u32,
    /// The files mentioned, referred to by index from [`CapsuleSymbol::file`].
    pub files: Vec<String>,
    /// The symbols, most relevant first.
    pub symbols: Vec<CapsuleSymbol>,
    /// The memories anchored to the symbols shown, or matching the query.
    pub memories: Vec<CapsuleMemory>,
    /// Relationships between the symbols shown.
    pub relations: Vec<CapsuleRelation>,
    /// Short remarks for the reader, such as "3 memories are stale".
    pub notes: Vec<String>,
}

/// The output format of a rendered capsule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Format {
    /// TOON plus raw source blocks. The default, because it costs the fewest tokens.
    #[default]
    Toon,
    /// One JSON document, with the source inline.
    Json,
    /// Short human-readable lines.
    Text,
}

impl Format {
    /// Looks a format up by its command-line name.
    ///
    /// # Examples
    /// ```
    /// use pn_ultramemory_codec::Format;
    ///
    /// assert_eq!(Format::from_name("toon"), Some(Format::Toon));
    /// assert_eq!(Format::from_name("yaml"), None);
    /// ```
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "toon" => Some(Self::Toon),
            "json" => Some(Self::Json),
            "text" => Some(Self::Text),
            _ => None,
        }
    }
}

/// How to render a capsule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RenderOptions {
    /// The output format.
    pub format: Format,
    /// The delimiter of TOON tables. Ignored by the other formats.
    pub delimiter: Delimiter,
}

/// The fields of a stored symbol that decide how it is shown at each level of detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SymbolView<'a> {
    /// The qualified name.
    pub qualified_name: &'a str,
    /// The declaration on one line.
    pub signature: &'a str,
    /// The first sentence of the documentation, or empty.
    pub summary: &'a str,
    /// Distinct names called inside the symbol.
    pub outline: &'a [String],
    /// The full source text of the declaration, or empty if it is not loaded.
    pub source: &'a str,
}

/// The tokens a table row costs beyond its text: numbers, separators and the line break.
const ROW_OVERHEAD_TOKENS: u32 = 9;

/// The extra tokens a source block costs for its header line.
const SOURCE_HEADER_TOKENS: u32 = 8;

/// The content shown for a symbol at a level of detail.
///
/// Level 0 is empty (the name is shown elsewhere), level 1 is the signature, level 2 adds the
/// first documentation sentence when there is one, level 3 adds the names the symbol calls when
/// there are any, and level 4 is the raw source. A level with nothing new to add repeats the
/// level below it, so every option is well defined.
///
/// # Examples
/// ```
/// use pn_ultramemory_codec::{SymbolView, symbol_text};
/// use pn_ultramemory_core::Detail;
///
/// let calls = vec!["read".to_owned(), "parse".to_owned()];
/// let view = SymbolView {
///     qualified_name: "load",
///     signature: "fn load(path: &str) -> Config",
///     summary: "Loads a config file.",
///     outline: &calls,
///     source: "fn load(path: &str) -> Config { parse(read(path)) }",
/// };
/// assert_eq!(symbol_text(&view, Detail::Name), "");
/// assert_eq!(symbol_text(&view, Detail::Signature), "fn load(path: &str) -> Config");
/// assert!(symbol_text(&view, Detail::Summary).ends_with("- Loads a config file."));
/// assert!(symbol_text(&view, Detail::Outline).contains("calls: read, parse"));
/// ```
#[must_use]
pub fn symbol_text(view: &SymbolView<'_>, detail: Detail) -> String {
    let with_summary = || {
        if view.summary.is_empty() {
            view.signature.to_owned()
        } else {
            format!("{} - {}", view.signature, view.summary)
        }
    };
    let with_outline = || {
        if view.outline.is_empty() {
            with_summary()
        } else {
            format!("{} | calls: {}", with_summary(), view.outline.join(", "))
        }
    };
    match detail {
        Detail::Name => String::new(),
        Detail::Signature => view.signature.to_owned(),
        Detail::Summary => with_summary(),
        Detail::Outline => with_outline(),
        Detail::Source if view.source.is_empty() => with_outline(),
        Detail::Source => view.source.to_owned(),
    }
}

/// The estimated tokens a symbol costs in a capsule at a level of detail: its row (or its entry in
/// the name list for level 0) plus its text.
///
/// # Examples
/// ```
/// use pn_ultramemory_codec::{SymbolView, symbol_cost};
/// use pn_ultramemory_core::Detail;
///
/// let view = SymbolView {
///     qualified_name: "load",
///     signature: "fn load(path: &str) -> Config",
///     summary: "",
///     outline: &[],
///     source: "",
/// };
/// assert!(symbol_cost(&view, Detail::Name) < symbol_cost(&view, Detail::Signature));
/// ```
#[must_use]
pub fn symbol_cost(view: &SymbolView<'_>, detail: Detail) -> u32 {
    let name = estimate_tokens(view.qualified_name);
    match detail {
        Detail::Name => name + 1,
        Detail::Source => {
            let text = symbol_text(view, detail);
            ROW_OVERHEAD_TOKENS + name + SOURCE_HEADER_TOKENS + estimate_tokens(&text)
        }
        _ => ROW_OVERHEAD_TOKENS + name + estimate_tokens(&symbol_text(view, detail)),
    }
}

/// The range of lines of a symbol, as written in the `lines` column.
fn line_range(symbol: &CapsuleSymbol) -> String {
    if symbol.start_line == symbol.end_line {
        symbol.start_line.to_string()
    } else {
        format!("{}-{}", symbol.start_line, symbol.end_line)
    }
}

/// The path of a symbol's file, or an empty string if the index is out of range.
fn file_path<'a>(capsule: &'a Capsule, symbol: &CapsuleSymbol) -> &'a str {
    capsule.files.get(symbol.file).map_or("", String::as_str)
}

/// Whether any symbol carries an explanation.
fn has_why(capsule: &Capsule) -> bool {
    capsule.symbols.iter().any(|s| s.why.is_some())
}

/// The header object shared by the structured formats.
fn header(capsule: &Capsule) -> Value {
    let mut map = Map::new();
    map.insert("query".into(), Value::from(capsule.query.as_str()));
    map.insert("budget".into(), Value::from(capsule.budget));
    map.insert("used".into(), Value::from(capsule.used));
    if capsule.omitted > 0 {
        map.insert("omitted".into(), Value::from(capsule.omitted));
    }
    Value::Object(map)
}

/// The memories table shared by the structured formats.
fn memory_rows(capsule: &Capsule) -> Vec<Value> {
    capsule
        .memories
        .iter()
        .map(|m| {
            json!({
                "id": m.id.0,
                "kind": m.kind.as_str(),
                "by": m.provenance.as_str(),
                "stale": if m.stale { "yes" } else { "no" },
                "text": m.text,
            })
        })
        .collect()
}

/// The relations table shared by the structured formats.
fn relation_rows(capsule: &Capsule) -> Vec<Value> {
    capsule
        .relations
        .iter()
        .map(|r| {
            json!({
                "from": r.from,
                "to": r.to,
                "kind": r.kind.as_str(),
                "confidence": r.confidence.as_str(),
            })
        })
        .collect()
}

/// The files the table rows of a capsule point at, in the order rows first mention them, as
/// indices into [`Capsule::files`].
///
/// A symbol shown by name only goes to the `also` list, which carries no file, so its file is not
/// listed: a path that nothing on the page refers to is tokens spent on nothing.
#[must_use]
pub fn row_files(capsule: &Capsule) -> Vec<usize> {
    let mut listed: Vec<usize> = Vec::new();
    for symbol in capsule.symbols.iter().filter(|s| s.detail != Detail::Name) {
        if !listed.contains(&symbol.file) {
            listed.push(symbol.file);
        }
    }
    listed
}

/// Builds the TOON document and the raw source blocks of a capsule.
fn toon_parts(capsule: &Capsule) -> (Value, Vec<String>) {
    let mut root = Map::new();
    root.insert("capsule".into(), header(capsule));
    let listed = row_files(capsule);
    if !listed.is_empty() {
        let files: Vec<Value> = listed
            .iter()
            .enumerate()
            .map(|(index, &file)| {
                json!({ "f": index, "path": capsule.files.get(file).map_or("", String::as_str) })
            })
            .collect();
        root.insert("files".into(), Value::Array(files));
    }

    let show_why = has_why(capsule);
    let mut rows = Vec::new();
    let mut names_only: Vec<Value> = Vec::new();
    let mut blocks = Vec::new();
    for symbol in &capsule.symbols {
        if symbol.detail == Detail::Name {
            names_only.push(Value::from(symbol.name.as_str()));
            continue;
        }
        let text = if symbol.detail == Detail::Source {
            blocks.push(format!(
                "@{} {}:{} {}\n{}",
                blocks.len() + 1,
                file_path(capsule, symbol),
                line_range(symbol),
                symbol.name,
                symbol.text.trim_end_matches('\n'),
            ));
            format!("@{}", blocks.len())
        } else {
            symbol.text.clone()
        };
        let mut row = Map::new();
        row.insert("id".into(), Value::from(symbol.id.0));
        let printed = listed.iter().position(|&file| file == symbol.file);
        row.insert("f".into(), Value::from(printed.unwrap_or(0)));
        row.insert("lines".into(), Value::from(line_range(symbol)));
        row.insert("kind".into(), Value::from(symbol.kind.as_str()));
        row.insert("name".into(), Value::from(symbol.name.as_str()));
        row.insert("d".into(), Value::from(symbol.detail.label()));
        row.insert("text".into(), Value::from(text));
        if show_why {
            row.insert(
                "why".into(),
                Value::from(symbol.why.clone().unwrap_or_default()),
            );
        }
        rows.push(Value::Object(row));
    }
    if !rows.is_empty() {
        root.insert("symbols".into(), Value::Array(rows));
    }
    if !names_only.is_empty() {
        root.insert("also".into(), Value::Array(names_only));
    }
    if !capsule.memories.is_empty() {
        root.insert("memories".into(), Value::Array(memory_rows(capsule)));
    }
    if !capsule.relations.is_empty() {
        root.insert("calls".into(), Value::Array(relation_rows(capsule)));
    }
    if !capsule.notes.is_empty() {
        let notes = capsule
            .notes
            .iter()
            .map(|n| Value::from(n.as_str()))
            .collect();
        root.insert("notes".into(), Value::Array(notes));
    }
    (Value::Object(root), blocks)
}

/// Builds the JSON document of a capsule, with paths and source inline.
fn json_value(capsule: &Capsule) -> Value {
    let symbols: Vec<Value> = capsule
        .symbols
        .iter()
        .map(|s| {
            let mut map = Map::new();
            map.insert("id".into(), Value::from(s.id.0));
            map.insert("path".into(), Value::from(file_path(capsule, s)));
            map.insert("lines".into(), Value::from(line_range(s)));
            map.insert("kind".into(), Value::from(s.kind.as_str()));
            map.insert("name".into(), Value::from(s.name.as_str()));
            map.insert("detail".into(), Value::from(s.detail.label()));
            map.insert("text".into(), Value::from(s.text.as_str()));
            if let Some(why) = &s.why {
                map.insert("why".into(), Value::from(why.as_str()));
            }
            Value::Object(map)
        })
        .collect();
    let mut root = Map::new();
    root.insert("capsule".into(), header(capsule));
    root.insert("symbols".into(), Value::Array(symbols));
    root.insert("memories".into(), Value::Array(memory_rows(capsule)));
    root.insert("calls".into(), Value::Array(relation_rows(capsule)));
    root.insert(
        "notes".into(),
        Value::Array(
            capsule
                .notes
                .iter()
                .map(|n| Value::from(n.as_str()))
                .collect(),
        ),
    );
    Value::Object(root)
}

/// Writes a capsule as short human-readable lines.
fn render_text(capsule: &Capsule) -> String {
    // Writing to a `String` cannot fail, so the `fmt::Result` of every `writeln!` is ignored.
    let mut out = String::new();
    let _ = writeln!(
        out,
        "capsule \"{}\": {} of {} tokens, {} omitted",
        capsule.query, capsule.used, capsule.budget, capsule.omitted
    );
    for symbol in &capsule.symbols {
        let _ = writeln!(
            out,
            "\n{} {} {} ({}:{})",
            symbol.detail.label(),
            symbol.kind,
            symbol.name,
            file_path(capsule, symbol),
            line_range(symbol),
        );
        if let Some(why) = &symbol.why {
            let _ = writeln!(out, "  why: {why}");
        }
        for line in symbol.text.lines() {
            let _ = writeln!(out, "  {line}");
        }
    }
    if !capsule.memories.is_empty() {
        out.push_str("\nmemories\n");
        for memory in &capsule.memories {
            let stale = if memory.stale { " [stale]" } else { "" };
            let _ = writeln!(
                out,
                "  #{} {} ({}){}: {}",
                memory.id, memory.kind, memory.provenance, stale, memory.text
            );
        }
    }
    if !capsule.relations.is_empty() {
        out.push_str("\ncalls\n");
        for relation in &capsule.relations {
            let _ = writeln!(
                out,
                "  {} -{}-> {} ({})",
                relation.from,
                relation.kind.as_str(),
                relation.to,
                relation.confidence
            );
        }
    }
    for note in &capsule.notes {
        let _ = write!(out, "\nnote: {note}\n");
    }
    out
}

/// Renders a capsule in the requested format. The output never ends with a newline.
///
/// # Examples
/// ```
/// use pn_ultramemory_codec::{Capsule, RenderOptions, render};
///
/// let capsule = Capsule { query: "parse config".into(), budget: 1500, ..Capsule::default() };
/// let text = render(&capsule, &RenderOptions::default());
/// assert!(text.starts_with("capsule:\n  query: parse config"));
/// ```
#[must_use]
pub fn render(capsule: &Capsule, options: &RenderOptions) -> String {
    match options.format {
        Format::Toon => {
            let (document, blocks) = toon_parts(capsule);
            let encode = EncodeOptions {
                delimiter: options.delimiter,
                ..EncodeOptions::default()
            };
            let mut out = pn_ultramemory_toon::encode(&document, &encode);
            for block in blocks {
                out.push_str("\n\n");
                out.push_str(&block);
            }
            out
        }
        Format::Json => serde_json::to_string(&json_value(capsule)).unwrap_or_default(),
        Format::Text => render_text(capsule).trim_end().to_owned(),
    }
}

/// The estimated tokens of a capsule as it will actually be printed, for checking that it stays
/// within its budget.
#[must_use]
pub fn measure(capsule: &Capsule, options: &RenderOptions) -> u32 {
    estimate_tokens(&render(capsule, options))
}

#[cfg(test)]
mod tests {
    use super::{
        Capsule, CapsuleMemory, CapsuleRelation, CapsuleSymbol, Format, RenderOptions, SymbolView,
        measure, render, symbol_cost, symbol_text,
    };
    use pn_ultramemory_core::{
        Confidence, Detail, EdgeKind, MemoryId, MemoryKind, Provenance, SymbolId, SymbolKind,
    };
    use pn_ultramemory_toon::{DecodeOptions, Delimiter, decode};

    /// A symbol with the given level of detail and text.
    fn symbol(id: i64, name: &str, detail: Detail, text: &str) -> CapsuleSymbol {
        CapsuleSymbol {
            id: SymbolId(id),
            file: 0,
            start_line: 10,
            end_line: 20,
            kind: SymbolKind::Function,
            name: name.to_owned(),
            detail,
            text: text.to_owned(),
            why: None,
        }
    }

    /// A small capsule with one symbol at each of several levels.
    fn sample() -> Capsule {
        Capsule {
            query: "parse config".into(),
            budget: 1500,
            used: 400,
            omitted: 3,
            files: vec!["src/config.rs".into(), "src/main.rs".into()],
            symbols: vec![
                symbol(
                    1,
                    "parse_config",
                    Detail::Signature,
                    "pub fn parse_config(path: &str) -> Config",
                ),
                symbol(
                    2,
                    "Config::load",
                    Detail::Source,
                    "pub fn load() -> Config {\n    todo_free()\n}\n",
                ),
                symbol(3, "default_config", Detail::Name, ""),
            ],
            memories: vec![CapsuleMemory {
                id: MemoryId(7),
                kind: MemoryKind::Decision,
                provenance: Provenance::User,
                stale: true,
                text: "Use a file, not env vars".into(),
            }],
            relations: vec![CapsuleRelation {
                from: "parse_config".into(),
                to: "Config::load".into(),
                kind: EdgeKind::Calls,
                confidence: Confidence::Resolved,
            }],
            notes: vec!["1 memory is stale".into()],
        }
    }

    /// The TOON output is a valid TOON document followed by raw source blocks.
    #[test]
    fn toon_output_has_a_document_then_source_blocks() {
        let output = render(&sample(), &RenderOptions::default());
        let (document, blocks) = output.split_once("\n\n@1 ").expect("one source block");
        let value = decode(document, &DecodeOptions::default()).expect("valid TOON");
        assert_eq!(value["capsule"]["budget"], 1500);
        assert_eq!(value["files"][0]["path"], "src/config.rs");
        assert_eq!(value["files"].as_array().map(Vec::len), Some(1));
        assert_eq!(value["symbols"][0]["d"], "L1");
        assert_eq!(value["symbols"][1]["text"], "@1");
        assert_eq!(value["also"][0], "default_config");
        assert_eq!(value["memories"][0]["stale"], "yes");
        assert_eq!(value["calls"][0]["confidence"], "resolved");
        assert!(blocks.starts_with("src/config.rs:10-20 Config::load\npub fn load()"));
        assert!(!output.ends_with('\n'));
    }

    /// Names-only symbols go to a compact list, not to table rows.
    #[test]
    fn level_zero_symbols_are_a_name_list() {
        let output = render(&sample(), &RenderOptions::default());
        assert!(output.contains("also[1]: default_config"));
        assert!(!output.contains("default_config,"));
    }

    /// The explanation column exists only when a symbol has one.
    #[test]
    fn why_column_appears_only_when_requested() {
        let mut capsule = sample();
        assert!(!render(&capsule, &RenderOptions::default()).contains("why"));
        capsule.symbols[0].why = Some("name matches the query".into());
        let output = render(&capsule, &RenderOptions::default());
        assert!(output.contains("{id,f,lines,kind,name,d,text,why}"));
        assert!(output.contains("name matches the query"));
    }

    /// Hostile text (commas, quotes, colons, newlines, unicode) survives the TOON round trip.
    #[test]
    fn hostile_text_round_trips() {
        let nasty = "a, \"b\": c\\d [x] {y} - # \u{00e9}\u{1F389}";
        let mut capsule = sample();
        capsule.symbols[0].text = nasty.to_owned();
        capsule.memories[0].text = nasty.to_owned();
        for delimiter in [Delimiter::Comma, Delimiter::Tab, Delimiter::Pipe] {
            let options = RenderOptions {
                format: Format::Toon,
                delimiter,
            };
            let output = render(&capsule, &options);
            let document = output.split("\n\n@1 ").next().expect("document");
            let value = decode(document, &DecodeOptions::default()).expect("valid TOON");
            assert_eq!(value["symbols"][0]["text"], nasty);
            assert_eq!(value["memories"][0]["text"], nasty);
        }
    }

    /// The JSON output parses and carries paths and source inline.
    #[test]
    fn json_output_is_self_contained() {
        let options = RenderOptions {
            format: Format::Json,
            ..RenderOptions::default()
        };
        let value: serde_json::Value =
            serde_json::from_str(&render(&sample(), &options)).expect("json");
        assert_eq!(value["symbols"][1]["path"], "src/config.rs");
        assert!(
            value["symbols"][1]["text"]
                .as_str()
                .is_some_and(|t| t.contains("todo_free"))
        );
        assert_eq!(value["capsule"]["omitted"], 3);
    }

    /// The text output mentions each symbol, memory and relation on its own lines.
    #[test]
    fn text_output_is_readable() {
        let options = RenderOptions {
            format: Format::Text,
            ..RenderOptions::default()
        };
        let output = render(&sample(), &options);
        assert!(output.starts_with("capsule \"parse config\": 400 of 1500 tokens, 3 omitted"));
        assert!(output.contains("L1 function parse_config (src/config.rs:10-20)"));
        assert!(output.contains("#7 decision (user) [stale]: Use a file, not env vars"));
        assert!(output.contains("parse_config -calls-> Config::load (resolved)"));
    }

    /// An empty capsule renders in every format without panicking.
    #[test]
    fn empty_capsule_renders() {
        for format in [Format::Toon, Format::Json, Format::Text] {
            let options = RenderOptions {
                format,
                ..RenderOptions::default()
            };
            assert!(!render(&Capsule::default(), &options).is_empty());
        }
    }

    /// Each level adds content and cost, and levels with nothing new repeat the one below.
    #[test]
    fn levels_add_content_and_cost() {
        let calls = vec!["read".to_owned()];
        let view = SymbolView {
            qualified_name: "load",
            signature: "fn load()",
            summary: "Loads it.",
            outline: &calls,
            source: "fn load() { read() }",
        };
        let costs: Vec<u32> = Detail::ALL.iter().map(|d| symbol_cost(&view, *d)).collect();
        assert!(costs.windows(2).all(|pair| pair[0] <= pair[1]), "{costs:?}");
        let bare = SymbolView {
            summary: "",
            outline: &[],
            source: "",
            ..view
        };
        assert_eq!(
            symbol_text(&bare, Detail::Summary),
            symbol_text(&bare, Detail::Signature)
        );
        assert_eq!(
            symbol_text(&bare, Detail::Outline),
            symbol_text(&bare, Detail::Signature)
        );
    }

    /// Builds a capsule of `count` signature-level symbols, and the sum of what the packer would
    /// have priced them at.
    fn priced_capsule(count: usize) -> (Capsule, u32) {
        let signatures = [
            "pub fn parse_config(path: &str) -> Result<Config, Error>",
            "pub fn load_defaults() -> Config",
            "pub fn validate(&self, strict: bool) -> Result<(), ValidationError>",
            "pub fn merge(base: Config, overlay: Config) -> Config",
        ];
        let mut capsule = Capsule {
            query: "config".into(),
            budget: 1000,
            files: vec!["src/config.rs".into()],
            ..Capsule::default()
        };
        let mut priced = 0;
        for index in 0..count {
            let sig = signatures[index % signatures.len()];
            let name = format!("config::function_number_{index}");
            let view = SymbolView {
                qualified_name: &name,
                signature: sig,
                summary: "",
                outline: &[],
                source: "",
            };
            priced += symbol_cost(&view, Detail::Signature);
            let id = i64::try_from(index).unwrap_or(0);
            capsule
                .symbols
                .push(symbol(id, &name, Detail::Signature, sig));
        }
        (capsule, priced)
    }

    /// The priced cost of the symbols tracks what they add to the rendered capsule, once the fixed
    /// cost of the header, the file table and the table header is set aside. The fixed part is
    /// small and constant, so a capsule packed to a budget stays near it. It is measured on a
    /// capsule of one symbol, because the file table is only printed for a row that points at it.
    #[test]
    fn priced_cost_tracks_the_rendered_cost() {
        let options = RenderOptions::default();
        let (one, one_priced) = priced_capsule(1);
        let base = f64::from(measure(&one, &options)) - f64::from(one_priced);
        for count in [4, 16, 64] {
            let (capsule, priced) = priced_capsule(count);
            let added = f64::from(measure(&capsule, &options)) - base;
            let ratio = added / f64::from(priced);
            assert!(
                (0.75..1.5).contains(&ratio),
                "{count} symbols: priced {priced}, added {added}"
            );
        }
    }
}
