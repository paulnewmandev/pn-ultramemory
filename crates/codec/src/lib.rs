// SPDX-License-Identifier: Apache-2.0
//! Token-budget capsule packing for pn-ultramemory.
//!
//! # Role in the architecture
//! Application layer (see `docs/architecture.md`). Depends on `pn-ultramemory-core` and on
//! `pn-ultramemory-toon` for output. It is deliberately independent of any real tokenizer,
//! storage engine or parser: callers hand it *costs* and *utilities* and get back *choices*, and
//! it prints the result.
//!
//! # What lives here
//! * [`estimate_tokens`]: a fast, calibrated estimate of what a text costs in tokens, used to price
//!   every option the packer can choose.
//! * [`Capsule`] and [`render`]: the context handed to an agent, and how it is written as TOON,
//!   JSON or text. [`symbol_text`] and [`symbol_cost`] define what a symbol looks like at each
//!   level and what it costs.
//! * [`pack`]: given candidates that can each be rendered at several levels of
//!   detail, choose one level per candidate so that total value is maximised
//!   without exceeding a token budget. See the [`packer`] module for the
//!   algorithm and its guarantees.

pub mod capsule;
pub mod packer;
pub mod tokens;

pub use capsule::{
    Capsule, CapsuleMemory, CapsuleRelation, CapsuleSymbol, Format, RenderOptions, SymbolView,
    measure, render, row_files, symbol_cost, symbol_text,
};
pub use packer::{Candidate, LevelOption, Packing, Selection, pack};
pub use tokens::estimate_tokens;
