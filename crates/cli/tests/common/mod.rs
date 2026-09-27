// SPDX-License-Identifier: Apache-2.0
//! Helpers shared by the integration tests of the installer and the hooks.
//!
//! Every test works inside a temporary directory that stands in for a repository, so the project
//! scope of each agent (`.cursor/mcp.json`, `.codex/config.toml`, ...) can be exercised without
//! ever reading or writing anything in the real home directory.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers"
)]

use std::path::{Path, PathBuf};

use tempfile::TempDir;

/// A throwaway repository.
pub(crate) struct Repo {
    /// The temporary directory, deleted when this value is dropped.
    dir: TempDir,
}

impl Repo {
    /// A new empty repository.
    pub(crate) fn new() -> Self {
        Self {
            dir: tempfile::tempdir().expect("a temporary directory"),
        }
    }

    /// The root of the repository.
    pub(crate) fn path(&self) -> &Path {
        self.dir.path()
    }

    /// The absolute path of a file inside the repository.
    pub(crate) fn at(&self, rel: &str) -> PathBuf {
        self.dir.path().join(rel)
    }

    /// Writes a file, creating its directory.
    pub(crate) fn write(&self, rel: &str, content: &str) {
        let path = self.at(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("a directory");
        }
        std::fs::write(&path, content).expect("a written file");
    }

    /// Creates a directory inside the repository.
    pub(crate) fn mkdir(&self, rel: &str) {
        std::fs::create_dir_all(self.at(rel)).expect("a directory");
    }

    /// The exact bytes of a file, as text.
    pub(crate) fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.at(rel)).expect("a readable file")
    }

    /// Whether something exists at this path.
    pub(crate) fn exists(&self, rel: &str) -> bool {
        self.at(rel).symlink_metadata().is_ok()
    }
}
