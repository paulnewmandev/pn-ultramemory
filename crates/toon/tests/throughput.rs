// SPDX-License-Identifier: Apache-2.0
//! Throughput measurement of the TOON encoder and decoder (printed, not asserted).
//!
//! # Role in the architecture
//! Performance claims need a benchmark anyone can rerun (see CONTRIBUTING.md). This test builds a
//! realistic payload (a table of symbol records with a nested group, a keyed table, a list-form
//! section and prose fields), encodes and decodes it with each delimiter, and prints megabytes per
//! second next to `serde_json` on the same data. Nothing is asserted about speed, because speed
//! depends on the machine and on the build profile; the test does assert that the round trip is
//! lossless so it cannot measure broken code.
//!
//! Run it with optimizations to get meaningful numbers:
//! `cargo test --release -p pn-ultramemory-toon --test throughput -- --nocapture`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code"
)]
#![allow(
    clippy::cast_precision_loss,
    reason = "megabytes per second is an approximate figure"
)]

#[allow(
    dead_code,
    reason = "each integration test uses a different subset of the helpers"
)]
mod common;

use common::{Rng, check_model_eq, normalize};
use pn_ultramemory_toon::{DecodeOptions, Delimiter, EncodeOptions, decode, encode};
use serde_json::{Value, json};
use std::hint::black_box;
use std::time::{Duration, Instant};

/// Words used to build prose-like fields.
const WORDS: &[&str] = &[
    "parse",
    "index",
    "symbol",
    "graph",
    "memory",
    "budget",
    "token",
    "capsule",
    "resolve",
    "anchor",
    "detail",
    "signature",
    "callers",
    "module",
    "export",
    "public",
    "helper",
    "cache",
    "walk",
    "merge",
];

/// Builds a deterministic payload of roughly `rows` symbol records plus side sections.
fn payload(rows: usize) -> Value {
    let mut rng = Rng::new(42);
    let symbols: Vec<Value> = (0..rows)
        .map(|i| {
            let words: Vec<&str> = (0..3).map(|_| *rng.pick(WORDS)).collect();
            json!({
                "id": i,
                "name": format!("{}_{}", words[0], words[1]),
                "kind": *rng.pick(&["function", "struct", "trait", "const", "module"]),
                "file": format!("crates/{}/src/{}.rs", words[2], words[0]),
                "span": {"start": rng.index(4000), "end": 4000 + rng.index(400)},
                "score": (rng.index(10_000) as f64) / 10_000.0,
                "exported": rng.chance(1, 2),
                "doc": if rng.chance(1, 3) { Value::Null } else { json!(words.join(" ")) },
            })
        })
        .collect();
    let keyed: serde_json::Map<String, Value> = (0..rows / 10)
        .map(|i| {
            (
                format!("group{i}"),
                json!({"count": rng.index(500), "label": rng.pick(WORDS)}),
            )
        })
        .collect();
    let notes: Vec<Value> = (0..rows / 20)
        .map(|i| {
            json!({
                "note": format!("decision {i}: {} the {}", rng.pick(WORDS), rng.pick(WORDS)),
                "anchors": [rng.index(rows), rng.index(rows)],
                "detail": {"why": "measured", "tags": [rng.pick(WORDS), rng.pick(WORDS)]},
            })
        })
        .collect();
    json!({"symbols": symbols, "groups": keyed, "notes": notes, "version": "1.0"})
}

/// Runs `work` several times and returns the fastest duration.
fn best_of(times: usize, mut work: impl FnMut()) -> Duration {
    (0..times)
        .map(|_| {
            let start = Instant::now();
            work();
            start.elapsed()
        })
        .min()
        .unwrap()
}

/// Megabytes per second for `bytes` processed in `time`.
fn mb_per_s(bytes: usize, time: Duration) -> f64 {
    bytes as f64 / 1_000_000.0 / time.as_secs_f64()
}

/// Prints encode and decode throughput for every delimiter, checking the round trip once.
#[test]
fn throughput_report() {
    let rows = if cfg!(debug_assertions) {
        4_000
    } else {
        40_000
    };
    let value = payload(rows);
    let json_text = serde_json::to_string(&value).unwrap();
    println!(
        "\nthroughput: {rows} symbol rows, {} bytes as compact JSON",
        json_text.len()
    );
    println!(
        "profile: {}",
        if cfg!(debug_assertions) {
            "debug (use --release for real numbers)"
        } else {
            "release"
        }
    );

    let json_encode = best_of(5, || {
        black_box(serde_json::to_string(&value).unwrap());
    });
    let json_decode = best_of(5, || {
        black_box(serde_json::from_str::<Value>(&json_text).unwrap());
    });
    println!(
        "{:<12} {:>10} bytes  encode {:>7.1} MB/s  decode {:>7.1} MB/s",
        "serde_json",
        json_text.len(),
        mb_per_s(json_text.len(), json_encode),
        mb_per_s(json_text.len(), json_decode),
    );

    for (name, delimiter) in [
        ("toon comma", Delimiter::Comma),
        ("toon tab", Delimiter::Tab),
        ("toon pipe", Delimiter::Pipe),
    ] {
        let options = EncodeOptions {
            indent: 2,
            delimiter,
        };
        let text = encode(&value, &options);
        let decoded = decode(&text, &DecodeOptions::default()).unwrap();
        check_model_eq(&normalize(&value), &decoded).unwrap();

        let encode_time = best_of(5, || {
            black_box(encode(&value, &options));
        });
        let decode_time = best_of(5, || {
            black_box(decode(&text, &DecodeOptions::default()).unwrap());
        });
        println!(
            "{name:<12} {:>10} bytes  encode {:>7.1} MB/s  decode {:>7.1} MB/s  ({:.0}% of the JSON size)",
            text.len(),
            mb_per_s(text.len(), encode_time),
            mb_per_s(text.len(), decode_time),
            100.0 * text.len() as f64 / json_text.len() as f64,
        );
    }
}
