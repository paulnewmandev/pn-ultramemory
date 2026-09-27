// SPDX-License-Identifier: Apache-2.0
//! Unit tests of the recursive-descent parser in [`super`].
//!
//! # Role in the architecture
//! Cover root-form discovery, object nesting and key order, duplicate keys, every array form, the
//! strict count checks, blank-line spans, orphan lines, header fall-through, the nesting limit and
//! hostile input. The official fixtures and the integration tests complement these.
//!
//! # Invariants
//! * Every case runs in strict and (where relevant) non-strict mode through the helpers below.

use super::*;
use serde_json::json;

/// Decodes in strict mode with the default indentation.
fn strict(input: &str) -> Result<Value, DecodeError> {
    decode(input, &DecodeOptions::default())
}

/// Decodes in non-strict mode with the default indentation.
fn lenient(input: &str) -> Result<Value, DecodeError> {
    decode(
        input,
        &DecodeOptions {
            strict: false,
            ..DecodeOptions::default()
        },
    )
}

/// The root form is discovered from the first line and the line count.
#[test]
fn root_forms() {
    assert_eq!(strict("").unwrap(), json!({}));
    assert_eq!(strict("\n# only a comment\n\n").unwrap(), json!({}));
    assert_eq!(strict("hello").unwrap(), json!("hello"));
    assert_eq!(strict("42").unwrap(), json!(42));
    assert_eq!(strict("\"a: b\"").unwrap(), json!("a: b"));
    assert_eq!(strict("[]").unwrap(), json!([]));
    assert_eq!(strict("[0]:").unwrap(), json!([]));
    assert_eq!(strict("[2]: 1,2").unwrap(), json!([1, 2]));
    assert_eq!(
        strict("[2]{a}:\n  1\n  2").unwrap(),
        json!([{"a": 1}, {"a": 2}])
    );
    assert_eq!(strict("[1:]{a}:\n  k: 1").unwrap(), json!({"k": {"a": 1}}));
    assert_eq!(strict("a: 1").unwrap(), json!({"a": 1}));
    assert!(strict("hello\nworld").is_err());
    assert!(strict("[1]: a\nb: 1").is_err());
    assert!(strict("[]\n[]").is_err());
    assert_eq!(lenient("[1]: a\nb: 1").unwrap(), json!(["a"]));
}

/// Objects nest by indentation and keep key order.
#[test]
fn objects_and_order() {
    let value = strict("z: 1\na:\n  y: 2\n  b: 3\nm: {x}").unwrap();
    assert_eq!(value, json!({"z": 1, "a": {"y": 2, "b": 3}, "m": "{x}"}));
    let keys: Vec<_> = value.as_object().unwrap().keys().cloned().collect();
    assert_eq!(keys, ["z", "a", "m"]);
    let inner: Vec<_> = value["a"].as_object().unwrap().keys().cloned().collect();
    assert_eq!(inner, ["y", "b"]);
    assert_eq!(strict("a:\nb:").unwrap(), json!({"a": {}, "b": {}}));
}

/// Strict mode reports duplicate keys, lenient mode keeps the last value.
#[test]
fn duplicate_keys() {
    assert!(strict("a: 1\na: 2").is_err());
    assert!(strict("o:\n  a: 1\n  a: 2").is_err());
    assert_eq!(
        lenient("a: 1\nb: 0\na: 2").unwrap(),
        json!({"a": 2, "b": 0})
    );
}

/// Inline, list, tabular and keyed arrays all decode, with nested groups.
#[test]
fn array_forms() {
    assert_eq!(
        strict("a[3]: 1,,\"x,y\"").unwrap(),
        json!({"a": [1, "", "x,y"]})
    );
    assert_eq!(strict("a[1]:\n  - x\n").unwrap(), json!({"a": ["x"]}));
    assert_eq!(
        strict("a[2]:\n  - [2]: 1,2\n  - []").unwrap(),
        json!({"a": [[1, 2], []]})
    );
    assert_eq!(
        strict("t[2]{id,c{n,k}}:\n  1,Ada,DK\n  2,Bob,UK").unwrap(),
        json!({"t": [{"id": 1, "c": {"n": "Ada", "k": "DK"}}, {"id": 2, "c": {"n": "Bob", "k": "UK"}}]})
    );
    assert_eq!(
        strict("m[2:|]{x|y}:\n  a: 1|2\n  \"b c\": 3|4").unwrap(),
        json!({"m": {"a": {"x": 1, "y": 2}, "b c": {"x": 3, "y": 4}}})
    );
    assert_eq!(strict("m[0:]{x}:").unwrap(), json!({"m": {}}));
}

