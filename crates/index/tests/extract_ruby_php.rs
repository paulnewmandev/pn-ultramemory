// SPDX-License-Identifier: Apache-2.0
//! Extraction tests for Ruby and PHP, driven by `ruby_sample.rb` and `php_sample.php`.
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

/// Ruby: modules, classes, methods and constants with their parents.
#[test]
fn ruby_symbols() {
    let (file, _) = extract_clean(Language::Ruby, "ruby_sample.rb");
    let expect = |name: &str, kind: SymbolKind, parent: Option<&str>| {
        let s = symbol(&file, name);
        assert_eq!(s.kind, kind, "{name}");
        assert_eq!(s.parent, parent.map(|p| index_of(&file, p)), "{name}");
    };
    expect("Billing", SymbolKind::Module, None);
    expect("Billing.VERSION", SymbolKind::Constant, Some("Billing"));
    expect("Billing.MAX_LINES", SymbolKind::Constant, Some("Billing"));
    expect("Billing.Payable", SymbolKind::Class, Some("Billing"));
    expect(
        "Billing.Payable.initialize",
        SymbolKind::Method,
        Some("Billing.Payable"),
    );
    expect(
        "Billing.Payable.<=>",
        SymbolKind::Method,
        Some("Billing.Payable"),
    );
    expect(
        "Billing.Payable.parse",
        SymbolKind::Method,
        Some("Billing.Payable"),
    );
    expect(
        "Billing.Payable.total",
        SymbolKind::Method,
        Some("Billing.Payable"),
    );
    expect("Billing.Invoice", SymbolKind::Class, Some("Billing"));
    expect(
        "Billing.Invoice.add_line",
        SymbolKind::Method,
        Some("Billing.Invoice"),
    );
    expect(
        "Billing.Invoice.empty",
        SymbolKind::Method,
        Some("Billing.Invoice"),
    );
    expect("Billing.Util", SymbolKind::Module, Some("Billing"));
    expect(
        "Billing.Util.slug",
        SymbolKind::Method,
        Some("Billing.Util"),
    );
    expect("top_level_helper", SymbolKind::Function, None);
    assert_eq!(file.symbols.len(), 19);
}

/// Ruby: visibility after bare `private`, `protected`, `public`, `private def` and `private :name`.
#[test]
fn ruby_visibility() {
    let (file, _) = extract_clean(Language::Ruby, "ruby_sample.rb");
    let vis = |name: &str| symbol(&file, name).visibility;
    assert_eq!(vis("Billing.Payable.initialize"), Visibility::Public);
    assert_eq!(vis("Billing.Payable.total"), Visibility::Public);
    assert_eq!(vis("Billing.Payable.parse"), Visibility::Public);
    assert_eq!(vis("Billing.Payable.tax"), Visibility::Private);
    assert_eq!(
        vis("Billing.Payable.visible"),
        Visibility::Public,
        "a bare `public` resets the access"
    );
    assert_eq!(vis("Billing.Payable.guarded"), Visibility::Private);
    assert_eq!(vis("Billing.Payable.inline_private"), Visibility::Private);
    assert_eq!(vis("Billing.Payable.later_private"), Visibility::Private);
    assert_eq!(
        vis("Billing.Invoice.add_line"),
        Visibility::Public,
        "access does not leak into the next class"
    );
    assert_eq!(vis("Billing.Invoice.empty"), Visibility::Public);
    assert_eq!(vis("top_level_helper"), Visibility::Public);
}

