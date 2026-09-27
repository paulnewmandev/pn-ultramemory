// SPDX-License-Identifier: Apache-2.0
//! Extraction tests for C and C++, driven by `c_sample.c` and `cpp_sample.cpp`.
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

/// C: macros, aggregate types, typedefs, variables, prototypes and definitions.
#[test]
fn c_symbols() {
    let (file, _) = extract_clean(Language::C, "c_sample.c");
    let kind_of = |name: &str| symbol(&file, name).kind;
    assert_eq!(kind_of("RING_CAPACITY"), SymbolKind::Macro);
    assert_eq!(kind_of("RING_MASK"), SymbolKind::Macro);
    assert_eq!(kind_of("Ring"), SymbolKind::Struct);
    assert_eq!(kind_of("Node"), SymbolKind::Struct);
    assert_eq!(kind_of("RingState"), SymbolKind::Enum);
    assert_eq!(
        kind_of("Mode"),
        SymbolKind::Enum,
        "an anonymous enum takes its typedef name"
    );
    assert_eq!(kind_of("visitor_fn"), SymbolKind::Type);
    assert_eq!(kind_of("Cell"), SymbolKind::Struct);
    assert_eq!(kind_of("allocated"), SymbolKind::Variable);
    assert_eq!(kind_of("global_counter"), SymbolKind::Variable);
    assert_eq!(kind_of("LIMIT"), SymbolKind::Constant);
    assert_eq!(kind_of("ring_new"), SymbolKind::Function);
    assert_eq!(kind_of("main"), SymbolKind::Function);
    assert!(
        file.symbols.iter().all(|s| s.name != "INCLUDE_GUARD_H"),
        "include guards are not macros"
    );
    assert!(
        file.symbols.iter().all(|s| s.name != "shared_flag"),
        "extern declarations are skipped"
    );
    // Prototypes and definitions are both listed.
    assert_eq!(
        file.symbols
            .iter()
            .filter(|s| s.name == "ring_push")
            .count(),
        2
    );
    assert_eq!(
        file.symbols
            .iter()
            .filter(|s| s.name == "log_event")
            .count(),
        2
    );
    assert_eq!(file.symbols.len(), 17);
    assert!(file.symbols.iter().all(|s| s.parent.is_none()));
}

/// C: signatures, Doxygen documentation, visibility and outlines.
#[test]
fn c_signatures_docs_visibility() {
    let (file, source) = extract_clean(Language::C, "c_sample.c");
    assert_eq!(
        symbol(&file, "RING_CAPACITY").signature,
        "#define RING_CAPACITY 64"
    );
    assert_eq!(symbol(&file, "RING_MASK").signature, "#define RING_MASK(x)");
    assert_eq!(symbol(&file, "Ring").signature, "typedef struct Ring");
    assert_eq!(symbol(&file, "Node").signature, "struct Node");
    assert_eq!(symbol(&file, "Mode").signature, "typedef enum Mode");
    assert_eq!(
        symbol(&file, "visitor_fn").signature,
        "typedef int (*visitor_fn)(int)"
    );
    assert_eq!(symbol(&file, "ring_new").signature, "Ring *ring_new(void)");
    assert_eq!(
        symbol(&file, "allocated").signature,
        "static int allocated = 0"
    );
    assert_eq!(
        symbol(&file, "main").signature,
        "int main(int argc, char **argv)"
    );

    assert_eq!(
        symbol(&file, "Ring").doc.as_deref(),
        Some("A fixed-size ring buffer.")
    );
    assert_eq!(
        symbol(&file, "RingState").doc.as_deref(),
        Some("States of a ring.")
    );
    assert_eq!(
        symbol(&file, "ring_new").doc.as_deref(),
        Some("Creates a ring.\nReturns NULL on failure.")
    );
    assert_eq!(symbol(&file, "main").doc, None);

    let vis = |name: &str| symbol(&file, name).visibility;
    assert_eq!(vis("allocated"), Visibility::Private);
    assert_eq!(vis("global_counter"), Visibility::Public);
    assert_eq!(vis("ring_new"), Visibility::Public);
    let private_log_events = file
        .symbols
        .iter()
        .filter(|s| s.name == "log_event" && s.visibility == Visibility::Private)
        .count();
    assert_eq!(
        private_log_events, 2,
        "static functions are private, prototype and definition"
    );

    assert_eq!(symbol(&file, "ring_new").outline, ["malloc", "log_event"]);
    assert_eq!(
        symbol(&file, "ring_push").outline,
        Vec::<String>::new(),
        "the prototype has no body"
    );
    let ring = symbol(&file, "Ring");
    let slice = &source[ring.span.start_byte as usize..ring.span.end_byte as usize];
    assert!(slice.starts_with("/** A fixed-size ring buffer. */\ntypedef struct Ring"));
    assert!(slice.trim_end().ends_with("} Ring;"));
}

