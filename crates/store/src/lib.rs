// SPDX-License-Identifier: Apache-2.0
//! SQLite storage adapter for pn-ultramemory: the index, the memories and what was learned.
//!
//! # Role in the architecture
//! This crate is an *exit adapter* of the hexagon described in `docs/architecture.md`. It
//! implements the [`Storage`](pn_ultramemory_core::Storage) port of `pn-ultramemory-core` with
//! one type, [`SqliteStorage`], and depends on nothing else in the workspace. Use cases never see
//! SQLite: they receive a `&dyn Storage`.
//!
//! # Schema
//! The schema is created by the numbered files in `migrations/` (`0001_initial.sql`, then
//! `0002_report_totals.sql`) and versioned with `PRAGMA user_version`; migrations only move
//! forward, and a database with a newer version is refused. A database written by version 1 is
//! migrated in place, keeping everything it held. The tables below are the ones the migrations
//! leave.
//!
//! | Table | Holds |
//! |---|---|
//! | `files` | One row per indexed file: path, language, content hash, size, line count, and how many syntax errors the parser recovered from. |
//! | `symbols` | Every declared symbol. `id` is **stable**: `hash64` of the path, qualified name, kind and ordinal among same-key symbols of the file, masked to 63 bits. `seq` keeps source order. |
//! | `refs` | Unresolved references: file, owning symbol (nullable), name, kind, line, qualifier, and whether the owner may link to itself. Indexed by name. |
//! | `edges` | Resolved relationships `(src, dst, kind)` with a confidence (0 guess, 1 heuristic, 2 resolved, 3 exact) and a line. Unique per `(src, dst, kind)`, indexed in both directions, carrying the files at both ends (for module reports), and removed with their symbols by a trigger. |
//! | `memories`, `memory_anchors` | What was learned, and what each memory is about, with the hashes the symbol had at the time. |
//! | `symbol_fts`, `memory_fts` | FTS5 indexes over identifier-split tokens computed in Rust. |
//! | `utility`, `signals`, `coaccess` | Local learning: decayed usefulness, the signal history, pairs used together (`a < b`). |
//! | `meta`, `dirty_names` | Key/value metadata, and names whose set of definitions changed since the last resolve. |
//!
//! # Guarantees
//! * **Atomic**: every write operation is one transaction; a failure leaves nothing behind.
//! * **Stable identity**: re-indexing the same content gives the same [`SymbolId`]s, so edges,
//!   anchors and learned statistics keep pointing at the same symbols.
//! * **Deterministic**: every list has a total order; nothing depends on hash-map iteration.
//! * **Safe queries**: caller text is always bound as a parameter or rebuilt from word parts; it
//!   never becomes SQL or FTS5 syntax.
//! * **Local**: no network, no telemetry, one file (plus its WAL).
//!
//! [`SymbolId`]: pn_ultramemory_core::SymbolId

mod convert;
mod db;
mod edges;
mod error;
mod files;
mod ids;
mod learning;
mod memories;
mod meta;
mod query;
mod reports;
mod schema;
mod search;
mod symbols;
mod tokens;
mod upsert;

pub use db::SqliteStorage;
