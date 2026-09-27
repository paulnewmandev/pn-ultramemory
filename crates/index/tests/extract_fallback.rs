// SPDX-License-Identifier: Apache-2.0
//! Tests of the lexical fallback: languages without a grammar (Kotlin, Swift, Lua, shell,
//! Elixir and others) are read from the shape of their declarations.
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

use common::{assert_ref, extract, fixture, index_of, symbol};
use pn_ultramemory_core::{Extractor, Language, RefKind, SymbolKind, Visibility};
use pn_ultramemory_index::TreeSitterExtractor;

/// Returns the fallback language with the given name.
fn language(name: &str) -> Language {
    Language::from_name(name).unwrap_or_else(|| panic!("unknown language {name}"))
}

/// Every fallback language is supported by the extractor.
#[test]
fn every_language_is_supported() {
    let extractor = TreeSitterExtractor::new();
    for name in [
        "kotlin", "swift", "scala", "dart", "lua", "shell", "perl", "elixir", "haskell", "clojure",
        "sql", "r", "julia", "zig",
    ] {
        assert!(extractor.supports(language(name)), "{name}");
    }
    assert!(extractor.supports(Language::Other("brainfuck")));
    let file = extractor
        .extract(Language::Other("brainfuck"), "+++[>+<-]")
        .unwrap();
    assert!(file.symbols.is_empty());
    assert_eq!(file.parse_errors, 0);
}

/// Kotlin: classes, members, nested types, extension of the header over several lines.
#[test]
fn kotlin_symbols() {
    let source = fixture("kotlin_sample.kt");
    let file = extract(language("kotlin"), &source);
    let expect = |name: &str, kind: SymbolKind, parent: Option<&str>| {
        let s = symbol(&file, name);
        assert_eq!(s.kind, kind, "{name}");
        assert_eq!(s.parent, parent.map(|p| index_of(&file, p)), "{name}");
    };
    expect("Cart", SymbolKind::Class, None);
    expect("Cart.add", SymbolKind::Method, Some("Cart"));
    expect("Cart.load", SymbolKind::Method, Some("Cart"));
    expect("Cart.compareTo", SymbolKind::Method, Some("Cart"));
    expect("Cart.create", SymbolKind::Method, Some("Cart"));
    expect("Priced", SymbolKind::Interface, None);
    expect("Priced.price", SymbolKind::Method, Some("Priced"));
    expect("Point", SymbolKind::Class, None);
    expect("Color", SymbolKind::Enum, None);
    expect("Result", SymbolKind::Class, None);
    expect("Result.Ok", SymbolKind::Class, Some("Result"));
    expect("Result.Failure", SymbolKind::Class, Some("Result"));
    expect("topLevel", SymbolKind::Function, None);
    expect("helper", SymbolKind::Function, None);
    expect("Registry", SymbolKind::Class, None);
    expect("Registry.register", SymbolKind::Method, Some("Registry"));
    assert_eq!(file.symbols.len(), 17);
    assert_eq!(file.parse_errors, 0);
    assert_eq!(file.line_count, 64);
}

/// Kotlin: signatures, documentation, visibility, spans and body extents.
#[test]
fn kotlin_details() {
    let source = fixture("kotlin_sample.kt");
    let file = extract(language("kotlin"), &source);
    assert_eq!(
        symbol(&file, "Cart").signature,
        "open class Cart(private val owner: String) : Base(), Comparable<Cart>"
    );
    assert_eq!(
        symbol(&file, "Cart.load").signature,
        "private suspend fun load(id: UUID, refresh: Boolean = false): Item",
        "a signature that spans several lines is joined"
    );
    assert_eq!(
        symbol(&file, "Point").signature,
        "data class Point(val x: Int, val y: Int)"
    );
    assert_eq!(symbol(&file, "Color").signature, "enum class Color");
    assert_eq!(
        symbol(&file, "topLevel").signature,
        "fun topLevel(a: Int): Int = a + helper(a)"
    );

    assert_eq!(
        symbol(&file, "Cart").doc.as_deref(),
        Some("A shopping cart.\nHolds items until checkout.")
    );
    assert_eq!(
        symbol(&file, "Cart.add").doc.as_deref(),
        Some("Adds an item and returns the new size.")
    );
    assert_eq!(
        symbol(&file, "helper").doc.as_deref(),
        Some("Block comment documentation.")
    );
    assert_eq!(symbol(&file, "Cart.load").doc, None);

    assert_eq!(symbol(&file, "Cart.load").visibility, Visibility::Private);
    assert_eq!(
        symbol(&file, "helper").visibility,
        Visibility::Private,
        "internal"
    );
    assert_eq!(symbol(&file, "Cart").visibility, Visibility::Unknown);
    assert_eq!(symbol(&file, "Cart.add").visibility, Visibility::Unknown);

    // Braces inside strings do not confuse the body finder: `add` ends at its own brace.
    let add = symbol(&file, "Cart.add");
    assert_eq!((add.span.start_line, add.span.end_line), (17, 23));
    let slice = &source[add.span.start_byte as usize..add.span.end_byte as usize];
    assert!(slice.starts_with("// Adds an item"));
    assert!(slice.ends_with('}'));
    let cart = symbol(&file, "Cart");
    assert_eq!(
        (cart.span.start_line, cart.span.end_line),
        (9, 38),
        "annotation lines stay in the span"
    );
    // A multi-line signature still closes at its own closing brace.
    let load = symbol(&file, "Cart.load");
    assert_eq!((load.span.start_line, load.span.end_line), (25, 30));
    // Single-line bodies end on their line, even when a comment follows.
    let top = symbol(&file, "topLevel");
    assert_eq!((top.span.start_line, top.span.end_line), (54, 54));
    assert_ne!(
        symbol(&file, "Cart.compareTo").body_hash,
        symbol(&file, "Cart.create").body_hash
    );
}

