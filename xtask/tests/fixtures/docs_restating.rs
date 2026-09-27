// SPDX-License-Identifier: Apache-2.0
//! Fixture for the documentation ratchet: docstrings that only restate the item's own name.
//!
//! Four items must be flagged: a free function, a struct, one of its fields and a method. Everything
//! else in the file documents itself properly and must be left alone.

/// A token budget.
pub struct TokenBudget {
    /// The limit.
    pub limit: usize,
    /// How many tokens are still unspent after the header and the index have been written.
    remaining: usize,
}

/// A packed selection of symbols, each rendered at the level of detail the budget allowed.
pub struct Capsule {
    /// The rendered text, ready to hand to an agent.
    body: String,
}

impl Capsule {
    /// Token count.
    pub fn token_count(&self) -> usize {
        self.body.len()
    }

    /// Returns the rendered text without copying it, so a caller can stream it.
    pub fn body(&self) -> &str {
        &self.body
    }
}

/// Parses config.
pub fn parse_config(text: &str) -> usize {
    text.len()
}

/// Reads the manifest, resolving every relative path against the directory it was found in.
pub fn read_manifest(text: &str) -> usize {
    text.len()
}