/// Ruby: signatures, comment documentation (magic comments excluded) and outlines.
#[test]
fn ruby_signatures_docs_outlines() {
    let (file, source) = extract_clean(Language::Ruby, "ruby_sample.rb");
    assert_eq!(symbol(&file, "Billing").signature, "module Billing");
    assert_eq!(symbol(&file, "Billing.Payable").signature, "class Payable");
    assert_eq!(
        symbol(&file, "Billing.Invoice").signature,
        "class Invoice < Payable"
    );
    assert_eq!(
        symbol(&file, "Billing.Payable.initialize").signature,
        "def initialize(amount)"
    );
    assert_eq!(
        symbol(&file, "Billing.Payable.<=>").signature,
        "def <=>(other)"
    );
    assert_eq!(
        symbol(&file, "Billing.Payable.parse").signature,
        "def self.parse(text)"
    );
    assert_eq!(
        symbol(&file, "Billing.Util.slug").signature,
        "def self.slug(text)"
    );
    assert_eq!(
        symbol(&file, "Billing.Payable.total").signature,
        "def total"
    );

    assert_eq!(
        symbol(&file, "Billing").doc.as_deref(),
        Some("Namespace for everything billing related.")
    );
    assert_eq!(
        symbol(&file, "Billing.Payable").doc.as_deref(),
        Some("Base class of every payable.")
    );
    assert_eq!(
        symbol(&file, "Billing.Payable.initialize").doc.as_deref(),
        Some("Builds a payable.")
    );
    assert_eq!(
        symbol(&file, "Billing.Payable.<=>").doc.as_deref(),
        Some("Compares by amount.")
    );
    assert_eq!(symbol(&file, "Billing.Payable.total").doc, None);
    let billing = symbol(&file, "Billing");
    let slice = &source[billing.span.start_byte as usize..billing.span.end_byte as usize];
    assert!(slice.starts_with("# Namespace for everything billing related.\nmodule Billing"));
    assert!(slice.trim_end().ends_with("end"));

    assert_eq!(
        symbol(&file, "Billing.Payable.total").outline,
        ["round", "tax", "size"]
    );
    assert_eq!(
        symbol(&file, "Billing.Payable.parse").outline,
        ["new", "parse"]
    );
}

/// Ruby: calls, instantiation, mixins, inheritance and `require`.
#[test]
fn ruby_references_and_imports() {
    let (file, _) = extract_clean(Language::Ruby, "ruby_sample.rb");
    assert_ref(
        &file,
        "round",
        RefKind::Call,
        Some("Billing.Payable.total"),
        Some("Helpers"),
    );
    assert_ref(
        &file,
        "tax",
        RefKind::Call,
        Some("Billing.Payable.total"),
        None,
    );
    assert_ref(
        &file,
        "parse",
        RefKind::Call,
        Some("Billing.Payable.parse"),
        Some("JSON"),
    );
    assert_ref(
        &file,
        "amount",
        RefKind::Call,
        Some("Billing.Payable.<=>"),
        Some("other"),
    );
    assert_ref(
        &file,
        "audit",
        RefKind::Call,
        Some("Billing.Invoice.add_line"),
        None,
    );
    assert_ref(&file, "puts", RefKind::Call, Some("top_level_helper"), None);
    assert_ref(
        &file,
        "Set",
        RefKind::Type,
        Some("Billing.Payable.total"),
        None,
    );
    assert_ref(
        &file,
        "Comparable",
        RefKind::Inherit,
        Some("Billing.Payable"),
        None,
    );
    assert_ref(
        &file,
        "Forwardable",
        RefKind::Inherit,
        Some("Billing.Payable"),
        None,
    );
    assert_ref(
        &file,
        "Payable",
        RefKind::Inherit,
        Some("Billing.Invoice"),
        None,
    );
    assert!(
        !file
            .references
            .iter()
            .any(|r| r.name == "attr_reader" || r.name == "private"),
        "directives are not calls"
    );
    assert_eq!(file.imports, ["json", "./support/helpers", "set"]);
}

/// Ruby: `class << self` methods and namespaced class names.
#[test]
fn ruby_singleton_class_and_scoped_names() {
    let source = "class A::B < C::D\n  class << self\n    def make; end\n  end\nend\n";
    let file = extract(Language::Ruby, source);
    assert_eq!(symbol(&file, "A.B").kind, SymbolKind::Class);
    assert_eq!(symbol(&file, "A.B").name, "B");
    assert_eq!(symbol(&file, "A.B.make").kind, SymbolKind::Method);
    assert_ref(&file, "D", RefKind::Inherit, Some("A.B"), Some("C"));
}

