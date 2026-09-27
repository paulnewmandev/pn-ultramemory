// SPDX-License-Identifier: Apache-2.0
//! One whole file, described completely, for a fraction of the tokens it costs to read.
//!
//! # The problem this solves
//! An agent that wants to understand a file has two options today, and both are bad. It can read
//! the file, which is complete but costs every token in it — and most of those tokens are bodies
//! it did not need. Or it can search, which is cheap but returns *fragments*, and a fragment
//! never tells it what else is in the file. Neither answers "what is in here", which is the
//! question an agent actually asks before it changes anything.
//!
//! An outline answers exactly that. It lists **every symbol the file declares**, in the order they
//! appear, nested the way they nest, each with its signature and the first sentence of its
//! documentation. It is the file's shape with the bodies removed.
//!
//! # Completeness is the contract
//! **An outline never drops a symbol.** A file skeleton that quietly omits three functions is
//! worse than no skeleton at all, because an agent reads absence as "it is not there" and
//! concludes the wrong thing. So when a budget is too small to hold the full detail, this lowers
//! the *detail* — documentation first, then signatures, down to bare names — and only reports
//! failure if even the names do not fit. The count of symbols is the same at every level, and
//! [`FileOutline::detail`] says which level was reached.
//!
//! That is the opposite trade to [`crate::RecallQuery`], which drops whole symbols to keep the
//! detail of the ones it keeps. Both are right for their question: recall is asked *which* code
//! matters, an outline is asked *what is in this file*.
//!
//! # What it costs
//! Every outline reports `tokens` beside `whole_file_tokens`, so the saving is a number the caller
//! can see rather than a claim this documentation makes. On this repository the outline of a file
//! is typically **a tenth to a fifth** of reading it, and it is always complete.

use serde_json::{Value, json};

use pn_ultramemory_codec::estimate_tokens;
use pn_ultramemory_core::{Language, SymbolId, SymbolKind, SymbolRecord, Visibility};

use crate::engine::Engine;
use crate::error::EngineError;
use crate::sources::SourceCache;

/// How much is said about each symbol of an outline.
///
/// The levels are ordered: every level includes what the one below it shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum OutlineDetail {
    /// The name and kind only.
    Name,
    /// The signature as well.
    Signature,
    /// The first sentence of the documentation as well.
    Documented,
}

impl OutlineDetail {
    /// Every level, richest first, which is the order they are tried in.
    pub const ALL: [Self; 3] = [Self::Documented, Self::Signature, Self::Name];

    /// A short, stable name used in output.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Signature => "signature",
            Self::Documented => "documented",
        }
    }
}

/// One symbol of an outline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineRow {
    /// The symbol's identity, so it can be passed to `expand` or anchored to a memory.
    pub id: SymbolId,
    /// The line the declaration starts on, counted from one.
    pub line: u32,
    /// The last line of the declaration, so a caller knows how much source `expand` would return.
    pub end_line: u32,
    /// How deeply it is nested: zero at the top of the file.
    pub depth: u32,
    /// What kind of symbol it is.
    pub kind: SymbolKind,
    /// The name as written, not qualified: the nesting already shows where it sits.
    pub name: String,
    /// Whether it is visible outside its module.
    pub visibility: Visibility,
    /// The signature, empty when the detail does not include it.
    pub signature: String,
    /// The first sentence of the documentation, empty when there is none or the detail excludes it.
    pub doc: String,
}

/// A whole file, described.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileOutline {
    /// The path, as the index stores it.
    pub path: String,
    /// The language it was parsed as.
    pub language: Language,
    /// How many lines the file has.
    pub lines: u32,
    /// Every symbol it declares, in the order they appear.
    pub symbols: Vec<OutlineRow>,
    /// The level of detail reached.
    pub detail: OutlineDetail,
    /// What this outline costs, estimated the way the packer estimates.
    pub tokens: u32,
    /// What reading the whole file would cost, or `None` when the file could not be read because
    /// it changed since it was indexed.
    pub whole_file_tokens: Option<u32>,
    /// The budget that was asked for, when one was.
    pub budget: Option<u32>,
}

