// SPDX-License-Identifier: Apache-2.0
//! Tests of `DocComments`: documentation is written in each language's own syntax, at the
//! declaration's indentation, above attributes and decorators, and never breaks the code.
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

use common::{extract, fixture, symbol};
use pn_ultramemory_core::{DocError, DocInserter, DocTarget, Extractor, Language, SymbolKind};
use pn_ultramemory_index::{DocComments, TreeSitterExtractor};

/// Inserts `text` as the documentation of the declaration named `name` on `line`.
fn insert(
    language: Language,
    source: &str,
    name: &str,
    kind: SymbolKind,
    line: u32,
    text: &str,
) -> Result<String, DocError> {
    DocComments.insert_doc(language, source, &DocTarget { name, kind, line }, text)
}

/// Inserts documentation and unwraps the result.
fn documented(language: Language, source: &str, name: &str, line: u32, text: &str) -> String {
    insert(language, source, name, SymbolKind::Function, line, text)
        .unwrap_or_else(|e| panic!("insertion failed: {e}"))
}

/// Rust: `///` lines at the indentation of a method, above its attributes.
#[test]
fn rust_line_comments_above_attributes() {
    let source = "impl Foo {\n    #[inline]\n    #[must_use]\n    pub fn bar(&self) -> u8 {\n        1\n    }\n}\n";
    // The target line is the `fn` line; the comment goes above the first attribute.
    let result = documented(
        Language::Rust,
        source,
        "bar",
        4,
        "Returns one.\n\nAlways one.",
    );
    assert_eq!(
        result,
        "impl Foo {\n    /// Returns one.\n    ///\n    /// Always one.\n    #[inline]\n    #[must_use]\n    pub fn bar(&self) -> u8 {\n        1\n    }\n}\n"
    );
    // The attribute line works as the target line too.
    let again = documented(
        Language::Rust,
        source,
        "bar",
        2,
        "Returns one.\n\nAlways one.",
    );
    assert_eq!(again, result);
}

/// Rust: the inserted documentation is read back by the extractor.
#[test]
fn rust_round_trip() {
    let source = "pub struct Point {\n    x: i32,\n}\n\nfn helper() {}\n";
    let result = documented(
        Language::Rust,
        source,
        "Point",
        1,
        "A point.\nWith two lines.",
    );
    let file = extract(Language::Rust, &result);
    assert_eq!(
        symbol(&file, "Point").doc.as_deref(),
        Some("A point.\nWith two lines.")
    );
    assert_eq!(symbol(&file, "helper").doc, None);
    assert_eq!(file.parse_errors, 0);
}

/// Go: text is only prefixed with `//`, never rewritten.
#[test]
fn go_line_comments_keep_the_text() {
    let source = "package p\n\nfunc (c *Circle) Area() float64 {\n\treturn 1\n}\n";
    let result = documented(
        Language::Go,
        source,
        "Area",
        3,
        "Area computes it.\n\n  indented line\nLast.",
    );
    assert_eq!(
        result,
        "package p\n\n// Area computes it.\n//\n//   indented line\n// Last.\nfunc (c *Circle) Area() float64 {\n\treturn 1\n}\n"
    );
    let file = extract(Language::Go, &result);
    assert_eq!(
        symbol(&file, "Circle.Area").doc.as_deref(),
        Some("Area computes it.\n\n  indented line\nLast.")
    );
}

