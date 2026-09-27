// SPDX-License-Identifier: Apache-2.0
//! Hostile-input tests for the decoder: empty, huge, malformed, and unusual Unicode input.
//!
//! # Role in the architecture
//! The decoder is fed text that may come from an untrusted source (section 15 of the TOON
//! specification), so these tests check that it never panics, never exhausts the stack, never
//! reserves memory in proportion to a declared length, reports errors with useful line numbers,
//! and handles odd but legal input (CRLF, byte-order marks, non-ASCII whitespace, prototype keys).
//!
//! # Invariants
//! * Every case returns normally; failures are `Err(DecodeError)`, never a panic.
//! * Large inputs finish in test time, which guards against accidentally quadratic behaviour.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code"
)]

use pn_ultramemory_toon::{DecodeOptions, decode, from_str, to_string};
use serde_json::{Value, json};
use std::fmt::Write as _;

/// Decodes strictly with the default options.
fn strict(input: &str) -> Result<Value, pn_ultramemory_toon::DecodeError> {
    from_str(input)
}

/// Decodes leniently with the default indentation.
fn lenient(input: &str) -> Result<Value, pn_ultramemory_toon::DecodeError> {
    decode(
        input,
        &DecodeOptions {
            strict: false,
            ..DecodeOptions::default()
        },
    )
}

/// Empty and blank documents are the empty object; only whitespace tricks differ by mode.
#[test]
fn empty_and_blank_documents() {
    for input in [
        "",
        "\n",
        "\r\n",
        "   ",
        "\n\n\n",
        "# c",
        "# c\n\n# d\n",
        "\u{feff}",
        "\u{feff}\n",
    ] {
        assert_eq!(strict(input).unwrap(), json!({}), "{input:?}");
        assert_eq!(lenient(input).unwrap(), json!({}), "{input:?}");
    }
    // A tab is not indentation in strict mode, even on an otherwise blank line.
    assert!(strict("\t").is_err());
    assert_eq!(lenient("\t").unwrap(), json!({}));
    // Only a single leading byte-order mark is stripped.
    assert_eq!(strict("\u{feff}\u{feff}").unwrap(), json!("\u{feff}"));
}

/// A table with many rows decodes and keeps every row.
#[test]
fn large_tables_decode() {
    let rows = 100_000;
    let mut text = format!("rows[{rows}]{{id,name,ok}}:\n");
    for i in 0..rows {
        writeln!(text, "  {i},name{i},true").unwrap();
    }
    let value = strict(&text).unwrap();
    let array = value["rows"].as_array().unwrap();
    assert_eq!(array.len(), rows);
    assert_eq!(
        array[rows - 1],
        json!({"id": rows - 1, "name": format!("name{}", rows - 1), "ok": true})
    );
}

/// One enormous inline array, one enormous cell and one enormous key are all linear.
#[test]
fn enormous_single_lines() {
    let n = 1_000_000;
    let inline = format!("a[{n}]: {}", vec!["1"; n].join(","));
    assert_eq!(strict(&inline).unwrap()["a"].as_array().unwrap().len(), n);

    let cell = "x".repeat(5_000_000);
    let quoted = format!("a: \"{cell}\"");
    assert_eq!(
        strict(&quoted).unwrap()["a"].as_str().unwrap().len(),
        cell.len()
    );

    let key = "k".repeat(1_000_000);
    let doc = format!("\"{key}\": 1");
    assert_eq!(
        strict(&doc)
            .unwrap()
            .as_object()
            .unwrap()
            .keys()
            .next()
            .unwrap()
            .len(),
        key.len()
    );

    let fields: Vec<String> = (0..50_000).map(|i| format!("f{i}")).collect();
    let cells: Vec<String> = (0..50_000).map(|i| i.to_string()).collect();
    let table = format!("t[1]{{{}}}:\n  {}", fields.join(","), cells.join(","));
    let decoded = strict(&table).unwrap();
    assert_eq!(decoded["t"][0].as_object().unwrap().len(), 50_000);
}

