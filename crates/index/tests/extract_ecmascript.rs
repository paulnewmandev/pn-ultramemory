// SPDX-License-Identifier: Apache-2.0
//! Extraction tests for JavaScript, TypeScript and TSX, driven by the fixtures
//! `javascript_sample.js`, `typescript_sample.ts` and `tsx_sample.tsx`.
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

/// JavaScript: classes, methods, functions and top-level declarations with their kinds.
#[test]
fn javascript_symbols() {
    let (file, _) = extract_clean(Language::JavaScript, "javascript_sample.js");
    let expect = |name: &str, kind: SymbolKind, parent: Option<&str>| {
        let s = symbol(&file, name);
        assert_eq!(s.kind, kind, "{name}");
        assert_eq!(s.parent, parent.map(|p| index_of(&file, p)), "{name}");
    };
    expect("DEFAULT_LIMIT", SymbolKind::Constant, None);
    expect("internalCounter", SymbolKind::Constant, None);
    expect("mutableState", SymbolKind::Variable, None);
    expect("Emitter", SymbolKind::Class, None);
    expect("Emitter.constructor", SymbolKind::Method, Some("Emitter"));
    expect("Emitter.on", SymbolKind::Method, Some("Emitter"));
    expect("Emitter.create", SymbolKind::Method, Some("Emitter"));
    expect("Emitter.size", SymbolKind::Method, Some("Emitter"));
    expect("Emitter.#store", SymbolKind::Method, Some("Emitter"));
    expect("Emitter.emit", SymbolKind::Method, Some("Emitter"));
    expect("Bus", SymbolKind::Class, None);
    expect("Bus.publish", SymbolKind::Method, Some("Bus"));
    expect("add", SymbolKind::Function, None);
    expect("double", SymbolKind::Function, None);
    expect("memo", SymbolKind::Function, None);
    expect("privateUtil", SymbolKind::Function, None);
    expect("ids", SymbolKind::Function, None);
    expect("legacy", SymbolKind::Function, None);
    assert!(
        file.symbols.iter().all(|s| s.name != "lodash"),
        "`require` is an import, not a declaration"
    );
    assert_eq!(file.symbols.len(), 18);
}

/// JavaScript: signatures, documentation and visibility rules.
#[test]
fn javascript_signatures_docs_visibility() {
    let (file, source) = extract_clean(Language::JavaScript, "javascript_sample.js");
    assert_eq!(symbol(&file, "add").signature, "export function add(a, b)");
    assert_eq!(
        symbol(&file, "double").signature,
        "export const double = async (x)"
    );
    assert_eq!(
        symbol(&file, "memo").signature,
        "const memo = function cached(key)"
    );
    assert_eq!(symbol(&file, "ids").signature, "function* ids()");
    assert_eq!(symbol(&file, "Emitter").signature, "export class Emitter");
    assert_eq!(
        symbol(&file, "Bus").signature,
        "export default class Bus extends Emitter"
    );
    assert_eq!(
        symbol(&file, "Emitter.create").signature,
        "static create(name)"
    );
    assert_eq!(symbol(&file, "Emitter.size").signature, "get size()");
    assert_eq!(
        symbol(&file, "DEFAULT_LIMIT").signature,
        "export const DEFAULT_LIMIT = 10"
    );

    assert_eq!(
        symbol(&file, "add").doc.as_deref(),
        Some("Adds two numbers.")
    );
    assert_eq!(
        symbol(&file, "Emitter").doc.as_deref(),
        Some("Base class of every emitter.")
    );
    assert_eq!(
        symbol(&file, "Emitter.constructor").doc.as_deref(),
        Some("Creates the emitter.")
    );
    assert_eq!(
        symbol(&file, "Emitter.on").doc.as_deref(),
        Some("Registers a listener.\n@param {string} event the event name")
    );
    assert_eq!(
        symbol(&file, "DEFAULT_LIMIT").doc.as_deref(),
        Some("Default number of listeners.")
    );
    assert_eq!(symbol(&file, "privateUtil").doc, None);

    let vis = |name: &str| symbol(&file, name).visibility;
    assert_eq!(vis("add"), Visibility::Public);
    assert_eq!(vis("Emitter"), Visibility::Public);
    assert_eq!(vis("Emitter.on"), Visibility::Public);
    assert_eq!(vis("Emitter.#store"), Visibility::Private);
    assert_eq!(vis("privateUtil"), Visibility::Private);
    assert_eq!(vis("internalCounter"), Visibility::Private);
    assert_eq!(vis("mutableState"), Visibility::Private);
    assert_eq!(vis("legacy"), Visibility::Public);

    // The span of an exported, documented function includes the comment and the `export`.
    let add = symbol(&file, "add");
    let slice = &source[add.span.start_byte as usize..add.span.end_byte as usize];
    assert!(slice.starts_with("/** Adds two numbers. */\nexport function add"));
}

