// SPDX-License-Identifier: Apache-2.0
//! Extraction tests for Java and C#, driven by `java_sample.java` and `csharp_sample.cs`.
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

/// Java: classes, members, nested types, constants and their parents.
#[test]
fn java_symbols() {
    let (file, _) = extract_clean(Language::Java, "java_sample.java");
    let expect = |name: &str, kind: SymbolKind, parent: Option<&str>| {
        let s = symbol(&file, name);
        assert_eq!(s.kind, kind, "{name}");
        assert_eq!(s.parent, parent.map(|p| index_of(&file, p)), "{name}");
    };
    expect("Cart", SymbolKind::Class, None);
    expect("Cart.MAX_ITEMS", SymbolKind::Constant, Some("Cart"));
    expect("Cart.PREFIX", SymbolKind::Constant, Some("Cart"));
    expect("Cart.Cart", SymbolKind::Method, Some("Cart"));
    expect("Cart.add", SymbolKind::Method, Some("Cart"));
    expect("Cart.copyOf", SymbolKind::Method, Some("Cart"));
    expect("Cart.validate", SymbolKind::Method, Some("Cart"));
    expect("Cart.Status", SymbolKind::Enum, Some("Cart"));
    expect("Cart.Visitor", SymbolKind::Interface, Some("Cart"));
    expect(
        "Cart.Visitor.visit",
        SymbolKind::Method,
        Some("Cart.Visitor"),
    );
    expect("Cart.Pair", SymbolKind::Struct, Some("Cart"));
    expect("Cart.Marker", SymbolKind::Interface, Some("Cart"));
    expect("Priced", SymbolKind::Interface, None);
    expect("Priced.price", SymbolKind::Method, Some("Priced"));
    expect("Color", SymbolKind::Enum, None);
    expect("Color.label", SymbolKind::Method, Some("Color"));
    assert!(
        file.symbols
            .iter()
            .all(|s| s.name != "items" && s.name != "version"),
        "plain fields are not listed"
    );
    assert_eq!(file.symbols.len(), 18);
}

/// Java: signatures without annotations, Javadoc, and visibility.
#[test]
fn java_signatures_docs_visibility() {
    let (file, source) = extract_clean(Language::Java, "java_sample.java");
    assert_eq!(
        symbol(&file, "Cart").signature,
        "public class Cart<T extends Item> extends AbstractCart implements Comparable<Cart>, Serializable"
    );
    assert_eq!(
        symbol(&file, "Cart.add").signature,
        "public boolean add(T item)"
    );
    assert_eq!(
        symbol(&file, "Cart.copyOf").signature,
        "private static <U extends Item> List<U> copyOf(Map<String, U> source, int limit) throws IOException"
    );
    assert_eq!(
        symbol(&file, "Cart.MAX_ITEMS").signature,
        "public static final int MAX_ITEMS = 50"
    );
    assert_eq!(
        symbol(&file, "Cart.Pair").signature,
        "record Pair(int left, int right) implements Comparable<Pair>"
    );
    assert_eq!(symbol(&file, "Cart.Marker").signature, "@interface Marker");

    assert_eq!(
        symbol(&file, "Cart").doc.as_deref(),
        Some("A shopping cart.\n\n<p>Holds items until checkout.")
    );
    assert_eq!(
        symbol(&file, "Cart.Cart").doc.as_deref(),
        Some("Creates an empty cart.")
    );
    assert_eq!(
        symbol(&file, "Cart.add").doc.as_deref(),
        Some("Adds an item.\n\n@param item the item to add\n@return true when added")
    );
    assert_eq!(symbol(&file, "Cart.size").doc, None);

    let vis = |name: &str| symbol(&file, name).visibility;
    assert_eq!(vis("Cart"), Visibility::Public);
    assert_eq!(vis("Cart.add"), Visibility::Public);
    assert_eq!(vis("Cart.copyOf"), Visibility::Private);
    assert_eq!(vis("Cart.validate"), Visibility::Private);
    assert_eq!(vis("Cart.PREFIX"), Visibility::Private);
    assert_eq!(
        vis("Priced"),
        Visibility::Private,
        "package-private interface"
    );
    assert_eq!(
        vis("Priced.price"),
        Visibility::Public,
        "interface members are implicitly public"
    );
    assert_eq!(vis("Cart.Visitor.done"), Visibility::Public);

    // The annotations are inside the span (under the documentation) but not in the signature.
    let cart = symbol(&file, "Cart");
    let slice = &source[cart.span.start_byte as usize..cart.span.end_byte as usize];
    assert!(slice.starts_with("/**\n * A shopping cart."));
    assert!(slice.contains("@Entity\n@Table(name = \"carts\")\npublic class Cart"));
}