/// Kotlin: calls inside bodies become references; the outline stays empty.
#[test]
fn kotlin_references() {
    let source = fixture("kotlin_sample.kt");
    let file = extract(language("kotlin"), &source);
    assert_ref(&file, "validate", RefKind::Call, Some("Cart.add"), None);
    assert_ref(&file, "add", RefKind::Call, Some("Cart.add"), Some("items"));
    assert_ref(&file, "max", RefKind::Call, Some("Cart.add"), None);
    assert_ref(
        &file,
        "find",
        RefKind::Call,
        Some("Cart.load"),
        Some("repository"),
    );
    assert!(file.symbols.iter().all(|s| s.outline.is_empty()));
    assert!(
        !file
            .references
            .iter()
            .any(|r| r.name == "Cart.add" || r.name == "add" && r.line == 17),
        "the declaration itself is not a call"
    );
    assert!(file.imports.is_empty());
}

/// Swift: protocols, structs, classes, extensions, enums and functions.
#[test]
fn swift_symbols() {
    let source = fixture("swift_sample.swift");
    let file = extract(language("swift"), &source);
    let kind = |name: &str| symbol(&file, name).kind;
    assert_eq!(kind("Shape"), SymbolKind::Interface);
    assert_eq!(kind("Shape.area"), SymbolKind::Method);
    assert_eq!(kind("Circle"), SymbolKind::Struct);
    assert_eq!(kind("Circle.area"), SymbolKind::Method);
    assert_eq!(kind("Circle.secret"), SymbolKind::Method);
    assert_eq!(kind("Circle.unit"), SymbolKind::Method);
    assert_eq!(kind("Model"), SymbolKind::Class);
    assert_eq!(kind("Model.append"), SymbolKind::Method);
    assert_eq!(kind("Direction"), SymbolKind::Enum);
    assert_eq!(kind("Direction.opposite"), SymbolKind::Method);
    assert_eq!(kind("topLevel"), SymbolKind::Function);
    // The extension is a symbol of its own kind, and its methods hang below it.
    let extensions: Vec<_> = file
        .symbols
        .iter()
        .filter(|s| s.qualified_name == "Circle" && s.kind == SymbolKind::Other)
        .collect();
    assert_eq!(extensions.len(), 1);
    assert_eq!(
        symbol(&file, "Circle.scaled").parent,
        Some(
            file.symbols
                .iter()
                .position(|s| s.kind == SymbolKind::Other)
                .unwrap()
        )
    );
    assert_eq!(
        symbol(&file, "Shape").doc.as_deref(),
        Some("A shape that has an area.")
    );
    assert_eq!(
        symbol(&file, "Circle.area").doc.as_deref(),
        Some("Computes the area.")
    );
    assert_eq!(symbol(&file, "Shape").visibility, Visibility::Public);
    assert_eq!(
        symbol(&file, "Circle.secret").visibility,
        Visibility::Private
    );
    assert_eq!(symbol(&file, "Model.append").visibility, Visibility::Public);
    assert_eq!(
        symbol(&file, "Model").signature,
        "final class Model<T>: ObservableObject"
    );
    assert_eq!(
        symbol(&file, "Circle.scaled").signature,
        "func scaled(by factor: Double) -> Circle"
    );
    assert_ref(&file, "helper", RefKind::Call, Some("Circle.secret"), None);
    assert_ref(&file, "compute", RefKind::Call, Some("topLevel"), None);
    assert_ref(&file, "notify", RefKind::Call, Some("Model.append"), None);
}

