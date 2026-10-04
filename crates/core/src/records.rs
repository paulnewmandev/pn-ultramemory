// SPDX-License-Identifier: Apache-2.0
//! The persistent shapes of the domain: files, symbols, edges and memories, as storage returns
//! them, together with the small request and result types of the storage port.

use core::fmt;

use crate::{Confidence, Language, MemoryKind, Provenance, Span, SymbolKind, Visibility};

/// Identity of an indexed file. Opaque to everything except the storage adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FileId(pub i64);

/// Identity of a symbol. It is stable across re-indexing as long as the symbol keeps its file,
/// qualified name and kind, so edges and memories survive edits elsewhere in the repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SymbolId(pub i64);

/// Identity of a stored memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MemoryId(pub i64);

impl fmt::Display for FileId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl fmt::Display for SymbolId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl fmt::Display for MemoryId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A file about to be stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileInput {
    /// Path relative to the repository root, always with forward slashes.
    pub path: String,
    /// The language the file was parsed as.
    pub language: Language,
    /// Hash of the file content, used to skip files that did not change.
    pub hash: String,
    /// Size in bytes.
    pub size: u64,
    /// Modification time, in seconds since the Unix epoch.
    pub mtime_secs: i64,
}

/// A stored file with a summary of what it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRecord {
    /// Identity of the file.
    pub id: FileId,
    /// Path relative to the repository root, with forward slashes.
    pub path: String,
    /// The language the file was parsed as.
    pub language: Language,
    /// Size in bytes.
    pub size: u64,
    /// Number of lines.
    pub lines: u32,
    /// Number of symbols stored for the file.
    pub symbol_count: u32,
}

/// A stored symbol.
#[derive(Debug, Clone, PartialEq)]
pub struct SymbolRecord {
    /// Identity of the symbol.
    pub id: SymbolId,
    /// The file that declares it.
    pub file_id: FileId,
    /// Path of that file, relative to the repository root.
    pub path: String,
    /// The language of that file.
    pub language: Language,
    /// The simple name.
    pub name: String,
    /// The name qualified by its enclosing symbols.
    pub qualified_name: String,
    /// What kind of element it is.
    pub kind: SymbolKind,
    /// The declaration on a single line.
    pub signature: String,
    /// The attached documentation, if any.
    pub doc: Option<String>,
    /// Whether the symbol is public.
    pub visibility: Visibility,
    /// Where the symbol is in its file.
    pub span: Span,
    /// The enclosing symbol, if any.
    pub parent: Option<SymbolId>,
    /// Distinct names called inside the symbol.
    pub outline: Vec<String>,
    /// Whitespace-normalized hash of the signature.
    pub sig_hash: u64,
    /// Whitespace-normalized hash of the whole declaration.
    pub body_hash: u64,
    /// Precomputed PageRank score (0..=1), populated during full index. Zero means not yet scored.
    pub pagerank: f64,
}

/// What an edge between two symbols means.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EdgeKind {
    /// The source calls the target.
    Calls,
    /// The source inherits from, implements or extends the target.
    Inherits,
    /// The source mentions the target as a type.
    Uses,
}

impl EdgeKind {
    /// Stable lowercase name used in storage and in output.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Calls => "calls",
            Self::Inherits => "inherits",
            Self::Uses => "uses",
        }
    }

    /// Looks a kind up by the name returned from [`EdgeKind::as_str`].
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        [Self::Calls, Self::Inherits, Self::Uses]
            .into_iter()
            .find(|k| k.as_str() == name)
    }
}

/// A directed relationship between two symbols, with how sure the indexer is about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EdgeRecord {
    /// The symbol the relationship starts from.
    pub src: SymbolId,
    /// The symbol it points to.
    pub dst: SymbolId,
    /// What the relationship means.
    pub kind: EdgeKind,
    /// How sure the indexer is that the relationship exists.
    pub confidence: Confidence,
    /// The 1-based line in the source file where it was found.
    pub line: u32,
}

