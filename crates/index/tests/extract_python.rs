// SPDX-License-Identifier: Apache-2.0
//! Extraction tests for Python, driven by the fixture `fixtures/python_sample.py`.
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

/// Classes, methods, nested definitions and module-level assignments are all listed.
#[test]
fn symbols_kinds_and_parents() {
    let (file, _) = extract_clean(Language::Python, "python_sample.py");
    let expect = |name: &str, kind: SymbolKind, parent: Option<&str>| {
        let symbol = symbol(&file, name);
        assert_eq!(symbol.kind, kind, "{name}");
        assert_eq!(symbol.parent, parent.map(|p| index_of(&file, p)), "{name}");
    };
    expect("MAX_RETRIES", SymbolKind::Constant, None);
    expect("DEFAULT_CURRENCY", SymbolKind::Constant, None);
    expect("registry", SymbolKind::Variable, None);
    expect("Repository", SymbolKind::Class, None);
    expect("Repository.find", SymbolKind::Method, Some("Repository"));
    expect("OrderRepository", SymbolKind::Class, None);
    expect(
        "OrderRepository.LIMIT",
        SymbolKind::Constant,
        Some("OrderRepository"),
    );
    expect(
        "OrderRepository.__init__",
        SymbolKind::Method,
        Some("OrderRepository"),
    );
    expect(
        "OrderRepository.size",
        SymbolKind::Method,
        Some("OrderRepository"),
    );
    expect(
        "OrderRepository.build",
        SymbolKind::Method,
        Some("OrderRepository"),
    );
    expect(
        "OrderRepository.Cursor",
        SymbolKind::Class,
        Some("OrderRepository"),
    );
    expect(
        "OrderRepository.Cursor.advance",
        SymbolKind::Method,
        Some("OrderRepository.Cursor"),
    );
    expect("fetch_all", SymbolKind::Function, None);
    expect("fetch_all.inner", SymbolKind::Function, Some("fetch_all"));
    expect("_private_helper", SymbolKind::Function, None);
    expect("main", SymbolKind::Function, None);
    assert!(
        file.symbols.iter().all(|s| s.name != "table"),
        "lowercase class attributes are not listed"
    );
    assert_eq!(file.symbols.len(), 19);
    assert_eq!(file.line_count, 83);
}

/// Signatures exclude decorators and the trailing colon, and keep annotations and defaults.
#[test]
fn signatures() {
    let (file, _) = extract_clean(Language::Python, "python_sample.py");
    assert_eq!(symbol(&file, "Repository").signature, "class Repository");
    assert_eq!(
        symbol(&file, "OrderRepository").signature,
        "class OrderRepository(Repository, metaclass=Meta)"
    );
    assert_eq!(
        symbol(&file, "OrderRepository.__init__").signature,
        "def __init__(self, path: str, retries: int = MAX_RETRIES) -> None"
    );
    assert_eq!(
        symbol(&file, "OrderRepository.size").signature,
        "def size(self) -> int"
    );
    assert_eq!(
        symbol(&file, "OrderRepository.build").signature,
        "def build(path: str) -> \"OrderRepository\""
    );
    assert_eq!(
        symbol(&file, "fetch_all").signature,
        "async def fetch_all(repo: OrderRepository, limit: int = 10) -> List[Order]"
    );
    assert_eq!(symbol(&file, "MAX_RETRIES").signature, "MAX_RETRIES = 3");
    assert_eq!(
        symbol(&file, "DEFAULT_CURRENCY").signature,
        "DEFAULT_CURRENCY: str = \"EUR\""
    );
    assert_eq!(
        symbol(&file, "OrderRepository.Cursor.advance").signature,
        "def advance(self)"
    );
}

/// Docstrings are the documentation; comments are not; indentation is removed.
#[test]
fn docstrings() {
    let (file, _) = extract_clean(Language::Python, "python_sample.py");
    assert_eq!(
        symbol(&file, "Repository").doc.as_deref(),
        Some("Base class of every repository.")
    );
    assert_eq!(
        symbol(&file, "OrderRepository").doc.as_deref(),
        Some("Stores orders.\n\nOrders are kept in memory.")
    );
    assert_eq!(
        symbol(&file, "OrderRepository.find").doc.as_deref(),
        Some("Finds an order.")
    );
    assert_eq!(
        symbol(&file, "fetch_all").doc.as_deref(),
        Some("Fetches all orders.\n\nUses the repository.")
    );
    assert_eq!(symbol(&file, "OrderRepository.__init__").doc, None);
    assert_eq!(symbol(&file, "main").doc, None);
    assert_eq!(symbol(&file, "MAX_RETRIES").doc, None);
}

/// A decorated definition starts at its first decorator.
#[test]
fn decorated_span() {
    let (file, source) = extract_clean(Language::Python, "python_sample.py");
    let fetch_all = symbol(&file, "fetch_all");
    assert_eq!(fetch_all.span.start_line, 58);
    let slice = &source[fetch_all.span.start_byte as usize..fetch_all.span.end_byte as usize];
    assert!(slice.starts_with("@decorator(3)\n@other.tag\nasync def fetch_all"));
    assert!(
        slice
            .trim_end()
            .ends_with("return [inner(r) for r in rows]")
    );
    let build = symbol(&file, "OrderRepository.build");
    let slice = &source[build.span.start_byte as usize..build.span.end_byte as usize];
    assert!(slice.starts_with("@staticmethod"));
}

