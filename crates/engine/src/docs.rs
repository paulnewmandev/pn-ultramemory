// SPDX-License-Identifier: Apache-2.0
//! Finding public symbols that carry no documentation, and writing documentation back into the
//! source of any language the indexer understands.
//!
//! # Role in the architecture
//! Application layer. The three operations form one loop that costs an agent very few tokens:
//!
//! 1. [`Engine::doc_gaps`] lists undocumented public symbols, optionally with a compact context
//!    (signature, callers, callees and the head of the source) so that a description can be written
//!    without reading the file.
//! 2. [`parse_doc_entries`] reads back what the agent wrote, as TOON or as JSON.
//! 3. [`Engine::doc_apply`] resolves each symbol, redacts the text, and hands it to the
//!    [`pn_ultramemory_core::DocInserter`] port, which writes the comment in the language's own
//!    syntax and refuses anything that would not parse.
//!
//! [`Engine::doc_markdown`] is the independent fourth operation: a Markdown API reference built
//! from the index, for any language, without reading a single source file.
//!
//! # Invariants
//! * **A file is written at most once.** Entries are grouped by file and the insertions of one file
//!   are applied from the bottom of the file upwards, so the line numbers recorded by the index stay
//!   valid while the file grows above them.
//! * **A stale file is refused whole.** If a file's content hash differs from the one stored at
//!   indexing time, none of its entries are applied: the recorded lines would point at the wrong
//!   declarations.
//! * **Nothing is fatal per entry.** A symbol that cannot be resolved, a text the inserter refuses
//!   and a file that changed are all reported in [`DocApplyReport::skipped`] with the reason; only a
//!   failure to write a file or to read the index fails the call.
//! * **Determinism.** Files are visited in path order, insertions within a file in source order,
//!   and every list in a report is built from those two orders alone.

mod apply;
mod entries;
mod markdown;

pub use apply::DocApplyReport;
pub use entries::{DocEntry, parse_doc_entries};

use pn_ultramemory_codec::estimate_tokens;
use pn_ultramemory_core::{Confidence, Direction, SymbolId, SymbolKind, SymbolRecord};
use serde_json::{Value, json};

use crate::engine::Engine;
use crate::error::EngineError;
use crate::recall::ranking::clip_chars;
use crate::sources::SourceCache;

/// How many gaps are listed when the caller asks for no particular number.
const DEFAULT_GAP_LIMIT: usize = 50;

/// The most gaps one call will list, however large a limit was asked for.
const MAX_GAP_LIMIT: usize = 2_000;

/// The most caller names a context names.
const MAX_CALLERS: usize = 4;

/// The most callee names a context names.
const MAX_CALLEES: usize = 6;

/// How many lines of the symbol's own source a context shows.
const CONTEXT_LINES: usize = 12;

/// How many characters of those lines a context shows.
const CONTEXT_CHARS: usize = 400;

/// The tokens a context is trimmed back to. The parts above aim at about 120 estimated tokens;
/// this is the ceiling that holds even for a signature and names that are all unusually long.
const MAX_CONTEXT_TOKENS: u32 = 140;

/// Which undocumented symbols to list.
///
/// # Examples
/// ```
/// use pn_ultramemory_engine::DocGapQuery;
///
/// let query = DocGapQuery::default();
/// assert_eq!(query.limit, 50);
/// assert!(!query.with_context);
/// assert_eq!(query.path_prefix, None);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocGapQuery {
    /// Only symbols whose path starts with this prefix, if given.
    pub path_prefix: Option<String>,
    /// How many symbols to list. Zero means the default of 50.
    pub limit: usize,
    /// Whether every gap carries the compact context described on [`Engine::doc_gaps`].
    pub with_context: bool,
}

impl Default for DocGapQuery {
    fn default() -> Self {
        Self {
            path_prefix: None,
            limit: DEFAULT_GAP_LIMIT,
            with_context: false,
        }
    }
}

/// One public symbol that has no documentation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocGap {
    /// Identity of the symbol.
    pub id: SymbolId,
    /// The name qualified by its enclosing symbols, in the form [`Engine::doc_apply`] accepts.
    pub name: String,
    /// What kind of element it is.
    pub kind: SymbolKind,
    /// Path of the file that declares it, relative to the repository root.
    pub path: String,
    /// The 1-based line the declaration starts on.
    pub line: u32,
    /// The declaration on one line.
    pub signature: String,
    /// Everything needed to write the documentation without opening the file, when it was asked
    /// for.
    pub context: Option<String>,
}

