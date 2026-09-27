// SPDX-License-Identifier: Apache-2.0
//! The ports of the hexagon: the interfaces the domain and the use cases need from the outside
//! world, implemented by adapters.
//!
//! * [`Extractor`] turns source text into symbols and references (implemented with tree-sitter).
//! * [`DocInserter`] writes a documentation comment into source text in the language's own syntax.
//! * [`SourceTree`] lists, reads and writes the files of a repository.
//! * [`Storage`] persists the index, the memories and what was learned (implemented with SQLite).
//! * [`Clock`] tells the time, so that everything that depends on it can be tested.

use core::fmt;
use std::error::Error;

use crate::{
    Confidence, Direction, DocCoverageRow, FileExtract, FileInput, FileRecord, FileTotals,
    IndexStats, Language, LearningStatus, MemoryFilter, MemoryId, MemoryRecord, ModuleEdge,
    ModuleStats, Neighbor, NewMemory, ResolveScope, ResolveStats, SearchHit, SearchQuery, SymbolId,
    SymbolKind, SymbolRecord, Target, UpsertOutcome, UtilityState,
};

/// Why an extraction failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtractError {
    /// The extractor has no support for this language.
    Unsupported(Language),
    /// The parser could not produce a tree, with the reason.
    Parse(String),
}

impl fmt::Display for ExtractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported(language) => write!(f, "language `{language}` is not supported"),
            Self::Parse(reason) => write!(f, "could not parse the source: {reason}"),
        }
    }
}

impl Error for ExtractError {}

/// Turns source text into symbols and references.
pub trait Extractor: Send + Sync {
    /// Returns `true` when the extractor can handle the language, with a grammar or with its
    /// lexical fallback.
    fn supports(&self, language: Language) -> bool;

    /// Extracts the symbols and references of one file.
    ///
    /// Malformed source is not an error: the parser recovers, and the number of errors it
    /// recovered from is reported in [`FileExtract::parse_errors`].
    ///
    /// # Errors
    /// Returns [`ExtractError::Unsupported`] for a language the extractor does not handle, and
    /// [`ExtractError::Parse`] when no tree could be built at all.
    fn extract(&self, language: Language, source: &str) -> Result<FileExtract, ExtractError>;
}

/// Why inserting a documentation comment failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocError {
    /// Documentation cannot be inserted for this language.
    Unsupported(Language),
    /// The declaration could not be found at the given line.
    TargetNotFound,
    /// The result would not be valid, with the reason. The source is left unchanged.
    Invalid(String),
}

impl fmt::Display for DocError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported(language) => {
                write!(f, "documentation cannot be inserted for `{language}`")
            }
            Self::TargetNotFound => f.write_str("the declaration was not found at that line"),
            Self::Invalid(reason) => write!(f, "the documentation would break the code: {reason}"),
        }
    }
}

impl Error for DocError {}

/// The declaration a documentation comment is written for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DocTarget<'a> {
    /// The simple name of the symbol.
    pub name: &'a str,
    /// What kind of symbol it is.
    pub kind: SymbolKind,
    /// The 1-based line where the declaration starts, ignoring any comment already attached.
    pub line: u32,
}

/// Writes documentation comments into source text.
pub trait DocInserter: Send + Sync {
    /// Returns `source` with `text` inserted as the documentation of the declaration.
    ///
    /// The comment uses the language's own syntax and the declaration's indentation. The result
    /// must parse with no more syntax errors than the original. If the declaration already has
    /// documentation, the call fails instead of duplicating it.
    ///
    /// # Errors
    /// Returns [`DocError::Unsupported`], [`DocError::TargetNotFound`] or
    /// [`DocError::Invalid`], and never a partly modified source.
    fn insert_doc(
        &self,
        language: Language,
        source: &str,
        target: &DocTarget<'_>,
        text: &str,
    ) -> Result<String, DocError>;
}

/// A file of a source tree, as listed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFile {
    /// Path relative to the repository root, always with forward slashes.
    pub path: String,
    /// Size in bytes.
    pub size: u64,
    /// Modification time, in seconds since the Unix epoch.
    pub mtime_secs: i64,
}

/// Why reading or writing a source file failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceError {
    /// The file system failed, with the message.
    Io(String),
    /// The file is not valid UTF-8 text, so it cannot be indexed.
    NotText(String),
    /// The path points outside the repository, so it was refused.
    OutsideRoot(String),
}

