// SPDX-License-Identifier: Apache-2.0
//! The [`Engine`] value and the dependencies it is built from.
//!
//! Every operation of the tool is a method of [`Engine`], spread over the sibling modules
//! (`indexer`, `recall`, `memory`, ...). This module only holds the shared state: the adapters
//! behind the ports, the settings, the in-memory record of the current session (what was shown to
//! the agent, used to learn from what it does next) and the local metrics.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use pn_ultramemory_core::{Clock, DocInserter, Extractor, SourceTree, Storage};

use crate::config::EngineConfig;
use crate::learn::Session;
use crate::metrics::Metrics;

/// The adapters an [`Engine`] works through, one for each port.
#[derive(Clone)]
pub struct Deps {
    /// Where the index, the memories and what was learned are kept.
    pub storage: Arc<dyn Storage>,
    /// Turns source text into symbols and references.
    pub extractor: Arc<dyn Extractor>,
    /// Lists, reads and writes the files of the repository.
    pub tree: Arc<dyn SourceTree>,
    /// Writes documentation comments into source text.
    pub docs: Arc<dyn DocInserter>,
    /// Tells the time.
    pub clock: Arc<dyn Clock>,
}

impl core::fmt::Debug for Deps {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Deps { .. }")
    }
}

/// A [`Clock`] that reads the system time.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_secs(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| {
                i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)
            })
    }
}

/// The use cases of pn-ultramemory, bound to one repository.
///
/// It is cheap to share behind an `Arc`: every method takes `&self`.
#[derive(Debug)]
pub struct Engine {
    /// The adapters behind the ports.
    pub(crate) deps: Deps,
    /// The settings.
    pub(crate) config: EngineConfig,
    /// What was shown to the agent recently, for learning.
    pub(crate) session: Mutex<Session>,
    /// Local usage counters.
    pub(crate) metrics: Metrics,
}

impl Engine {
    /// Builds an engine from its adapters and settings.
    #[must_use]
    pub fn new(deps: Deps, config: EngineConfig) -> Self {
        let metrics = Metrics::new(config.metrics_path.clone());
        Self {
            deps,
            config,
            session: Mutex::new(Session::default()),
            metrics,
        }
    }

    /// The settings the engine was built with.
    #[must_use]
    pub const fn config(&self) -> &EngineConfig {
        &self.config
    }

    /// The current time, in seconds since the Unix epoch.
    pub(crate) fn now(&self) -> i64 {
        self.deps.clock.now_secs()
    }

    /// Counts describing the index: files, symbols, edges, memories and languages.
    ///
    /// # Errors
    /// Returns a storage error when the counts cannot be read.
    pub fn index_stats(
        &self,
    ) -> Result<pn_ultramemory_core::IndexStats, crate::error::EngineError> {
        Ok(self.storage().stats()?)
    }

    /// The storage adapter.
    pub(crate) fn storage(&self) -> &dyn Storage {
        self.deps.storage.as_ref()
    }

    /// Locks the session record, recovering from a poisoned lock instead of failing.
    pub(crate) fn session(&self) -> MutexGuard<'_, Session> {
        self.session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}