/// JavaScript: calls, `new`, inheritance and imports.
#[test]
fn javascript_references_and_imports() {
    let (file, _) = extract_clean(Language::JavaScript, "javascript_sample.js");
    assert_ref(&file, "helper", RefKind::Call, Some("add"), None);
    assert_ref(&file, "max", RefKind::Call, Some("add"), Some("Math"));
    assert_ref(&file, "compute", RefKind::Call, Some("double"), None);
    assert_ref(
        &file,
        "join",
        RefKind::Call,
        Some("Bus.publish"),
        Some("path"),
    );
    assert_ref(
        &file,
        "on",
        RefKind::Call,
        Some("Bus.publish"),
        Some("super"),
    );
    assert_ref(&file, "get", RefKind::Call, Some("memo"), Some("lodash"));
    assert_ref(
        &file,
        "Emitter",
        RefKind::Type,
        Some("Emitter.create"),
        None,
    );
    assert_ref(&file, "Map", RefKind::Type, Some("Emitter"), None);
    assert_ref(&file, "Emitter", RefKind::Inherit, Some("Bus"), None);
    assert_eq!(
        file.imports,
        ["node:fs", "path", "./polyfills.js", "lodash", "./helper.js"]
    );
    assert_eq!(
        symbol(&file, "Emitter").outline,
        ["#store", "push", "get", "cb"]
    );
    assert_eq!(symbol(&file, "add").outline, ["helper", "max"]);
}

/// TypeScript: interfaces, types, enums, namespaces, classes and members.
#[test]
fn typescript_symbols() {
    let (file, _) = extract_clean(Language::TypeScript, "typescript_sample.ts");
    let expect = |name: &str, kind: SymbolKind, parent: Option<&str>| {
        let s = symbol(&file, name);
        assert_eq!(s.kind, kind, "{name}");
        assert_eq!(s.parent, parent.map(|p| index_of(&file, p)), "{name}");
    };
    expect("Entity", SymbolKind::Interface, None);
    expect("Entity.describe", SymbolKind::Method, Some("Entity"));
    expect("Repo", SymbolKind::Interface, None);
    expect("Repo.find", SymbolKind::Method, Some("Repo"));
    expect("Id", SymbolKind::Type, None);
    expect("Callback", SymbolKind::Type, None);
    expect("Role", SymbolKind::Enum, None);
    expect("Validation", SymbolKind::Module, None);
    expect("Validation.check", SymbolKind::Function, Some("Validation"));
    expect(
        "Validation.internal",
        SymbolKind::Function,
        Some("Validation"),
    );
    expect("MemoryRepo", SymbolKind::Class, None);
    expect(
        "MemoryRepo.constructor",
        SymbolKind::Method,
        Some("MemoryRepo"),
    );
    expect("MemoryRepo.find", SymbolKind::Method, Some("MemoryRepo"));
    expect(
        "MemoryRepo.validate",
        SymbolKind::Method,
        Some("MemoryRepo"),
    );
    expect("MemoryRepo.length", SymbolKind::Method, Some("MemoryRepo"));
    expect("handler", SymbolKind::Function, None);
    expect("local", SymbolKind::Function, None);
    assert_eq!(
        file.symbols.iter().filter(|s| s.name == "over").count(),
        3,
        "every overload is listed"
    );
    assert_eq!(file.symbols.len(), 24);
}