/// PHP: namespaces, classes, interfaces, traits, enums, methods and constants.
#[test]
fn php_symbols() {
    let (file, _) = extract_clean(Language::Php, "php_sample.php");
    let expect = |name: &str, kind: SymbolKind, parent: Option<&str>| {
        let s = symbol(&file, name);
        assert_eq!(s.kind, kind, "{name}");
        assert_eq!(s.parent, parent.map(|p| index_of(&file, p)), "{name}");
    };
    expect("Shop.Billing", SymbolKind::Module, None);
    expect(
        "Shop.Billing.DEFAULT_CURRENCY",
        SymbolKind::Constant,
        Some("Shop.Billing"),
    );
    expect(
        "Shop.Billing.Invoice",
        SymbolKind::Class,
        Some("Shop.Billing"),
    );
    expect(
        "Shop.Billing.Invoice.MAX_LINES",
        SymbolKind::Constant,
        Some("Shop.Billing.Invoice"),
    );
    expect(
        "Shop.Billing.Invoice.__construct",
        SymbolKind::Method,
        Some("Shop.Billing.Invoice"),
    );
    expect(
        "Shop.Billing.Invoice.addLine",
        SymbolKind::Method,
        Some("Shop.Billing.Invoice"),
    );
    expect(
        "Shop.Billing.Invoice.fromArray",
        SymbolKind::Method,
        Some("Shop.Billing.Invoice"),
    );
    expect(
        "Shop.Billing.Priced2",
        SymbolKind::Interface,
        Some("Shop.Billing"),
    );
    expect(
        "Shop.Billing.Priced2.price",
        SymbolKind::Method,
        Some("Shop.Billing.Priced2"),
    );
    expect(
        "Shop.Billing.Timestamps",
        SymbolKind::Interface,
        Some("Shop.Billing"),
    );
    expect(
        "Shop.Billing.Status",
        SymbolKind::Enum,
        Some("Shop.Billing"),
    );
    expect(
        "Shop.Billing.Status.label",
        SymbolKind::Method,
        Some("Shop.Billing.Status"),
    );
    expect(
        "Shop.Billing.format_invoice",
        SymbolKind::Function,
        Some("Shop.Billing"),
    );
    assert_eq!(file.symbols.len(), 18);
}

/// PHP: signatures without attributes, `PHPDoc`, and visibility.
#[test]
fn php_signatures_docs_visibility() {
    let (file, source) = extract_clean(Language::Php, "php_sample.php");
    assert_eq!(
        symbol(&file, "Shop.Billing.Invoice").signature,
        "abstract class Invoice extends Model implements Priced, \\Countable"
    );
    assert_eq!(
        symbol(&file, "Shop.Billing.Invoice.__construct").signature,
        "public function __construct(private string $customer, protected ?Money $total = null)"
    );
    assert_eq!(
        symbol(&file, "Shop.Billing.Invoice.addLine").signature,
        "public function addLine(Line $line): static"
    );
    assert_eq!(
        symbol(&file, "Shop.Billing.format_invoice").signature,
        "function format_invoice(Invoice $invoice, array $options = []): string"
    );
    assert_eq!(
        symbol(&file, "Shop.Billing.Status").signature,
        "enum Status: string"
    );
    assert_eq!(
        symbol(&file, "Shop.Billing.Timestamps").signature,
        "trait Timestamps"
    );

    assert_eq!(
        symbol(&file, "Shop.Billing.Invoice").doc.as_deref(),
        Some("A customer invoice.\n\nHolds lines until it is issued.")
    );
    assert_eq!(
        symbol(&file, "Shop.Billing.Invoice.__construct")
            .doc
            .as_deref(),
        Some("Creates an invoice.")
    );
    assert_eq!(
        symbol(&file, "Shop.Billing.Invoice.addLine").doc.as_deref(),
        Some("Adds a line.")
    );
    assert_eq!(symbol(&file, "Shop.Billing.Invoice.fromArray").doc, None);
    let invoice = symbol(&file, "Shop.Billing.Invoice");
    let slice = &source[invoice.span.start_byte as usize..invoice.span.end_byte as usize];
    assert!(slice.starts_with("/**\n * A customer invoice."));
    assert!(slice.contains("#[Entity(table: 'invoices')]\nabstract class Invoice"));

    let vis = |name: &str| symbol(&file, name).visibility;
    assert_eq!(vis("Shop.Billing.Invoice"), Visibility::Public);
    assert_eq!(vis("Shop.Billing.Invoice.addLine"), Visibility::Public);
    assert_eq!(
        vis("Shop.Billing.Invoice.validate"),
        Visibility::Private,
        "protected is not public"
    );
    assert_eq!(vis("Shop.Billing.Invoice.secret"), Visibility::Private);
    assert_eq!(
        vis("Shop.Billing.Invoice.total"),
        Visibility::Public,
        "abstract public method"
    );
    assert_eq!(vis("Shop.Billing.Invoice.SECRET"), Visibility::Private);
    assert_eq!(vis("Shop.Billing.Invoice.MAX_LINES"), Visibility::Public);
    assert_eq!(vis("Shop.Billing.format_invoice"), Visibility::Public);
}