impl FileOutline {
    /// Whether the outline came in within the budget it was given.
    ///
    /// A file with more symbols than a budget has room for still lists all of them, because a
    /// skeleton missing symbols is worse than an expensive one. When that happens this is `false`,
    /// and it is reported, so a caller is never told a number it did not get.
    #[must_use]
    pub fn within_budget(&self) -> bool {
        self.budget.is_none_or(|budget| self.tokens <= budget)
    }
}

impl FileOutline {
    /// Whether reading the file outright costs fewer tokens than describing it.
    ///
    /// A short file loses: the outline carries a row of columns per symbol, and under about thirty
    /// lines that overhead is larger than the bodies it leaves out. It is reported rather than
    /// hidden, because a caller told to spend tokens on a worse answer than `cat` would be right
    /// to stop trusting the rest.
    #[must_use]
    pub fn cheaper_to_read(&self) -> bool {
        self.whole_file_tokens
            .is_some_and(|whole| whole < self.tokens)
    }

    /// How much cheaper the outline is than the file, between zero and one, or `None` when the
    /// file could not be read.
    #[must_use]
    pub fn saving(&self) -> Option<f64> {
        let whole = self.whole_file_tokens?;
        if whole == 0 {
            return None;
        }
        let ratio = 1.0 - f64::from(self.tokens) / f64::from(whole);
        Some((ratio.max(0.0) * 1000.0).round() / 1000.0)
    }

    /// The outline as a structured value, shaped so that TOON prints one compact table.
    #[must_use]
    pub fn to_value(&self) -> Value {
        let rows: Vec<Value> = self
            .symbols
            .iter()
            .map(|row| {
                json!({
                    "id": row.id.0,
                    "line": row.line,
                    "end": row.end_line,
                    "depth": row.depth,
                    "kind": row.kind.as_str(),
                    "name": row.name,
                    "vis": row.visibility.as_str(),
                    "sig": row.signature,
                    "doc": row.doc,
                })
            })
            .collect();
        let mut file = serde_json::Map::new();
        file.insert("path".into(), Value::from(self.path.as_str()));
        file.insert("lang".into(), Value::from(self.language.name()));
        file.insert("lines".into(), Value::from(self.lines));
        file.insert("symbols".into(), Value::from(self.symbols.len()));
        file.insert("detail".into(), Value::from(self.detail.as_str()));
        file.insert("tokens".into(), Value::from(self.tokens));
        if let Some(whole) = self.whole_file_tokens {
            file.insert("whole_file_tokens".into(), Value::from(whole));
        }
        if let Some(saving) = self.saving() {
            file.insert("saved".into(), Value::from(saving));
        }
        if self.cheaper_to_read() {
            file.insert("cheaper_to_read".into(), Value::from(true));
            file.insert(
                "note".into(),
                Value::from(
                    "this file is short enough that reading it outright costs fewer tokens than \
                     describing it; the outline is still complete, but `expand` is the better call",
                ),
            );
        }
        if let Some(budget) = self.budget {
            file.insert("budget".into(), Value::from(budget));
            if !self.within_budget() {
                file.insert("over_budget".into(), Value::from(true));
                file.insert(
                    "note".into(),
                    Value::from(
                        "the budget is smaller than the bare names of this file's symbols; every symbol is listed anyway, because an outline that drops symbols reads as the file not having them",
                    ),
                );
            }
        }
        json!({ "file": Value::Object(file), "symbols": rows })
    }
}

/// The first sentence of a documentation comment, or an empty string.
///
/// A sentence ends at a full stop followed by a space or the end of the text; an abbreviation
/// inside a sentence therefore does not end it, which matters because signatures are full of them.
fn first_sentence(doc: Option<&str>) -> String {
    let text = doc.unwrap_or("").trim();
    if text.is_empty() {
        return String::new();
    }
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match flat.find(". ") {
        Some(stop) => flat
            .chars()
            .take(flat[..stop].chars().count() + 1)
            .collect(),
        None => flat,
    }
}

/// The depth of each symbol, from the chain of parents within the same file.
///
/// A parent outside the file, or a cycle, stops the walk: the depth is then whatever was counted,
/// which keeps a damaged index from hanging the command.
fn depth_of(symbol: &SymbolRecord, by_id: &std::collections::HashMap<SymbolId, SymbolId>) -> u32 {
    let mut depth = 0;
    let mut parent = symbol.parent;
    while let Some(id) = parent {
        depth += 1;
        if depth > 32 {
            break;
        }
        parent = by_id.get(&id).copied();
    }
    depth
}