/// C: calls, type mentions, includes.
#[test]
fn c_references_and_imports() {
    let (file, _) = extract_clean(Language::C, "c_sample.c");
    let new = index_of(&file, "ring_new");
    assert!(
        file.references
            .iter()
            .any(|r| r.name == "malloc" && r.kind == RefKind::Call && r.owner == Some(new))
    );
    assert_ref(&file, "log_event", RefKind::Call, Some("ring_new"), None);
    assert_ref(&file, "fprintf", RefKind::Call, Some("log_event"), None);
    assert_ref(&file, "ring_new", RefKind::Call, Some("main"), None);
    assert_ref(&file, "Ring", RefKind::Type, Some("main"), None);
    assert_ref(&file, "Node", RefKind::Type, Some("Node"), None);
    // The typedef name of a struct is not reported as a use of the type.
    assert!(
        !file
            .references
            .iter()
            .any(|r| r.name == "Ring" && r.owner.is_none())
    );
    assert_eq!(file.imports, ["stdio.h", "stdlib.h", "ring.h"]);
}

/// C++: namespaces, classes, structs, enums, methods and out-of-class definitions.
#[test]
fn cpp_symbols() {
    let (file, _) = extract_clean(Language::Cpp, "cpp_sample.cpp");
    let expect = |name: &str, kind: SymbolKind, parent: Option<&str>| {
        let s = symbol(&file, name);
        assert_eq!(s.kind, kind, "{name}");
        assert_eq!(s.parent, parent.map(|p| index_of(&file, p)), "{name}");
    };
    expect("Points", SymbolKind::Type, None);
    expect("geo", SymbolKind::Module, None);
    expect("geo::detail", SymbolKind::Module, Some("geo"));
    expect(
        "geo::detail::clamp",
        SymbolKind::Function,
        Some("geo::detail"),
    );
    expect("geo::Shape", SymbolKind::Class, Some("geo"));
    expect("geo::Shape::Shape", SymbolKind::Method, Some("geo::Shape"));
    expect("geo::Shape::~Shape", SymbolKind::Method, Some("geo::Shape"));
    expect("geo::Shape::area", SymbolKind::Method, Some("geo::Shape"));
    expect(
        "geo::Shape::operator=",
        SymbolKind::Method,
        Some("geo::Shape"),
    );
    expect("geo::Point", SymbolKind::Struct, Some("geo"));
    expect("geo::Point::norm", SymbolKind::Method, Some("geo::Point"));
    expect("geo::Mode", SymbolKind::Enum, Some("geo"));
    expect("geo::Circle", SymbolKind::Class, Some("geo"));
    expect(
        "geo::Circle::Circle",
        SymbolKind::Method,
        Some("geo::Circle"),
    );
    expect("geo::internal_only", SymbolKind::Function, Some("geo"));
    expect("geo::total", SymbolKind::Function, Some("geo"));
    expect("main", SymbolKind::Function, None);
    // `void Shape::hidden()` and `double Circle::area() const` are attached to their classes.
    let hidden: Vec<_> = file
        .symbols
        .iter()
        .filter(|s| s.qualified_name == "geo::Shape::hidden")
        .collect();
    assert_eq!(
        hidden.len(),
        2,
        "declaration in the class and definition outside"
    );
    assert!(
        hidden
            .iter()
            .all(|s| s.parent == Some(index_of(&file, "geo::Shape")))
    );
    let areas: Vec<_> = file
        .symbols
        .iter()
        .filter(|s| s.qualified_name == "geo::Circle::area")
        .collect();
    assert_eq!(areas.len(), 2);
    assert_eq!(file.symbols.len(), 24);
}

