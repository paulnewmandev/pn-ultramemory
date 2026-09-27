// SPDX-License-Identifier: Apache-2.0
//! Edge cases of extraction that the fixtures do not cover: import forms, output bounds,
//! multi-byte text before a symbol, and reference details.
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

use common::{assert_ref, extract, symbol};
use pn_ultramemory_core::{Language, RefKind, SymbolKind};

/// Rust `use` forms: nested groups, `self`, aliases, globs and re-exports.
#[test]
fn rust_use_forms() {
    let source = "use a::{b::{c, d}, e, f::{self, g as h}};\npub use crate::x::*;\nuse ::std::io;\nextern crate core;\nfn f() {}\n";
    let file = extract(Language::Rust, source);
    assert_eq!(
        file.imports,
        [
            "a::b::c",
            "a::b::d",
            "a::e",
            "a::f",
            "a::f::g",
            "crate::x::*",
            "::std::io",
        ]
    );
}

/// Rust macro invocations with a path, and calls through generics.
#[test]
fn rust_macros_and_generic_calls() {
    let source = "fn f() {\n    std::println!(\"x\");\n    let v = Vec::<u8>::with_capacity(2);\n    helper::<u8>(1);\n}\n";
    let file = extract(Language::Rust, source);
    assert_ref(&file, "println", RefKind::Call, Some("f"), Some("std"));
    assert_ref(
        &file,
        "with_capacity",
        RefKind::Call,
        Some("f"),
        Some("Vec"),
    );
    assert_ref(&file, "helper", RefKind::Call, Some("f"), None);
}

/// References are capped at 20 000 and the outline at 24 distinct names, in order.
#[test]
fn output_is_bounded() {
    let mut source = String::from("fn many() {\n");
    for i in 0..30_000 {
        writeln!(source, "    call{i}();").unwrap();
    }
    source.push_str("}\nfn after() { late(); }\n");
    let file = extract(Language::Rust, &source);
    assert_eq!(file.references.len(), 20_000);
    let outline = &symbol(&file, "many").outline;
    assert_eq!(outline.len(), 24);
    assert_eq!(outline[0], "call0");
    assert_eq!(outline[23], "call23");
    // The reference cap drops the last references, but outlines keep collecting names.
    assert!(!file.references.iter().any(|r| r.name == "late"));
    assert_eq!(symbol(&file, "after").outline, ["late"]);
}

/// A signature longer than 240 characters is shortened, on one line, ending with `...`.
#[test]
fn long_signatures_are_shortened() {
    let parameters: Vec<String> = (0..80).map(|i| format!("argument_{i}: u32")).collect();
    let source = format!("fn long({}) {{}}\n", parameters.join(",\n    "));
    let file = extract(Language::Rust, &source);
    let signature = &symbol(&file, "long").signature;
    assert_eq!(signature.chars().count(), 240);
    assert!(signature.starts_with("fn long(argument_0: u32, argument_1: u32"));
    assert!(signature.ends_with("..."));
    assert!(!signature.contains('\n'));
}

/// Symbols after multi-byte text still get exact byte spans and line numbers.
#[test]
fn multibyte_text_before_symbols() {
    let source = "// 日本語のコメント 🦀\nconst S: &str = \"héllo wörld ✓\";\n\n/// Ünïcode doc.\nfn ünï() {\n    println!(\"→\");\n}\n";
    let file = extract(Language::Rust, source);
    let function = symbol(&file, "ünï");
    assert_eq!(function.doc.as_deref(), Some("Ünïcode doc."));
    assert_eq!(function.span.start_line, 4);
    assert_eq!(function.span.end_line, 7);
    let slice = &source[function.span.start_byte as usize..function.span.end_byte as usize];
    assert!(slice.starts_with("/// Ünïcode doc.\nfn ünï()"));
    assert!(slice.ends_with('}'));
    assert_eq!(symbol(&file, "S").kind, SymbolKind::Constant);
}

/// Python import forms: aliases, several modules, relative and star imports.
#[test]
fn python_import_forms() {
    let source = "import a.b as ab, c\nfrom . import x as y, z\nfrom .. import q\nfrom ..pkg import (\n    r,\n    s,\n)\nfrom m import *\nfrom .rel import w\nif True:\n    import lazy\n";
    let file = extract(Language::Python, source);
    assert_eq!(
        file.imports,
        ["a.b", "c", ".x", ".z", "..q", "..pkg", "m", ".rel", "lazy"]
    );
}

/// JavaScript module forms: dynamic import, re-exports and `CommonJS` in nested positions.
#[test]
fn javascript_import_forms() {
    let source = "import a from 'a';\nimport 'side';\nexport * from './all';\nexport { x } from \"./named\";\nasync function load() {\n  const m = await import('./lazy');\n  return require(`tpl`);\n}\nconst cfg = require('./config');\n";
    let file = extract(Language::JavaScript, source);
    assert_eq!(
        file.imports,
        ["a", "side", "./all", "./named", "./lazy", "tpl", "./config"]
    );
    assert!(!file.references.iter().any(|r| r.name == "require"));
    assert!(
        file.symbols.iter().all(|s| s.name != "cfg"),
        "require results are not symbols"
    );
}