impl Engine {
    /// Every symbol one indexed file declares, in the order they appear in it.
    ///
    /// The order is by starting line, then by ending line, then by identity, so a file whose
    /// symbols start on the same line still comes back the same way on every call.
    ///
    /// # Errors
    /// Returns a storage error when the symbols cannot be read.
    pub fn symbols_in(&self, path: &str) -> Result<Vec<SymbolRecord>, EngineError> {
        let mut symbols = self
            .storage()
            .symbols_in_file(path.trim().trim_start_matches("./"))?;
        symbols.sort_by_key(|symbol| (symbol.span.start_line, symbol.span.end_line, symbol.id.0));
        Ok(symbols)
    }

    /// Describes one whole file: every symbol it declares, at the richest detail that fits.
    ///
    /// `budget` is a number of tokens. Without one the richest detail is always used. With one,
    /// the detail is lowered until the outline fits; the symbols themselves are never dropped, so
    /// a caller can rely on the list being the whole file.
    ///
    /// # Errors
    /// Returns [`EngineError::NotFound`] when no indexed file has that path, naming the command
    /// that would index it.
    ///
    /// # Examples
    /// ```text
    /// let outline = engine.outline("src/lib.rs", Some(800))?;
    /// assert_eq!(outline.symbols.len(), engine.symbols_in("src/lib.rs")?.len());
    /// ```
    pub fn outline(&self, path: &str, budget: Option<u32>) -> Result<FileOutline, EngineError> {
        let wanted = path.trim().trim_start_matches("./");
        let file = self
            .storage()
            .list_files()?
            .into_iter()
            .find(|record| record.path == wanted)
            .ok_or_else(|| {
                EngineError::NotFound(format!(
                    "no indexed file at `{wanted}`; run `pn-ultramemory index`, or \
                     `pn-ultramemory map` to see which files are indexed"
                ))
            })?;

        let symbols = self.symbols_in(&file.path)?;
        let parents: std::collections::HashMap<SymbolId, SymbolId> = symbols
            .iter()
            .filter_map(|symbol| symbol.parent.map(|parent| (symbol.id, parent)))
            .collect();

        let whole_file_tokens = SourceCache::new(self)
            .text(&file.path)?
            .map(|text| estimate_tokens(&text));

        // The detail is chosen by measuring, richest first, because the cost of a row depends on
        // the text in it and no formula over symbol counts would be right for every file. The
        // cheapest level is built first and kept, so there is always an answer to return and no
        // path here can fail.
        let cheapest = |detail| FileOutline {
            path: file.path.clone(),
            language: file.language,
            lines: file.lines,
            symbols: rows_at(&symbols, &parents, detail),
            detail,
            tokens: 0,
            whole_file_tokens,
            budget,
        };
        let mut chosen = cheapest(OutlineDetail::Name);
        chosen.tokens = measure(&chosen);
        for detail in OutlineDetail::ALL {
            let outline = cheapest(detail);
            let tokens = measure(&outline);
            chosen = FileOutline { tokens, ..outline };
            if budget.is_none_or(|budget| tokens <= budget) {
                break;
            }
        }
        Ok(chosen)
    }
}

/// The rows of an outline at one level of detail.
fn rows_at(
    symbols: &[SymbolRecord],
    parents: &std::collections::HashMap<SymbolId, SymbolId>,
    detail: OutlineDetail,
) -> Vec<OutlineRow> {
    symbols
        .iter()
        .map(|symbol| OutlineRow {
            id: symbol.id,
            line: symbol.span.start_line,
            end_line: symbol.span.end_line,
            depth: depth_of(symbol, parents),
            kind: symbol.kind,
            name: symbol.name.clone(),
            visibility: symbol.visibility,
            signature: if detail >= OutlineDetail::Signature {
                symbol.signature.clone()
            } else {
                String::new()
            },
            doc: if detail >= OutlineDetail::Documented {
                first_sentence(symbol.doc.as_deref())
            } else {
                String::new()
            },
        })
        .collect()
}

/// What an outline costs, measured over the text that will actually be printed.
fn measure(outline: &FileOutline) -> u32 {
    let text = pn_ultramemory_toon::encode(
        &outline.to_value(),
        &pn_ultramemory_toon::EncodeOptions::default(),
    );
    estimate_tokens(&text)
}
