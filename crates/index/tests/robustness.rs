// SPDX-License-Identifier: Apache-2.0
//! Hostile-input tests: every language, grammar-backed or lexical, must return quickly and
//! without panicking on empty, huge, deeply nested, random and unterminated input.
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
use std::time::{Duration, Instant};

use common::assert_invariants;
use pn_ultramemory_core::{ExtractError, Extractor, FileExtract, Language};
use pn_ultramemory_index::TreeSitterExtractor;

/// Every language the extractor handles: the twelve grammars and the fallback languages.
fn all_languages() -> Vec<Language> {
    let mut languages = Language::GRAMMAR_BACKED.to_vec();
    for name in [
        "kotlin", "swift", "scala", "dart", "lua", "shell", "perl", "elixir", "haskell", "clojure",
        "sql", "r", "julia", "zig",
    ] {
        languages.push(Language::from_name(name).unwrap());
    }
    languages
}

/// The slowest acceptable time for one hostile input, generous because tests run unoptimized.
const BUDGET: Duration = Duration::from_secs(60);

/// A deterministic pseudo-random generator, so the inputs are the same on every run.
struct Lcg(u64);

impl Lcg {
    /// Returns the next pseudo-random value.
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }
}

/// Extracts on a thread with a small stack, checks the invariants and the time budget.
///
/// Running on a 512 KiB stack proves that nothing in the extraction recurses with the depth of
/// the input. A parser that gives up (the error the port allows when no tree could be built) is
/// returned to the caller instead of failing the test.
fn try_check(language: Language, label: &str, source: &str) -> Result<FileExtract, ExtractError> {
    let source = source.to_owned();
    let label = label.to_owned();
    let handle = std::thread::Builder::new()
        .stack_size(512 * 1024)
        .spawn(move || {
            let started = Instant::now();
            let outcome = TreeSitterExtractor::new().extract(language, &source);
            let elapsed = started.elapsed();
            assert!(elapsed < BUDGET, "{language} {label} took {elapsed:?}");
            if let Ok(file) = &outcome {
                assert_invariants(file, &source);
            }
            outcome
        })
        .unwrap();
    handle
        .join()
        .unwrap_or_else(|_| panic!("{language}: panic on input"))
}

/// Like [`try_check`] for inputs that every language must be able to read.
fn check(language: Language, label: &str, source: &str) -> FileExtract {
    try_check(language, label, source).unwrap_or_else(|e| panic!("{language} {label}: {e}"))
}

/// Empty and whitespace-only input give an empty extraction.
#[test]
fn empty_and_blank_input() {
    for language in all_languages() {
        for source in ["", "\n", "   \n\t\n", "\r\n\r\n", "\u{feff}"] {
            let file = check(language, "blank", source);
            assert!(file.symbols.is_empty(), "{language}: {source:?}");
            assert!(file.references.is_empty());
            assert!(file.imports.is_empty());
        }
        assert_eq!(check(language, "empty", "").line_count, 0);
        assert_eq!(check(language, "one line", "x").line_count, 1);
        assert_eq!(check(language, "trailing newline", "x\n").line_count, 1);
        assert_eq!(check(language, "two lines", "x\n\n").line_count, 2);
    }
}

/// Files that hold only comments have no symbols.
#[test]
fn only_comments() {
    let comments: [(Language, &str); 8] = [
        (Language::Rust, "// one\n/// two\n/* three */\n//! four\n"),
        (Language::Python, "# one\n# two\n"),
        (Language::JavaScript, "// one\n/** two */\n/* three */\n"),
        (Language::Go, "// one\n/* two */\n"),
        (Language::C, "// one\n/* two\n three */\n"),
        (Language::Ruby, "# one\n=begin\ntwo\n=end\n"),
        (Language::Php, "<?php\n// one\n# two\n/** three */\n"),
        (Language::Other("lua"), "-- one\n--[[ two\nthree ]]\n"),
    ];
    for (language, source) in comments {
        let file = check(language, "comments", source);
        assert!(file.symbols.is_empty(), "{language}: {:?}", file.symbols);
        assert_eq!(file.parse_errors, 0, "{language}");
    }
}

/// A 50 000-line file is handled in bounded time and gives bounded output.
#[test]
fn fifty_thousand_lines() {
    let snippets: [(Language, &str); 5] = [
        (Language::Rust, "pub fn f() { g(); }\n"),
        (Language::Python, "def f():\n    return g()\n"),
        (Language::Java, "class A { void f() { g(); } }\n"),
        (Language::Ruby, "def f\n  g\nend\n"),
        (Language::Other("kotlin"), "fun f() { g() }\n"),
    ];
    for (language, snippet) in snippets {
        let mut source = String::new();
        let mut lines = 0;
        while lines < 50_000 {
            source.push_str(snippet);
            lines += snippet.matches('\n').count();
        }
        let file = check(language, "50k lines", &source);
        assert!(file.line_count >= 50_000, "{language}");
        assert!(
            file.references.len() <= 20_000,
            "{language}: references are capped"
        );
        assert!(!file.symbols.is_empty(), "{language}");
    }
}