/// A leading underscore makes a name private, except for dunder names.
#[test]
fn visibility() {
    let (file, _) = extract_clean(Language::Python, "python_sample.py");
    assert_eq!(
        symbol(&file, "_private_helper").visibility,
        Visibility::Private
    );
    assert_eq!(
        symbol(&file, "OrderRepository._lookup").visibility,
        Visibility::Private
    );
    assert_eq!(
        symbol(&file, "OrderRepository.__init__").visibility,
        Visibility::Public
    );
    assert_eq!(
        symbol(&file, "OrderRepository.__repr__").visibility,
        Visibility::Public
    );
    assert_eq!(symbol(&file, "main").visibility, Visibility::Public);
    assert_eq!(
        symbol(&file, "OrderRepository").visibility,
        Visibility::Public
    );
}

/// Outlines list distinct callee names in order of first appearance, including nested ones.
#[test]
fn outlines() {
    let (file, _) = extract_clean(Language::Python, "python_sample.py");
    assert_eq!(
        symbol(&file, "OrderRepository.__init__").outline,
        ["join", "super", "__init__"]
    );
    assert_eq!(symbol(&file, "OrderRepository._lookup").outline, ["lookup"]);
    assert_eq!(
        symbol(&file, "fetch_all").outline,
        ["decorator", "tag", "load", "normalize", "inner"]
    );
    assert_eq!(
        symbol(&file, "main").outline,
        ["OrderRepository", "print", "fetch_all"]
    );
    assert!(symbol(&file, "Repository.find").outline.is_empty());
}

/// Calls, annotations and base classes are reported with owner and qualifier.
#[test]
fn references() {
    let (file, _) = extract_clean(Language::Python, "python_sample.py");
    assert_ref(
        &file,
        "join",
        RefKind::Call,
        Some("OrderRepository.__init__"),
        Some("os.path"),
    );
    assert_ref(
        &file,
        "lookup",
        RefKind::Call,
        Some("OrderRepository._lookup"),
        Some("helpers"),
    );
    assert_ref(
        &file,
        "_lookup",
        RefKind::Call,
        Some("OrderRepository.find"),
        Some("self"),
    );
    assert_ref(
        &file,
        "normalize",
        RefKind::Call,
        Some("fetch_all.inner"),
        None,
    );
    assert_ref(
        &file,
        "load",
        RefKind::Call,
        Some("fetch_all"),
        Some("repo"),
    );
    assert_ref(&file, "main", RefKind::Call, None, None);
    // Decorators are owned by the definition they decorate.
    assert_ref(&file, "decorator", RefKind::Call, Some("fetch_all"), None);
    assert_ref(
        &file,
        "tag",
        RefKind::Call,
        Some("fetch_all"),
        Some("other"),
    );
    // Type annotations.
    assert_ref(
        &file,
        "Optional",
        RefKind::Type,
        Some("OrderRepository.find"),
        None,
    );
    assert_ref(
        &file,
        "Order",
        RefKind::Type,
        Some("OrderRepository.find"),
        None,
    );
    assert_ref(
        &file,
        "OrderRepository",
        RefKind::Type,
        Some("fetch_all"),
        None,
    );
    assert_ref(&file, "List", RefKind::Type, Some("fetch_all"), None);
    assert!(
        !file
            .references
            .iter()
            .any(|r| r.kind == RefKind::Type && r.name == "str"),
        "builtin types are not reported"
    );
    // Inheritance and the metaclass.
    assert_ref(
        &file,
        "Repository",
        RefKind::Inherit,
        Some("OrderRepository"),
        None,
    );
    assert_ref(&file, "Meta", RefKind::Type, Some("OrderRepository"), None);
}

/// Imports keep the module path as written; relative imports keep their dots.
#[test]
fn imports() {
    let (file, _) = extract_clean(Language::Python, "python_sample.py");
    assert_eq!(
        file.imports,
        ["os", "sys", ".sibling", "..pkg.models", "typing", "a.b.c"]
    );
}

/// Single quoted, raw and concatenated strings and f-strings behave as docstrings should.
#[test]
fn docstring_forms() {
    let source = "def a():\n    '''single'''\n\ndef b():\n    r'''raw \\d'''\n\ndef c():\n    f'''not {a} doc'''\n\ndef d():\n    x = 1\n    '''not first'''\n";
    let file = extract(Language::Python, source);
    assert_eq!(symbol(&file, "a").doc.as_deref(), Some("single"));
    assert_eq!(symbol(&file, "b").doc.as_deref(), Some("raw \\d"));
    assert_eq!(symbol(&file, "c").doc, None);
    assert_eq!(symbol(&file, "d").doc, None);
}

/// Tabs, CRLF line endings and one-line bodies are handled.
#[test]
fn layout_variants() {
    let source = "class A:\r\n\tdef f(self): return 1\r\n\r\n\tdef g(self):\r\n\t\t\"\"\"Docs.\"\"\"\r\n\t\treturn 2\r\n";
    let file = extract(Language::Python, source);
    assert_eq!(symbol(&file, "A.f").signature, "def f(self)");
    assert_eq!(symbol(&file, "A.g").doc.as_deref(), Some("Docs."));
    assert_eq!(file.parse_errors, 0);
}