/// Which way to follow edges from a symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Edges that start at the symbol: what it calls or extends.
    Out,
    /// Edges that end at the symbol: who calls or extends it.
    In,
}

/// A symbol reached by following one edge, together with that edge.
#[derive(Debug, Clone, PartialEq)]
pub struct Neighbor {
    /// The symbol at the other end of the edge.
    pub symbol: SymbolRecord,
    /// The edge that led to it.
    pub edge: EdgeRecord,
}

/// A request to search symbols by text.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SearchQuery {
    /// Free text: identifiers, words from documentation, parts of a signature.
    pub text: String,
    /// Restrict results to these kinds. Empty means every kind.
    pub kinds: Vec<SymbolKind>,
    /// Restrict results to paths that start with this prefix, if given.
    pub path_prefix: Option<String>,
    /// Match symbols that carry any word of the text instead of every word. The score then adds up
    /// over the words matched, each weighed by how rare it is in the index, so a symbol that
    /// matches the two rare words of a question ranks above one that matches its one common word.
    pub any_word: bool,
}

/// A symbol that matched a search, with its relevance.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    /// The matching symbol.
    pub symbol: SymbolRecord,
    /// Relevance of the match. Higher is better. Only comparable within one result list.
    pub score: f64,
}

/// Why a memory went stale: what changed in the code it was anchored to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StaleReason {
    /// The symbol no longer exists anywhere in the index.
    SymbolDeleted,
    /// The symbol's signature (name, parameters, return type) changed.
    SignatureChanged,
    /// Only the symbol's body changed; its signature is intact.
    BodyChanged,
    /// The file that held the symbol was removed from the repository.
    FileRemoved,
}

impl StaleReason {
    /// Stable lowercase name used in storage and output.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SymbolDeleted => "symbol_deleted",
            Self::SignatureChanged => "signature_changed",
            Self::BodyChanged => "body_changed",
            Self::FileRemoved => "file_removed",
        }
    }

    /// Looks a reason up by the name returned from [`StaleReason::as_str`].
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "symbol_deleted" => Some(Self::SymbolDeleted),
            "signature_changed" => Some(Self::SignatureChanged),
            "body_changed" => Some(Self::BodyChanged),
            "file_removed" => Some(Self::FileRemoved),
            _ => None,
        }
    }
}

/// A link between a memory and a symbol, remembering what the symbol looked like at that moment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Anchor {
    /// The symbol the memory is about, if it still exists.
    pub symbol: Option<SymbolId>,
    /// The qualified name of the symbol when the memory was written.
    pub qualified_name: String,
    /// The path of the symbol's file when the memory was written.
    pub path: String,
    /// The signature hash when the memory was written.
    pub sig_hash: u64,
    /// The declaration hash when the memory was written.
    pub body_hash: u64,
}

impl Anchor {
    /// Decides whether the code this anchor points to has changed since the memory was written.
    ///
    /// `current` holds the signature and declaration hashes of the symbol as it is now, or `None`
    /// if the symbol no longer exists. A missing symbol or either hash differing means stale.
    ///
    /// # Examples
    /// ```
    /// use pn_ultramemory_core::Anchor;
    ///
    /// let anchor = Anchor {
    ///     symbol: None,
    ///     qualified_name: "f".into(),
    ///     path: "a.rs".into(),
    ///     sig_hash: 1,
    ///     body_hash: 2,
    /// };
    /// assert!(!anchor.is_stale_against(Some((1, 2))));
    /// assert!(anchor.is_stale_against(Some((1, 3))));
    /// assert!(anchor.is_stale_against(None));
    /// ```
    #[must_use]
    pub const fn is_stale_against(&self, current: Option<(u64, u64)>) -> bool {
        self.stale_reason_against(current).is_some()
    }