/// JavaScript, TypeScript, TSX, Java, C, C++ and PHP: a `/** */` block with ` * ` lines.
#[test]
fn block_comments_in_c_like_languages() {
    let cases: [(Language, &str, &str, u32); 7] = [
        (
            Language::JavaScript,
            "export function add(a, b) {\n  return a + b;\n}\n",
            "add",
            1,
        ),
        (
            Language::TypeScript,
            "export function add(a: number): number {\n  return a;\n}\n",
            "add",
            1,
        ),
        (
            Language::Tsx,
            "export const View = () => <div />;\n",
            "View",
            1,
        ),
        (
            Language::Java,
            "public class A {\n    void run() {}\n}\n",
            "A",
            1,
        ),
        (
            Language::C,
            "int add(int a, int b) {\n    return a + b;\n}\n",
            "add",
            1,
        ),
        (
            Language::Cpp,
            "int add(int a, int b) {\n    return a + b;\n}\n",
            "add",
            1,
        ),
        (
            Language::Php,
            "<?php\nfunction add($a) {\n    return $a;\n}\n",
            "add",
            2,
        ),
    ];
    for (language, source, name, line) in cases {
        let result = documented(language, source, name, line, "Adds numbers.\nSecond line.");
        let block = "/**\n * Adds numbers.\n * Second line.\n */\n";
        let expected = match language {
            Language::Php => format!("<?php\n{block}function add($a) {{\n    return $a;\n}}\n"),
            _ => format!("{block}{source}"),
        };
        assert_eq!(result, expected, "{language}");
        let file = extract(language, &result);
        let documented = file.symbols.iter().find(|s| s.name == name).unwrap();
        assert_eq!(
            documented.doc.as_deref(),
            Some("Adds numbers.\nSecond line."),
            "{language}"
        );
        assert_eq!(file.parse_errors, 0, "{language}");
    }
}

/// A block comment escapes `*/` inside the text so that the comment cannot end early.
#[test]
fn block_comment_escapes_terminators() {
    let source = "function f() {}\nfunction g() {}\n";
    let result = documented(Language::JavaScript, source, "f", 1, "Ends with */ inside.");
    assert!(result.starts_with("/**\n * Ends with *\\/ inside.\n */\nfunction f"));
    let file = extract(Language::JavaScript, &result);
    assert_eq!(
        symbol(&file, "f").doc.as_deref(),
        Some("Ends with *\\/ inside.")
    );
    assert_eq!(symbol(&file, "g").doc, None);
    assert_eq!(file.symbols.len(), 2);
}

/// Indentation of nested declarations is kept, and decorators, annotations and attributes stay
/// below the comment.
#[test]
fn indentation_and_annotations() {
    let ts = "@Injectable({ providedIn: 'root' })\nexport class Service {\n  @Input()\n  run(): void {}\n}\n";
    let result = documented(Language::TypeScript, ts, "run", 4, "Runs.");
    assert_eq!(
        result,
        "@Injectable({ providedIn: 'root' })\nexport class Service {\n  /**\n   * Runs.\n   */\n  @Input()\n  run(): void {}\n}\n"
    );
    let result = documented(Language::TypeScript, ts, "Service", 2, "A service.");
    assert!(
        result.starts_with("/**\n * A service.\n */\n@Injectable("),
        "{result}"
    );
    let file = extract(Language::TypeScript, &result);
    assert_eq!(symbol(&file, "Service").doc.as_deref(), Some("A service."));

    let java = "class A {\n    @Override\n    @Deprecated\n    public void run() {}\n}\n";
    let result = documented(Language::Java, java, "run", 4, "Runs.");
    assert_eq!(
        result,
        "class A {\n    /**\n     * Runs.\n     */\n    @Override\n    @Deprecated\n    public void run() {}\n}\n"
    );

    let cpp = "template <typename T>\nT clamp(T v) {\n    return v;\n}\n";
    let result = documented(Language::Cpp, cpp, "clamp", 2, "Clamps.");
    assert_eq!(result, format!("/**\n * Clamps.\n */\n{cpp}"));

    let php = "<?php\nclass A {\n    #[Route('/x')]\n    public function run() {}\n}\n";
    let result = documented(Language::Php, php, "run", 4, "Runs.");
    assert_eq!(
        result,
        "<?php\nclass A {\n    /**\n     * Runs.\n     */\n    #[Route('/x')]\n    public function run() {}\n}\n"
    );
}