/// A tabular first field of a list item puts its rows two levels below the hyphen.
#[test]
fn list_item_with_tabular_first_field() {
    let text = "items[2]:\n  - users[2]{id}:\n      1\n      2\n    status: ok\n  - status: no";
    let expected =
        json!({"items": [{"users": [{"id": 1}, {"id": 2}], "status": "ok"}, {"status": "no"}]});
    assert_eq!(strict(text).unwrap(), expected);
}

/// Declared lengths are checked only in strict mode and never truncate.
#[test]
fn count_checks() {
    assert!(strict("a[2]: 1").is_err());
    assert!(strict("a[1]: 1,2").is_err());
    assert!(strict("a[2]:\n  - 1").is_err());
    assert!(strict("a[1]{x}:\n  1\n  2").is_err());
    assert!(strict("a[1]{x,y}:\n  1").is_err());
    assert!(strict("m[1:]{x}:\n  a: 1\n  b: 2").is_err());
    assert_eq!(lenient("a[1]: 1,2").unwrap(), json!({"a": [1, 2]}));
    assert_eq!(
        lenient("a[1]{x,y}:\n  1").unwrap(),
        json!({"a": [{"x": 1}]})
    );
    assert_eq!(
        lenient("a[1]{g{x,y},z}:\n  1,2,3,4").unwrap(),
        json!({"a": [{"g": {"x": 1, "y": 2}, "z": 3}]})
    );
    assert_eq!(
        lenient("a[1]{z,g{x,y}}:\n  1").unwrap(),
        json!({"a": [{"z": 1}]})
    );
}

/// A blank line inside a header's span is an error only in strict mode.
#[test]
fn blank_lines_in_spans() {
    assert!(strict("a[2]:\n  - x\n\n  - y").is_err());
    assert!(strict("a[2]{x}:\n  1\n\n  2").is_err());
    assert!(strict("o[1]:\n  - k:\n\n      v: 1").is_err());
    assert_eq!(
        strict("a[2]:\n\n  - x\n  - y\n\nb: 1").unwrap(),
        json!({"a": ["x", "y"], "b": 1})
    );
    assert_eq!(
        lenient("a[2]:\n  - x\n\n  - y").unwrap(),
        json!({"a": ["x", "y"]})
    );
}

/// Lines that belong to no scope are errors in strict mode.
#[test]
fn orphan_lines() {
    assert!(strict("a: 1\n  b: 2").is_err());
    assert!(strict("a:\n    b: 1").is_err());
    assert!(strict("a[1]:\n  - x\n  y: 1").is_err());
    assert_eq!(
        lenient("a: 1\n  b: 2\nc: 3").unwrap(),
        json!({"a": 1, "c": 3})
    );
    assert!(lenient("a: 1\n  hello").is_err());
}

/// Non-strict decoding falls back to key-value lines for malformed headers.
#[test]
fn lenient_header_fallthrough() {
    assert_eq!(lenient("foo[bar]: 1").unwrap(), json!({"foo[bar]": 1}));
    assert!(strict("foo[bar]: 1").is_err());
    assert_eq!(
        lenient("a:\n  [2]: x,y").unwrap(),
        json!({"a": {"[2]": "x,y"}})
    );
}