/// Java: calls, object creation, inheritance, annotations and imports.
#[test]
fn java_references_and_imports() {
    let (file, _) = extract_clean(Language::Java, "java_sample.java");
    assert_ref(&file, "validate", RefKind::Call, Some("Cart.add"), None);
    assert_ref(&file, "add", RefKind::Call, Some("Cart.add"), Some("items"));
    assert_ref(
        &file,
        "info",
        RefKind::Call,
        Some("Cart.add"),
        Some("Logger"),
    );
    assert_ref(
        &file,
        "load",
        RefKind::Call,
        Some("Cart.copyOf"),
        Some("repo"),
    );
    assert_ref(&file, "max", RefKind::Call, Some("Cart.copyOf"), None);
    assert_ref(
        &file,
        "Repository",
        RefKind::Type,
        Some("Cart.copyOf"),
        None,
    );
    assert_ref(&file, "ArrayList", RefKind::Type, Some("Cart.Cart"), None);
    assert_ref(&file, "AbstractCart", RefKind::Inherit, Some("Cart"), None);
    assert_ref(&file, "Comparable", RefKind::Inherit, Some("Cart"), None);
    assert_ref(&file, "Serializable", RefKind::Inherit, Some("Cart"), None);
    assert_ref(&file, "Base", RefKind::Inherit, Some("Cart.Visitor"), None);
    assert_ref(
        &file,
        "Comparable",
        RefKind::Inherit,
        Some("Cart.Pair"),
        None,
    );
    assert_ref(&file, "Entity", RefKind::Type, Some("Cart"), None);
    assert_ref(&file, "Override", RefKind::Type, Some("Cart.add"), None);
    assert_eq!(
        file.imports,
        [
            "java.util.List",
            "java.util.Map",
            "java.lang.Math.max",
            "com.example.model.*"
        ]
    );
    assert_eq!(
        symbol(&file, "Cart.add").outline,
        ["validate", "add", "info"]
    );
    assert_eq!(symbol(&file, "Cart.copyOf").outline, ["load", "max"]);
}

/// C#: namespaces, types, members, constants and delegates.
#[test]
fn csharp_symbols() {
    let (file, _) = extract_clean(Language::CSharp, "csharp_sample.cs");
    let expect = |name: &str, kind: SymbolKind, parent: Option<&str>| {
        let s = symbol(&file, name);
        assert_eq!(s.kind, kind, "{name}");
        assert_eq!(s.parent, parent.map(|p| index_of(&file, p)), "{name}");
    };
    expect("Shop.Billing", SymbolKind::Module, None);
    expect(
        "Shop.Billing.Invoice",
        SymbolKind::Class,
        Some("Shop.Billing"),
    );
    expect(
        "Shop.Billing.Invoice.MaxLines",
        SymbolKind::Constant,
        Some("Shop.Billing.Invoice"),
    );
    expect(
        "Shop.Billing.Invoice.Prefix",
        SymbolKind::Constant,
        Some("Shop.Billing.Invoice"),
    );
    expect(
        "Shop.Billing.Invoice.Invoice",
        SymbolKind::Method,
        Some("Shop.Billing.Invoice"),
    );
    expect(
        "Shop.Billing.Invoice.Add",
        SymbolKind::Method,
        Some("Shop.Billing.Invoice"),
    );
    expect(
        "Shop.Billing.Invoice.CountAsync",
        SymbolKind::Method,
        Some("Shop.Billing.Invoice"),
    );
    expect(
        "Shop.Billing.Invoice.Status",
        SymbolKind::Enum,
        Some("Shop.Billing.Invoice"),
    );
    expect(
        "Shop.Billing.IPriced",
        SymbolKind::Interface,
        Some("Shop.Billing"),
    );
    expect(
        "Shop.Billing.IPriced.Price",
        SymbolKind::Method,
        Some("Shop.Billing.IPriced"),
    );
    expect(
        "Shop.Billing.Line",
        SymbolKind::Struct,
        Some("Shop.Billing"),
    );
    expect(
        "Shop.Billing.Money",
        SymbolKind::Class,
        Some("Shop.Billing"),
    );
    expect(
        "Shop.Billing.Notify",
        SymbolKind::Type,
        Some("Shop.Billing"),
    );
    expect(
        "Shop.Billing.Helpers.Format",
        SymbolKind::Method,
        Some("Shop.Billing.Helpers"),
    );
    assert_eq!(file.symbols.len(), 18);
}

