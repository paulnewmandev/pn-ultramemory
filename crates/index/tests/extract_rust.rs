// SPDX-License-Identifier: Apache-2.0
//! Extraction tests for Rust, driven by the fixture `fixtures/rust_sample.rs`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests unwrap and panic to fail loudly"
)]

#[allow(
    dead_code,
    reason = "each test binary uses a different subset of the shared helpers"
)]
mod common;

use std::fmt::Write;

use common::{assert_ref, extract, extract_clean, index_of, refs_owned_by, symbol};
use pn_ultramemory_core::{Language, RefKind, SymbolKind, Visibility};

/// Every symbol of the fixture is found, in source order, with the right kind and parent.
#[test]
fn symbols_kinds_and_parents() {
    let (file, _) = extract_clean(Language::Rust, "rust_sample.rs");
    let listing: Vec<(String, SymbolKind, Option<usize>)> = file
        .symbols
        .iter()
        .map(|s| (s.qualified_name.clone(), s.kind, s.parent))
        .collect();
    let expect = |name: &str, kind: SymbolKind, parent: Option<&str>| {
        let parent = parent.map(|p| index_of(&file, p));
        assert!(
            listing
                .iter()
                .any(|(n, k, p)| n == name && *k == kind && *p == parent),
            "expected {kind} {name} with parent {parent:?} in {listing:?}"
        );
    };
    expect("MAX_ITEMS", SymbolKind::Constant, None);
    expect("SECRET_SEED", SymbolKind::Constant, None);
    expect("COUNTER", SymbolKind::Constant, None);
    expect("Sku", SymbolKind::Struct, None);
    expect("Priced", SymbolKind::Interface, None);
    expect("Priced::cents", SymbolKind::Method, Some("Priced"));
    expect("Priced::with_tax", SymbolKind::Method, Some("Priced"));
    expect("Kind", SymbolKind::Enum, None);
    expect("Inventory", SymbolKind::Struct, None);
    expect("Inventory::new", SymbolKind::Method, Some("Inventory"));
    expect("Inventory::add", SymbolKind::Method, Some("Inventory"));
    expect("Sku::fmt", SymbolKind::Method, Some("Sku"));
    expect("log_change", SymbolKind::Function, None);
    expect("report", SymbolKind::Module, None);
    expect("report::render", SymbolKind::Function, Some("report"));
    expect(
        "report::private_helper",
        SymbolKind::Function,
        Some("report"),
    );
    expect("sku", SymbolKind::Macro, None);
    expect("Table", SymbolKind::Type, None);
    expect("main", SymbolKind::Function, None);
    assert_eq!(file.symbols.len(), 21);
    assert_eq!(file.line_count, 105);
}

/// Signatures keep modifiers and generics, drop bodies and attributes, and fit on one line.
#[test]
fn signatures() {
    let (file, _) = extract_clean(Language::Rust, "rust_sample.rs");
    assert_eq!(
        symbol(&file, "MAX_ITEMS").signature,
        "pub const MAX_ITEMS: usize = 1024"
    );
    assert_eq!(symbol(&file, "Sku").signature, "pub struct Sku");
    assert_eq!(
        symbol(&file, "Priced").signature,
        "pub trait Priced: Display + Send"
    );
    assert_eq!(
        symbol(&file, "Priced::cents").signature,
        "fn cents(&self) -> u64"
    );
    assert_eq!(
        symbol(&file, "Inventory").signature,
        "pub(crate) struct Inventory<T: Priced>"
    );
    assert_eq!(
        symbol(&file, "Inventory::add").signature,
        "pub fn add(&mut self, key: &str, item: T) -> Result<(), Error>"
    );
    assert_eq!(symbol(&file, "report").signature, "pub mod report");
    assert_eq!(symbol(&file, "sku").signature, "macro_rules! sku");
    assert_eq!(
        symbol(&file, "Table").signature,
        "type Table = BTreeMap<String, Sku>"
    );
    assert!(
        !symbol(&file, "Inventory::add")
            .signature
            .contains("must_use")
    );
}