/// Go import forms: grouped, aliased, dot and raw-string imports.
#[test]
fn go_import_forms() {
    let source = "package p\nimport (\n\t. \"dot\"\n\talias \"path/to/pkg\"\n\t`raw`\n)\nimport \"single\"\n";
    let file = extract(Language::Go, source);
    assert_eq!(file.imports, ["dot", "path/to/pkg", "raw", "single"]);
}

/// C and C++ include forms lose their delimiters; conditional includes are still found.
#[test]
fn include_forms() {
    let source =
        "#include <sys/types.h>\n#include \"local/api.h\"\n#ifdef X\n#include <x.h>\n#endif\n";
    assert_eq!(
        extract(Language::C, source).imports,
        ["sys/types.h", "local/api.h", "x.h"]
    );
    assert_eq!(extract(Language::Cpp, source).imports.len(), 3);
}

/// Ruby: `Foo::Bar.new` is a type mention with its scope, accessors are not symbols.
#[test]
fn ruby_new_and_accessors() {
    let source = "class A\n  attr_accessor :x, :y\n  def build\n    Foo::Bar.new(1)\n    baz.new\n  end\nend\n";
    let file = extract(Language::Ruby, source);
    assert_ref(&file, "Bar", RefKind::Type, Some("A.build"), Some("Foo"));
    assert_eq!(file.symbols.len(), 2);
    assert_ref(&file, "new", RefKind::Call, Some("A.build"), Some("baz"));
}

/// PHP: braced namespaces and `define` calls.
#[test]
fn php_braced_namespace() {
    let source = "<?php\nnamespace A\\B {\n    class C {\n        public function m() {}\n    }\n}\nnamespace {\n    function global_fn() {}\n}\n";
    let file = extract(Language::Php, source);
    assert_eq!(symbol(&file, "A.B").name, "B");
    assert_eq!(symbol(&file, "A.B").kind, SymbolKind::Module);
    assert_eq!(symbol(&file, "A.B.C.m").kind, SymbolKind::Method);
    assert_eq!(symbol(&file, "A.B.C").parent, Some(0));
    assert!(file.symbols.iter().any(|s| s.name == "global_fn"));
}

/// TypeScript: overloads, abstract members, parameter properties and `declare` blocks.
#[test]
fn typescript_declarations() {
    let source = "declare function ext(a: number): string;\ndeclare module 'm' {\n  export interface I { go(): void }\n}\nexport default abstract class Base {\n  abstract run(): void;\n  constructor(protected readonly dep: Dep) {}\n}\n";
    let file = extract(Language::TypeScript, source);
    assert_eq!(symbol(&file, "ext").kind, SymbolKind::Function);
    assert_eq!(symbol(&file, "m").kind, SymbolKind::Module);
    assert_eq!(symbol(&file, "m.I").kind, SymbolKind::Interface);
    assert_eq!(symbol(&file, "m.I.go").kind, SymbolKind::Method);
    assert_eq!(symbol(&file, "Base").kind, SymbolKind::Class);
    assert_eq!(symbol(&file, "Base.run").signature, "abstract run(): void");
    assert_ref(&file, "Dep", RefKind::Type, Some("Base.constructor"), None);
}

/// Java: constructors, static nested classes, generic methods and annotation members.
#[test]
fn java_members() {
    let source = "public class Outer {\n    public static class Inner { }\n    public <T> T pick(T a) { return a; }\n    private Outer() { }\n}\n";
    let file = extract(Language::Java, source);
    assert_eq!(symbol(&file, "Outer.Inner").kind, SymbolKind::Class);
    assert_eq!(
        symbol(&file, "Outer.pick").signature,
        "public <T> T pick(T a)"
    );
    assert_eq!(symbol(&file, "Outer.Outer").signature, "private Outer()");
}

/// A C++ header read as C is re-read with the C++ grammar when that reads it better, and the
/// result says which language it was parsed as.
#[test]
fn cpp_headers_named_as_c_are_read_as_cpp() {
    let source = "#ifndef Z_H\n#define Z_H\nnamespace fp {\nclass Rng {\npublic:\n    Rng(unsigned seed = 1);\n    unsigned next();\nprivate:\n    unsigned state_;\n};\n}\n#endif\n";
    let file = extract(Language::C, source);
    assert_eq!(file.language, Language::Cpp);
    assert_eq!(file.parse_errors, 0);
    assert_eq!(symbol(&file, "fp::Rng").kind, SymbolKind::Class);
    assert_eq!(symbol(&file, "fp::Rng::next").kind, SymbolKind::Method);
    // Plain C stays C, and broken C that C++ does not read better stays C.
    assert_eq!(
        extract(Language::C, "int f(void) { return 1; }\n").language,
        Language::C
    );
    let broken = extract(Language::C, "int f( {\n");
    assert_eq!(broken.language, Language::C);
    assert!(broken.parse_errors > 0);
}

/// A macro that continues over several lines, with blank lines after it, has exact line numbers.
#[test]
fn c_macro_with_continuations() {
    let source =
        "#define LIST(X) \\\n    X(1) \\\n    X(2) \\\n\n\nint after(void) { return 1; }\n";
    let file = extract(Language::C, source);
    let list = symbol(&file, "LIST");
    assert_eq!(list.span.start_line, 1);
    assert!(list.span.end_line <= 3, "{:?}", list.span);
    assert_eq!(symbol(&file, "after").span.start_line, 6);
}
