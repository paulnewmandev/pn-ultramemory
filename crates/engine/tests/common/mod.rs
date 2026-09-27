// SPDX-License-Identifier: Apache-2.0
//! Shared fixtures for the engine's integration tests: a small multi-language repository written
//! to a temporary directory and an [`Engine`] wired to the real SQLite and tree-sitter adapters.

// Test helpers outside a `#[test]` function are not covered by the `allow-*-in-tests` settings in
// `clippy.toml`, so a failed setup step here needs its own allowance; it should stop the test
// loudly rather than be reported as a failure of the behavior under test.
#![allow(
    clippy::expect_used,
    reason = "a failed setup step should stop the test loudly"
)]
#![allow(
    dead_code,
    reason = "each test file uses a different part of these helpers"
)]

use std::path::Path;
use std::sync::Arc;

use pn_ultramemory_core::{Clock, Language};
use pn_ultramemory_engine::{Deps, Engine, EngineConfig, IndexOptions, SystemClock};
use pn_ultramemory_index::{DocComments, FsSourceTree, TreeSitterExtractor};
use pn_ultramemory_store::SqliteStorage;
use tempfile::TempDir;

/// A clock that only moves when told to, so tests can check staleness and decay.
#[derive(Debug, Default)]
pub(crate) struct ManualClock(pub std::sync::atomic::AtomicI64);

impl Clock for ManualClock {
    /// The time set by the test.
    fn now_secs(&self) -> i64 {
        self.0.load(std::sync::atomic::Ordering::SeqCst)
    }
}

/// A repository in a temporary directory, indexed and ready to query.
pub(crate) struct Fixture {
    /// The directory that holds the repository. Removed when the fixture is dropped.
    pub dir: TempDir,
    /// The engine, wired to the real adapters.
    pub engine: Engine,
}

/// Writes `content` to `relative` inside `root`, creating directories as needed.
pub(crate) fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create directories");
    }
    std::fs::write(path, content).expect("write file");
}

/// Builds an engine over `root` with an in-memory database and the real adapters.
pub(crate) fn engine_at(root: &Path, config: EngineConfig) -> Engine {
    let deps = Deps {
        storage: Arc::new(SqliteStorage::open_in_memory().expect("open storage")),
        extractor: Arc::new(TreeSitterExtractor::new()),
        tree: Arc::new(FsSourceTree::new(root, 1_000_000).expect("open tree")),
        docs: Arc::new(DocComments),
        clock: Arc::new(SystemClock),
    };
    Engine::new(deps, config)
}

/// The sample repository: a Rust configuration module with a small call graph, a Rust entry point,
/// a Python server class and a TypeScript API, with documentation on some symbols and none on
/// others.
pub(crate) fn sample_files() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "src/config.rs",
            r"//! Configuration handling.

/// The application configuration.
pub struct Config {
    /// The listening port.
    pub port: u16,
}

impl Config {
    /// Checks that every field holds a usable value.
    pub fn validate(&self) -> bool {
        self.port > 0
    }
}

/// Loads the configuration from a file.
pub fn load_config(path: &str) -> Config {
    let text = read_file(path);
    parse_config(&text)
}

/// Parses configuration text into a [`Config`].
pub fn parse_config(text: &str) -> Config {
    Config { port: text.trim().parse().unwrap_or(8080) }
}

pub fn default_config() -> Config {
    Config { port: 8080 }
}

fn read_file(path: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}
",
        ),
        (
            "src/main.rs",
            r#"use crate::config::load_config;

fn main() {
    let config = load_config("app.toml");
    if config.validate() {
        run(config);
    }
}

fn run(config: Config) {
    println!("listening on {}", config.port);
}
"#,
        ),
        (
            "app/server.py",
            r#"class Server:
    """Serves requests on a port."""

    def __init__(self, port):
        self.port = port

    def start(self):
        """Starts listening."""
        self._bind()

    def stop(self):
        self._close()

    def _bind(self):
        return self.port

    def _close(self):
        return None


def make_server(port):
    """Creates a server."""
    return Server(port)
"#,
        ),
        (
            "web/api.ts",
            r"/** Fetches a user by id. */
export function fetchUser(id: number): Promise<User> {
  return request(`/users/${id}`);
}

export function request(url: string): Promise<any> {
  return fetch(url).then((r) => r.json());
}

export interface User {
  id: number;
  name: string;
}
",
        ),
    ]
}

/// Writes the sample repository, builds an engine over it and indexes it.
pub(crate) fn sample_repo() -> Fixture {
    let dir = tempfile::tempdir().expect("temporary directory");
    for (path, content) in sample_files() {
        write(dir.path(), path, content);
    }
    let engine = engine_at(dir.path(), EngineConfig::default());
    engine
        .index(&IndexOptions::default())
        .expect("index the sample repository");
    Fixture { dir, engine }
}

/// The language of a path, for assertions.
pub(crate) fn language_of(path: &str) -> Option<Language> {
    Language::from_path(Path::new(path))
}

/// Generators for large deterministic repositories, used by the recall, indexing and benchmark
/// tests of the retrieval side of the engine.
pub(crate) mod generated;