/// Documentation is attached across attributes, joined, and stripped of its markers.
#[test]
fn documentation() {
    let (file, source) = extract_clean(Language::Rust, "rust_sample.rs");
    assert_eq!(
        symbol(&file, "Sku").doc.as_deref(),
        Some("A stock keeping unit.\n\nIdentifies one product.")
    );
    assert_eq!(
        symbol(&file, "MAX_ITEMS").doc.as_deref(),
        Some("Maximum number of items an inventory holds.")
    );
    assert_eq!(
        symbol(&file, "Inventory::add").doc.as_deref(),
        Some("Adds an item unless the inventory is full.")
    );
    assert_eq!(
        symbol(&file, "report::render").doc.as_deref(),
        Some("Renders an inventory.")
    );
    assert_eq!(
        symbol(&file, "Priced::cents").doc.as_deref(),
        Some("The price in cents.")
    );
    assert_eq!(symbol(&file, "SECRET_SEED").doc, None);
    assert_eq!(symbol(&file, "Kind").doc, None);
    assert_eq!(
        symbol(&file, "report").doc,
        None,
        "inner documentation is not attached to the module"
    );
    // The span of a documented item starts at its documentation comment (and covers the
    // attributes in between).
    let sku = symbol(&file, "Sku");
    let slice = &source[sku.span.start_byte as usize..sku.span.end_byte as usize];
    assert!(slice.starts_with("/// A stock keeping unit."));
    assert!(slice.contains("#[derive(Debug, Clone, PartialEq)]"));
    assert!(slice.trim_end().ends_with('}'));
    let add = symbol(&file, "Inventory::add");
    let slice = &source[add.span.start_byte as usize..add.span.end_byte as usize];
    assert!(slice.starts_with("/// Adds an item"));
    assert_eq!(add.span.start_line, 51);
    assert_eq!(add.span.end_line, 65);
}

/// Visibility follows `pub` forms; trait members inherit the visibility of their trait.
#[test]
fn visibility() {
    let (file, _) = extract_clean(Language::Rust, "rust_sample.rs");
    let vis = |name: &str| symbol(&file, name).visibility;
    assert_eq!(vis("MAX_ITEMS"), Visibility::Public);
    assert_eq!(vis("SECRET_SEED"), Visibility::Private);
    assert_eq!(vis("Sku"), Visibility::Public);
    assert_eq!(
        vis("Inventory"),
        Visibility::Public,
        "pub(crate) is a pub form"
    );
    assert_eq!(vis("Inventory::new"), Visibility::Public);
    assert_eq!(vis("Inventory::total"), Visibility::Private);
    assert_eq!(vis("Priced::hidden_in_trait"), Visibility::Public);
    assert_eq!(vis("report::private_helper"), Visibility::Private);
    assert_eq!(vis("Table"), Visibility::Private);
    assert_eq!(vis("main"), Visibility::Private);
}

/// The outline lists distinct callee names in order of first appearance.
#[test]
fn outlines() {
    let (file, _) = extract_clean(Language::Rust, "rust_sample.rs");
    assert_eq!(
        symbol(&file, "Inventory::add").outline,
        ["len", "Err", "insert", "to_owned", "log_change", "Ok"]
    );
    assert_eq!(
        symbol(&file, "Inventory::total").outline,
        ["values", "map", "cents", "sum"]
    );
    assert_eq!(
        symbol(&file, "log_change").outline,
        ["println", "global", "record"]
    );
    assert_eq!(symbol(&file, "main").outline, ["new", "add", "sku", "ok"]);
    assert!(symbol(&file, "Sku").outline.is_empty());
}

