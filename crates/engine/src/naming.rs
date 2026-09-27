// SPDX-License-Identifier: Apache-2.0
//! Turning what a person or an agent typed into a symbol.
//!
//! Callers name symbols in three ways: by numeric id (`1042`, `#1042`), by name (`parse_config`,
//! `Config::load`), or by file and name (`src/config.rs:parse_config`). Resolution never guesses
//! between equally good matches: it either returns exactly one symbol or explains what to type
//! instead.

use pn_ultramemory_core::{SearchQuery, SymbolId, SymbolRecord};

use crate::engine::Engine;
use crate::error::EngineError;

/// The most alternatives listed in an error message.
const MAX_CANDIDATES: usize = 8;

/// Formats a symbol as `id path qualified_name`, the way ambiguity errors list candidates.
fn describe(symbol: &SymbolRecord) -> String {
    format!("{} {} {}", symbol.id, symbol.path, symbol.qualified_name)
}

impl Engine {
    /// Finds the one symbol a reference names.
    ///
    /// # Errors
    /// Returns [`EngineError::NotFound`] (with suggestions when the text search finds close
    /// matches), [`EngineError::Ambiguous`] when several symbols match and none is an exact
    /// qualified-name match, and [`EngineError::Invalid`] for an empty reference.
    pub fn resolve_symbol(&self, reference: &str) -> Result<SymbolRecord, EngineError> {
        let reference = reference.trim();
        if reference.is_empty() {
            return Err(EngineError::Invalid("the symbol name is empty".into()));
        }
        let digits = reference.strip_prefix('#').unwrap_or(reference);
        if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
            let id: i64 = digits
                .parse()
                .map_err(|_| EngineError::Invalid(format!("`{reference}` is not a valid id")))?;
            return self
                .storage()
                .symbol(SymbolId(id))?
                .ok_or_else(|| EngineError::NotFound(format!("no symbol with id {id}")));
        }

        // `path:name` narrows the search to files whose path ends with the given part.
        let (path_part, name) = match reference.rsplit_once(':') {
            Some((left, right))
                if !left.is_empty()
                    && !right.is_empty()
                    && !left.ends_with(':')
                    && (left.contains('/') || left.contains('.')) =>
            {
                (Some(left), right)
            }
            _ => (None, reference),
        };

        let mut found = self.storage().find_symbols(name, 50)?;
        if let Some(path_part) = path_part {
            found.retain(|symbol| symbol.path.ends_with(path_part));
        }
        match found.len() {
            0 => Err(EngineError::NotFound(self.suggest(reference)?)),
            1 => Ok(found.remove(0)),
            _ => {
                let exact: Vec<usize> = found
                    .iter()
                    .enumerate()
                    .filter(|(_, symbol)| symbol.qualified_name == name)
                    .map(|(index, _)| index)
                    .collect();
                if let [only] = exact.as_slice() {
                    return Ok(found.swap_remove(*only));
                }
                Err(EngineError::Ambiguous {
                    query: reference.to_owned(),
                    candidates: found.iter().take(MAX_CANDIDATES).map(describe).collect(),
                })
            }
        }
    }

    /// Builds the "not found" message, with the closest text matches as suggestions.
    fn suggest(&self, reference: &str) -> Result<String, EngineError> {
        let query = SearchQuery {
            text: reference.to_owned(),
            ..SearchQuery::default()
        };
        let hits = self.storage().search_symbols(&query, 5)?;
        if hits.is_empty() {
            return Ok(format!("no symbol named `{reference}`"));
        }
        let names: Vec<String> = hits
            .iter()
            .map(|hit| hit.symbol.qualified_name.clone())
            .collect();
        Ok(format!(
            "no symbol named `{reference}`; did you mean: {}",
            names.join(", ")
        ))
    }
}
