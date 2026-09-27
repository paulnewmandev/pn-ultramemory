// SPDX-License-Identifier: Apache-2.0
//! Local usage metrics: counts and timings of operations, never any text.
//!
//! Events are appended, one JSON object per line, to a file the user controls. They contain
//! numbers only: no queries, no symbol names, no paths and no memory text. The file never leaves
//! the machine, and setting no path turns the whole thing off. The implementation is provided by
//! the analytics work; until then [`Metrics::record`] does nothing.

use std::path::PathBuf;

use crate::engine::Engine;

/// Something that happened, described by numbers only.
///
/// Only the indexing and recall variants have callers so far; the rest are recorded by the
/// operations still being written.
#[allow(dead_code, reason = "recorded by the operations still being written")]
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Event {
    /// An index run finished.
    Index {
        /// Files that were parsed and stored.
        files: u32,
        /// Duration in milliseconds.
        elapsed_ms: u64,
    },
    /// A capsule was built.
    Recall {
        /// The token budget.
        budget: u32,
        /// The tokens the capsule uses.
        used: u32,
        /// Symbols in the capsule.
        symbols: u32,
        /// Duration in milliseconds.
        elapsed_ms: u64,
    },
    /// A symbol was expanded.
    Expand {
        /// Lines returned.
        lines: u32,
    },
    /// An impact analysis ran.
    Impact {
        /// Callers found.
        callers: u32,
        /// Duration in milliseconds.
        elapsed_ms: u64,
    },
    /// A memory was stored.
    Remember {
        /// Secrets that were redacted.
        redactions: u32,
        /// Warnings raised.
        warnings: u32,
    },
    /// Documentation was applied.
    DocApply {
        /// Entries applied.
        applied: u32,
        /// Entries skipped.
        skipped: u32,
    },
}

/// The sink of usage events.
#[derive(Debug)]
pub(crate) struct Metrics {
    /// The file events are appended to, or `None` when metrics are off.
    pub(crate) path: Option<PathBuf>,
}

impl Metrics {
    /// Creates a sink writing to `path`, or a disabled one for `None`.
    pub(crate) const fn new(path: Option<PathBuf>) -> Self {
        Self { path }
    }

    /// Appends an event, stamped with `now`, when metrics are on.
    pub(crate) fn record(&self, event: &Event, now: i64) {
        let _ = (event, now, &self.path);
    }
}

impl Engine {
    /// Records an event with the current time.
    pub(crate) fn record(&self, event: Event) {
        self.metrics.record(&event, self.now());
    }
}