impl fmt::Display for SourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(message) => write!(f, "file system error: {message}"),
            Self::NotText(path) => write!(f, "`{path}` is not UTF-8 text"),
            Self::OutsideRoot(path) => write!(f, "`{path}` is outside the repository"),
        }
    }
}

impl Error for SourceError {}

/// Lists, reads and writes the files of a repository.
pub trait SourceTree: Send + Sync {
    /// Lists the files worth indexing: known source languages, not ignored by version-control
    /// ignore rules, not hidden, and not larger than the adapter's size limit. Paths are relative
    /// to the root with forward slashes, in sorted order.
    ///
    /// # Errors
    /// Returns [`SourceError::Io`] when the tree cannot be walked.
    fn list(&self) -> Result<Vec<SourceFile>, SourceError>;

    /// Reads a file as UTF-8 text.
    ///
    /// # Errors
    /// Returns [`SourceError::NotText`] for binary content, [`SourceError::OutsideRoot`] for a
    /// path that escapes the root (`..`, absolute paths, symbolic links that leave the tree), and
    /// [`SourceError::Io`] for anything else.
    fn read(&self, path: &str) -> Result<String, SourceError>;

    /// Replaces the content of an existing file, atomically where the platform allows it.
    ///
    /// # Errors
    /// Returns [`SourceError::OutsideRoot`] for a path that escapes the root and
    /// [`SourceError::Io`] for anything else.
    fn write(&self, path: &str, content: &str) -> Result<(), SourceError>;
}

/// Tells the time.
pub trait Clock: Send + Sync {
    /// Seconds since the Unix epoch.
    fn now_secs(&self) -> i64;
}

/// Why a storage operation failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageError {
    /// The backend failed, with its message.
    Backend(String),
    /// The stored data is not what this version expects, with a description.
    Corrupt(String),
    /// Something that had to exist does not, with a description.
    NotFound(String),
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Backend(message) => write!(f, "storage backend error: {message}"),
            Self::Corrupt(message) => write!(f, "stored data is unusable: {message}"),
            Self::NotFound(message) => write!(f, "not found: {message}"),
        }
    }
}

impl Error for StorageError {}

/// Persists the index, the memories and what was learned.
///
/// Every method takes `&self`: implementations synchronize internally, so a server can share one
/// value between threads. Every method fails only with a [`StorageError`].
///
/// Times are seconds since the Unix epoch, passed in so that storage never reads the clock.
#[allow(clippy::missing_errors_doc)]
pub trait Storage: Send + Sync {
    // ---- indexing -------------------------------------------------------------------------

    /// Returns the stored content hash of a file, or `None` if the file is not indexed.
    fn file_hash(&self, path: &str) -> Result<Option<String>, StorageError>;

    /// Replaces everything stored for one file with a fresh extraction, in one transaction.
    ///
    /// Symbol identities must be stable: a symbol with the same file, qualified name and kind
    /// (and ordinal among identical ones) keeps its [`SymbolId`], so edges and memory anchors
    /// that point to it stay valid. References are stored unresolved. Edges that start at a
    /// reference of this file, and edges that end at a symbol that no longer exists, are dropped
    /// ([`Storage::resolve_edges`] recreates them); **edges that end at a symbol that survives are
    /// kept**, so re-indexing one file stays fast. The names whose set of definitions changed
    /// (symbols added or removed, not merely modified) are remembered and re-resolved by the next
    /// [`Storage::resolve_edges`] with a [`ResolveScope::Touching`] scope, because only those can
    /// change what a reference means.
    /// Memories whose anchor no longer matches ([`crate::Anchor::is_stale_against`]) must get
    /// `stale_since = now` if they were still fresh. When [`FileInput::language`] and
    /// [`FileExtract::language`] differ, the former wins.
    fn upsert_file(
        &self,
        file: &FileInput,
        extract: &FileExtract,
        now: i64,
    ) -> Result<UpsertOutcome, StorageError>;

    /// Stores many files in a single transaction, returning one outcome per input in the same
    /// order. Semantically it is [`Storage::upsert_file`] applied to each file in turn; adapters
    /// override it to make bulk indexing much faster than one transaction per file.
    fn upsert_files(
        &self,
        files: &[(FileInput, FileExtract)],
        now: i64,
    ) -> Result<Vec<UpsertOutcome>, StorageError> {
        files
            .iter()
            .map(|(file, extract)| self.upsert_file(file, extract, now))
            .collect()
    }