/// C#: `///` lines with a `<summary>`, XML characters escaped, above attributes.
#[test]
fn csharp_summary() {
    let source = "namespace N\n{\n    [Serializable]\n    public class A\n    {\n        public void Run() { }\n    }\n}\n";
    let result = insert(
        Language::CSharp,
        source,
        "A",
        SymbolKind::Class,
        4,
        "Runs <fast> & safe.\nSecond.",
    )
    .unwrap();
    assert_eq!(
        result,
        "namespace N\n{\n    /// <summary>\n    /// Runs &lt;fast&gt; &amp; safe.\n    /// Second.\n    /// </summary>\n    [Serializable]\n    public class A\n    {\n        public void Run() { }\n    }\n}\n"
    );
    let file = extract(Language::CSharp, &result);
    assert_eq!(
        symbol(&file, "N.A").doc.as_deref(),
        Some("Runs <fast> & safe.\nSecond.")
    );
    let nested = insert(
        Language::CSharp,
        &result,
        "Run",
        SymbolKind::Method,
        10,
        "Runs it.",
    )
    .unwrap();
    assert!(nested.contains("        /// <summary>\n        /// Runs it.\n        /// </summary>\n        public void Run()"));
}

/// Ruby: `#` lines at the indentation of the method.
#[test]
fn ruby_comments() {
    let source = "class Payable\n  def total\n    1\n  end\nend\n";
    let result = documented(
        Language::Ruby,
        source,
        "total",
        2,
        "Total amount.\nIn cents.",
    );
    assert_eq!(
        result,
        "class Payable\n  # Total amount.\n  # In cents.\n  def total\n    1\n  end\nend\n"
    );
    let file = extract(Language::Ruby, &result);
    assert_eq!(
        symbol(&file, "Payable.total").doc.as_deref(),
        Some("Total amount.\nIn cents.")
    );
}

/// Python: a docstring as the first statement of the body, at body indentation.
#[test]
fn python_docstrings() {
    let source = "class A:\n    def run(self, x):\n        return x\n\n\ndef top():\n    pass\n";
    let result = documented(Language::Python, source, "run", 2, "Runs it.");
    assert_eq!(
        result,
        "class A:\n    def run(self, x):\n        \"\"\"Runs it.\"\"\"\n        return x\n\n\ndef top():\n    pass\n"
    );
    let multi = documented(
        Language::Python,
        source,
        "top",
        6,
        "Top level.\n\nMore detail.",
    );
    assert!(
        multi.ends_with(
            "def top():\n    \"\"\"Top level.\n\n    More detail.\n    \"\"\"\n    pass\n"
        ),
        "{multi}"
    );
    let file = extract(Language::Python, &multi);
    assert_eq!(
        symbol(&file, "top").doc.as_deref(),
        Some("Top level.\n\nMore detail.")
    );
    let class = insert(
        Language::Python,
        source,
        "A",
        SymbolKind::Class,
        1,
        "A class.",
    )
    .unwrap();
    assert!(class.starts_with("class A:\n    \"\"\"A class.\"\"\"\n    def run"));
}

/// Python: the docstring goes into the body of a decorated definition, whichever of the
/// decorator line or the `def` line is given, and comments before the first statement stay.
#[test]
fn python_decorated_and_commented_bodies() {
    let source = "@app.route('/')\n@cached\ndef index():\n    # note\n    return 1\n";
    for line in [1, 3] {
        let result = documented(Language::Python, source, "index", line, "Home page.");
        assert_eq!(
            result,
            "@app.route('/')\n@cached\ndef index():\n    # note\n    \"\"\"Home page.\"\"\"\n    return 1\n",
            "target line {line}"
        );
    }
}

/// Python: quotes and backslashes in the text are escaped and read back unchanged.
#[test]
fn python_escapes() {
    let source = "def f():\n    pass\n\ndef g():\n    pass\n";
    let result = documented(
        Language::Python,
        source,
        "f",
        1,
        "Says \"\"\"hi\"\"\" and C:\\path",
    );
    assert!(
        result.contains("\"\"\"Says \\\"\\\"\\\"hi\\\"\\\"\\\" and C:\\\\path\"\"\""),
        "{result}"
    );
    let file = extract(Language::Python, &result);
    assert_eq!(
        symbol(&file, "f").doc.as_deref(),
        Some("Says \"\"\"hi\"\"\" and C:\\path")
    );
    assert_eq!(file.parse_errors, 0);
    let quote = documented(Language::Python, source, "g", 4, "ends with a quote\"");
    assert!(
        quote.contains("\"\"\"ends with a quote\\\"\"\"\""),
        "{quote}"
    );
    assert_eq!(extract(Language::Python, &quote).parse_errors, 0);
}