/// Lua: `function`, `local function`, method syntax and long-bracket documentation.
#[test]
fn lua_symbols() {
    let source = fixture("lua_sample.lua");
    let file = extract(language("lua"), &source);
    let add = symbol(&file, "M.add");
    assert_eq!(add.kind, SymbolKind::Function);
    assert_eq!(add.name, "add");
    assert_eq!(add.signature, "function M.add(a, b)");
    assert_eq!(
        add.doc.as_deref(),
        Some("Adds two numbers.\nThe second line of the documentation.")
    );
    assert_ref(&file, "helper", RefKind::Call, Some("M.add"), None);
    assert_eq!(symbol(&file, "helper").visibility, Visibility::Private);
    assert_eq!(symbol(&file, "M.add").visibility, Visibility::Unknown);
    assert_eq!(
        symbol(&file, "M.method").signature,
        "function M:method(arg)"
    );
    assert_eq!(
        symbol(&file, "documented_block").doc.as_deref(),
        Some("Block\ndocumentation")
    );
    assert_ref(
        &file,
        "setmetatable",
        RefKind::Call,
        Some("Class.new"),
        None,
    );
    assert_ref(
        &file,
        "format",
        RefKind::Call,
        Some("Class.greet"),
        Some("string"),
    );
    assert_ref(&file, "max", RefKind::Call, Some("M.add"), Some("math"));
    assert_ref(&file, "print", RefKind::Call, Some("M.method"), None);
    // `end` closes the function at the header's indentation, not before.
    assert_eq!((add.span.start_line, add.span.end_line), (5, 10));
}

/// Shell: functions in every syntax, comment documentation, and no calls without parentheses.
#[test]
fn shell_symbols() {
    let source = fixture("shell_sample.sh");
    let file = extract(language("shell"), &source);
    let names: Vec<_> = file.symbols.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(
        names,
        ["log", "build_project", "cleanup", "deploy_app", "main"]
    );
    assert!(file.symbols.iter().all(|s| s.kind == SymbolKind::Function));
    assert_eq!(
        symbol(&file, "log").doc.as_deref(),
        Some("Prints a message with a prefix.")
    );
    assert_eq!(
        symbol(&file, "build_project").doc.as_deref(),
        Some("Builds the project for a target.")
    );
    assert_eq!(symbol(&file, "cleanup").doc, None);
    assert_eq!(symbol(&file, "cleanup").signature, "function cleanup");
    assert_eq!(symbol(&file, "deploy_app").signature, "deploy_app()");
    let build = symbol(&file, "build_project");
    assert_eq!((build.span.start_line, build.span.end_line), (11, 21));
    let deploy = symbol(&file, "deploy_app");
    assert_eq!(
        (deploy.span.start_line, deploy.span.end_line),
        (27, 31),
        "a brace on the next line"
    );
    assert!(
        file.symbols
            .iter()
            .all(|s| s.visibility == Visibility::Unknown)
    );
}

