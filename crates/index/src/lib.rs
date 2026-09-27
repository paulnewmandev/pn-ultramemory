// SPDX-License-Identifier: Apache-2.0
//! Source extraction with tree-sitter, documentation insertion and a file-system source tree for
//! pn-ultramemory.
//!
//! # Role in the architecture
//! This crate holds the exit adapters that turn a repository into data. It implements three
//! ports of `pn-ultramemory-core` and depends on nothing else in the workspace:
//!
//! * [`TreeSitterExtractor`] implements [`Extractor`](pn_ultramemory_core::Extractor). Twelve
//!   languages (Rust, Python, JavaScript, TypeScript, TSX, Go, Java, C, C++, C#, Ruby, PHP) are
//!   parsed with tree-sitter grammars; every other language listed in
//!   [`Language::Other`](pn_ultramemory_core::Language::Other) is read by a lexical fallback
//!   that finds declarations from their shape.
//! * [`DocComments`] implements [`DocInserter`](pn_ultramemory_core::DocInserter). It writes
//!   documentation in the syntax of each language and refuses to break the code.
//! * [`FsSourceTree`] implements [`SourceTree`](pn_ultramemory_core::SourceTree): it lists,
//!   reads and atomically writes the files of a repository on disk, and never leaves the root.
//!
//! # Invariants
//! * **No panics on any input.** Extraction is total: malformed source yields a best-effort
//!   result and a non-zero `parse_errors`. The tree is walked without recursion, so deeply
//!   nested input cannot overflow the stack, and every output size is bounded.
//! * **Deterministic.** The same input always gives the same output, in source order.
//! * **Local only.** No network access; the only file-system access is in [`FsSourceTree`].
//!
//! # Examples
//! ```
//! use pn_ultramemory_core::{Extractor, Language, RefKind, SymbolKind};
//! use pn_ultramemory_index::TreeSitterExtractor;
//!
//! let source = "\
//! /// Greets a user.
//! pub fn greet(name: &str) -> String {
//!     format(name)
//! }
//! ";
//! let file = TreeSitterExtractor::new().extract(Language::Rust, source).unwrap();
//! let greet = &file.symbols[0];
//! assert_eq!(greet.kind, SymbolKind::Function);
//! assert_eq!(greet.signature, "pub fn greet(name: &str) -> String");
//! assert_eq!(greet.doc.as_deref(), Some("Greets a user."));
//! assert!(file.references.iter().any(|r| r.kind == RefKind::Call && r.name == "format"));
//! ```

mod doc;
mod extract;
mod fallback;
mod tree;

pub use doc::DocComments;
pub use extract::TreeSitterExtractor;
pub use tree::FsSourceTree;