/// C++: templates are part of the signature and of the documented extent; access sections
/// decide the visibility of members.
#[test]
fn cpp_signatures_docs_visibility() {
    let (file, source) = extract_clean(Language::Cpp, "cpp_sample.cpp");
    assert_eq!(
        symbol(&file, "geo::Shape").signature,
        "template <typename T> class Shape : public Base<T>, private Named"
    );
    assert_eq!(
        symbol(&file, "geo::detail::clamp").signature,
        "template <typename T> T clamp(T value, T low, T high)"
    );
    assert_eq!(
        symbol(&file, "geo::Shape::area").signature,
        "virtual double area() const = 0"
    );
    assert_eq!(
        symbol(&file, "geo::Shape::~Shape").signature,
        "virtual ~Shape()"
    );
    assert_eq!(
        symbol(&file, "geo::Shape::operator=").signature,
        "Shape &operator=(const Shape &other)"
    );
    assert_eq!(symbol(&file, "geo::Mode").signature, "enum class Mode");
    assert_eq!(
        symbol(&file, "Points").signature,
        "using Points = std::vector<int>"
    );
    assert_eq!(
        symbol(&file, "geo::Circle").signature,
        "class Circle : public Shape<double>"
    );

    assert_eq!(
        symbol(&file, "geo::Shape").doc.as_deref(),
        Some("Base class of all shapes.")
    );
    assert_eq!(
        symbol(&file, "geo::Shape::area").doc.as_deref(),
        Some("Area of the shape.")
    );
    assert_eq!(
        symbol(&file, "geo::detail::clamp").doc.as_deref(),
        Some("Clamps a value.")
    );
    assert_eq!(
        symbol(&file, "geo::Circle").doc.as_deref(),
        Some("A circle.")
    );
    let shape = symbol(&file, "geo::Shape");
    let slice = &source[shape.span.start_byte as usize..shape.span.end_byte as usize];
    assert!(slice.starts_with("/// Base class of all shapes.\ntemplate <typename T>\nclass Shape"));

    let vis = |name: &str| symbol(&file, name).visibility;
    assert_eq!(vis("geo::Shape::area"), Visibility::Public);
    assert_eq!(
        vis("geo::Shape::count"),
        Visibility::Public,
        "a static member follows its access section"
    );
    assert_eq!(
        vis("geo::Shape::update"),
        Visibility::Private,
        "protected members are not public"
    );
    assert_eq!(
        vis("geo::Point::norm"),
        Visibility::Public,
        "struct members are public by default"
    );
    assert_eq!(
        vis("geo::internal_only"),
        Visibility::Private,
        "a static function has internal linkage"
    );
    assert_eq!(vis("geo::total"), Visibility::Public);
    let shape_hidden: Vec<_> = file
        .symbols
        .iter()
        .filter(|s| s.qualified_name == "geo::Shape::hidden")
        .map(|s| s.visibility)
        .collect();
    assert_eq!(shape_hidden, [Visibility::Private, Visibility::Public]);
}

/// C++: calls, `new`, base classes, type mentions and includes.
#[test]
fn cpp_references_and_imports() {
    let (file, _) = extract_clean(Language::Cpp, "cpp_sample.cpp");
    assert_ref(&file, "Base", RefKind::Inherit, Some("geo::Shape"), None);
    assert_ref(&file, "Named", RefKind::Inherit, Some("geo::Shape"), None);
    assert_ref(&file, "Shape", RefKind::Inherit, Some("geo::Circle"), None);
    assert_ref(
        &file,
        "helper",
        RefKind::Call,
        Some("geo::Shape::size"),
        None,
    );
    assert_ref(
        &file,
        "extra",
        RefKind::Call,
        Some("geo::Shape::size"),
        Some("this"),
    );
    assert_ref(
        &file,
        "sqrt",
        RefKind::Call,
        Some("geo::Point::norm"),
        Some("std"),
    );
}

/// C++: the remaining reference shapes, split from the previous test to keep it readable.
#[test]
fn cpp_more_references() {
    let (file, _) = extract_clean(Language::Cpp, "cpp_sample.cpp");
    let hidden_definition = file
        .symbols
        .iter()
        .enumerate()
        .filter(|(_, s)| s.qualified_name == "geo::Shape::hidden")
        .map(|(i, _)| i)
        .next_back()
        .unwrap();
    let owned = |name: &str, kind: RefKind, qualifier: Option<&str>| {
        file.references.iter().any(|r| {
            r.name == name
                && r.kind == kind
                && r.owner == Some(hidden_definition)
                && r.qualifier.as_deref() == qualifier
        })
    };
    assert!(owned("clamp", RefKind::Call, Some("detail")));
    assert!(owned("info", RefKind::Call, Some("logger")));
    assert!(
        owned("Point", RefKind::Type, None),
        "`new Point()` is a type mention"
    );
    assert_ref(&file, "Circle", RefKind::Type, Some("main"), Some("geo"));
    assert_ref(&file, "area", RefKind::Call, Some("main"), Some("c"));
    assert_eq!(
        symbol(&file, "geo::Shape::size").outline,
        ["helper", "extra"]
    );
    assert_eq!(file.imports, ["vector", "memory", "shape.hpp"]);
}

/// C++: anonymous namespaces and nested namespace definitions are handled.
#[test]
fn cpp_namespace_forms() {
    let source =
        "namespace a::b { int f() { return 1; } }\nnamespace { int hidden() { return 2; } }\n";
    let file = extract(Language::Cpp, source);
    assert_eq!(symbol(&file, "a::b").kind, SymbolKind::Module);
    assert_eq!(
        symbol(&file, "a::b::f").parent,
        Some(index_of(&file, "a::b"))
    );
    assert_eq!(symbol(&file, "hidden").kind, SymbolKind::Function);
}