/// TypeScript: signatures without decorators, visibility markers and documentation.
#[test]
fn typescript_signatures_and_visibility() {
    let (file, source) = extract_clean(Language::TypeScript, "typescript_sample.ts");
    assert_eq!(
        symbol(&file, "MemoryRepo").signature,
        "export abstract class MemoryRepo<T extends Entity> extends Store<T> implements Repo<T>, Loggable"
    );
    assert_eq!(
        symbol(&file, "Repo").signature,
        "export interface Repo<T extends Entity> extends Base, Disposable"
    );
    assert_eq!(
        symbol(&file, "Id").signature,
        "export type Id = string | number"
    );
    assert_eq!(
        symbol(&file, "MemoryRepo.find").signature,
        "public async find(id: number): Promise<T | undefined>"
    );
    assert_eq!(
        symbol(&file, "MemoryRepo.secret").signature,
        "private secret(value: Id): Result<Id>"
    );
    assert_eq!(
        symbol(&file, "handler").signature,
        "export const handler = async (req: Request): Promise<Response>"
    );
    assert_eq!(
        symbol(&file, "Validation").signature,
        "export namespace Validation"
    );

    assert_eq!(
        symbol(&file, "Entity").doc.as_deref(),
        Some("Something with an identity.")
    );
    assert_eq!(
        symbol(&file, "MemoryRepo").doc.as_deref(),
        Some("An in-memory repository.")
    );
    assert_eq!(
        symbol(&file, "MemoryRepo.find").doc.as_deref(),
        Some("Finds one item.")
    );

    let vis = |name: &str| symbol(&file, name).visibility;
    assert_eq!(vis("MemoryRepo"), Visibility::Public);
    assert_eq!(vis("MemoryRepo.find"), Visibility::Public);
    assert_eq!(vis("MemoryRepo.secret"), Visibility::Private);
    assert_eq!(vis("MemoryRepo.#hidden"), Visibility::Private);
    assert_eq!(vis("Callback"), Visibility::Private);
    assert_eq!(vis("Validation.check"), Visibility::Public);
    assert_eq!(vis("Validation.internal"), Visibility::Private);
    assert_eq!(vis("local"), Visibility::Private);

    // The decorator is part of the span (documentation above it) but not of the signature.
    let repo = symbol(&file, "MemoryRepo");
    let slice = &source[repo.span.start_byte as usize..repo.span.end_byte as usize];
    assert!(slice.starts_with("/**\n * An in-memory repository.\n */\n@Injectable"));
}

/// TypeScript: type mentions, inheritance and imports.
#[test]
fn typescript_references_and_imports() {
    let (file, _) = extract_clean(Language::TypeScript, "typescript_sample.ts");
    assert_ref(&file, "Base", RefKind::Inherit, Some("Repo"), None);
    assert_ref(&file, "Disposable", RefKind::Inherit, Some("Repo"), None);
    assert_ref(&file, "Store", RefKind::Inherit, Some("MemoryRepo"), None);
    assert_ref(&file, "Repo", RefKind::Inherit, Some("MemoryRepo"), None);
    assert_ref(
        &file,
        "Loggable",
        RefKind::Inherit,
        Some("MemoryRepo"),
        None,
    );
    assert_ref(
        &file,
        "Promise",
        RefKind::Type,
        Some("MemoryRepo.find"),
        None,
    );
    assert_ref(
        &file,
        "Result",
        RefKind::Type,
        Some("MemoryRepo.secret"),
        None,
    );
    assert_ref(&file, "Request", RefKind::Type, Some("handler"), None);
    assert_ref(
        &file,
        "debug",
        RefKind::Call,
        Some("MemoryRepo.find"),
        Some("this.logger"),
    );
    assert_ref(
        &file,
        "inspect",
        RefKind::Call,
        Some("MemoryRepo.save"),
        Some("util"),
    );
    // A decorator call is owned by the class it decorates.
    assert_ref(&file, "Injectable", RefKind::Call, Some("MemoryRepo"), None);
    assert_eq!(file.imports, ["./models", "./logger", "util", "legacy-lib"]);
}