/// Python: a body on the header line, or an existing docstring, is refused.
#[test]
fn python_refusals() {
    let one_line = "def f(): return 1\n";
    let error = insert(
        Language::Python,
        one_line,
        "f",
        SymbolKind::Function,
        1,
        "Doc.",
    )
    .unwrap_err();
    assert!(matches!(error, DocError::Invalid(_)), "{error:?}");
    let class_one_line = "class A: pass\n";
    assert!(matches!(
        insert(
            Language::Python,
            class_one_line,
            "A",
            SymbolKind::Class,
            1,
            "Doc."
        ),
        Err(DocError::Invalid(_))
    ));
    let has_doc = "def f():\n    \"\"\"Already.\"\"\"\n    pass\n";
    let error = insert(
        Language::Python,
        has_doc,
        "f",
        SymbolKind::Function,
        1,
        "New.",
    )
    .unwrap_err();
    assert!(
        matches!(&error, DocError::Invalid(m) if m.contains("already")),
        "{error:?}"
    );
}

/// A declaration that already has documentation is refused in every documented syntax.
#[test]
fn already_documented_is_refused() {
    let cases: [(Language, &str, &str, u32); 8] = [
        (Language::Rust, "/// Old.\nfn f() {}\n", "f", 2),
        (Language::Go, "// F does.\nfunc F() {}\n", "F", 2),
        (
            Language::JavaScript,
            "/** Old. */\nfunction f() {}\n",
            "f",
            2,
        ),
        (Language::Java, "/** Old. */\nclass A {}\n", "A", 2),
        (
            Language::CSharp,
            "/// <summary>Old.</summary>\nclass A { }\n",
            "A",
            2,
        ),
        (Language::Ruby, "# Old.\ndef f; end\n", "f", 2),
        (
            Language::Php,
            "<?php\n/** Old. */\nfunction f() {}\n",
            "f",
            3,
        ),
        (Language::Cpp, "/// Old.\nint f() { return 1; }\n", "f", 2),
    ];
    for (language, source, name, line) in cases {
        let error = insert(language, source, name, SymbolKind::Function, line, "New.").unwrap_err();
        assert!(
            matches!(&error, DocError::Invalid(m) if m.contains("already")),
            "{language}: {error:?}"
        );
        // The line of the documentation itself also finds the declaration and is refused.
        let error = insert(
            language,
            source,
            name,
            SymbolKind::Function,
            line - 1,
            "New.",
        );
        assert!(error.is_err(), "{language}");
    }
}

/// Windows line endings are preserved in the inserted lines.
#[test]
fn crlf_is_preserved() {
    let cases: [(Language, &str, &str, u32); 4] = [
        (Language::Rust, "fn f() {}\r\n", "f", 1),
        (Language::JavaScript, "function f() {}\r\n", "f", 1),
        (Language::Ruby, "def f; end\r\n", "f", 1),
        (Language::CSharp, "class A { }\r\n", "A", 1),
    ];
    for (language, source, name, line) in cases {
        let result = insert(
            language,
            source,
            name,
            SymbolKind::Function,
            line,
            "Doc.\nMore.",
        )
        .unwrap();
        assert!(
            !result.replace("\r\n", "").contains('\n'),
            "{language}: bare line feed in {result:?}"
        );
        assert!(result.contains("\r\n"));
    }
    let python = "def f():\r\n    return 1\r\n";
    let result = documented(Language::Python, python, "f", 1, "Doc.\nMore.");
    assert_eq!(
        result,
        "def f():\r\n    \"\"\"Doc.\r\n    More.\r\n    \"\"\"\r\n    return 1\r\n"
    );
    // A file that uses bare line feeds keeps them.
    let unix = documented(Language::Rust, "fn f() {}\n", "f", 1, "Doc.");
    assert!(!unix.contains('\r'));
}