/// Nesting up to the limit is accepted and one level more is an error, in every mode.
#[test]
fn nesting_limit() {
    let build = |levels: usize| {
        (0..levels)
            .map(|i| format!("{}k:", "  ".repeat(i)))
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert!(strict(&build(MAX_DEPTH - 1)).is_ok());
    assert!(strict(&build(MAX_DEPTH)).is_err());
    assert!(lenient(&build(MAX_DEPTH)).is_err());
    let lists = |levels: usize| {
        (0..levels)
            .map(|i| format!("{}- [1]:", "  ".repeat(i + 1)))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let err = strict(&format!("[1]:\n{}", lists(MAX_DEPTH + 5))).unwrap_err();
    assert!(err.message().contains("nesting"), "{err}");
}

/// A tiny input that declares an enormous length neither allocates nor hangs.
#[test]
fn huge_declared_lengths_are_cheap() {
    assert!(strict("a[18446744073709551615]: 1").is_err());
    assert!(strict("a[99999999999999999999]:\n  - x").is_err());
    assert_eq!(
        lenient("a[99999999999999999999]: 1").unwrap(),
        json!({"a": [1]})
    );
}

/// Input that is not valid TOON never panics.
#[test]
fn hostile_inputs_do_not_panic() {
    let cases = [
        "\"",
        "\\",
        "-",
        "- ",
        "[",
        "]",
        "[[",
        "{",
        "}",
        ":",
        "::",
        "a:",
        "a[",
        "a[1",
        "a[1]",
        "a[1]{",
        "a[1]{x",
        "a[1]{x}",
        "a[1]{x}:",
        "[:]:",
        "[1:]:",
        "\u{feff}",
        "\r",
        "\r\r\n",
        "a: \"",
        "a: \"\\",
        "a: \"\\u",
        "a: \"\\u12",
        "\"a\"",
        "\"a\"[",
        "\"a\"[1]: x",
        "-\n-",
        "a[1]:\n  -",
        "a[1]:\n  - -",
        "a[1]:\n  - [",
        "a[1]:\n  - [1]",
        "a[1]:\n  - [1]:",
        "a[1]:\n  - \"",
        "a[1]{x}:\n  \"",
        "m[1:]{x}:\n  \"",
        "m[1:]{x}:\n  \"a\"",
        "\u{0}",
        "a\u{0}: \u{0}",
        "# c",
        "  # c\n  a: 1",
        "a: 1 # c",
        "\t",
        " \t ",
        "a:\n\t",
        "0",
        "-0",
        "1e999",
        "\u{1F680}: \u{1F680}",
    ];
    for case in cases {
        let _ = strict(case);
        let _ = lenient(case);
    }
}

/// The examples of Appendix A of the specification decode to the values the text describes.
#[test]
fn appendix_a_examples() {
    let links = "links[2]{id,url}:\n  1,\"http://a:b\"\n  2,\"https://example.com?q=a:b\"";
    assert_eq!(
        strict(links).unwrap(),
        json!({"links": [
            {"id": 1, "url": "http://a:b"},
            {"id": 2, "url": "https://example.com?q=a:b"},
        ]})
    );
    let edge = "name: \"\"\n\ntags: []\n\nversion: \"123\"\nenabled: \"true\"\n\nroot:\n  level1:\n    level2:\n      level3:\n        items[2]{id,val}:\n          1,a\n          2,b\n\nmessage: Hello \u{4e16}\u{754c} \u{1f44b}\ntags2[3]: \u{1f389},\u{1f38a},\u{1f388}\n\nbignum: 9007199254740992\ndecimal: 0.3333333333333333";
    let value = strict(edge).unwrap();
    assert_eq!(value["name"], json!(""));
    assert_eq!(value["tags"], json!([]));
    assert_eq!(value["version"], json!("123"));
    assert_eq!(value["enabled"], json!("true"));
    assert_eq!(
        value["root"]["level1"]["level2"]["level3"]["items"],
        json!([{"id": 1, "val": "a"}, {"id": 2, "val": "b"}])
    );
    assert_eq!(value["message"], json!("Hello \u{4e16}\u{754c} \u{1f44b}"));
    assert_eq!(value["bignum"], json!(9_007_199_254_740_992_u64));
    assert_eq!(value["decimal"], json!(0.333_333_333_333_333_3));
    let mixed = "items[3]:\n  - 1\n  - a: 1\n  - text";
    assert_eq!(
        strict(mixed).unwrap(),
        json!({"items": [1, {"a": 1}, "text"]})
    );
    let tagged = "\"x-items\"[2]: \n  - id: 1\n  - id: 2\n    label: archived";
    assert_eq!(
        strict(tagged).unwrap(),
        json!({"x-items": [{"id": 1}, {"id": 2, "label": "archived"}]})
    );
    for bad in [
        "user:\n  key value",
        "name: \"bad\\xescape\"",
        "items[1]:\n   - value",
        "items[3]{id,name}:\n  1,Ada\n  2,Bob",
        "tags[5]: a,b,c",
        "id 123\nname Ada",
    ] {
        assert!(strict(bad).is_err(), "{bad:?} must be rejected");
    }
}