/// Absurd indentation and pathological structure characters produce errors, not panics.
#[test]
fn pathological_structure_is_safe() {
    let spaces = " ".repeat(1_000_000);
    assert!(strict(&format!("{spaces}a: 1")).is_err());
    assert_eq!(
        decode(
            &format!("{spaces}a: 1"),
            &DecodeOptions {
                indent: 1,
                strict: false
            }
        )
        .unwrap(),
        json!({})
    );
    let big = 1_000_000;
    let cases = [
        "[".repeat(big),
        "]".repeat(big),
        "{".repeat(big),
        ":".repeat(big),
        "\"".repeat(big),
        "-".repeat(big),
        "- ".repeat(big),
        "#".repeat(big),
        format!("a{}", "[1]".repeat(big / 3)),
        format!("a[1]{{{}x{}}}:", "g{".repeat(big / 2), "}".repeat(big / 2)),
        format!("a: {}", "\\".repeat(big)),
        format!("a: \"{}", "\\\"".repeat(big / 2)),
        format!("[{big}]: {}", ",".repeat(big)),
        "a:\n".repeat(big / 2),
        "- a\n".repeat(big / 2),
        format!("a[{}]:\n{}", 3, "  - x\n".repeat(big / 6)),
    ];
    for case in &cases {
        let _ = strict(case);
        let _ = lenient(case);
    }
}

/// A declared length is only a claim: it neither allocates nor truncates.
#[test]
fn huge_declared_lengths() {
    for text in [
        "a[18446744073709551615]:",
        "a[99999999999999999999999999999999]: 1",
        "a[4294967295]{x}:\n  1",
        "m[18446744073709551616:]{x}:\n  a: 1",
        "[999999999999]:\n  - 1",
        "[999999999999]: 1,2,3",
    ] {
        assert!(strict(text).is_err(), "{text}");
        assert!(lenient(text).is_ok(), "{text}");
    }
    assert_eq!(
        lenient("a[999999999999]: 1,2,3").unwrap(),
        json!({"a": [1, 2, 3]})
    );
}

/// Non-ASCII whitespace is content, not indentation or padding (section 12).
#[test]
fn unicode_whitespace_is_content() {
    assert_eq!(strict("a:\u{a0}").unwrap(), json!({"a": "\u{a0}"}));
    assert_eq!(
        strict("a: \u{a0}x\u{a0}").unwrap(),
        json!({"a": "\u{a0}x\u{a0}"})
    );
    assert_eq!(strict("\u{a0}a: 1").unwrap(), json!({"\u{a0}a": 1}));
    assert_eq!(
        strict("a[2]: \u{2003}x,y\u{2003}").unwrap(),
        json!({"a": ["\u{2003}x", "y\u{2003}"]})
    );
    // NBSP never counts as indentation: this is a second top-level key.
    assert_eq!(
        strict("a:\n\u{a0}\u{a0}b: 1").unwrap(),
        json!({"a": {}, "\u{a0}\u{a0}b": 1})
    );
    assert_eq!(
        strict("\"\u{200b}\": \u{200b}").unwrap(),
        json!({"\u{200b}": "\u{200b}"})
    );
}

/// Emoji, combining marks, right-to-left text and NUL survive in keys and values.
#[test]
fn exotic_text_round_trips() {
    let value = json!({
        "🚀": "e\u{301}",
        "עברית": ["مرحبا", "x\u{0}y"],
        "rows": [{"k": "🚀 launch", "v": "\u{1F468}\u{200D}\u{1F469}"}, {"k": "é", "v": "ß"}],
    });
    assert_eq!(strict(&to_string(&value)).unwrap(), value);
}

/// CRLF documents, mixed endings and a byte-order mark decode like their LF form.
#[test]
fn line_endings_and_bom() {
    let lf = "a: 1\nb:\n  c[2]: x,y\n  d[1]{p,q}:\n    1,2\nt[2]:\n  - u\n  - v: 1";
    let expected = strict(lf).unwrap();
    assert_eq!(strict(&lf.replace('\n', "\r\n")).unwrap(), expected);
    assert_eq!(strict(&format!("\u{feff}{lf}")).unwrap(), expected);
    assert_eq!(strict(&format!("{lf}\r\n")).unwrap(), expected);
    let mut mixed = String::new();
    for (i, line) in lf.split('\n').enumerate() {
        mixed.push_str(line);
        mixed.push_str(if i % 2 == 0 { "\r\n" } else { "\n" });
    }
    assert_eq!(strict(&mixed).unwrap(), expected);
    // A CR that is not at the end of a line is content.
    assert_eq!(strict("a: x\ry").unwrap(), json!({"a": "x\ry"}));
    assert_eq!(
        strict("a: x\r\r\nb: 1").unwrap(),
        json!({"a": "x\r", "b": 1})
    );
}

