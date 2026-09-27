// SPDX-License-Identifier: Apache-2.0
//! The SQLite implementation of the storage port.
//!
//! [`SqliteStorage`] owns one SQLite connection behind a [`Mutex`], which is what lets it be
//! `Send + Sync` as the [`Storage`] trait requires: every method locks the connection for the
//! duration of one operation, and every operation that writes runs in a single transaction.
//! One connection is a deliberate choice. SQLite serializes writers anyway, an index update or a
//! search takes milliseconds, and a pool would add cross-connection visibility rules (and no
//! in-memory database) for no measurable gain at this scale. Several *processes* can still open
//! the same file: WAL lets them read while another writes, and a 5 second busy timeout makes
//! writers queue instead of failing.
//!
//! The trait methods are thin: each one hands the locked connection to the module that owns the
//! behavior (`upsert`, `files`, `symbols`, `search`, `edges`, `memories`, `learning`, `meta`).

use std::fmt;
use std::fs;
use std::path::Path;
use std::sync::{Mutex, MutexGuard, PoisonError};

use pn_ultramemory_core::{
    Confidence, Direction, DocCoverageRow, FileExtract, FileInput, FileRecord, FileTotals,
    IndexStats, LearningStatus, MemoryFilter, MemoryId, MemoryRecord, ModuleEdge, ModuleStats,
    Neighbor, NewMemory, ResolveScope, ResolveStats, SearchHit, SearchQuery, SignalKind, Storage,
    StorageError, SymbolId, SymbolRecord, Target, UpsertOutcome, UtilityState,
};
use rusqlite::Connection;

use crate::error::{DbResult, Result};
use crate::{edges, files, learning, memories, meta, reports, schema, search, symbols, upsert};

/// Storage of the index, the memories and what was learned, in one SQLite database.
///
/// # Behavior worth knowing
/// The port's documentation is the contract; these are the choices this adapter makes where the
/// contract leaves room.
///
/// * **Search.** Every word of a query must match (in the name, qualified name, signature or
///   documentation) and the last word also matches by prefix. Memory search first requires every
///   word and, if that finds nothing, accepts any word.
/// * **Resolution.** See the rules in the `edges` module: same-file `Resolved`, a unique name
///   `Heuristic`, up to four candidates chosen by shared path prefix `Guess`; a reference with no
///   owner symbol makes no edge. Storing a file drops the edges that start in it and keeps the
///   ones that end at symbols that survive; the names whose set of definitions changed (a symbol
///   added or removed) are remembered until the next `resolve_edges`, so a partial resolve stays
///   equal to a full one even when the caller passes fewer names than changed.
/// * **Bulk indexing.** `upsert_files` stores any number of files in one transaction with the
///   statements prepared once, and is atomic. A batch that fills an empty index with many rows
///   builds the secondary indexes at the end instead of maintaining them row by row, and full-text
///   rows are written in id order, which is what makes FTS5 fast. The result is exactly what
///   storing the files one at a time gives.
/// * **Reports.** `module_stats`, `module_edges`, `doc_coverage` and `file_totals` are single
///   set-based statements over the tables (see the `reports` module); modules are directory
///   prefixes of file paths, and documentation that is only whitespace counts as missing.
/// * **Errors.** Storing a memory about a symbol that does not exist is
///   [`StorageError::NotFound`] and stores nothing. Non-finite numbers passed to
///   `put_utility_state` or `bump_coaccess` are refused as [`StorageError::Backend`].
/// * **Hostile extractions.** A `parent` that is not an earlier symbol, and a reference `owner`
///   outside the symbol list, are stored as "none" rather than failing the whole file.
///
/// # Examples
/// ```
/// use pn_ultramemory_core::{
///     FileExtract, FileInput, Language, SearchQuery, Span, Storage, SymbolDraft, SymbolKind,
///     Visibility,
/// };
/// use pn_ultramemory_store::SqliteStorage;
///
/// # fn main() -> Result<(), pn_ultramemory_core::StorageError> {
/// let store = SqliteStorage::open_in_memory()?;
/// let file = FileInput {
///     path: "src/config.rs".into(),
///     language: Language::Rust,
///     hash: "abc".into(),
///     size: 42,
///     mtime_secs: 0,
/// };
/// let extract = FileExtract {
///     language: Language::Rust,
///     symbols: vec![SymbolDraft {
///         name: "parse_config".into(),
///         qualified_name: "parse_config".into(),
///         kind: SymbolKind::Function,
///         signature: "pub fn parse_config(text: &str) -> Config".into(),
///         doc: Some("Parses the configuration file.".into()),
///         visibility: Visibility::Public,
///         span: Span { start_line: 1, end_line: 3, start_byte: 0, end_byte: 60 },
///         parent: None,
///         outline: vec![],
///         sig_hash: 1,
///         body_hash: 2,
///     }],
///     references: vec![],
///     imports: vec![],
///     line_count: 3,
///     parse_errors: 0,
/// };
/// store.upsert_file(&file, &extract, 1_700_000_000)?;
///
/// // `parseConfig` is split like the stored name, so it finds `parse_config`.
/// let query = SearchQuery { text: "parseConfig".into(), ..SearchQuery::default() };
/// let hits = store.search_symbols(&query, 10)?;
/// assert_eq!(hits.len(), 1);
/// assert_eq!(hits[0].symbol.name, "parse_config");
/// assert!(hits[0].score > 0.0);
/// # Ok(())
/// # }
/// ```
pub struct SqliteStorage {
    /// The one connection, locked for the length of every operation.
    conn: Mutex<Connection>,
}

