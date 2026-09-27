// SPDX-License-Identifier: Apache-2.0
//! Property tests: `decode(encode(v)) == v` for generated values, plus formatting invariants.
//!
//! # Role in the architecture
//! The official fixtures pin individual cases; these tests explore the space between them with a
//! deterministic pseudo-random generator (no external crate). Values are at most four containers
//! deep and are built to hit every form: primitives with hostile text, keys with special
//! characters, inline arrays, tabular arrays with nested field groups, keyed objects, list form,
//! and near-misses that must fall back from tabular to list form.
//!
//! # Invariants
//! * Round trips hold under the equality of section 2 of the specification: numbers compare by
//!   value, and tabular arrays and keyed objects list their keys in the header's order (the
//!   [`normalize`] oracle predicts exactly which).
//! * Encoder output never has a trailing space, a trailing newline, a CR, a tab in indentation, or a
//!   comment line, and is byte-for-byte deterministic.
//! * Whatever the decoder accepts, even from corrupted input, re-encodes to text that decodes back.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code"
)]

#[allow(
    dead_code,
    reason = "each integration test uses a different subset of the helpers"
)]
mod common;

use common::generator::{Gen, NASTY_KEYS, NASTY_STRINGS};
use common::{check_model_eq, normalize};
use pn_ultramemory_toon::{DecodeOptions, Delimiter, EncodeOptions, decode, encode};
use serde_json::{Map, Value, json};

/// Every delimiter the encoder supports.
const DELIMITERS: [Delimiter; 3] = [Delimiter::Comma, Delimiter::Tab, Delimiter::Pipe];

/// Indentation sizes exercised by the round-trip tests.
const INDENTS: [usize; 2] = [2, 4];

/// Asserts the formatting invariants of section 12 on encoder output.
fn assert_clean(text: &str, indent: usize, context: &str) {
    assert!(!text.contains('\r'), "CR in output ({context}): {text:?}");
    assert!(
        !text.ends_with('\n') && !text.starts_with('\n'),
        "stray newline ({context}): {text:?}"
    );
    assert!(!text.ends_with(' '), "trailing space ({context}): {text:?}");
    assert!(!text.starts_with('\u{feff}'), "BOM in output ({context})");
    for line in text.split('\n') {
        assert!(
            !line.ends_with(' '),
            "trailing space on {line:?} ({context})"
        );
        let body = line.trim_start_matches(' ');
        let spaces = line.len() - body.len();
        assert_eq!(
            spaces % indent,
            0,
            "indentation of {line:?} is not a multiple of {indent} ({context})"
        );
        assert!(
            !body.starts_with('\t'),
            "tab in indentation of {line:?} ({context})"
        );
        assert!(
            !body.starts_with('#'),
            "comment-like line {line:?} ({context})"
        );
    }
}

/// Encodes `value`, checks the output, decodes it strictly and compares under the spec equality.
fn check_round_trip(value: &Value, delimiter: Delimiter, indent: usize, context: &str) {
    let options = EncodeOptions { indent, delimiter };
    let text = encode(value, &options);
    let context = format!("{context}, {delimiter:?}, indent {indent}, input {value}");
    assert_clean(&text, indent, &context);
    assert_eq!(
        text,
        encode(value, &options),
        "encoding is not deterministic ({context})"
    );
    let decoded = decode(
        &text,
        &DecodeOptions {
            indent,
            strict: true,
        },
    )
    .unwrap_or_else(|e| panic!("decode failed: {e} ({context})\ntext: {text:?}"));
    if let Err(difference) = check_model_eq(&normalize(value), &decoded) {
        panic!("round trip differs: {difference} ({context})\ntext: {text:?}\ndecoded: {decoded}");
    }
    let lenient = decode(
        &text,
        &DecodeOptions {
            indent,
            strict: false,
        },
    )
    .unwrap();
    assert_eq!(
        lenient, decoded,
        "lenient and strict decoding disagree ({context})"
    );
}

/// Random values round trip for every delimiter and indentation size.
#[test]
fn random_values_round_trip() {
    let mut generator = Gen::new(0x70_6E_75_6D);
    for case in 0..4_000 {
        let value = generator.value(4);
        for delimiter in DELIMITERS {
            for indent in INDENTS {
                check_round_trip(&value, delimiter, indent, &format!("random case {case}"));
            }
        }
    }
}