    /// Returns why the anchor is stale, or `None` when the code still matches.
    ///
    /// This is the richer version of [`is_stale_against`](Self::is_stale_against): instead of a
    /// bare boolean it names the change so a brief can tell an agent *what* moved.
    ///
    /// # Examples
    /// ```
    /// use pn_ultramemory_core::{Anchor, StaleReason};
    ///
    /// let anchor = Anchor {
    ///     symbol: None,
    ///     qualified_name: "f".into(),
    ///     path: "a.rs".into(),
    ///     sig_hash: 1,
    ///     body_hash: 2,
    /// };
    /// assert_eq!(anchor.stale_reason_against(Some((1, 2))), None);
    /// assert_eq!(anchor.stale_reason_against(Some((9, 2))), Some(StaleReason::SignatureChanged));
    /// assert_eq!(anchor.stale_reason_against(Some((1, 9))), Some(StaleReason::BodyChanged));
    /// assert_eq!(anchor.stale_reason_against(None), Some(StaleReason::SymbolDeleted));
    /// ```
    #[must_use]
    pub const fn stale_reason_against(&self, current: Option<(u64, u64)>) -> Option<StaleReason> {
        match current {
            None => Some(StaleReason::SymbolDeleted),
            Some((sig_hash, body_hash)) => {
                if sig_hash != self.sig_hash {
                    Some(StaleReason::SignatureChanged)
                } else if body_hash != self.body_hash {
                    Some(StaleReason::BodyChanged)
                } else {
                    None
                }
            }
        }
    }
}

/// A stored memory.
#[derive(Debug, Clone, PartialEq)]
pub struct MemoryRecord {
    /// Identity of the memory.
    pub id: MemoryId,
    /// What kind of memory it is.
    pub kind: MemoryKind,
    /// The text of the memory.
    pub text: String,
    /// Who produced it.
    pub provenance: Provenance,
    /// When it was written, in seconds since the Unix epoch.
    pub created_at: i64,
    /// When the code it describes first changed, if it did. `None` means still fresh.
    pub stale_since: Option<i64>,
    /// Why the memory went stale, when known. `None` means fresh or legacy (pre-v5).
    pub stale_reason: Option<StaleReason>,
    /// The symbols it is about.
    pub anchors: Vec<Anchor>,
}

/// A memory about to be stored.
#[derive(Debug, Clone, PartialEq)]
pub struct NewMemory {
    /// What kind of memory it is.
    pub kind: MemoryKind,
    /// The text of the memory.
    pub text: String,
    /// Who produced it.
    pub provenance: Provenance,
    /// The symbols it is about. Storage looks each up to record its current hashes.
    pub about: Vec<SymbolId>,
}

/// A memory suggested automatically from observed agent behavior, awaiting confirmation.
///
/// Drafts are created when the engine detects that an expand-after-recall pattern likely
/// resolved the agent's query, and surfaced in `brief` so the next session can accept or
/// discard them without restating what already happened. They never participate in ranking
/// until confirmed through [`NewMemory`] with [`Provenance::Auto`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftMemory {
    /// Identity of the draft, assigned by storage on insert.
    pub id: i64,
    /// What kind of memory this would become if confirmed.
    pub kind: MemoryKind,
    /// The suggested text.
    pub text: String,
    /// Qualified names of the symbols this draft is about, comma-separated for storage.
    pub about_symbols: Vec<String>,
    /// When the draft was suggested, in seconds since the Unix epoch.
    pub suggested_at: i64,
    /// The recall query that led to this suggestion, if any.
    pub source_query: String,
}

/// Which memories to list.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MemoryFilter {
    /// Only this kind, if given.
    pub kind: Option<MemoryKind>,
    /// Only memories whose code changed.
    pub only_stale: bool,
    /// Maximum number of memories to return. Zero means the storage default.
    pub limit: usize,
}