impl fmt::Debug for SqliteStorage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SqliteStorage").finish_non_exhaustive()
    }
}

impl SqliteStorage {
    /// The schema version this build reads and writes (`PRAGMA user_version`).
    pub const SCHEMA_VERSION: u32 = schema::SCHEMA_VERSION;

    /// Opens the database at `path`, creating it and its parent directories if needed, and brings
    /// it to the schema of this build.
    ///
    /// Applies WAL journaling, `synchronous=NORMAL`, foreign keys, a 5 second busy timeout,
    /// in-memory temporary storage, a 256 MiB memory map and a 64 MiB page cache, then runs the
    /// forward-only migrations recorded in `PRAGMA user_version`.
    ///
    /// # Errors
    /// * [`StorageError::Backend`] when the directory or the file cannot be created or opened.
    /// * [`StorageError::Corrupt`] when the file is not a SQLite database, is not a
    ///   pn-ultramemory database, or has a **newer** schema version than this build (it is never
    ///   modified in that case).
    ///
    /// # Examples
    /// ```
    /// use pn_ultramemory_core::Storage;
    /// use pn_ultramemory_store::SqliteStorage;
    ///
    /// # fn main() -> Result<(), pn_ultramemory_core::StorageError> {
    /// let dir = std::env::temp_dir().join(format!("pnum-doc-open-{}", std::process::id()));
    /// let store = SqliteStorage::open(&dir.join("nested").join("index.db"))?;
    /// store.set_meta("root", "/repo")?;
    /// assert_eq!(store.get_meta("root")?.as_deref(), Some("/repo"));
    /// # drop(store);
    /// # let _ = std::fs::remove_dir_all(&dir);
    /// # Ok(())
    /// # }
    /// ```
    pub fn open(path: &Path) -> std::result::Result<Self, StorageError> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent).map_err(|error| {
                StorageError::Backend(format!(
                    "could not create the directory `{}`: {error}",
                    parent.display()
                ))
            })?;
        }
        let conn = Connection::open(path).db()?;
        Self::from_connection(conn, true)
    }

    /// Opens a private in-memory database with the same schema, for tests and throwaway indexes.
    ///
    /// # Errors
    /// [`StorageError::Backend`] when SQLite cannot create the database.
    ///
    /// # Examples
    /// ```
    /// use pn_ultramemory_core::Storage;
    /// use pn_ultramemory_store::SqliteStorage;
    ///
    /// # fn main() -> Result<(), pn_ultramemory_core::StorageError> {
    /// let store = SqliteStorage::open_in_memory()?;
    /// assert_eq!(store.stats()?.files, 0);
    /// # Ok(())
    /// # }
    /// ```
    pub fn open_in_memory() -> std::result::Result<Self, StorageError> {
        let conn = Connection::open_in_memory().db()?;
        Self::from_connection(conn, false)
    }

    /// Configures, migrates and wraps a fresh connection.
    fn from_connection(mut conn: Connection, persistent: bool) -> Result<Self> {
        schema::configure(&conn, persistent)?;
        schema::migrate(&mut conn)?;
        schema::create_temp_tables(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Runs one operation with the connection locked.
    ///
    /// A panic in another thread while it held the lock poisons the mutex, but the transaction it
    /// was running is rolled back when its guard drops during unwinding, so the connection is in
    /// a consistent state and is used as is.
    fn with<T>(&self, operation: impl FnOnce(&mut Connection) -> Result<T>) -> Result<T> {
        let mut guard: MutexGuard<'_, Connection> =
            self.conn.lock().unwrap_or_else(PoisonError::into_inner);
        operation(&mut guard)
    }
}

impl Storage for SqliteStorage {
    fn file_hash(&self, path: &str) -> Result<Option<String>> {
        self.with(|conn| files::file_hash(conn, path))
    }

    fn upsert_file(
        &self,
        file: &FileInput,
        extract: &FileExtract,
        now: i64,
    ) -> Result<UpsertOutcome> {
        let mut outcomes = self.with(|conn| upsert::upsert_files(conn, &[(file, extract)], now))?;
        outcomes
            .pop()
            .ok_or_else(|| StorageError::Backend("the file was not stored".to_owned()))
    }

    fn upsert_files(
        &self,
        files: &[(FileInput, FileExtract)],
        now: i64,
    ) -> Result<Vec<UpsertOutcome>> {
        let borrowed: Vec<(&FileInput, &FileExtract)> = files
            .iter()
            .map(|(file, extract)| (file, extract))
            .collect();
        self.with(|conn| upsert::upsert_files(conn, &borrowed, now))
    }

    fn remove_files_not_in(&self, present: &[String], now: i64) -> Result<u32> {
        self.with(|conn| files::remove_files_not_in(conn, present, now))
    }

    fn resolve_edges(&self, scope: &ResolveScope) -> Result<ResolveStats> {
        self.with(|conn| edges::resolve_edges(conn, scope))
    }

    fn stats(&self) -> Result<IndexStats> {
        self.with(|conn| files::stats(conn))
    }

    fn list_files(&self) -> Result<Vec<FileRecord>> {
        self.with(|conn| files::list_files(conn))
    }

    fn symbols_in_file(&self, path: &str) -> Result<Vec<SymbolRecord>> {
        self.with(|conn| symbols::symbols_in_file(conn, path))
    }

    fn find_symbols(&self, name: &str, limit: usize) -> Result<Vec<SymbolRecord>> {
        self.with(|conn| symbols::find_symbols(conn, name, limit))
    }

    fn search_symbols(&self, query: &SearchQuery, limit: usize) -> Result<Vec<SearchHit>> {
        self.with(|conn| search::search_symbols(conn, query, limit))
    }

    fn symbol(&self, id: SymbolId) -> Result<Option<SymbolRecord>> {
        self.with(|conn| symbols::symbol(conn, id))
    }

    fn neighbors(
        &self,
        id: SymbolId,
        direction: Direction,
        min_confidence: Confidence,
        limit: usize,
    ) -> Result<Vec<Neighbor>> {
        self.with(|conn| edges::neighbors(conn, id, direction, min_confidence, limit))
    }

    fn central_symbols(
        &self,
        limit: usize,
        path_prefix: Option<&str>,
    ) -> Result<Vec<(SymbolRecord, u32)>> {
        self.with(|conn| symbols::central_symbols(conn, limit, path_prefix))
    }

    fn undocumented_public(
        &self,
        limit: usize,
        path_prefix: Option<&str>,
    ) -> Result<Vec<SymbolRecord>> {
        self.with(|conn| symbols::undocumented_public(conn, limit, path_prefix))
    }

    fn module_stats(&self, depth: usize) -> Result<Vec<ModuleStats>> {
        self.with(|conn| reports::module_stats(conn, depth))
    }

    fn module_edges(
        &self,
        depth: usize,
        min_confidence: Confidence,
        limit: usize,
    ) -> Result<Vec<ModuleEdge>> {
        self.with(|conn| reports::module_edges(conn, depth, min_confidence, limit))
    }

    fn doc_coverage(&self) -> Result<Vec<DocCoverageRow>> {
        self.with(|conn| reports::doc_coverage(conn))
    }

    fn file_totals(&self) -> Result<FileTotals> {
        self.with(|conn| reports::file_totals(conn))
    }

    fn add_memory(&self, new: &NewMemory, now: i64) -> Result<MemoryRecord> {
        self.with(|conn| memories::add_memory(conn, new, now))
    }

    fn memories_for_symbols(&self, ids: &[SymbolId], limit: usize) -> Result<Vec<MemoryRecord>> {
        self.with(|conn| memories::memories_for_symbols(conn, ids, limit))
    }

    fn search_memories(&self, text: &str, limit: usize) -> Result<Vec<MemoryRecord>> {
        self.with(|conn| memories::search_memories(conn, text, limit))
    }

    fn list_memories(&self, filter: &MemoryFilter) -> Result<Vec<MemoryRecord>> {
        self.with(|conn| memories::list_memories(conn, filter))
    }

    fn forget_memory(&self, id: MemoryId) -> Result<bool> {
        self.with(|conn| memories::forget_memory(conn, id))
    }

    fn reanchor_memory(&self, id: MemoryId) -> Result<bool> {
        self.with(|conn| memories::reanchor_memory(conn, id))
    }

    fn utility_states(&self, targets: &[Target]) -> Result<Vec<(Target, UtilityState)>> {
        self.with(|conn| learning::utility_states(conn, targets))
    }

    fn put_utility_state(&self, target: Target, state: UtilityState) -> Result<()> {
        self.with(|conn| learning::put_utility_state(conn, target, state))
    }

    fn log_signal(&self, target: Target, kind: SignalKind, now: i64) -> Result<()> {
        self.with(|conn| learning::log_signal(conn, target, kind, now))
    }

    fn bump_coaccess(&self, a: SymbolId, b: SymbolId, weight: f64, now: i64) -> Result<()> {
        self.with(|conn| learning::bump_coaccess(conn, a, b, weight, now))
    }

    fn coaccess_neighbors(
        &self,
        id: SymbolId,
        limit: usize,
        now: i64,
    ) -> Result<Vec<(SymbolId, f64)>> {
        self.with(|conn| learning::coaccess_neighbors(conn, id, limit, now))
    }

    fn learning_status(&self) -> Result<LearningStatus> {
        self.with(|conn| learning::learning_status(conn))
    }

    fn reset_learning(&self) -> Result<()> {
        self.with(learning::reset_learning)
    }

    fn get_meta(&self, key: &str) -> Result<Option<String>> {
        self.with(|conn| meta::get_meta(conn, key))
    }

    fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.with(|conn| meta::set_meta(conn, key, value))
    }
}