/// Deeper random values (five levels) round trip with the default options.
#[test]
fn deeper_random_values_round_trip() {
    let mut generator = Gen::new(0xDEE9);
    for case in 0..1_500 {
        let value = generator.value(5);
        check_round_trip(&value, Delimiter::Comma, 2, &format!("deep case {case}"));
    }
}

/// Every nasty string survives at every position where a string can appear.
#[test]
fn nasty_strings_round_trip_everywhere() {
    for &s in NASTY_STRINGS {
        let v = json!(s);
        let shapes = [
            v.clone(),
            json!({ "k": s }),
            json!({ "a": [s, "x"], "b": [s] }),
            json!([s, s]),
            json!({ "a": [{"x": s, "y": 1}, {"x": "b", "y": 2}] }),
            json!([{"x": s}, {"x": s}]),
            json!({ "m": {"p": {"x": s}, "q": {"x": "t"}} }),
            json!({ "p": {"x": s, "y": s}, "q": {"x": "t", "y": s} }),
            json!({ "a": [s, {"x": 1}], "b": [[s], 1] }),
            json!({ "a": [{"x": s}, {"y": 1}], "b": [{"y": 1, "x": s}, {"z": 2}] }),
            json!([{"g": {"x": s}, "h": 1}, {"g": {"x": s}, "h": 2}]),
            json!({ "in": {"deep": {"er": [s]}} }),
        ];
        let mut all: Vec<Value> = shapes.to_vec();
        // The same string as a key at every position where a key can appear.
        let mut single = Map::new();
        single.insert(s.to_string(), json!(1));
        let mut listed = Map::new();
        listed.insert(s.to_string(), json!([1, 2]));
        let mut tabular = Map::new();
        tabular.insert("t".to_string(), json!([{ s: 1 }, { s: 2 }]));
        let mut grouped = Map::new();
        grouped.insert("t".to_string(), json!([{"g": { s: 1 }}, {"g": { s: 2 }}]));
        let mut keyed = Map::new();
        keyed.insert("m".to_string(), json!({ s: {"x": 1}, "other": {"x": 2} }));
        let mut first_field = Map::new();
        first_field.insert("t".to_string(), json!([{ s: 1 }, {"z": 2}]));
        let mut nested = Map::new();
        nested.insert(s.to_string(), json!({ s: { s: [s] } }));
        for map in [single, listed, tabular, grouped, keyed, first_field, nested] {
            all.push(Value::Object(map));
        }
        all.push(json!({ s: {"x": 1}, "other": {"x": 2} }));
        for value in &all {
            for delimiter in DELIMITERS {
                for indent in INDENTS {
                    check_round_trip(value, delimiter, indent, &format!("nasty string {s:?}"));
                }
            }
        }
    }
}

/// Every nasty key survives at every position where a key can appear (covered by strings above),
/// and the pool of keys used by the generator also round trips one by one.
#[test]
fn nasty_keys_round_trip_one_by_one() {
    for &key in NASTY_KEYS {
        let mut object = Map::new();
        object.insert(key.to_string(), json!([{"a": 1}, {"a": 2}]));
        object.insert("plain".to_string(), json!({"k": key}));
        for delimiter in DELIMITERS {
            check_round_trip(
                &Value::Object(object.clone()),
                delimiter,
                2,
                &format!("key {key:?}"),
            );
        }
    }
}

/// Numbers of every magnitude round trip at the root, in fields, inline arrays and cells.
#[test]
fn numbers_round_trip() {
    let literals = [
        "0",
        "-0",
        "1",
        "-1",
        "1.5",
        "-1.5",
        "0.1",
        "1e-7",
        "1e21",
        "1e-6",
        "1e300",
        "5e-324",
        "1.7976931348623157e308",
        "9007199254740992",
        "9007199254740993",
        "-9007199254740993",
        "18446744073709551615",
        "12345678901234567890",
        "-9223372036854775808",
        "1e19",
        "1e18",
        "123456789012345680000",
        "0.000001",
        "0.0000001234",
        "1.0",
        "100.0",
        "-0.0",
        "2.5e-8",
        "4.9e-324",
        "1152921504606846976",
        "0.30000000000000004",
        "1e22",
        "1.2345e21",
    ];
    for literal in literals {
        let n: Value = serde_json::from_str(literal).unwrap();
        let shapes = [
            n.clone(),
            json!({ "v": n }),
            json!({ "v": [n, n] }),
            json!([{"v": n, "w": 1}, {"v": n, "w": 2}]),
            json!({"m": {"a": {"v": n}, "b": {"v": n}}}),
            json!([[n], {"k": n}]),
        ];
        for value in &shapes {
            for delimiter in DELIMITERS {
                check_round_trip(value, delimiter, 2, &format!("number {literal}"));
            }
        }
    }
    let mut generator = Gen::new(7);
    for i in 0..20_000 {
        let n = generator.number();
        check_round_trip(
            &json!({ "v": [n] }),
            Delimiter::Comma,
            2,
            &format!("random number {i}"),
        );
    }
}