/// A 50 000-line file of garbage is bounded too, and reports its syntax errors (or the parser
/// gives up, which the port allows).
#[test]
fn fifty_thousand_lines_of_garbage() {
    let source = "}{ )\n".repeat(50_000);
    for language in [Language::Rust, Language::Go, Language::Other("swift")] {
        match try_check(language, "50k garbage lines", &source) {
            Ok(file) => {
                assert!(file.line_count >= 50_000);
                if language.is_grammar_backed() {
                    assert!(file.parse_errors > 0, "{language}");
                }
            }
            Err(ExtractError::Parse(_)) => assert!(language.is_grammar_backed()),
            Err(other) => panic!("{language}: {other}"),
        }
    }
}

/// Ten thousand nested parentheses, braces and brackets do not overflow the stack.
#[test]
fn deeply_nested_input() {
    for language in all_languages() {
        let depth = 10_000;
        let inputs = [
            (
                "parens",
                format!("x = {}1{}\n", "(".repeat(depth), ")".repeat(depth)),
            ),
            (
                "braces",
                format!("{}{}\n", "{".repeat(depth), "}".repeat(depth)),
            ),
            (
                "brackets",
                format!("x = {}{}\n", "[".repeat(depth), "]".repeat(depth)),
            ),
            ("unclosed", "(".repeat(depth)),
            (
                "calls",
                format!("{}x{}\n", "f(".repeat(depth), ")".repeat(depth)),
            ),
            ("blocks", format!("{}\n", "if x {".repeat(depth))),
        ];
        for (label, source) in inputs {
            check(language, label, &source);
        }
    }
}

/// Nested declarations beyond the limit are ignored, not a failure.
#[test]
fn deeply_nested_declarations() {
    let mut source = String::new();
    for i in 0..3000 {
        write!(source, "function f{i}() {{ ").unwrap();
    }
    let file = check(Language::JavaScript, "nested functions", &source);
    assert!(file.symbols.len() <= 64);
    let mut source = String::new();
    for i in 0..3000 {
        write!(source, "class C{i} {{ ").unwrap();
    }
    let file = check(Language::Java, "nested classes", &source);
    assert!(file.symbols.len() <= 64);
}

/// Random bytes, read lossily as UTF-8, never panic and always stay within the bounds.
#[test]
fn random_bytes() {
    let mut random = Lcg(0x5eed);
    for round in 0..2 {
        let bytes: Vec<u8> = (0..30_000).map(|_| (random.next() & 0xff) as u8).collect();
        let source = String::from_utf8_lossy(&bytes).into_owned();
        for language in all_languages() {
            let file = check(language, &format!("random {round}"), &source);
            if language.is_grammar_backed() {
                assert!(
                    file.parse_errors > 0,
                    "{language}: random bytes are not valid code"
                );
            }
        }
    }
}

/// Random text built from language tokens exercises the recovery paths of every grammar.
///
/// Some grammars can take very long on such input; a parser that gives up is an acceptable
/// outcome, a panic or a hang is not.
#[test]
fn random_token_soup() {
    let tokens = [
        "fn ",
        "def ",
        "class ",
        "function ",
        "func ",
        "struct ",
        "impl ",
        "{",
        "}",
        "(",
        ")",
        "[",
        "]",
        ";",
        ":",
        ",",
        "=",
        "=>",
        "->",
        "::",
        ".",
        "\"",
        "'",
        "`",
        "/*",
        "*/",
        "//",
        "#",
        "\n",
        "    ",
        "x",
        "foo",
        "Bar",
        "1",
        "pub ",
        "private ",
        "public ",
        "static ",
        "@",
        "#[",
        "<",
        ">",
        "end",
        "do",
        "if ",
        "else",
        "return ",
    ];
    let mut random = Lcg(42);
    for language in all_languages() {
        for round in 0..3 {
            let mut source = String::new();
            for _ in 0..1200 {
                let pick = random.next() % u64::try_from(tokens.len()).unwrap();
                source.push_str(tokens[usize::try_from(pick).unwrap()]);
            }
            match try_check(language, &format!("soup {round}"), &source) {
                Ok(_) | Err(ExtractError::Parse(_)) => {}
                Err(other) => panic!("{language}: {other}"),
            }
        }
    }
}

