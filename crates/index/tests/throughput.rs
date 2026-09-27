// SPDX-License-Identifier: Apache-2.0
//! Prints the extraction throughput per language on a synthetic 5 000-line file.
//!
//! Nothing is asserted about the speed (it depends on the machine and on the build profile);
//! run `cargo test --release -p pn-ultramemory-index --test throughput -- --nocapture` to see
//! the numbers.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests unwrap and panic to fail loudly"
)]

use std::time::Instant;

use pn_ultramemory_core::{Extractor, Language};
use pn_ultramemory_index::TreeSitterExtractor;

/// A block of realistic code for a language, repeated to build a synthetic file.
fn block(language: Language, n: usize) -> String {
    match language {
        Language::Rust => format!(
            "/// Computes item {n}.\npub fn item_{n}(a: i32, b: &str) -> Result<i32, Error> {{\n    let v = helper_{n}(a);\n    if v > 0 {{\n        log(b);\n    }}\n    Ok(v)\n}}\n\n"
        ),
        Language::Python => format!(
            "def item_{n}(a, b=1):\n    \"\"\"Computes item {n}.\"\"\"\n    v = helper_{n}(a)\n    if v > 0:\n        log(b)\n    return v\n\n"
        ),
        Language::JavaScript | Language::TypeScript | Language::Tsx => format!(
            "/** Computes item {n}. */\nexport function item_{n}(a, b) {{\n  const v = helper_{n}(a);\n  if (v > 0) {{\n    log(b);\n  }}\n  return v;\n}}\n\n"
        ),
        Language::Go => format!(
            "// Item{n} computes item {n}.\nfunc Item{n}(a int, b string) (int, error) {{\n\tv := helper{n}(a)\n\tif v > 0 {{\n\t\tlog(b)\n\t}}\n\treturn v, nil\n}}\n\n"
        ),
        Language::Java => format!(
            "    /** Computes item {n}. */\n    public int item{n}(int a, String b) {{\n        int v = helper{n}(a);\n        if (v > 0) {{\n            log(b);\n        }}\n        return v;\n    }}\n\n"
        ),
        Language::C | Language::Cpp => format!(
            "/** Computes item {n}. */\nint item_{n}(int a, const char *b) {{\n    int v = helper_{n}(a);\n    if (v > 0) {{\n        log(b);\n    }}\n    return v;\n}}\n\n"
        ),
        Language::CSharp => format!(
            "    /// <summary>Computes item {n}.</summary>\n    public int Item{n}(int a, string b)\n    {{\n        var v = Helper{n}(a);\n        if (v > 0)\n        {{\n            Log(b);\n        }}\n        return v;\n    }}\n\n"
        ),
        Language::Ruby => format!(
            "  # Computes item {n}.\n  def item_{n}(a, b = 1)\n    v = helper_{n}(a)\n    log(b) if v > 0\n    v\n  end\n\n"
        ),
        Language::Php => format!(
            "    /** Computes item {n}. */\n    public function item{n}(int $a, string $b): int\n    {{\n        $v = $this->helper{n}($a);\n        if ($v > 0) {{\n            log($b);\n        }}\n        return $v;\n    }}\n\n"
        ),
        Language::Other(_) => format!(
            "// Computes item {n}.\nfun item{n}(a: Int, b: String): Int {{\n    val v = helper{n}(a)\n    if (v > 0) {{\n        log(b)\n    }}\n    return v\n}}\n\n"
        ),
    }
}

/// Wraps repeated blocks in the container the language needs, until the file has 5 000 lines.
fn synthetic_file(language: Language) -> String {
    let (head, tail) = match language {
        Language::Java => ("public class Big {\n", "}\n"),
        Language::CSharp => ("namespace Big {\n  public class Item {\n", "  }\n}\n"),
        Language::Ruby => ("class Big\n", "end\n"),
        Language::Php => ("<?php\nclass Big {\n", "}\n"),
        _ => ("", ""),
    };
    let mut source = String::from(head);
    let mut n = 0;
    while source.lines().count() < 5_000 {
        source.push_str(&block(language, n));
        n += 1;
    }
    source.push_str(tail);
    source
}

/// Extracts a synthetic file of each language and prints the throughput in MB/s.
#[test]
fn print_throughput_per_language() {
    let extractor = TreeSitterExtractor::new();
    let mut languages = Language::GRAMMAR_BACKED.to_vec();
    languages.push(Language::Other("kotlin"));
    println!("\nlanguage        lines   bytes  symbols  refs  ms/file  MB/s");
    for language in languages {
        let source = synthetic_file(language);
        let rounds = 5;
        let mut best = f64::MAX;
        let mut last = None;
        for _ in 0..rounds {
            let started = Instant::now();
            let file = extractor.extract(language, &source).unwrap();
            best = best.min(started.elapsed().as_secs_f64());
            last = Some(file);
        }
        let file = last.unwrap();
        assert_eq!(
            file.parse_errors, 0,
            "{language}: the synthetic file must be valid"
        );
        assert!(
            file.symbols.len() > 300,
            "{language}: {}",
            file.symbols.len()
        );
        let megabytes = f64::from(u32::try_from(source.len()).unwrap()) / 1_000_000.0;
        println!(
            "{:<12} {:>8} {:>7} {:>8} {:>5} {:>8.2} {:>6.1}",
            language.name(),
            file.line_count,
            source.len(),
            file.symbols.len(),
            file.references.len(),
            best * 1000.0,
            megabytes / best
        );
    }
}