/// Elixir: modules, `def`/`defp`/`defmacro`, `@doc`/`@moduledoc` documentation and `end` matching.
#[test]
fn elixir_symbols() {
    let source = fixture("elixir_sample.ex");
    let file = extract(language("elixir"), &source);
    let expect = |name: &str, kind: SymbolKind, parent: Option<&str>| {
        let s = symbol(&file, name);
        assert_eq!(s.kind, kind, "{name}");
        assert_eq!(s.parent, parent.map(|p| index_of(&file, p)), "{name}");
    };
    expect("MyApp.Worker", SymbolKind::Module, None);
    expect(
        "MyApp.Worker.start_link",
        SymbolKind::Function,
        Some("MyApp.Worker"),
    );
    expect(
        "MyApp.Worker.run",
        SymbolKind::Function,
        Some("MyApp.Worker"),
    );
    expect(
        "MyApp.Worker.short",
        SymbolKind::Function,
        Some("MyApp.Worker"),
    );
    expect(
        "MyApp.Worker.helper",
        SymbolKind::Function,
        Some("MyApp.Worker"),
    );
    expect(
        "MyApp.Worker.debug",
        SymbolKind::Macro,
        Some("MyApp.Worker"),
    );
    expect(
        "MyApp.Worker.Nested",
        SymbolKind::Module,
        Some("MyApp.Worker"),
    );
    expect(
        "MyApp.Worker.Nested.inner",
        SymbolKind::Function,
        Some("MyApp.Worker.Nested"),
    );
    expect("Sizeable", SymbolKind::Interface, None);
    expect("Sizeable.size", SymbolKind::Method, Some("Sizeable"));
    assert_eq!(symbol(&file, "MyApp.Worker").name, "Worker");
    assert_eq!(
        symbol(&file, "MyApp.Worker").doc.as_deref(),
        Some("Does background work.\n\nFixture for the extraction tests.")
    );
    assert_eq!(
        symbol(&file, "MyApp.Worker.start_link").doc.as_deref(),
        Some("Starts the worker.")
    );
    assert_eq!(
        symbol(&file, "MyApp.Worker.run").doc.as_deref(),
        Some("Runs a job.\n\nReturns the result.")
    );
    assert_eq!(
        symbol(&file, "MyApp.Worker.helper").visibility,
        Visibility::Private
    );
    assert_eq!(
        symbol(&file, "MyApp.Worker.run").visibility,
        Visibility::Public
    );
    assert_eq!(
        symbol(&file, "MyApp.Worker.short").signature,
        "def short(a), do: a + 1"
    );
    let run = symbol(&file, "MyApp.Worker.run");
    assert_eq!((run.span.start_line, run.span.end_line), (17, 26));
    let short = symbol(&file, "MyApp.Worker.short");
    assert_eq!(
        (short.span.start_line, short.span.end_line),
        (28, 28),
        "a one-line def does not take the module's `end`"
    );
    let module = symbol(&file, "MyApp.Worker");
    assert_eq!(module.span.end_line, 43);
    assert_ref(
        &file,
        "helper",
        RefKind::Call,
        Some("MyApp.Worker.run"),
        None,
    );
    assert_ref(
        &file,
        "map",
        RefKind::Call,
        Some("MyApp.Worker.run"),
        Some("Enum"),
    );
}

/// Haskell, Clojure, SQL, Zig, R, Julia, Perl, Scala and Dart samples.
#[test]
fn other_languages() {
    let haskell = extract(
        language("haskell"),
        "module Data.Shop where\n\n-- | Parses a number.\nparse :: String -> Int\nparse s = read s\n\ndata Item = Item { name :: String }\n\nclass Priced a where\n  price :: a -> Int\n",
    );
    assert_eq!(symbol(&haskell, "Data.Shop").kind, SymbolKind::Module);
    assert_eq!(
        symbol(&haskell, "parse").doc.as_deref(),
        Some("Parses a number.")
    );
    assert_eq!(symbol(&haskell, "Item").kind, SymbolKind::Type);
    assert_eq!(symbol(&haskell, "Priced").kind, SymbolKind::Interface);
    assert_eq!(symbol(&haskell, "Priced.price").kind, SymbolKind::Method);

    let clojure = extract(
        language("clojure"),
        "(ns shop.core\n  (:require [clojure.string :as str]))\n\n;; Formats a price.\n(defn format-price [cents]\n  (str (/ cents 100)))\n\n(defn- helper [x]\n  (inc x))\n\n(def limit 10)\n",
    );
    assert_eq!(symbol(&clojure, "shop.core").kind, SymbolKind::Module);
    assert_eq!(
        symbol(&clojure, "format-price").doc.as_deref(),
        Some("Formats a price.")
    );
    assert_eq!(symbol(&clojure, "format-price").span.end_line, 6);
    assert_eq!(symbol(&clojure, "helper").visibility, Visibility::Private);
    assert_eq!(symbol(&clojure, "limit").kind, SymbolKind::Constant);

    let sql = extract(
        language("sql"),
        "-- Creates the orders table.\nCREATE TABLE IF NOT EXISTS orders (\n  id INTEGER PRIMARY KEY\n);\n\nCREATE OR REPLACE FUNCTION order_total(order_id integer)\nRETURNS integer AS $$\nBEGIN\n  RETURN 1;\nEND;\n$$ LANGUAGE plpgsql;\n",
    );
    assert_eq!(symbol(&sql, "orders").kind, SymbolKind::Struct);
    assert_eq!(
        symbol(&sql, "orders").doc.as_deref(),
        Some("Creates the orders table.")
    );
    assert_eq!(symbol(&sql, "orders").span.end_line, 4);
    assert_eq!(symbol(&sql, "order_total").kind, SymbolKind::Function);

    let zig = extract(
        language("zig"),
        "/// A point.\npub const Point = struct {\n    x: i32,\n    pub fn norm(self: Point) i32 {\n        return helper(self.x);\n    }\n};\n\nfn helper(v: i32) i32 {\n    return v;\n}\n",
    );
    assert_eq!(symbol(&zig, "Point").kind, SymbolKind::Struct);
    assert_eq!(symbol(&zig, "Point").doc.as_deref(), Some("A point."));
    assert_eq!(symbol(&zig, "Point.norm").kind, SymbolKind::Method);
    assert_eq!(symbol(&zig, "helper").kind, SymbolKind::Function);

    let r = extract(
        language("r"),
        "# Squares a number.\nsquare <- function(x) {\n  helper(x) * x\n}\n",
    );
    assert_eq!(
        symbol(&r, "square").doc.as_deref(),
        Some("Squares a number.")
    );
    assert_eq!(symbol(&r, "square").span.end_line, 4);

    let julia = extract(
        language("julia"),
        "module Geo\nstruct Point\n    x::Float64\nend\nfunction norm(p::Point)\n    return sqrt(p.x)\nend\nend\n",
    );
    assert_eq!(symbol(&julia, "Geo").kind, SymbolKind::Module);
    assert_eq!(symbol(&julia, "Point").kind, SymbolKind::Struct);
    assert_eq!(symbol(&julia, "norm").kind, SymbolKind::Function);
    assert_eq!(symbol(&julia, "norm").span.end_line, 7);

    let perl = extract(
        language("perl"),
        "package Shop::Cart;\n\n# Adds an item.\nsub add_item {\n    log_change($item);\n}\n",
    );
    assert_eq!(symbol(&perl, "Shop.Cart").kind, SymbolKind::Module);
    assert_eq!(
        symbol(&perl, "add_item").doc.as_deref(),
        Some("Adds an item.")
    );
    assert_ref(&perl, "log_change", RefKind::Call, Some("add_item"), None);

    let scala = extract(
        language("scala"),
        "/** A cart. */\ncase class Cart(items: List[Item]) {\n  private def price(i: Item): Int = i.cents\n}\n\ntrait Priced {\n  def cents: Int\n}\n",
    );
    assert_eq!(symbol(&scala, "Cart").doc.as_deref(), Some("A cart."));
    assert_eq!(symbol(&scala, "Cart.price").visibility, Visibility::Private);
    assert_eq!(symbol(&scala, "Priced").kind, SymbolKind::Interface);
    assert_eq!(symbol(&scala, "Priced.cents").kind, SymbolKind::Method);

    let dart = extract(
        language("dart"),
        "/// A shape.\nabstract class Shape {\n  double area();\n}\n\nclass Circle extends Shape {}\n",
    );
    assert_eq!(symbol(&dart, "Shape").doc.as_deref(), Some("A shape."));
    assert_eq!(symbol(&dart, "Circle").kind, SymbolKind::Class);
}

