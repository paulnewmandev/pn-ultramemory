// SPDX-License-Identifier: Apache-2.0
//! The use cases of pn-ultramemory: what the tool does, independent of how it stores data, parses
//! code or talks to an agent.
//!
//! # Role in the architecture
//! Application layer (see `docs/architecture.md`). Everything here depends only on the ports of
//! `pn-ultramemory-core` ([`pn_ultramemory_core::Storage`], [`pn_ultramemory_core::Extractor`],
//! [`pn_ultramemory_core::SourceTree`], [`pn_ultramemory_core::DocInserter`],
//! [`pn_ultramemory_core::Clock`]), on `pn-ultramemory-codec` for packing and rendering and on
//! `pn-ultramemory-toon` for output. Adapters (SQLite, tree-sitter, the file system) are wired in
//! by the caller through [`Deps`], and appear here only as development dependencies of the tests.
//!
//! # The operations
//! One [`Engine`] value offers every operation as a method:
//!
//! * indexing: [`Engine::index`];
//! * retrieval: [`Engine::recall`], [`Engine::expand`], [`Engine::impact`], [`Engine::graph`],
//!   [`Engine::brain`], [`Engine::repo_map`];
//! * memory and learning: [`Engine::remember`], [`Engine::memories`], [`Engine::feedback`];
//! * documentation: [`Engine::doc_gaps`], [`Engine::doc_apply`], [`Engine::doc_markdown`];
//! * analytics: [`Engine::stats`], [`Engine::insights`], [`Engine::bench`].
//!
//! Every result is a plain value with a `to_value` method that returns a `serde_json::Value`
//! shaped so that TOON prints uniform lists as compact tables.

mod bench;
mod brain;
mod brief;
mod config;
mod docs;
mod engine;
mod error;
mod expand;
mod graph;
mod guard;
mod impact;
mod indexer;
mod insights;
mod learn;
mod map;
mod memory;
mod metrics;
mod naming;
mod outline;
mod recall;
mod render;
mod sources;
mod stats;

pub use bench::*;
pub use brain::*;
pub use brief::*;
pub use config::EngineConfig;
pub use docs::*;
pub use engine::{Deps, Engine, SystemClock};
pub use error::EngineError;
pub use expand::*;
pub use graph::*;
pub use impact::*;
pub use indexer::{IndexOptions, IndexReport, content_hash};
pub use insights::*;
pub use map::*;
pub use memory::tongue::{Tongue, detect_tongue};
pub use memory::*;
pub use outline::*;
pub use recall::*;
pub use render::*;
pub use stats::*;