/// Unterminated strings, comments and blocks are tolerated.
#[test]
fn unterminated_constructs() {
    let inputs = [
        "let s = \"never closed\nfn after() {}\n",
        "fn f() { /* never closed\nfn g() {}\n",
        "x = '''triple\nnever closed\ndef f(): pass\n",
        "class A {\n  void f() {\n",
        "def f():\n    \"\"\"open docstring\n",
        "<?php\nfunction f() {\n  $x = \"open\n",
        "fun a() {\n  val s = \"\"\"raw\n",
        "func (r *T) M() {\n\tx := `raw\n",
        "`template ${ never closed\nfunction f() {}\n",
        "#define X \\\n",
        "namespace N {\nclass A {\n",
        "(defn f [x]\n  (str \"open\n",
    ];
    for language in all_languages() {
        for (i, source) in inputs.iter().enumerate() {
            check(language, &format!("unterminated {i}"), source);
        }
    }
}

/// Very long lines and very long names are bounded.
#[test]
fn extreme_lines_and_names() {
    for language in all_languages() {
        let long_name = "a".repeat(50_000);
        check(
            language,
            "long identifier",
            &format!("fn {long_name}() {{}}\ndef {long_name}():\n  pass\n"),
        );
        check(
            language,
            "one huge line",
            &format!("f({})\n", "a, ".repeat(30_000)),
        );
        check(
            language,
            "long signature",
            &format!("fn f({}) {{}}\n", "x: i32, ".repeat(8_000)),
        );
        let mut many_calls = String::new();
        for i in 0..25_000 {
            write!(many_calls, "call{i}();").unwrap();
        }
        let file = check(
            language,
            "many calls",
            &format!("fn f() {{ {many_calls} }}\n"),
        );
        assert!(file.references.len() <= 20_000, "{language}");
        for symbol in &file.symbols {
            assert!(symbol.outline.len() <= 24, "{language}");
            assert!(symbol.signature.chars().count() <= 240, "{language}");
        }
    }
}

/// Unicode identifiers, emoji, combining marks, right-to-left text and NUL characters are fine.
#[test]
fn unicode_and_control_characters() {
    let inputs = [
        "fn ünïcödé_名前() { 日本語(); }\n// 🦀 emoji comment\n",
        "def función_ñ():\n    \"\"\"Docstring with 🐍 and é.\"\"\"\n    return ñandú()\n",
        "class Ünï { void 함수() {} }\n",
        "func Ω() { fmt.Println(\"héllo\") }\n",
        "fn a\u{0}b() {}\n\u{0}\u{0}\u{0}\n",
        "// \u{202e}rtl override\nfn f() {}\n",
        "fn e\u{0301}() {}\n",
        "fn f() {}\u{2028}fn g() {}\u{2029}\n",
    ];
    for language in all_languages() {
        for (i, source) in inputs.iter().enumerate() {
            check(language, &format!("unicode {i}"), source);
        }
    }
    let file = check(Language::Rust, "unicode symbol", inputs[0]);
    assert_eq!(file.symbols[0].name, "ünïcödé_名前");
    let file = check(Language::Python, "unicode docstring", inputs[1]);
    assert_eq!(
        file.symbols[0].doc.as_deref(),
        Some("Docstring with 🐍 and é.")
    );
}

/// Line-ending variants: CRLF, lone CR and mixed endings give the same symbols.
#[test]
fn line_endings() {
    let lf = "// doc\nfn a() {\n    b();\n}\n\nfn c() {}\n";
    let crlf = lf.replace('\n', "\r\n");
    let extractor = TreeSitterExtractor::new();
    let a = extractor.extract(Language::Rust, lf).unwrap();
    let b = extractor.extract(Language::Rust, &crlf).unwrap();
    assert_eq!(a.symbols.len(), b.symbols.len());
    for (x, y) in a.symbols.iter().zip(&b.symbols) {
        assert_eq!(
            (x.name.as_str(), x.kind, &x.doc),
            (y.name.as_str(), y.kind, &y.doc)
        );
        assert_eq!(x.span.start_line, y.span.start_line);
        assert_eq!(
            x.body_hash, y.body_hash,
            "hashes ignore whitespace, so line endings do not matter"
        );
    }
    let cr_only = lf.replace('\n', "\r");
    check(Language::Rust, "lone CR", &cr_only);
    check(Language::Other("kotlin"), "lone CR", &cr_only);
}

/// The extractor can be shared between threads and gives the same answer everywhere.
#[test]
fn shared_between_threads() {
    let extractor = std::sync::Arc::new(TreeSitterExtractor::new());
    let source = "pub fn f() { g(); }\nstruct S;\nimpl S { fn m(&self) {} }\n";
    let expected = extractor.extract(Language::Rust, source).unwrap();
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let extractor = extractor.clone();
            std::thread::spawn(move || {
                (0..50).all(|_| {
                    extractor.extract(Language::Rust, source).unwrap() == expected_clone(source)
                })
            })
        })
        .collect();
    for handle in handles {
        assert!(handle.join().unwrap());
    }
    assert_eq!(expected.symbols.len(), 3);
}

/// Extracts a source with a fresh extractor (a helper for the threads above).
fn expected_clone(source: &str) -> FileExtract {
    TreeSitterExtractor::new()
        .extract(Language::Rust, source)
        .unwrap()
}
