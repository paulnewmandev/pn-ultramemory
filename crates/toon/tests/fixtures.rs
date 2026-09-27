// SPDX-License-Identifier: Apache-2.0
//! Runs every official TOON conformance fixture against the encoder and the decoder.
//!
//! # Role in the architecture
//! The fixtures under `tests/fixtures/{encode,decode}` are the language-agnostic suite of the TOON
//! 4.1 specification (MIT, see the README in that directory). This runner loads every file, keys
//! each case by file and index (never by name), honours `options` and `shouldError`, and asserts
//! that all cases pass while printing per-file counts.
//!
//! # Invariants
//! * A case never passes vacuously: a directory with no files, or a file with no cases, fails.
//! * Decoded objects are compared with key order significant, since decoders must preserve it.
//! * Every successfully decoded fixture is also re-encoded and decoded again, which must give the
//!   value the specification predicts (tabular and keyed forms reorder keys).
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

use common::{check_model_eq, normalize};
use pn_ultramemory_toon::{DecodeOptions, Delimiter, EncodeOptions, decode, encode, to_string};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

/// Directory holding the vendored fixtures.
fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

/// A fixture file that was loaded and parsed.
struct FixtureFile {
    /// File name, for reporting.
    name: String,
    /// The `tests` array of the file.
    cases: Vec<Value>,
}

/// Loads every `*.json` file of a fixture sub-directory, sorted by name.
fn load(kind: &str) -> Vec<FixtureFile> {
    let dir = fixtures_dir().join(kind);
    let mut paths: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    paths.sort();
    paths
        .iter()
        .map(|path| {
            let text = fs::read_to_string(path).unwrap();
            let root: Value = serde_json::from_str(&text).unwrap();
            assert_eq!(root["category"], kind, "{}: wrong category", path.display());
            let cases = root["tests"].as_array().unwrap().clone();
            FixtureFile {
                name: path.file_name().unwrap().to_string_lossy().into_owned(),
                cases,
            }
        })
        .collect()
}

/// Reads the `indentSize` option of a case (default 2).
fn indent_of(case: &Value) -> usize {
    case["options"]["indentSize"]
        .as_u64()
        .map_or(2, |n| usize::try_from(n).unwrap())
}

/// Reads the `delimiter` option of a case (default comma).
fn delimiter_of(case: &Value) -> Delimiter {
    match case["options"]["delimiter"].as_str() {
        None | Some(",") => Delimiter::Comma,
        Some("\t") => Delimiter::Tab,
        Some("|") => Delimiter::Pipe,
        Some(other) => panic!("unknown delimiter option {other:?}"),
    }
}

/// Runs one encode case, returning a description of the failure if there is one.
fn run_encode_case(case: &Value) -> Result<(), String> {
    let options = EncodeOptions {
        indent: indent_of(case),
        delimiter: delimiter_of(case),
    };
    let expected = case["expected"]
        .as_str()
        .ok_or("expected is not a string")?;
    let actual = encode(&case["input"], &options);
    if actual == expected {
        Ok(())
    } else {
        Err(format!("expected {expected:?}, got {actual:?}"))
    }
}

/// Runs one decode case, returning a description of the failure if there is one.
fn run_decode_case(case: &Value) -> Result<(), String> {
    let strict = case["options"]["strict"].as_bool().unwrap_or(true);
    let options = DecodeOptions {
        indent: indent_of(case),
        strict,
    };
    let input = case["input"].as_str().ok_or("input is not a string")?;
    let result = decode(input, &options);
    if case["shouldError"].as_bool().unwrap_or(false) {
        return match result {
            Ok(value) => Err(format!("expected an error, got {value}")),
            Err(e) if e.line() >= 1 && !e.message().is_empty() => Ok(()),
            Err(e) => Err(format!("error without a line or message: {e:?}")),
        };
    }
    let value = result.map_err(|e| format!("unexpected error: {e}"))?;
    check_model_eq(&case["expected"], &value)?;
    // Re-encoding what was decoded and decoding it again must be lossless.
    let text = to_string(&value);
    let again = decode(&text, &DecodeOptions::default())
        .map_err(|e| format!("re-decoding {text:?} failed: {e}"))?;
    check_model_eq(&normalize(&value), &again).map_err(|e| format!("re-encoded {text:?}: {e}"))
}

/// Runs all files of one kind and returns the failures, printing per-file counts.
fn run_all(kind: &str, run: fn(&Value) -> Result<(), String>) -> (usize, Vec<String>) {
    let files = load(kind);
    assert!(
        files.len() >= 9,
        "{kind}: only {} fixture files found",
        files.len()
    );
    let mut failures = Vec::new();
    let mut total = 0;
    for file in &files {
        assert!(!file.cases.is_empty(), "{}/{}: no cases", kind, file.name);
        let mut passed = 0;
        for (index, case) in file.cases.iter().enumerate() {
            match run(case) {
                Ok(()) => passed += 1,
                Err(message) => failures.push(format!(
                    "{kind}/{}[{index}] ({}): {message}",
                    file.name,
                    case["name"].as_str().unwrap_or("?")
                )),
            }
        }
        println!(
            "{kind}/{:<24} {passed:>3}/{:<3} passed",
            file.name,
            file.cases.len()
        );
        total += file.cases.len();
    }
    (total, failures)
}

/// Every official encode fixture produces exactly the expected TOON text.
#[test]
fn all_encode_fixtures_pass() {
    let (total, failures) = run_all("encode", run_encode_case);
    println!("encode: {}/{total} cases passed", total - failures.len());
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Every official decode fixture decodes to the expected value, or fails when it must.
#[test]
fn all_decode_fixtures_pass() {
    let (total, failures) = run_all("decode", run_decode_case);
    println!("decode: {}/{total} cases passed", total - failures.len());
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// The runner itself detects a wrong expectation, so a green run is meaningful.
#[test]
fn the_runner_can_fail() {
    let wrong_encode = serde_json::json!({"input": {"a": 1}, "expected": "a: 2"});
    assert!(run_encode_case(&wrong_encode).is_err());
    let wrong_decode = serde_json::json!({"input": "a: 1", "expected": {"a": 2}});
    assert!(run_decode_case(&wrong_decode).is_err());
    let missing_error = serde_json::json!({"input": "a: 1", "expected": null, "shouldError": true});
    assert!(run_decode_case(&missing_error).is_err());
    let wrong_order = serde_json::json!({"input": "a: 1\nb: 2", "expected": {"b": 2, "a": 1}});
    assert!(run_decode_case(&wrong_order).is_err());
}
