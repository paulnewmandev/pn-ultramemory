// SPDX-License-Identifier: Apache-2.0
//! Extraction tests for Go, driven by the fixture `fixtures/go_sample.go`.
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

use common::{assert_ref, extract, extract_clean, index_of, symbol};
use pn_ultramemory_core::{Language, RefKind, SymbolKind, Visibility};

/// Types, functions, methods, constants and variables are listed with the right kinds; methods
/// are qualified by their receiver and attached to the receiver type.
#[test]
fn symbols_kinds_and_parents() {
    let (file, _) = extract_clean(Language::Go, "go_sample.go");
    let expect = |name: &str, kind: SymbolKind, parent: Option<&str>| {
        let s = symbol(&file, name);
        assert_eq!(s.kind, kind, "{name}");
        assert_eq!(s.parent, parent.map(|p| index_of(&file, p)), "{name}");
    };
    expect("MaxLines", SymbolKind::Constant, None);
    expect("statusOpen", SymbolKind::Constant, None);
    expect("StatusPaid", SymbolKind::Constant, None);
    expect("DefaultCurrency", SymbolKind::Variable, None);
    expect("cache", SymbolKind::Variable, None);
    expect("Debug", SymbolKind::Variable, None);
    expect("Priced", SymbolKind::Interface, None);
    expect("Priced.Price", SymbolKind::Method, Some("Priced"));
    expect("Line", SymbolKind::Struct, None);
    expect("Invoice", SymbolKind::Struct, None);
    expect("Currency", SymbolKind::Type, None);
    expect("Alias", SymbolKind::Type, None);
    expect("handler", SymbolKind::Type, None);
    expect("New", SymbolKind::Function, None);
    expect("Invoice.Add", SymbolKind::Method, Some("Invoice"));
    expect("Invoice.recompute", SymbolKind::Method, Some("Invoice"));
    expect("Format", SymbolKind::Function, None);
    expect("helper", SymbolKind::Function, None);
    assert_eq!(file.symbols.len(), 18);
    assert!(
        file.symbols.iter().all(|s| s.name != "billing"),
        "the package clause is not a symbol"
    );
}

/// Visibility follows capitalization.
#[test]
fn visibility() {
    let (file, _) = extract_clean(Language::Go, "go_sample.go");
    let vis = |name: &str| symbol(&file, name).visibility;
    assert_eq!(vis("MaxLines"), Visibility::Public);
    assert_eq!(vis("statusOpen"), Visibility::Private);
    assert_eq!(vis("Invoice"), Visibility::Public);
    assert_eq!(vis("Invoice.Add"), Visibility::Public);
    assert_eq!(vis("Invoice.recompute"), Visibility::Private);
    assert_eq!(vis("handler"), Visibility::Private);
    assert_eq!(vis("helper"), Visibility::Private);
    assert_eq!(vis("cache"), Visibility::Private);
    assert_eq!(vis("Debug"), Visibility::Public);
}

/// Signatures keep the receiver and type parameters and drop bodies.
#[test]
fn signatures() {
    let (file, _) = extract_clean(Language::Go, "go_sample.go");
    assert_eq!(
        symbol(&file, "New").signature,
        "func New(customer *Customer) *Invoice"
    );
    assert_eq!(
        symbol(&file, "Invoice.Add").signature,
        "func (inv *Invoice) Add(l Line) error"
    );
    assert_eq!(
        symbol(&file, "Invoice.recompute").signature,
        "func (inv Invoice) recompute()"
    );
    assert_eq!(
        symbol(&file, "Format").signature,
        "func Format[T Priced](items []T) string"
    );
    assert_eq!(symbol(&file, "Invoice").signature, "type Invoice struct");
    assert_eq!(symbol(&file, "Priced").signature, "type Priced interface");
    assert_eq!(symbol(&file, "Currency").signature, "type Currency string");
    assert_eq!(symbol(&file, "Alias").signature, "type Alias = Invoice");
    assert_eq!(
        symbol(&file, "handler").signature,
        "type handler func(*Invoice) error"
    );
    assert_eq!(symbol(&file, "MaxLines").signature, "const MaxLines = 500");
    assert_eq!(
        symbol(&file, "DefaultCurrency").signature,
        "var DefaultCurrency = \"EUR\""
    );
    assert_eq!(symbol(&file, "Priced.Price").signature, "Price() int64");
}