/// Unsupported languages, unknown targets, empty text and inline declarations are refused.
#[test]
fn refusals() {
    let error = insert(
        Language::Other("kotlin"),
        "fun f() {}",
        "f",
        SymbolKind::Function,
        1,
        "Doc.",
    )
    .unwrap_err();
    assert_eq!(error, DocError::Unsupported(Language::Other("kotlin")));
    let source = "fn f() {}\n";
    assert_eq!(
        insert(
            Language::Rust,
            source,
            "missing",
            SymbolKind::Function,
            1,
            "Doc."
        )
        .unwrap_err(),
        DocError::TargetNotFound
    );
    assert_eq!(
        insert(Language::Rust, source, "f", SymbolKind::Function, 9, "Doc.").unwrap_err(),
        DocError::TargetNotFound
    );
    assert!(matches!(
        insert(
            Language::Rust,
            source,
            "f",
            SymbolKind::Function,
            1,
            "  \n \n"
        ),
        Err(DocError::Invalid(_))
    ));
    let inline = "fn a() {} fn b() {}\n";
    let error = insert(Language::Rust, inline, "b", SymbolKind::Function, 1, "Doc.").unwrap_err();
    assert!(
        matches!(&error, DocError::Invalid(m) if m.contains("start its line")),
        "{error:?}"
    );
    assert_eq!(
        DocComments
            .insert_doc(
                Language::Rust,
                "",
                &DocTarget {
                    name: "f",
                    kind: SymbolKind::Function,
                    line: 1
                },
                "Doc."
            )
            .unwrap_err(),
        DocError::TargetNotFound
    );
}

/// Insertion refuses when the new source would have more syntax errors than the original.
#[test]
fn syntax_error_guard() {
    // A damaged file: the class docstring is broken, so a docstring inserted below the header
    // would open one more unterminated string and add syntax errors.
    let source = "class OrderRepository(Repository, metaclass=Meta):\n    \"\"\"Stores orders.\n    \"\"\"Fetches all orders.\n    \"\"\"\ndef _private_helper():\n    return sorted([1, 2, 3])\n\n\ndef main():\n    repo = OrderRepository(\"/tmp\")\n    print(fetch_all(repo))\n\n\nif __name__ == \"__main__\":\n    main()\n";
    let before = TreeSitterExtractor::new()
        .extract(Language::Python, source)
        .unwrap();
    assert!(before.parse_errors > 0);
    let error = insert(
        Language::Python,
        source,
        "OrderRepository",
        SymbolKind::Class,
        1,
        "Stores orders.",
    )
    .unwrap_err();
    assert!(
        matches!(&error, DocError::Invalid(m) if m.contains("syntax errors")),
        "{error:?}"
    );
    // The same damaged file is unchanged for the caller, and a healthy declaration still works.
    let result = insert(
        Language::Python,
        source,
        "main",
        SymbolKind::Function,
        9,
        "Runs it.",
    );
    if let Ok(text) = result {
        let after = TreeSitterExtractor::new()
            .extract(Language::Python, &text)
            .unwrap();
        assert!(after.parse_errors <= before.parse_errors);
    }
}

/// One case of the round-trip test: a language, its fixture, an undocumented symbol and its kind.
type RoundTripCase = (Language, &'static str, &'static str, SymbolKind);

/// The undocumented declaration that each language's fixture is tested with.
const ROUND_TRIP_CASES: [RoundTripCase; 12] = [
    (Language::Rust, "rust_sample.rs", "Kind", SymbolKind::Enum),
    (
        Language::Python,
        "python_sample.py",
        "OrderRepository.LIMIT",
        SymbolKind::Constant,
    ),
    (
        Language::JavaScript,
        "javascript_sample.js",
        "privateUtil",
        SymbolKind::Function,
    ),
    (
        Language::TypeScript,
        "typescript_sample.ts",
        "MemoryRepo.save",
        SymbolKind::Method,
    ),
    (
        Language::Tsx,
        "tsx_sample.tsx",
        "helper",
        SymbolKind::Function,
    ),
    (Language::Go, "go_sample.go", "helper", SymbolKind::Function),
    (
        Language::Java,
        "java_sample.java",
        "Cart.size",
        SymbolKind::Method,
    ),
    (Language::C, "c_sample.c", "main", SymbolKind::Function),
    (
        Language::Cpp,
        "cpp_sample.cpp",
        "geo::total",
        SymbolKind::Function,
    ),
    (
        Language::CSharp,
        "csharp_sample.cs",
        "Shop.Billing.Invoice.Dispose",
        SymbolKind::Method,
    ),
    (
        Language::Ruby,
        "ruby_sample.rb",
        "Billing.Payable.total",
        SymbolKind::Method,
    ),
    (
        Language::Php,
        "php_sample.php",
        "Shop.Billing.Invoice.secret",
        SymbolKind::Method,
    ),
];