    /// Removes every file whose path is not in `present`, with its symbols, references and
    /// edges, and marks the memories anchored to removed symbols as stale. Returns the number of
    /// files removed.
    fn remove_files_not_in(&self, present: &[String], now: i64) -> Result<u32, StorageError>;

    /// Turns stored references into edges for the given scope, replacing the edges that
    /// previously came from those references.
    ///
    /// A reference only ever resolves to symbols whose file is in the same language family
    /// ([`Language::family`]): a TypeScript `Record` must not link to a C# class of the same name.
    /// Every rule below applies among the candidates of the same family.
    ///
    /// Confidence rules, from the strongest: a reference whose name matches a symbol in the same
    /// file is [`Confidence::Resolved`] (when several symbols of the file match, the four nearest
    /// by line are linked, all `Resolved`); a name that matches exactly one symbol in the whole
    /// index is [`Confidence::Heuristic`]; a name that matches several is linked to at most
    /// four of them, preferring symbols whose path shares the longest prefix with the referencing
    /// file, at [`Confidence::Guess`]. Names that match nothing produce no edge, and neither do
    /// references at file level (no owning symbol). A symbol links to itself only when the
    /// reference is unqualified or qualified by `self`, `this`, `Self`, `cls`, `$this` or `static`
    /// (recursion), never through another qualifier.
    fn resolve_edges(&self, scope: &ResolveScope) -> Result<ResolveStats, StorageError>;

    /// Returns counts describing the index.
    fn stats(&self) -> Result<IndexStats, StorageError>;

    /// Lists indexed files, ordered by path.
    fn list_files(&self) -> Result<Vec<FileRecord>, StorageError>;

    /// Lists the symbols of one file in source order.
    fn symbols_in_file(&self, path: &str) -> Result<Vec<SymbolRecord>, StorageError>;

    // ---- symbol queries -------------------------------------------------------------------

    /// Finds symbols whose simple or qualified name equals `name`. Exact matches come first,
    /// then matches that differ only in case. At most `limit` symbols are returned.
    fn find_symbols(&self, name: &str, limit: usize) -> Result<Vec<SymbolRecord>, StorageError>;

    /// Full-text search over names, qualified names, signatures and documentation. Identifiers
    /// are split at `camelCase` and `snake_case` boundaries, so `parseConfig` matches
    /// `parse config`. Results are ordered by descending score.
    fn search_symbols(
        &self,
        query: &SearchQuery,
        limit: usize,
    ) -> Result<Vec<SearchHit>, StorageError>;

    /// Fetches one symbol.
    fn symbol(&self, id: SymbolId) -> Result<Option<SymbolRecord>, StorageError>;

    /// Follows edges from a symbol, keeping only those at least as confident as
    /// `min_confidence`. Results are ordered by descending confidence, then by path and line.
    fn neighbors(
        &self,
        id: SymbolId,
        direction: Direction,
        min_confidence: Confidence,
        limit: usize,
    ) -> Result<Vec<Neighbor>, StorageError>;

    /// The most referenced symbols with their in-degree, counting edges at least as confident as
    /// [`Confidence::Heuristic`], optionally restricted to a path prefix. Ties break by path and
    /// name.
    fn central_symbols(
        &self,
        limit: usize,
        path_prefix: Option<&str>,
    ) -> Result<Vec<(SymbolRecord, u32)>, StorageError>;

    /// Public symbols that have no documentation, ordered by path and line. Modules are skipped.
    fn undocumented_public(
        &self,
        limit: usize,
        path_prefix: Option<&str>,
    ) -> Result<Vec<SymbolRecord>, StorageError>;

    // ---- aggregates for reports -----------------------------------------------------------

    /// Groups files by the first `depth` directories of their path (files at the root form the
    /// module `.`) and reports each group's size and how many edges at least as confident as
    /// [`Confidence::Heuristic`] cross its boundary. Ordered by descending symbol count, then by
    /// name. The default returns nothing; adapters override it.
    fn module_stats(&self, depth: usize) -> Result<Vec<ModuleStats>, StorageError> {
        let _ = depth;
        Ok(Vec::new())
    }

    /// The edges between different modules (see [`Storage::module_stats`]) summed per pair of
    /// modules, heaviest first, at most `limit` pairs, counting only edges at least as confident
    /// as `min_confidence`. The default returns nothing; adapters override it.
    fn module_edges(
        &self,
        depth: usize,
        min_confidence: Confidence,
        limit: usize,
    ) -> Result<Vec<ModuleEdge>, StorageError> {
        let _ = (depth, min_confidence, limit);
        Ok(Vec::new())
    }