/// The gaps as a uniform table `gaps[n]{id,kind,name,path,line,signature,context}`.
///
/// Every row carries every column, with an empty `context` when none was built, so that TOON can
/// declare the shape once instead of repeating the field names.
///
/// # Examples
/// ```
/// use pn_ultramemory_core::{SymbolId, SymbolKind};
/// use pn_ultramemory_engine::{DocGap, doc_gaps_to_value};
///
/// let gap = DocGap {
///     id: SymbolId(7),
///     name: "Config::validate".into(),
///     kind: SymbolKind::Method,
///     path: "src/config.rs".into(),
///     line: 12,
///     signature: "pub fn validate(&self) -> bool".into(),
///     context: None,
/// };
/// let value = doc_gaps_to_value(&[gap]);
/// assert_eq!(value["gaps"][0]["kind"], "method");
/// assert_eq!(value["gaps"][0]["context"], "");
/// ```
#[must_use]
pub fn doc_gaps_to_value(gaps: &[DocGap]) -> Value {
    json!({ "gaps": gap_rows(gaps) })
}

/// The rows of the gap table, so that a report can embed them under its own key.
pub(crate) fn gap_rows(gaps: &[DocGap]) -> Vec<Value> {
    gaps.iter()
        .map(|gap| {
            json!({
                "id": gap.id.0,
                "kind": gap.kind.as_str(),
                "name": gap.name,
                "path": gap.path,
                "line": gap.line,
                "signature": gap.signature,
                "context": gap.context.clone().unwrap_or_default(),
            })
        })
        .collect()
}

/// The first [`CONTEXT_LINES`] lines of a source text, cut to [`CONTEXT_CHARS`] characters.
fn source_head(source: &str) -> String {
    let mut head = String::new();
    for line in source.lines().take(CONTEXT_LINES) {
        if !head.is_empty() {
            head.push('\n');
        }
        head.push_str(line.trim_end());
    }
    clip_chars(&head, CONTEXT_CHARS)
}

/// Appends a labelled list of names, doing nothing when the list is empty.
fn push_names(out: &mut String, label: &str, names: &[String]) {
    if names.is_empty() {
        return;
    }
    out.push_str(label);
    for (index, name) in names.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        out.push_str(name);
    }
    out.push('\n');
}

/// Trims a context back to [`MAX_CONTEXT_TOKENS`] by dropping characters from its end.
///
/// Each round keeps nine tenths of the characters, so the loop ends after a handful of rounds
/// however long the text was.
fn trim_to_budget(text: String) -> String {
    let mut text = text;
    while estimate_tokens(&text) > MAX_CONTEXT_TOKENS {
        let chars = text.chars().count();
        if chars <= 40 {
            break;
        }
        text = clip_chars(&text, chars.saturating_mul(9) / 10);
    }
    text
}

impl Engine {
    /// The compact context of one gap: the signature, who calls it, what it calls, and the head of
    /// its own source.
    ///
    /// # Errors
    /// Returns a storage error when the callers or the stored file hash cannot be read.
    fn gap_context(
        &self,
        cache: &mut SourceCache<'_>,
        record: &SymbolRecord,
    ) -> Result<String, EngineError> {
        let mut callers: Vec<String> = Vec::new();
        for neighbor in self.storage().neighbors(
            record.id,
            Direction::In,
            Confidence::Heuristic,
            MAX_CALLERS,
        )? {
            let name = neighbor.symbol.name;
            if !callers.contains(&name) {
                callers.push(name);
            }
        }
        let outgoing: Vec<String> = record.outline.iter().take(MAX_CALLEES).cloned().collect();

        let mut out = String::new();
        out.push_str(&record.signature);
        out.push('\n');
        push_names(&mut out, "callers: ", &callers);
        push_names(&mut out, "calls: ", &outgoing);
        if let Some(source) = cache.slice(record)? {
            out.push_str(&source_head(&source));
        }
        Ok(trim_to_budget(out))
    }