/// Counts describing an index.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IndexStats {
    /// Number of indexed files.
    pub files: u64,
    /// Number of symbols.
    pub symbols: u64,
    /// Number of edges.
    pub edges: u64,
    /// Number of memories.
    pub memories: u64,
    /// Number of memories whose code changed.
    pub stale_memories: u64,
    /// Files per language name, most files first.
    pub languages: Vec<(String, u64)>,
}

/// A group of files that share a directory prefix, with how connected it is to the rest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleStats {
    /// The directory prefix that names the module, such as `crates/core`, or `.` for files at the
    /// root of the repository.
    pub name: String,
    /// Number of files in the module.
    pub files: u64,
    /// Number of symbols declared in the module.
    pub symbols: u64,
    /// Edges that start in another module and end in this one.
    pub incoming: u64,
    /// Edges that start in this module and end in another one.
    pub outgoing: u64,
}

/// The total weight of the edges that go from one module to another.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleEdge {
    /// The module the edges start in.
    pub from: String,
    /// The module the edges end in.
    pub to: String,
    /// Number of symbol-to-symbol edges between the two modules.
    pub weight: u64,
}

/// How many public symbols of one language are documented.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocCoverageRow {
    /// The language name.
    pub language: String,
    /// Public symbols, not counting modules.
    pub public_symbols: u64,
    /// Public symbols that have documentation.
    pub documented: u64,
}

/// Totals over every indexed file that [`crate::IndexStats`] does not carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FileTotals {
    /// Total number of lines.
    pub lines: u64,
    /// Number of files the parser had to recover errors in.
    pub parse_error_files: u64,
}

/// What changed when one file was stored.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UpsertOutcome {
    /// Identity of the stored file.
    pub file_id: Option<FileId>,
    /// Symbols that did not exist before.
    pub symbols_added: u32,
    /// Symbols that existed before and are gone.
    pub symbols_removed: u32,
    /// Symbols that kept their identity but whose signature or declaration changed.
    pub symbols_modified: u32,
    /// Memories that became stale because of this change.
    pub memories_marked_stale: u32,
    /// The simple names of every added, removed or modified symbol, without duplicates.
    pub changed_names: Vec<String>,
}

/// Which references to resolve into edges.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveScope {
    /// Every reference in the index.
    All,
    /// Only references that belong to one of these files, or that mention one of these names.
    Touching {
        /// Files whose references must be resolved again.
        file_ids: Vec<FileId>,
        /// Names whose definitions changed, so every reference to them must be resolved again.
        names: Vec<String>,
    },
}

/// What resolving references produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ResolveStats {
    /// Edges written.
    pub edges_written: u64,
}

/// Counts describing what has been learned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LearningStatus {
    /// Signals recorded.
    pub signals: u64,
    /// Symbols and memories that have a utility estimate.
    pub tracked_targets: u64,
    /// Pairs of symbols that were used together.
    pub coaccess_pairs: u64,
}

#[cfg(test)]
mod tests {
    use super::{Anchor, EdgeKind};

    /// Edge kind names round-trip.
    #[test]
    fn edge_kind_names_round_trip() {
        for kind in [EdgeKind::Calls, EdgeKind::Inherits, EdgeKind::Uses] {
            assert_eq!(EdgeKind::from_name(kind.as_str()), Some(kind));
        }
        assert_eq!(EdgeKind::from_name("nope"), None);
    }

    /// An anchor is stale when the symbol vanished or either hash differs.
    #[test]
    fn anchor_staleness_rules() {
        let anchor = Anchor {
            symbol: None,
            qualified_name: "A::f".into(),
            path: "a.rs".into(),
            sig_hash: 7,
            body_hash: 9,
        };
        assert!(!anchor.is_stale_against(Some((7, 9))));
        assert!(anchor.is_stale_against(Some((8, 9))));
        assert!(anchor.is_stale_against(Some((7, 10))));
        assert!(anchor.is_stale_against(None));
    }
}