/// C#: signatures, XML documentation, visibility and constants.
#[test]
fn csharp_signatures_docs_visibility() {
    let (file, source) = extract_clean(Language::CSharp, "csharp_sample.cs");
    assert_eq!(
        symbol(&file, "Shop.Billing.Invoice").signature,
        "public class Invoice<T> : BaseInvoice, IPriced, IDisposable where T : class"
    );
    assert_eq!(
        symbol(&file, "Shop.Billing.Invoice.Add").signature,
        "public virtual void Add(Line line)"
    );
    assert_eq!(
        symbol(&file, "Shop.Billing.Invoice.CountAsync").signature,
        "internal static async Task<int> CountAsync(IEnumerable<Line> lines)"
    );
    assert_eq!(
        symbol(&file, "Shop.Billing.Money").signature,
        "public record Money(decimal Amount, string Currency)"
    );
    assert_eq!(
        symbol(&file, "Shop.Billing.Notify").signature,
        "public delegate void Notify(string message)"
    );
    assert_eq!(
        symbol(&file, "Shop.Billing.Invoice.MaxLines").signature,
        "public const int MaxLines = 500"
    );

    assert_eq!(
        symbol(&file, "Shop.Billing.Invoice").doc.as_deref(),
        Some("A customer invoice.")
    );
    assert_eq!(
        symbol(&file, "Shop.Billing.Invoice.Invoice").doc.as_deref(),
        Some("Creates an invoice.")
    );
    assert_eq!(
        symbol(&file, "Shop.Billing.Invoice.Add").doc.as_deref(),
        Some("Adds a line & recomputes."),
        "the summary text with its XML entities unescaped"
    );
    assert_eq!(symbol(&file, "Shop.Billing.Invoice.Dispose").doc, None);

    let vis = |name: &str| symbol(&file, name).visibility;
    assert_eq!(vis("Shop.Billing.Invoice"), Visibility::Public);
    assert_eq!(vis("Shop.Billing.Invoice.Add"), Visibility::Public);
    assert_eq!(
        vis("Shop.Billing.Invoice.CountAsync"),
        Visibility::Private,
        "internal is not public"
    );
    assert_eq!(vis("Shop.Billing.Invoice.Validate"), Visibility::Private);
    assert_eq!(vis("Shop.Billing.Invoice.Total"), Visibility::Private);
    assert_eq!(vis("Shop.Billing.Helpers"), Visibility::Private);
    assert_eq!(vis("Shop.Billing.IPriced.Price"), Visibility::Public);

    let invoice = symbol(&file, "Shop.Billing.Invoice");
    let slice = &source[invoice.span.start_byte as usize..invoice.span.end_byte as usize];
    assert!(slice.starts_with("/// <summary>"));
    assert!(slice.contains("[Serializable]"));
}

/// C#: calls, object creation, base types, attributes and usings.
#[test]
fn csharp_references_and_imports() {
    let (file, _) = extract_clean(Language::CSharp, "csharp_sample.cs");
    let add = "Shop.Billing.Invoice.Add";
    assert_ref(&file, "Validate", RefKind::Call, Some(add), None);
    assert_ref(
        &file,
        "WriteLine",
        RefKind::Call,
        Some(add),
        Some("Console"),
    );
    assert_ref(&file, "Info", RefKind::Call, Some(add), Some("Logger"));
    assert_ref(&file, "Format", RefKind::Call, Some(add), None);
    assert_ref(&file, "Line", RefKind::Type, Some(add), None);
    assert_ref(&file, "Obsolete", RefKind::Type, Some(add), None);
    assert_ref(
        &file,
        "FromResult",
        RefKind::Call,
        Some("Shop.Billing.Invoice.CountAsync"),
        Some("Task"),
    );
    assert_ref(
        &file,
        "BaseInvoice",
        RefKind::Inherit,
        Some("Shop.Billing.Invoice"),
        None,
    );
    assert_ref(
        &file,
        "IPriced",
        RefKind::Inherit,
        Some("Shop.Billing.Invoice"),
        None,
    );
    assert_ref(
        &file,
        "IDisposable",
        RefKind::Inherit,
        Some("Shop.Billing.Invoice"),
        None,
    );
    assert_ref(
        &file,
        "IComparable",
        RefKind::Inherit,
        Some("Shop.Billing.IPriced"),
        None,
    );
    assert_eq!(
        file.imports,
        [
            "System",
            "System.Collections.Generic",
            "System.Math",
            "Newtonsoft.Json.JsonConvert"
        ]
    );
    assert_eq!(
        symbol(&file, "Shop.Billing.Invoice.Add").outline,
        ["Validate", "WriteLine", "Format", "Info"]
    );
}

/// C#: a file-scoped namespace covers the rest of the file and qualifies what follows it.
#[test]
fn csharp_file_scoped_namespace() {
    let source = "using System;\nnamespace Shop.Reports;\n\npublic class Report\n{\n    public void Run() { }\n}\n\nclass Other { }\n";
    let file = extract(Language::CSharp, source);
    let namespace = symbol(&file, "Shop.Reports");
    assert_eq!(namespace.kind, SymbolKind::Module);
    assert_eq!(namespace.span.end_line, 9);
    assert_eq!(
        symbol(&file, "Shop.Reports.Report").parent,
        Some(index_of(&file, "Shop.Reports"))
    );
    assert_eq!(
        symbol(&file, "Shop.Reports.Report.Run").parent,
        Some(index_of(&file, "Shop.Reports.Report"))
    );
    assert_eq!(
        symbol(&file, "Shop.Reports.Other").visibility,
        Visibility::Private
    );
}