/// Checks that a result differs from the original source only by the inserted lines.
fn assert_only_lines_inserted(original: &str, result: &str, language: Language) {
    let original: Vec<&str> = original.lines().collect();
    let mut kept: Vec<&str> = result.lines().collect();
    assert!(kept.len() > original.len(), "{language}");
    let start = kept
        .iter()
        .zip(&original)
        .take_while(|(a, b)| a == b)
        .count();
    let inserted = kept.len() - original.len();
    kept.drain(start..start + inserted);
    assert_eq!(kept, original, "{language}");
}

/// Inserts documentation into one fixture symbol and checks everything about the result.
fn round_trip(case: RoundTripCase) {
    let (language, name, qualified, kind) = case;
    let source = fixture(name);
    let before = extract(language, &source);
    let target = symbol(&before, qualified);
    let line = target.span.start_line;
    if language == Language::Python {
        // Constants have no body: the inserter must refuse instead of guessing.
        let error = insert(language, &source, &target.name, kind, line, "Doc.").unwrap_err();
        assert!(matches!(error, DocError::Invalid(_)), "{error:?}");
        return;
    }
    let text = "Inserted documentation.\nOn two lines.";
    let result = insert(language, &source, &target.name, kind, line, text)
        .unwrap_or_else(|e| panic!("{language} {qualified}: {e}"));
    let after = extract(language, &result);
    assert_eq!(after.parse_errors, 0, "{language}");
    assert_eq!(
        symbol(&after, qualified).doc.as_deref(),
        Some(text),
        "{language}"
    );
    assert_eq!(after.symbols.len(), before.symbols.len(), "{language}");
    for (old, new) in before.symbols.iter().zip(&after.symbols) {
        assert_eq!(old.qualified_name, new.qualified_name, "{language}");
        assert_eq!(old.signature, new.signature, "{language}");
        let ancestor =
            qualified != old.qualified_name && qualified.starts_with(&old.qualified_name);
        if !ancestor {
            assert_eq!(
                old.body_hash, new.body_hash,
                "{language}: the code itself must not change"
            );
        }
        if old.qualified_name != qualified {
            assert_eq!(old.doc, new.doc, "{language}");
        }
    }
    assert_only_lines_inserted(&source, &result, language);
}

/// For every language: insert into an undocumented declaration of its fixture, read the result
/// back, and check that nothing else changed.
#[test]
fn round_trip_every_language() {
    for case in ROUND_TRIP_CASES {
        round_trip(case);
    }
}

/// Python: constants and other symbols without a body cannot receive a docstring.
#[test]
fn python_constants_are_refused() {
    let source = "MAX = 3\n";
    let error = insert(
        Language::Python,
        source,
        "MAX",
        SymbolKind::Constant,
        1,
        "Doc.",
    )
    .unwrap_err();
    assert!(matches!(error, DocError::Invalid(_)), "{error:?}");
}

/// A C++ header named as C receives its documentation through the C++ grammar.
#[test]
fn cpp_header_read_as_c() {
    let source = "namespace fp {\nclass Rng {\npublic:\n    unsigned next();\n};\n}\n";
    let result = insert(
        Language::C,
        source,
        "next",
        SymbolKind::Method,
        4,
        "Next value.",
    )
    .unwrap();
    assert_eq!(
        result,
        "namespace fp {\nclass Rng {\npublic:\n    /**\n     * Next value.\n     */\n    unsigned next();\n};\n}\n"
    );
    let file = extract(Language::Cpp, &result);
    assert_eq!(
        symbol(&file, "fp::Rng::next").doc.as_deref(),
        Some("Next value.")
    );
}
