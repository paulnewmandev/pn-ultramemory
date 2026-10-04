// SPDX-License-Identifier: Apache-2.0
//! Pure domain vocabulary and ports of pn-ultramemory.
//!
//! # Role in the architecture
//! This crate is the centre of the hexagon described in `docs/architecture.md`.
//! It performs **no I/O** and depends on **no other workspace crate and no external crate**;
//! every other crate depends inward on it. Keeping it free of storage, parsing and transport
//! concerns is what lets those be swapped without touching the rules here.
//!
//! # Contents
//! * Vocabulary: [`Confidence`], [`Detail`], [`MemoryKind`], [`Provenance`], [`Language`],
//!   [`SymbolKind`], [`Visibility`], [`Span`].
//! * What extraction reports: [`FileExtract`], [`SymbolDraft`], [`ReferenceDraft`].
//! * What storage returns: [`SymbolRecord`], [`EdgeRecord`], [`MemoryRecord`], [`Anchor`] and the
//!   request and result types around them.
//! * Learning rules: [`SignalKind`], [`UtilityState`], [`Target`].
//! * Ports: [`Extractor`], [`DocInserter`], [`SourceTree`], [`Storage`], [`Clock`].
//! * Hashing helpers: [`hash64`], [`hash_normalized`].

mod confidence;
mod detail;
mod hash;
mod language;
mod learning;
mod memory;
mod ports;
mod records;
mod symbol;

pub use confidence::Confidence;
pub use detail::Detail;
pub use hash::{hash_normalized, hash64};
pub use language::Language;
pub use learning::{DEFAULT_HALF_LIFE_SECS, SignalKind, Target, UtilityState, decay_factor};
pub use memory::{MemoryKind, Provenance};
pub use ports::{
    Clock, DocError, DocInserter, DocTarget, ExtractError, Extractor, SourceError, SourceFile,
    SourceTree, Storage, StorageError,
};
pub use records::{
    Anchor, Direction, DocCoverageRow, DraftMemory, EdgeKind, EdgeRecord, FileId, FileInput,
    FileRecord, FileTotals, IndexStats, LearningStatus, MemoryFilter, MemoryId, MemoryRecord,
    ModuleEdge, ModuleStats, Neighbor, NewMemory, ResolveScope, ResolveStats, SearchHit,
    SearchQuery, StaleReason, SymbolId, SymbolRecord, UpsertOutcome,
};
pub use symbol::{
    EXPRESSION_QUALIFIER, FileExtract, RefKind, ReferenceDraft, Span, SymbolDraft, SymbolKind,
    Visibility, first_sentence,
};
