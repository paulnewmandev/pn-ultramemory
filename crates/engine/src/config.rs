// SPDX-License-Identifier: Apache-2.0
//! Tunable settings of the engine, with defaults chosen for interactive use.

use std::path::PathBuf;

use pn_ultramemory_core::DEFAULT_HALF_LIFE_SECS;

/// The settings of an [`crate::Engine`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineConfig {
    /// Token budget of a capsule or a map when the caller gives none.
    pub default_budget: u32,
    /// Smallest budget accepted: smaller requests are raised to this value.
    pub min_budget: u32,
    /// Largest budget accepted: larger requests are lowered to this value.
    pub max_budget: u32,
    /// Worker threads used to parse files while indexing. Zero means "use the available cores,
    /// at most eight".
    pub index_threads: usize,
    /// A symbol whose full source would cost more tokens than this is never offered at the
    /// source level of detail in a capsule; the agent can still page through it with `expand`.
    pub max_source_tokens: u32,
    /// Where to append usage events (counts and timings only, never text). `None` turns local
    /// metrics off.
    pub metrics_path: Option<PathBuf>,
    /// Half-life of learned evidence, in seconds.
    pub half_life_secs: i64,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            default_budget: 1500,
            min_budget: 100,
            max_budget: 32_000,
            index_threads: 0,
            max_source_tokens: 3000,
            metrics_path: None,
            half_life_secs: DEFAULT_HALF_LIFE_SECS,
        }
    }
}

impl EngineConfig {
    /// Clamps a requested budget into `[min_budget, max_budget]`, using the default when none was
    /// requested.
    ///
    /// # Examples
    /// ```
    /// use pn_ultramemory_engine::EngineConfig;
    ///
    /// let config = EngineConfig::default();
    /// assert_eq!(config.effective_budget(None), 1500);
    /// assert_eq!(config.effective_budget(Some(5)), config.min_budget);
    /// assert_eq!(config.effective_budget(Some(u32::MAX)), config.max_budget);
    /// ```
    #[must_use]
    pub fn effective_budget(&self, requested: Option<u32>) -> u32 {
        requested
            .unwrap_or(self.default_budget)
            .clamp(self.min_budget, self.max_budget)
    }

    /// The number of parsing threads to use for indexing.
    #[must_use]
    pub fn threads(&self) -> usize {
        if self.index_threads > 0 {
            return self.index_threads;
        }
        std::thread::available_parallelism()
            .map_or(1, usize::from)
            .min(8)
    }
}