/// Canonical number text for a few well-known values.
#[test]
fn canonical_number_text() {
    let cases = [
        ("1.0", "1"),
        ("-0.0", "0"),
        ("1e6", "1000000"),
        ("1e-6", "0.000001"),
        ("1e-7", "1e-7"),
        ("1e21", "1e+21"),
        ("1e20", "100000000000000000000"),
        ("0.1", "0.1"),
        ("1.5000", "1.5"),
        ("18446744073709551615", "18446744073709551615"),
        ("2.5e-8", "2.5e-8"),
    ];
    for (input, expected) in cases {
        let n: Value = serde_json::from_str(input).unwrap();
        assert_eq!(encode(&n, &EncodeOptions::default()), expected, "{input}");
    }
}

/// Values nested up to the decoder limit round trip; one level more is a clean error.
#[test]
fn deep_nesting_round_trips_up_to_the_limit() {
    let build = |levels: usize, array: bool| {
        let mut value = json!(1);
        for _ in 0..levels {
            value = if array {
                json!([value])
            } else {
                json!({ "k": value })
            };
        }
        value
    };
    for array in [false, true] {
        for levels in [1, 10, 100, 200] {
            check_round_trip(
                &build(levels, array),
                Delimiter::Comma,
                2,
                &format!("nesting {levels}"),
            );
        }
        let too_deep = encode(&build(300, array), &EncodeOptions::default());
        let err = decode(&too_deep, &DecodeOptions::default()).unwrap_err();
        assert!(err.message().contains("nesting"), "{err}");
        assert!(
            decode(
                &too_deep,
                &DecodeOptions {
                    strict: false,
                    ..DecodeOptions::default()
                }
            )
            .is_err()
        );
    }
}

/// Characters used to corrupt encoded documents.
const CORRUPTION: &[char] = &[
    '[', ']', '{', '}', ':', ',', '|', '\t', '\n', '"', '\\', '-', '#', ' ', '\r', '\u{feff}', '0',
    '9', 'e', '\u{0}', 'é',
];

/// Corrupted or truncated documents never panic, and whatever decodes re-encodes losslessly.
#[test]
fn corrupted_documents_never_panic() {
    let mut generator = Gen::new(0xC0DE);
    for _ in 0..600 {
        let value = generator.value(3);
        let delimiter = *generator.rng.pick(&DELIMITERS);
        let text = encode(
            &value,
            &EncodeOptions {
                indent: 2,
                delimiter,
            },
        );
        let chars: Vec<char> = text.chars().collect();
        for _ in 0..40 {
            let mut mutated = chars.clone();
            match generator.rng.index(4) {
                0 => mutated.truncate(generator.rng.index(chars.len() + 1)),
                1 if !mutated.is_empty() => {
                    mutated.remove(generator.rng.index(mutated.len()));
                }
                2 => {
                    let at = generator.rng.index(mutated.len() + 1);
                    mutated.insert(at, *generator.rng.pick(CORRUPTION));
                }
                _ if !mutated.is_empty() => {
                    let at = generator.rng.index(mutated.len());
                    mutated[at] = *generator.rng.pick(CORRUPTION);
                }
                _ => {}
            }
            let corrupted: String = mutated.into_iter().collect();
            for strict in [true, false] {
                let Ok(decoded) = decode(&corrupted, &DecodeOptions { indent: 2, strict }) else {
                    continue;
                };
                let again = encode(&decoded, &EncodeOptions::default());
                let back = decode(&again, &DecodeOptions::default()).unwrap_or_else(|e| {
                    panic!("re-decode failed: {e}\ncorrupted: {corrupted:?}\nre-encoded: {again:?}")
                });
                if let Err(difference) = check_model_eq(&normalize(&decoded), &back) {
                    panic!(
                        "{difference}\ncorrupted: {corrupted:?}\ndecoded: {decoded}\nre-encoded: {again:?}"
                    );
                }
            }
        }
    }
}