/// Calls, macro invocations, type mentions and inheritance are reported with owner and qualifier.
#[test]
fn references() {
    let (file, _) = extract_clean(Language::Rust, "rust_sample.rs");
    assert_ref(
        &file,
        "log_change",
        RefKind::Call,
        Some("Inventory::add"),
        None,
    );
    assert_ref(
        &file,
        "insert",
        RefKind::Call,
        Some("Inventory::add"),
        Some("self.seen"),
    );
    assert_ref(&file, "println", RefKind::Call, Some("log_change"), None);
    assert_ref(
        &file,
        "global",
        RefKind::Call,
        Some("log_change"),
        Some("Backend"),
    );
    assert_ref(
        &file,
        "new",
        RefKind::Call,
        Some("Inventory::new"),
        Some("BTreeMap"),
    );
    assert_ref(
        &file,
        "new",
        RefKind::Call,
        Some("COUNTER"),
        Some("AtomicUsize"),
    );
    assert_ref(&file, "sku", RefKind::Call, Some("main"), None);
    assert_ref(&file, "format", RefKind::Call, Some("report::render"), None);
    assert_ref(
        &file,
        "cents",
        RefKind::Call,
        Some("Priced::with_tax"),
        Some("self"),
    );
    // Type mentions in signatures and bodies.
    assert_ref(&file, "Result", RefKind::Type, Some("Inventory::add"), None);
    assert_ref(&file, "Error", RefKind::Type, Some("Inventory::add"), None);
    assert_ref(
        &file,
        "Formatter",
        RefKind::Type,
        Some("Sku::fmt"),
        Some("fmt"),
    );
    assert_ref(&file, "Inventory", RefKind::Type, Some("main"), None);
    assert_ref(&file, "BTreeMap", RefKind::Type, Some("Table"), None);
    // Inheritance: supertraits, and `impl Trait for Type` owned by the type.
    assert_ref(&file, "Display", RefKind::Inherit, Some("Priced"), None);
    assert_ref(&file, "Send", RefKind::Inherit, Some("Priced"), None);
    assert_ref(&file, "Display", RefKind::Inherit, Some("Sku"), None);
    // The implemented type itself is not reported as a mention of itself in its impl header.
    assert!(refs_owned_by(&file, "Sku", RefKind::Type, "Sku").is_empty());
    for r in &file.references {
        assert!(r.line >= 1 && r.line <= 105);
    }
}

/// Imports are expanded to one path per imported item.
#[test]
fn imports() {
    let (file, _) = extract_clean(Language::Rust, "rust_sample.rs");
    assert_eq!(
        file.imports,
        [
            "std::collections::BTreeMap",
            "std::collections::HashSet",
            "std::fmt",
            "std::fmt::Display",
            "crate::store::Store",
            "super::Inventory",
        ]
    );
}

/// An `impl` block for a type declared later has no parent but keeps its qualified name.
#[test]
fn impl_before_type_has_no_parent() {
    let source = "impl Later {\n    fn go(&self) {}\n}\nstruct Later;\n";
    let file = extract(Language::Rust, source);
    let go = symbol(&file, "Later::go");
    assert_eq!(go.parent, None);
    assert_eq!(go.kind, SymbolKind::Method);
}

/// Malformed input reports syntax errors but still yields the symbols that could be read.
#[test]
fn malformed_source_reports_errors() {
    let source = "fn good() {}\nfn bad( {\nfn also_good() { helper(); }\n";
    let file = extract(Language::Rust, source);
    assert!(file.parse_errors > 0);
    assert!(file.symbols.iter().any(|s| s.name == "good"));
}

/// Deeply nested modules stop being listed after the nesting limit instead of failing.
#[test]
fn nesting_is_bounded() {
    let depth = 200;
    let mut source = String::new();
    for i in 0..depth {
        write!(source, "mod m{i} {{ ").unwrap();
    }
    source.push_str("fn deepest() {}");
    source.push_str(&"}".repeat(depth));
    let file = extract(Language::Rust, &source);
    assert!(file.symbols.len() <= 65);
    assert!(file.symbols.len() >= 60);
}