/// An allman-style brace on the next line and unbalanced closers do not break the scan.
#[test]
fn brace_styles() {
    let source = "fun a()\n{\n  b()\n}\n\nfun c() {\n}\n}\n}\nfun d() { e() }\n";
    let file = extract(language("kotlin"), source);
    let a = symbol(&file, "a");
    assert_eq!((a.span.start_line, a.span.end_line), (1, 4));
    let c = symbol(&file, "c");
    assert_eq!((c.span.start_line, c.span.end_line), (6, 7));
    let d = symbol(&file, "d");
    assert_eq!((d.span.start_line, d.span.end_line), (10, 10));
    assert!(
        !file.references.iter().any(|r| r.name == "e"),
        "calls on a declaration line are not reported"
    );
    assert_ref(&file, "b", RefKind::Call, Some("a"), None);
}

/// `new X(` is a type mention and keywords followed by parentheses are not calls.
#[test]
fn keywords_are_not_calls() {
    let source = "fun run() {\n  if (ready()) {\n    while (more()) { step() }\n  }\n  val x = new Widget(1)\n}\n";
    let file = extract(language("kotlin"), source);
    let called: Vec<&str> = file
        .references
        .iter()
        .filter(|r| r.kind == RefKind::Call)
        .map(|r| r.name.as_str())
        .collect();
    assert_eq!(called, ["ready", "more", "step"]);
    assert_ref(&file, "Widget", RefKind::Type, Some("run"), None);
}

/// Declarations nested more than the limit are not tracked, and the scan stays linear.
#[test]
fn nesting_limit() {
    let mut source = String::new();
    for i in 0..500 {
        writeln!(source, "class C{i} {{").unwrap();
    }
    source.push_str(&"}\n".repeat(500));
    let file = extract(language("kotlin"), &source);
    assert!(file.symbols.len() <= 64, "{}", file.symbols.len());
    assert!(file.symbols.len() >= 60);
}