/// Decoding text that was encoded with one delimiter never depends on the decoder options.
#[test]
fn the_declared_delimiter_is_enough() {
    let value = json!({
        "rows": [{"a": "x|y", "b": "p,q"}, {"a": "m", "b": "n\to"}],
        "inline": ["u|v", "w,z", "t\ts"],
        "keyed": {"k1": {"a": "1|2"}, "k2": {"a": "3,4"}},
    });
    for delimiter in DELIMITERS {
        let text = encode(
            &value,
            &EncodeOptions {
                indent: 2,
                delimiter,
            },
        );
        let decoded = decode(&text, &DecodeOptions::default()).unwrap();
        check_model_eq(&normalize(&value), &decoded).unwrap();
    }
}

/// Counters of the forms found in encoded documents.
#[derive(Default, Debug)]
struct FormCounts {
    /// Tabular headers `key[N]{...}:`.
    tabular: usize,
    /// Tabular headers with a nested field group.
    grouped: usize,
    /// Keyed headers `key[N:]{...}:`.
    keyed: usize,
    /// Arrays in list form.
    list: usize,
    /// Inline primitive arrays.
    inline: usize,
    /// List items that are inner arrays (`- [N]:`).
    inner_arrays: usize,
    /// Empty objects as list items (bare hyphen).
    empty_items: usize,
    /// Tabular or keyed headers carried on a hyphen line.
    header_on_hyphen: usize,
    /// Empty array fields (`key: []`).
    empty_arrays: usize,
}

/// Classifies the headers found on the lines of `text` into `counts`.
fn count_forms(text: &str, counts: &mut FormCounts) {
    for line in text.split('\n') {
        let body = line.trim_start_matches(' ');
        let on_hyphen = body.starts_with("- ") && !body.starts_with("- [");
        if body == "-" {
            counts.empty_items += 1;
        }
        if body.starts_with("- [") {
            counts.inner_arrays += 1;
        }
        if body.ends_with(": []") {
            counts.empty_arrays += 1;
        }
        let Some(open) = body.find(['[']) else {
            continue;
        };
        let Some(close) = body[open..].find(']').map(|i| i + open) else {
            continue;
        };
        let inside = &body[open + 1..close];
        if inside.is_empty() || !inside.starts_with(|c: char| c.is_ascii_digit()) {
            continue;
        }
        let after = &body[close + 1..];
        if after.starts_with('{') && body.ends_with(':') {
            if inside.contains(':') {
                counts.keyed += 1;
            } else {
                counts.tabular += 1;
            }
            if after.matches('{').count() > 1 {
                counts.grouped += 1;
            }
            if on_hyphen {
                counts.header_on_hyphen += 1;
            }
        } else if after == ":" {
            counts.list += 1;
        } else if after.starts_with(": ") {
            counts.inline += 1;
        }
    }
}

/// The generator reaches every form of the format, so the round-trip properties are not vacuous.
#[test]
fn generator_covers_every_form() {
    let mut generator = Gen::new(0x70_6E_75_6D);
    let mut counts = FormCounts::default();
    for _ in 0..4_000 {
        let value = generator.value(4);
        count_forms(&encode(&value, &EncodeOptions::default()), &mut counts);
    }
    println!("forms seen by the generator: {counts:?}");
    assert!(counts.tabular > 500, "{counts:?}");
    assert!(counts.grouped > 50, "{counts:?}");
    assert!(counts.keyed > 100, "{counts:?}");
    assert!(counts.list > 500, "{counts:?}");
    assert!(counts.inline > 500, "{counts:?}");
    assert!(counts.inner_arrays > 100, "{counts:?}");
    assert!(counts.empty_items > 20, "{counts:?}");
    assert!(counts.header_on_hyphen > 20, "{counts:?}");
    assert!(counts.empty_arrays > 100, "{counts:?}");
}