    /// Public symbols and how many of them are documented, per language, most public symbols
    /// first. Modules are not counted. The default returns nothing; adapters override it.
    fn doc_coverage(&self) -> Result<Vec<DocCoverageRow>, StorageError> {
        Ok(Vec::new())
    }

    /// Line and parse-error totals over every indexed file. The default returns zeros; adapters
    /// override it.
    fn file_totals(&self) -> Result<FileTotals, StorageError> {
        Ok(FileTotals::default())
    }

    // ---- memory ---------------------------------------------------------------------------

    /// Stores a memory and anchors it to the given symbols, recording their current hashes.
    /// Fails with [`StorageError::NotFound`], storing nothing, if a symbol does not exist.
    fn add_memory(&self, new: &NewMemory, now: i64) -> Result<MemoryRecord, StorageError>;

    /// Memories anchored to any of these symbols, newest first.
    fn memories_for_symbols(
        &self,
        ids: &[SymbolId],
        limit: usize,
    ) -> Result<Vec<MemoryRecord>, StorageError>;

    /// Full-text search over the text of memories, best matches first. Every word must match;
    /// if nothing does, any word may, ranked by relevance.
    fn search_memories(&self, text: &str, limit: usize) -> Result<Vec<MemoryRecord>, StorageError>;

    /// Lists memories, newest first. A [`MemoryFilter::limit`] of zero means 100.
    fn list_memories(&self, filter: &MemoryFilter) -> Result<Vec<MemoryRecord>, StorageError>;

    /// Fetches one memory by its identity.
    ///
    /// The default implementation scans the list, which is correct but linear; adapters override
    /// it with a direct lookup.
    fn memory(&self, id: MemoryId) -> Result<Option<MemoryRecord>, StorageError> {
        let filter = MemoryFilter {
            limit: usize::MAX,
            ..MemoryFilter::default()
        };
        Ok(self
            .list_memories(&filter)?
            .into_iter()
            .find(|record| record.id == id))
    }

    /// Deletes a memory. Returns `false` if it did not exist.
    fn forget_memory(&self, id: MemoryId) -> Result<bool, StorageError>;

    /// Re-anchors a memory to the current hashes of the symbols it is about and clears its
    /// stale mark, after someone confirmed it is still true. Returns `false` if it did not exist.
    fn reanchor_memory(&self, id: MemoryId) -> Result<bool, StorageError>;

    // ---- learning -------------------------------------------------------------------------

    /// The stored utility state of each target that has one. Targets without one are omitted.
    fn utility_states(
        &self,
        targets: &[Target],
    ) -> Result<Vec<(Target, UtilityState)>, StorageError>;

    /// Stores the utility state of a target, replacing any previous one.
    fn put_utility_state(&self, target: Target, state: UtilityState) -> Result<(), StorageError>;

    /// Appends one signal to the history, which is only used for counting and inspection.
    fn log_signal(
        &self,
        target: Target,
        kind: crate::SignalKind,
        now: i64,
    ) -> Result<(), StorageError>;

    /// Adds `weight` to the co-access strength of two symbols that were used together, after
    /// decaying what was stored with [`crate::decay_factor`] and
    /// [`crate::DEFAULT_HALF_LIFE_SECS`]. The pair is unordered.
    fn bump_coaccess(
        &self,
        a: SymbolId,
        b: SymbolId,
        weight: f64,
        now: i64,
    ) -> Result<(), StorageError>;

    /// The symbols most strongly co-accessed with `id`, with their strength decayed to `now`,
    /// strongest first.
    fn coaccess_neighbors(
        &self,
        id: SymbolId,
        limit: usize,
        now: i64,
    ) -> Result<Vec<(SymbolId, f64)>, StorageError>;

    /// Counts describing what has been learned.
    fn learning_status(&self) -> Result<LearningStatus, StorageError>;

    /// Forgets everything that was learned: utility states, signals and co-access. Symbols and
    /// memories are untouched.
    fn reset_learning(&self) -> Result<(), StorageError>;

    // ---- metadata -------------------------------------------------------------------------

    /// Reads a metadata value, such as the repository root or the time of the last index.
    fn get_meta(&self, key: &str) -> Result<Option<String>, StorageError>;

    /// Writes a metadata value.
    fn set_meta(&self, key: &str, value: &str) -> Result<(), StorageError>;
}
