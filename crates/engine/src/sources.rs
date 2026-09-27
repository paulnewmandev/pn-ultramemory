// SPDX-License-Identifier: Apache-2.0
//! Reading the source of symbols safely.
//!
//! A symbol's span refers to byte offsets in the file **as it was when it was indexed**. If the
//! file changed since, those offsets point at the wrong text. [`SourceCache`] therefore reads a file
//! once per operation, compares its content hash with the one stored at indexing time, and refuses
//! to slice a file that changed, so an agent is never shown code that does not match its
//! description.

use std::collections::HashMap;
use std::sync::Arc;

use pn_ultramemory_core::SymbolRecord;

use crate::engine::Engine;
use crate::error::EngineError;
use crate::indexer::content_hash;

/// A per-operation cache of file contents that match the index.
pub(crate) struct SourceCache<'a> {
    /// The engine whose adapters are used.
    engine: &'a Engine,
    /// Path to content, or `None` when the file changed since it was indexed or cannot be read.
    files: HashMap<String, Option<Arc<str>>>,
}

impl Engine {
    /// The exact source text of a symbol's declaration, including its attached documentation, or
    /// `None` if the file changed since it was indexed (so the recorded position is unreliable).
    ///
    /// # Errors
    /// Returns a storage error when the stored file hash cannot be read.
    pub fn source_of(&self, symbol: &SymbolRecord) -> Result<Option<String>, EngineError> {
        SourceCache::new(self).slice(symbol)
    }
}

impl<'a> SourceCache<'a> {
    /// Creates an empty cache.
    pub(crate) fn new(engine: &'a Engine) -> Self {
        Self {
            engine,
            files: HashMap::new(),
        }
    }

    /// The content of a file, or `None` if it changed since it was indexed, is not indexed, or
    /// cannot be read as text.
    ///
    /// # Errors
    /// Returns a storage error when the stored hash cannot be read.
    pub(crate) fn text(&mut self, path: &str) -> Result<Option<Arc<str>>, EngineError> {
        if let Some(cached) = self.files.get(path) {
            return Ok(cached.clone());
        }
        let stored = self.engine.storage().file_hash(path)?;
        let text = match (stored, self.engine.deps.tree.read(path)) {
            (Some(stored), Ok(text)) if content_hash(&text) == stored => {
                Some(Arc::<str>::from(text))
            }
            _ => None,
        };
        self.files.insert(path.to_owned(), text.clone());
        Ok(text)
    }

    /// The exact source text of a symbol's declaration, including its attached documentation, or
    /// `None` if the file changed since it was indexed.
    ///
    /// # Errors
    /// Returns a storage error when the stored hash cannot be read.
    pub(crate) fn slice(&mut self, symbol: &SymbolRecord) -> Result<Option<String>, EngineError> {
        let Some(text) = self.text(&symbol.path)? else {
            return Ok(None);
        };
        let start = symbol.span.start_byte as usize;
        let end = symbol.span.end_byte as usize;
        Ok(text.get(start..end).map(str::to_owned))
    }
}