    /// Lists public symbols that have no documentation, nearest the top of the repository first.
    ///
    /// Modules are never listed: a module's documentation is a different job from a declaration's.
    /// With `with_context` every gap also carries one compact text holding the signature, up to
    /// four caller names, up to six callee names and the head of the symbol's own source, sized so
    /// that a batch of gaps stays affordable (about 120 estimated tokens each).
    ///
    /// # Errors
    /// Returns a storage error when the index cannot be read.
    pub fn doc_gaps(&self, query: &DocGapQuery) -> Result<Vec<DocGap>, EngineError> {
        let limit = if query.limit == 0 {
            DEFAULT_GAP_LIMIT
        } else {
            query.limit.min(MAX_GAP_LIMIT)
        };
        let prefix = query
            .path_prefix
            .as_deref()
            .map(|prefix| prefix.trim().trim_start_matches("./"))
            .filter(|prefix| !prefix.is_empty());
        let found = self.storage().undocumented_public(limit, prefix)?;
        let mut cache = SourceCache::new(self);
        let mut gaps = Vec::with_capacity(found.len());
        for record in found {
            let context = if query.with_context {
                Some(self.gap_context(&mut cache, &record)?)
            } else {
                None
            };
            gaps.push(DocGap {
                id: record.id,
                name: record.qualified_name,
                kind: record.kind,
                path: record.path,
                line: record.span.start_line,
                signature: record.signature,
                context,
            });
        }
        Ok(gaps)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CONTEXT_CHARS, DocGap, DocGapQuery, MAX_CONTEXT_TOKENS, doc_gaps_to_value, push_names,
        source_head, trim_to_budget,
    };
    use pn_ultramemory_codec::estimate_tokens;
    use pn_ultramemory_core::{SymbolId, SymbolKind};

    /// The default query lists fifty gaps without context over the whole repository.
    #[test]
    fn default_query() {
        let query = DocGapQuery::default();
        assert_eq!(query.limit, 50);
        assert!(!query.with_context);
        assert!(query.path_prefix.is_none());
    }

    /// The source head keeps at most twelve lines and at most the character limit.
    #[test]
    fn source_head_is_bounded() {
        let mut source = String::new();
        for number in 0..40 {
            source.push_str("line ");
            source.push_str(&number.to_string());
            source.push_str("   \n");
        }
        let head = source_head(&source);
        assert!(head.lines().count() <= 12, "{head}");
        assert!(head.chars().count() <= CONTEXT_CHARS);
        assert!(head.starts_with("line 0\n"), "{head}");
        let long = "x".repeat(2000);
        assert_eq!(source_head(&long).chars().count(), CONTEXT_CHARS);
    }

    /// A list of names is labelled and comma-separated, and an empty list writes nothing.
    #[test]
    fn names_are_joined() {
        let mut out = String::new();
        push_names(&mut out, "calls: ", &[]);
        assert!(out.is_empty());
        push_names(&mut out, "calls: ", &["a".to_owned(), "b".to_owned()]);
        assert_eq!(out, "calls: a, b\n");
    }

    /// A context longer than the ceiling is cut back to it, and a short one is left alone.
    #[test]
    fn contexts_are_trimmed_to_the_budget() {
        let short = "fn f()\n".to_owned();
        assert_eq!(trim_to_budget(short.clone()), short);
        let long = "fn f(a: u32) -> u32 { a + 1 }\n".repeat(80);
        let trimmed = trim_to_budget(long);
        assert!(estimate_tokens(&trimmed) <= MAX_CONTEXT_TOKENS);
        assert!(trimmed.starts_with("fn f(a: u32)"));
    }

    /// Every row of the table carries every column, with an empty context when there is none.
    #[test]
    fn table_rows_are_uniform() {
        let gaps = vec![
            DocGap {
                id: SymbolId(1),
                name: "a".to_owned(),
                kind: SymbolKind::Function,
                path: "a.rs".to_owned(),
                line: 1,
                signature: "fn a()".to_owned(),
                context: Some("fn a()\n".to_owned()),
            },
            DocGap {
                id: SymbolId(2),
                name: "B".to_owned(),
                kind: SymbolKind::Struct,
                path: "b.rs".to_owned(),
                line: 4,
                signature: "struct B".to_owned(),
                context: None,
            },
        ];
        let value = doc_gaps_to_value(&gaps);
        let rows = value["gaps"].as_array().map(Vec::len);
        assert_eq!(rows, Some(2));
        for row in value["gaps"].as_array().into_iter().flatten() {
            let object = row.as_object().map(serde_json::Map::len);
            assert_eq!(object, Some(7));
        }
        assert_eq!(value["gaps"][1]["context"], "");
        assert_eq!(value["gaps"][0]["id"], 1);
    }
}