/// PHP: calls of every form, `new`, inheritance, trait use and imports.
#[test]
fn php_references_and_imports() {
    let (file, _) = extract_clean(Language::Php, "php_sample.php");
    let add = "Shop.Billing.Invoice.addLine";
    assert_ref(&file, "validate", RefKind::Call, Some(add), Some("$this"));
    assert_ref(&file, "log", RefKind::Call, Some(add), Some("self"));
    assert_ref(&file, "round_cents", RefKind::Call, Some(add), None);
    assert_ref(&file, "Line", RefKind::Type, Some(add), None);
    assert_ref(
        &file,
        "query",
        RefKind::Call,
        Some("Shop.Billing.Invoice.fromArray"),
        Some("static"),
    );
    assert_ref(
        &file,
        "where",
        RefKind::Call,
        Some("Shop.Billing.Invoice.fromArray"),
        None,
    );
    assert_ref(
        &file,
        "__construct",
        RefKind::Call,
        Some("Shop.Billing.Invoice.__construct"),
        Some("parent"),
    );
    assert_ref(
        &file,
        "render",
        RefKind::Call,
        Some("Shop.Billing.format_invoice"),
        Some("Fmt"),
    );
    assert_ref(
        &file,
        "Model",
        RefKind::Inherit,
        Some("Shop.Billing.Invoice"),
        None,
    );
    assert_ref(
        &file,
        "Priced",
        RefKind::Inherit,
        Some("Shop.Billing.Invoice"),
        None,
    );
    assert_ref(
        &file,
        "Countable",
        RefKind::Inherit,
        Some("Shop.Billing.Invoice"),
        None,
    );
    assert_ref(
        &file,
        "Timestamps",
        RefKind::Inherit,
        Some("Shop.Billing.Invoice"),
        None,
    );
    assert_ref(
        &file,
        "SoftDeletes",
        RefKind::Inherit,
        Some("Shop.Billing.Invoice"),
        None,
    );
    assert_ref(
        &file,
        "Base",
        RefKind::Inherit,
        Some("Shop.Billing.Priced2"),
        None,
    );
    assert_ref(
        &file,
        "Entity",
        RefKind::Type,
        Some("Shop.Billing.Invoice"),
        None,
    );
    assert_eq!(
        file.imports,
        [
            "Shop\\Contracts\\Priced",
            "Shop\\Support\\Money",
            "Shop\\Support\\Formatter",
            "Shop\\Support\\round_cents",
            "vendor/autoload.php",
            "helpers.php",
        ]
    );
}

/// PHP: a namespace written as a statement lasts until the end of the file.
#[test]
fn php_statement_namespace_and_functions() {
    let source = "<?php\nnamespace App;\n\nfunction one(): int { return two(); }\n\nfunction two(): int { return 2; }\n";
    let file = extract(Language::Php, source);
    assert_eq!(
        symbol(&file, "App.one").parent,
        Some(index_of(&file, "App"))
    );
    assert_eq!(
        symbol(&file, "App.two").parent,
        Some(index_of(&file, "App"))
    );
    assert_eq!(symbol(&file, "App").span.end_line, 6);
}