/// No key is special: prototype-related names are ordinary entries (section 15).
#[test]
fn prototype_keys_are_ordinary() {
    let text = "__proto__:\n  admin: true\nconstructor: 1\n\"prototype\": 2\nrows[1]{__proto__,constructor}:\n  a,b\nm[2:]{x}:\n  __proto__: 1\n  constructor: 2";
    let value = strict(text).unwrap();
    assert_eq!(value["__proto__"], json!({"admin": true}));
    assert_eq!(value["constructor"], json!(1));
    assert_eq!(value["prototype"], json!(2));
    assert_eq!(
        value["rows"][0],
        json!({"__proto__": "a", "constructor": "b"})
    );
    assert_eq!(value["m"]["__proto__"], json!({"x": 1}));
    assert_eq!(strict(&to_string(&value)).unwrap(), value);
}

/// Errors point at the line where the problem is, counting comments and blank lines.
#[test]
fn error_line_numbers() {
    let line_of = |text: &str| strict(text).unwrap_err().line();
    assert_eq!(line_of("# c\n\na: 1\nb\n"), 4);
    assert_eq!(line_of("a: 1\n\n# c\nb: \"x\\q\""), 4);
    assert_eq!(line_of("x: 1\nt[3]: a,b"), 2);
    assert_eq!(line_of("x: 1\n\nt[2]{a,b}:\n  1,2\n# c\n  3"), 6);
    assert_eq!(line_of("x: 1\n\nt[3]{a,b}:\n  1,2\n# c\n  3,4"), 3);
    assert_eq!(line_of("x: 1\nx: 2"), 2);
    assert_eq!(line_of("a:\n  b:\n     c: 1"), 3);
    assert_eq!(line_of("[2]: 1,2\n\njunk: 1"), 3);
    assert_eq!(line_of("t[2]:\n  - a\n\n  - b"), 3);
    assert_eq!(line_of("a: 1\r\nb: \"\r\n"), 2);
}

/// Every prefix and every single-line deletion of a document with all forms is handled safely.
#[test]
fn prefixes_and_deletions_are_safe() {
    let doc = "# comment\nid: 1\nname: \"a,b\"\ntags[3|]: x|\"y|z\"|\nuser:\n  role: admin\n  prefs: []\n  keys[2:]{a,b}:\n    k1: 1,2\n    k2: 3,4\nrows[2\t]{id\tg{x\ty}}:\n  1\t2\t3\n  4\t5\t6\nlist[4]:\n  - 1\n  - [2]: a,b\n  - -\n  - k: v\n    rows[1]{a}:\n      9\n    n:\n      deep: true\n\nend: \"\\u00e9\"";
    assert!(strict(doc).is_ok() || lenient(doc).is_ok());
    let boundaries: Vec<usize> = doc
        .char_indices()
        .map(|(i, _)| i)
        .chain([doc.len()])
        .collect();
    for &end in &boundaries {
        let _ = strict(&doc[..end]);
        let _ = lenient(&doc[..end]);
    }
    let lines: Vec<&str> = doc.split('\n').collect();
    for skip in 0..lines.len() {
        let text: Vec<&str> = lines
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != skip)
            .map(|(_, l)| *l)
            .collect();
        let text = text.join("\n");
        let _ = strict(&text);
        let _ = lenient(&text);
    }
}

/// The strict-only checks are exactly that: lenient mode reads the same text.
#[test]
fn strict_errors_are_lenient_successes() {
    let cases = [
        ("a[3]: x,y", json!({"a": ["x", "y"]})),
        ("a[1]{x}:\n  1\n  2", json!({"a": [{"x": 1}, {"x": 2}]})),
        ("a:\n   b: 1", json!({"a": {"b": 1}})),
        ("a: 1\na: 2", json!({"a": 2})),
        ("t[2]:\n  - x\n\n  - y", json!({"t": ["x", "y"]})),
        ("t[1]{a,b}:\n  1", json!({"t": [{"a": 1}]})),
        ("m[3:]{v}:\n  a: 1\n  bogus", json!({"m": {"a": {"v": 1}}})),
    ];
    for (text, expected) in cases {
        assert!(strict(text).is_err(), "{text:?} must fail in strict mode");
        assert_eq!(lenient(text).unwrap(), expected, "{text:?}");
    }
}

/// Text after a completed root array or keyed root object is rejected in strict mode only.
#[test]
fn trailing_content_after_root_forms() {
    for text in [
        "[]\nx",
        "[1]: a\nx: 1",
        "[1]{a}:\n  1\nx: 1",
        "[1:]{a}:\n  k: 1\nx: 1",
        "[1]:\n  - a\nx: 1",
    ] {
        let err = strict(text).unwrap_err();
        assert!(err.message().contains("after"), "{err}");
        assert!(lenient(text).is_ok(), "{text:?}");
    }
    // Comments and blank lines after a root form are fine.
    assert_eq!(strict("[1]: a\n\n# c\n").unwrap(), json!(["a"]));
}