/// TSX: components, class components and JSX element usage.
#[test]
fn tsx_symbols_and_jsx() {
    let (file, _) = extract_clean(Language::Tsx, "tsx_sample.tsx");
    assert_eq!(symbol(&file, "ListProps").kind, SymbolKind::Interface);
    assert_eq!(symbol(&file, "State").kind, SymbolKind::Type);
    assert_eq!(symbol(&file, "PAGE_SIZE").kind, SymbolKind::Constant);
    assert_eq!(symbol(&file, "List").kind, SymbolKind::Function);
    assert_eq!(
        symbol(&file, "List").signature,
        "export function List<T>(props: ListProps<T>): JSX.Element"
    );
    assert_eq!(
        symbol(&file, "List").doc.as_deref(),
        Some("Renders a selectable list.")
    );
    assert_eq!(symbol(&file, "Boundary").kind, SymbolKind::Class);
    assert_eq!(
        symbol(&file, "Boundary").doc.as_deref(),
        Some("A class component with an error boundary.")
    );
    assert_eq!(
        symbol(&file, "Boundary.log").visibility,
        Visibility::Private
    );
    assert_eq!(
        symbol(&file, "Boundary.render").visibility,
        Visibility::Public
    );
    assert_eq!(symbol(&file, "Fallback").kind, SymbolKind::Function);
    assert_eq!(symbol(&file, "helper").visibility, Visibility::Private);
    assert_eq!(symbol(&file, "App").visibility, Visibility::Public);
    assert_eq!(
        symbol(&file, "App").signature,
        "export default function App()"
    );
    // JSX elements with a capitalized name are type mentions; intrinsic elements are not.
    assert_ref(&file, "Button", RefKind::Type, Some("List"), None);
    assert_ref(
        &file,
        "Fallback",
        RefKind::Type,
        Some("Boundary.render"),
        None,
    );
    assert_ref(&file, "Boundary", RefKind::Type, Some("App"), None);
    assert_ref(&file, "List", RefKind::Type, Some("App"), None);
    assert!(
        !file
            .references
            .iter()
            .any(|r| r.name == "ul" || r.name == "li")
    );
    assert_ref(
        &file,
        "Component",
        RefKind::Inherit,
        Some("Boundary"),
        Some("React"),
    );
    assert_ref(&file, "useState", RefKind::Call, Some("List"), None);
    assert_eq!(
        file.imports,
        ["react", "react", "./components/Button", "./List.module.css"]
    );
}

/// The same TypeScript source gives the same symbols as TSX when it has no JSX.
#[test]
fn tsx_accepts_plain_typescript_declarations() {
    let source = "export function f(a: number): number { return g(a); }\n";
    let ts = extract(Language::TypeScript, source);
    let tsx = extract(Language::Tsx, source);
    assert_eq!(ts.symbols, tsx.symbols);
}

/// Anonymous default exports and destructuring declarations are skipped, not mis-named.
#[test]
fn anonymous_and_destructured_declarations() {
    let source = "export default function () {}\nexport const { a, b } = obj;\nconst [c] = arr;\nexport default {};\n";
    let file = extract(Language::JavaScript, source);
    assert!(file.symbols.is_empty(), "{:?}", file.symbols);
}

/// Nested function declarations are listed but never public.
#[test]
fn nested_functions_are_private() {
    let source = "export function outer() {\n  function inner() {}\n  const helper = () => 1;\n  return inner();\n}\n";
    let file = extract(Language::JavaScript, source);
    assert_eq!(symbol(&file, "outer").visibility, Visibility::Public);
    assert_eq!(symbol(&file, "outer.inner").visibility, Visibility::Private);
    assert!(
        file.symbols.iter().all(|s| s.name != "helper"),
        "locals are not listed"
    );
}