/// Documentation is the comment block above the declaration; the package comment is not
/// attached to anything and a blank line detaches a comment.
#[test]
fn documentation() {
    let (file, source) = extract_clean(Language::Go, "go_sample.go");
    assert_eq!(
        symbol(&file, "MaxLines").doc.as_deref(),
        Some("MaxLines is the largest number of lines on one invoice.")
    );
    assert_eq!(
        symbol(&file, "Priced").doc.as_deref(),
        Some("Priced is implemented by everything that has a price.")
    );
    assert_eq!(
        symbol(&file, "Invoice").doc.as_deref(),
        Some("Invoice groups lines for one customer.")
    );
    assert_eq!(
        symbol(&file, "New").doc.as_deref(),
        Some("New creates an empty invoice.")
    );
    assert_eq!(
        symbol(&file, "Invoice.Add").doc.as_deref(),
        Some("Add appends a line and updates the total.")
    );
    assert_eq!(
        symbol(&file, "Currency").doc.as_deref(),
        Some("Currency is an ISO currency code.")
    );
    assert_eq!(symbol(&file, "helper").doc, None);
    assert_eq!(symbol(&file, "Alias").doc, None);
    let new = symbol(&file, "New");
    let slice = &source[new.span.start_byte as usize..new.span.end_byte as usize];
    assert!(slice.starts_with("// New creates an empty invoice.\nfunc New"));
}

/// Calls carry the package or receiver as qualifier; embedded types are inheritance.
#[test]
fn references() {
    let (file, _) = extract_clean(Language::Go, "go_sample.go");
    assert_ref(&file, "Println", RefKind::Call, Some("New"), Some("fmt"));
    assert_ref(
        &file,
        "New",
        RefKind::Call,
        Some("Invoice.Add"),
        Some("errors"),
    );
    assert_ref(
        &file,
        "recompute",
        RefKind::Call,
        Some("Invoice.Add"),
        Some("inv"),
    );
    assert_ref(
        &file,
        "ToUpper",
        RefKind::Call,
        Some("Invoice.recompute"),
        Some("str"),
    );
    assert_ref(&file, "Getenv", RefKind::Call, Some("Format"), Some("os"));
    assert_ref(&file, "Customer", RefKind::Type, Some("New"), None);
    assert_ref(&file, "Invoice", RefKind::Type, Some("New"), None);
    assert_ref(&file, "Line", RefKind::Type, Some("Invoice.Add"), None);
    assert_ref(&file, "Priced", RefKind::Type, Some("Format"), None);
    // Struct embedding and interface embedding.
    assert_ref(&file, "Line", RefKind::Inherit, Some("Invoice"), None);
    assert_ref(&file, "Customer", RefKind::Inherit, Some("Invoice"), None);
    assert_ref(
        &file,
        "Stringer",
        RefKind::Inherit,
        Some("Priced"),
        Some("fmt"),
    );
    // Predeclared identifiers and builtins are not reported.
    for name in ["int64", "string", "error", "len", "append", "int"] {
        assert!(
            !file.references.iter().any(|r| r.name == name),
            "{name} must not be reported"
        );
    }
    assert_eq!(symbol(&file, "Invoice.Add").outline, ["New", "recompute"]);
}

/// Import paths are reported without their quotes; aliases and blank imports included.
#[test]
fn imports() {
    let (file, _) = extract_clean(Language::Go, "go_sample.go");
    assert_eq!(file.imports, ["errors", "fmt", "strings", "embed", "os"]);
}

/// A method whose receiver type is declared later has no parent but a qualified name.
#[test]
fn method_before_type() {
    let source = "package p\nfunc (t *T) Run() {}\ntype T struct{}\n";
    let file = extract(Language::Go, source);
    let run = symbol(&file, "T.Run");
    assert_eq!(run.parent, None);
    assert_eq!(run.kind, SymbolKind::Method);
}

/// Compiler directives are not documentation, and generic receivers resolve to their type.
#[test]
fn directives_and_generic_receivers() {
    let source = "package p\n//go:generate stringer\ntype Box[T any] struct{ v T }\n\n// Get returns the value.\nfunc (b *Box[T]) Get() T { return b.v }\n";
    let file = extract(Language::Go, source);
    assert_eq!(symbol(&file, "Box").doc, None);
    let get = symbol(&file, "Box.Get");
    assert_eq!(get.parent, Some(index_of(&file, "Box")));
    assert_eq!(get.doc.as_deref(), Some("Get returns the value."));
}
