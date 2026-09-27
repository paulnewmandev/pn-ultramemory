// SPDX-License-Identifier: Apache-2.0
//! Hostile input: none of it may panic, hang or corrupt the session, and every response must be a
//! well-formed single-line JSON-RPC message of bounded size.
//!
//! The suite has three parts: a corpus of more than 200 hand-built hostile messages run against
//! servers in three session states, the same corpus pushed through `serve` as one stream, and a
//! deterministic mutation fuzzer that damages valid messages byte by byte.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code may unwrap and panic, as clippy.toml allows; helpers outside #[test] fns are not detected as tests"
)]

mod common;

use common::{Recorder, call, harness, initialize, parse_response, request, stateless};
use pn_ultramemory_mcp::Server;
use serde_json::{Value, json};

/// A labelled hostile input.
type Case = (String, String);

/// Collects hostile inputs.
#[derive(Default)]
struct Corpus(Vec<Case>);

impl Corpus {
    /// Adds one input under a label.
    fn add(&mut self, label: impl Into<String>, input: impl Into<String>) {
        self.0.push((label.into(), input.into()));
    }
}

/// A `tools/call` line for `recall` with `arguments` given as raw JSON text.
fn recall_with(arguments: &str) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"recall","arguments":{arguments}}}}}"#
    )
}

/// The JSON value kinds used to probe every argument.
const KINDS: [(&str, &str); 9] = [
    ("null", "null"),
    ("true", "true"),
    ("zero", "0"),
    ("negative", "-1"),
    ("float", "1.5"),
    ("string", "\"s\""),
    ("empty-string", "\"\""),
    ("array", "[1]"),
    ("object", "{\"a\":1}"),
];

/// Structurally broken or unusual text.
fn add_garbage(corpus: &mut Corpus) {
    for (index, text) in [
        "",
        " ",
        "\t\t",
        "\r",
        "\u{feff}",
        "\u{feff}{}",
        "{",
        "}",
        "[",
        "]",
        ",",
        ":",
        "\"",
        "'",
        "null",
        "true",
        "false",
        "0",
        "-",
        "-0",
        "1e",
        "1e999",
        "-1e999",
        "0x10",
        "NaN",
        "Infinity",
        "-Infinity",
        "{\"a\"}",
        "{\"a\":}",
        "{,}",
        "[,]",
        "[1,]",
        "{\"a\":1,}",
        "{\"jsonrpc\":\"2.0\"",
        "\\",
        "\\u0000",
        "\"\\ud800\"",
        "\"\\udc00\"",
        "{\"a\":\"\\ud800\"}",
        "\u{0}",
        "\u{7f}",
        "\u{200b}",
        "\u{202e}",
        "//comment",
        "/* c */ {}",
        "{}{}",
        "{} {}",
        "[] []",
        "{}\u{0}",
        "\u{0}{}",
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"} trailing",
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}}",
        "\"just a string\"",
        "12345",
    ]
    .into_iter()
    .enumerate()
    {
        corpus.add(format!("garbage #{index}"), text);
    }
}

/// Deeply nested arrays and objects, alone and inside every part of a request.
fn add_deep_nesting(corpus: &mut Corpus) {
    for depth in [
        1_usize, 2, 10, 64, 100, 127, 128, 129, 130, 200, 500, 1_000, 10_000, 100_000, 1_000_000,
    ] {
        let arrays = format!("{}{}", "[".repeat(depth), "]".repeat(depth));
        let objects = format!("{}1{}", "{\"a\":".repeat(depth), "}".repeat(depth));
        corpus.add(format!("arrays depth {depth}"), arrays.clone());
        corpus.add(format!("objects depth {depth}"), objects.clone());
        corpus.add(format!("unclosed depth {depth}"), "[".repeat(depth));
        corpus.add(
            format!("id depth {depth}"),
            format!(r#"{{"jsonrpc":"2.0","id":{arrays},"method":"ping"}}"#),
        );
        corpus.add(
            format!("params depth {depth}"),
            format!(r#"{{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{objects}}}"#),
        );
        corpus.add(
            format!("argument depth {depth}"),
            recall_with(&format!(r#"{{"q":{arrays}}}"#)),
        );
        corpus.add(
            format!("meta depth {depth}"),
            format!(
                r#"{{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{{"_meta":{{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{objects}}}}}}}"#
            ),
        );
    }
}

/// Numbers that overflow, underflow or are not integers, as ids and as arguments.
fn add_huge_numbers(corpus: &mut Corpus) {
    let digits = "9".repeat(10_000);
    for number in [
        "1e999",
        "-1e999",
        "1E+400",
        "1e-999",
        "123456789012345678901234567890",
        "-123456789012345678901234567890",
        "18446744073709551616",
        "9223372036854775808",
        "-9223372036854775809",
        "0.0000000000000000000000000001",
        "4294967296",
        "4294967295",
        "-0",
        "-0.0",
        "1.7976931348623157e308",
        "5e-324",
        digits.as_str(),
    ] {
        let short: String = number.chars().take(24).collect();
        corpus.add(
            format!("id {short}"),
            format!(r#"{{"jsonrpc":"2.0","id":{number},"method":"ping"}}"#),
        );
        corpus.add(
            format!("budget {short}"),
            recall_with(&format!(r#"{{"q":"x","budget":{number}}}"#)),
        );
        corpus.add(
            format!("depth {short}"),
            format!(
                r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"impact","arguments":{{"symbol":"s","depth":{number}}}}}}}"#
            ),
        );
    }
}

/// Every envelope field with every wrong type.
fn add_envelope_types(corpus: &mut Corpus) {
    for (label, value) in KINDS
        .iter()
        .chain([("nested", "[[]]"), ("bad-version", "\"1.0\"")].iter())
    {
        corpus.add(
            format!("jsonrpc {label}"),
            format!(r#"{{"jsonrpc":{value},"id":1,"method":"ping"}}"#),
        );
        corpus.add(
            format!("id {label}"),
            format!(r#"{{"jsonrpc":"2.0","id":{value},"method":"ping"}}"#),
        );
        corpus.add(
            format!("method {label}"),
            format!(r#"{{"jsonrpc":"2.0","id":1,"method":{value}}}"#),
        );
        corpus.add(
            format!("params {label}"),
            format!(r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{value}}}"#),
        );
        corpus.add(
            format!("name {label}"),
            format!(
                r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":{value}}}}}"#
            ),
        );
        corpus.add(
            format!("init version {label}"),
            format!(r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":{value}}}}}"#),
        );
        corpus.add(
            format!("cursor {label}"),
            format!(
                r#"{{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{{"cursor":{value}}}}}"#
            ),
        );
        corpus.add(
            format!("meta {label}"),
            format!(
                r#"{{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{{"_meta":{value}}}}}"#
            ),
        );
    }
}

/// Every tool argument with every wrong type.
fn add_argument_types(corpus: &mut Corpus) {
    let tools: [(&str, &[&str]); 4] = [
        ("recall", &["q", "budget", "explain"]),
        ("impact", &["symbol", "depth"]),
        ("remember", &["kind", "text", "about"]),
        ("expand", &["id", "from", "to"]),
    ];
    for (tool, arguments) in tools {
        for argument in arguments {
            for (label, value) in KINDS {
                corpus.add(
                    format!("{tool}.{argument} = {label}"),
                    format!(
                        r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"{tool}","arguments":{{"{argument}":{value}}}}}}}"#
                    ),
                );
            }
        }
        for (label, value) in KINDS {
            corpus.add(
                format!("{tool} arguments = {label}"),
                format!(
                    r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"{tool}","arguments":{value}}}}}"#
                ),
            );
        }
    }
    for (label, arguments) in [
        ("from above to", r#"{"id":"n","from":9,"to":3}"#),
        ("from and to zero", r#"{"id":"n","from":0,"to":0}"#),
        (
            "window at the u32 edge",
            r#"{"id":"n","from":4294967295,"to":4294967295}"#,
        ),
        (
            "window past the u32 edge",
            r#"{"id":"n","from":4294967296,"to":4294967297}"#,
        ),
        ("window with floats", r#"{"id":"n","from":1e0,"to":1.0e1}"#),
        (
            "window with huge float",
            r#"{"id":"n","from":1e308,"to":1e-308}"#,
        ),
        (
            "window with negative zero",
            r#"{"id":"n","from":-0.0,"to":-0.0}"#,
        ),
    ] {
        corpus.add(
            format!("expand window: {label}"),
            format!(
                r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"expand","arguments":{arguments}}}}}"#
            ),
        );
    }
    corpus.add("about with mixed items", r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"remember","arguments":{"kind":"fact","text":"t","about":["a",1,null,[],{}]}}}"#);
}

/// Line breaks, control characters and characters that were once invalid UTF-8.
fn add_encoding_tricks(corpus: &mut Corpus) {
    corpus.add(
        "raw newline inside a string",
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"pi\nng\"}",
    );
    corpus.add(
        "raw CR inside a string",
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"pi\rng\"}",
    );
    corpus.add(
        "pretty printed message",
        "{\n  \"jsonrpc\": \"2.0\",\n  \"id\": 1,\n  \"method\": \"ping\"\n}",
    );
    corpus.add(
        "crlf pretty printed",
        "{\r\n\"jsonrpc\":\"2.0\",\r\n\"id\":1,\r\n\"method\":\"ping\"\r\n}",
    );
    corpus.add(
        "escaped newline in id",
        r#"{"jsonrpc":"2.0","id":"a\nb\r\nc","method":"ping"}"#,
    );
    corpus.add(
        "escaped line separators",
        "{\"jsonrpc\":\"2.0\",\"id\":\"\\u2028\\u2029\",\"method\":\"ping\"}",
    );
    corpus.add(
        "raw line separators",
        "{\"jsonrpc\":\"2.0\",\"id\":\"\u{2028}\u{2029}\",\"method\":\"ping\"}",
    );
    corpus.add(
        "raw control char in string",
        "{\"jsonrpc\":\"2.0\",\"id\":\"\u{1}\",\"method\":\"ping\"}",
    );
    corpus.add(
        "raw NUL in string",
        "{\"jsonrpc\":\"2.0\",\"id\":\"\u{0}\",\"method\":\"ping\"}",
    );
    for (index, bytes) in [
        &b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"\xff\"}"[..],
        b"{\"jsonrpc\":\"2.0\",\"id\":\"\xc3\x28\",\"method\":\"ping\"}",
        b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"recall\",\"arguments\":{\"q\":\"\xed\xa0\x80\"}}}",
        b"\xf0\x28\x8c\xbc",
        b"\xff\xfe\xfd",
        b"{\"a\":\"\xe2\x82\"}",
    ]
    .into_iter()
    .enumerate()
    {
        corpus.add(format!("lossy utf-8 #{index}"), String::from_utf8_lossy(bytes).into_owned());
    }
    corpus.add(
        "replacement characters",
        "{\"jsonrpc\":\"2.0\",\"id\":\"\u{fffd}\u{fffd}\",\"method\":\"\u{fffd}\"}",
    );
}

/// Batches of every awkward shape.
fn add_batches(corpus: &mut Corpus) {
    let ping = r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;
    for (label, text) in [
        ("empty", "[]".to_owned()),
        ("nested empty", "[[]]".to_owned()),
        ("null item", "[null]".to_owned()),
        ("numbers", "[1,2,3]".to_owned()),
        ("empty objects", "[{},{}]".to_owned()),
        ("arrays", "[[],[]]".to_owned()),
        ("one ping", format!("[{ping}]")),
        ("300 pings", format!("[{}]", vec![ping; 300].join(","))),
        (
            "300 empty objects",
            format!("[{}]", vec!["{}"; 300].join(",")),
        ),
        (
            "100000 nulls",
            format!("[{}]", vec!["null"; 100_000].join(",")),
        ),
        (
            "nested 100",
            format!("{}{ping}{}", "[".repeat(100), "]".repeat(100)),
        ),
        (
            "batch with initialize",
            format!(
                r#"[{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"2025-03-26"}}}},{ping}]"#
            ),
        ),
        (
            "only notifications",
            r#"[{"jsonrpc":"2.0","method":"a"},{"jsonrpc":"2.0","method":"b"}]"#.to_owned(),
        ),
        (
            "mixed garbage",
            format!(r#"[{ping},"x",7,null,{{}},[],{{"id":1}}]"#),
        ),
    ] {
        corpus.add(format!("batch {label}"), text);
    }
}

/// Odd method names.
fn add_method_names(corpus: &mut Corpus) {
    let long = "m".repeat(100_000);
    for (index, method) in [
        long.as_str(),
        "",
        " ",
        "tools/call ",
        "tools/call\\n",
        "TOOLS/CALL",
        "tools//call",
        "../../etc/passwd",
        "rpc.discover",
        "rpc.",
        "__proto__",
        "constructor",
        "\\u0000",
        "\u{1F600}",
        "\u{202e}llac/sloot",
        "initialize ",
        "Initialize",
        "ping\\u0000",
        "notifications/initialized",
        "server/discover ",
        "tools/call/../list",
    ]
    .into_iter()
    .enumerate()
    {
        let quoted = serde_json::to_string(method).unwrap();
        corpus.add(
            format!("method #{index}"),
            format!(r#"{{"jsonrpc":"2.0","id":{index},"method":{quoted}}}"#),
        );
        corpus.add(
            format!("notification method #{index}"),
            format!(r#"{{"jsonrpc":"2.0","method":{quoted}}}"#),
        );
    }
}

/// Large and adversarial tool arguments.
fn add_tool_payloads(corpus: &mut Corpus) {
    let megabyte = "q".repeat(1 << 20);
    let many_keys: String = (0..10_000)
        .map(|i| format!("\"k{i}\":{i}"))
        .collect::<Vec<_>>()
        .join(",");
    let many_about: String = vec!["\"a\""; 100_000].join(",");
    corpus.add(
        "1 MiB query",
        recall_with(&format!(r#"{{"q":"{megabyte}"}}"#)),
    );
    corpus.add(
        "1 MiB tool name",
        format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"{megabyte}"}}}}"#
        ),
    );
    corpus.add(
        "1 MiB argument key",
        recall_with(&format!(r#"{{"{megabyte}":1}}"#)),
    );
    corpus.add(
        "10000 argument keys",
        recall_with(&format!("{{{many_keys}}}")),
    );
    corpus.add("100000 about items", format!(r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"remember","arguments":{{"kind":"fact","text":"t","about":[{many_about}]}}}}}}"#));
    corpus.add(
        "duplicate keys",
        recall_with(r#"{"q":"first","q":"second","budget":1,"budget":2}"#),
    );
    corpus.add(
        "proto keys",
        recall_with(r#"{"__proto__":{"q":"x"},"constructor":1,"q":"x"}"#),
    );
    corpus.add("NUL in query", recall_with(r#"{"q":"a\u0000b"}"#));
    corpus.add(
        "emoji and rtl",
        recall_with(r#"{"q":"\ud83e\udd80 \u202e\u200b\u2028 caf\u00e9"}"#),
    );
    corpus.add("lone surrogate", recall_with(r#"{"q":"\ud800"}"#));
    corpus.add(
        "template injection",
        recall_with(r#"{"q":"{{7*7}} ${jndi:ldap://x} <script>alert(1)</script>"}"#),
    );
    corpus.add(
        "sql injection",
        recall_with(r#"{"q":"'; DROP TABLE memories; --"}"#),
    );
    corpus.add(
        "shell injection",
        recall_with(r#"{"q":"$(rm -rf /) `id` ; | &"}"#),
    );
    corpus.add("path traversal id", r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"expand","arguments":{"id":"../../../../etc/passwd"}}}"#);
    corpus.add("trigger words", recall_with(r#"{"q":"FAIL"}"#));
    corpus.add("whitespace only", recall_with(r#"{"q":" \t\n\r"}"#));
    corpus.add(
        "tool name with NUL",
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"recall\u0000"}}"#,
    );
    corpus.add("extra params", r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"recall","arguments":{"q":"x"},"extra":[1,2,3],"cursor":5}}"#);
}

/// Hostile per-request metadata for the stateless era.
fn add_meta_attacks(corpus: &mut Corpus) {
    let many: String = (0..10_000)
        .map(|i| format!("\"k{i}\":{i}"))
        .collect::<Vec<_>>()
        .join(",");
    let long_version = "v".repeat(1 << 20);
    for (label, meta) in [
        ("many keys", format!(r#"{{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{{{many}}}}}"#)),
        ("huge version", format!(r#"{{"io.modelcontextprotocol/protocolVersion":"{long_version}","io.modelcontextprotocol/clientCapabilities":{{}}}}"#)),
        ("nul version", r#"{"io.modelcontextprotocol/protocolVersion":"2026-07-28\u0000","io.modelcontextprotocol/clientCapabilities":{}}"#.to_owned()),
        ("emoji version", r#"{"io.modelcontextprotocol/protocolVersion":"\ud83e\udd80","io.modelcontextprotocol/clientCapabilities":{}}"#.to_owned()),
        ("array meta", "[]".to_owned()),
        ("string meta", r#""io.modelcontextprotocol/protocolVersion""#.to_owned()),
        ("null caps", r#"{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":null}"#.to_owned()),
        ("only caps", r#"{"io.modelcontextprotocol/clientCapabilities":{}}"#.to_owned()),
        ("duplicate version", r#"{"io.modelcontextprotocol/protocolVersion":"1","io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{}}"#.to_owned()),
    ] {
        for method in ["tools/list", "server/discover", "tools/call", "ping", "initialize"] {
            corpus.add(
                format!("meta {label} on {method}"),
                format!(r#"{{"jsonrpc":"2.0","id":1,"method":"{method}","params":{{"name":"recall","_meta":{meta}}}}}"#),
            );
        }
    }
}

/// Builds the whole corpus.
fn corpus() -> Vec<Case> {
    let mut corpus = Corpus::default();
    add_garbage(&mut corpus);
    add_deep_nesting(&mut corpus);
    add_huge_numbers(&mut corpus);
    add_envelope_types(&mut corpus);
    add_argument_types(&mut corpus);
    add_encoding_tricks(&mut corpus);
    add_batches(&mut corpus);
    add_method_names(&mut corpus);
    add_tool_payloads(&mut corpus);
    add_meta_attacks(&mut corpus);
    corpus.0
}

/// Feeds one input to the server and checks the response is well formed and bounded.
///
/// `batch_allowed` widens the size bound, because a batch legitimately answers every element.
fn check(server: &Server<Recorder>, label: &str, input: &str, batch_allowed: bool) {
    let Some(text) = server.handle_line(input) else {
        return;
    };
    let value = parse_response(&text);
    let is_batch = value.is_array();
    let bound = 4096
        + 2 * input.len()
        + if batch_allowed && is_batch {
            256 * 512
        } else {
            0
        };
    assert!(
        text.len() <= bound,
        "{label}: {} byte response to {} byte input",
        text.len(),
        input.len()
    );
}

/// The corpus is large enough to count as a serious hostile suite.
#[test]
fn corpus_has_more_than_200_inputs() {
    let cases = corpus();
    println!("hostile corpus: {} inputs", cases.len());
    assert!(cases.len() >= 200, "only {} inputs", cases.len());
    let mut labels: Vec<&str> = cases.iter().map(|(label, _)| label.as_str()).collect();
    labels.sort_unstable();
    labels.dedup();
    assert_eq!(labels.len(), cases.len(), "labels must be unique");
}

/// No hostile input panics against a fresh server, and the server still works afterwards.
#[test]
fn hostile_inputs_against_a_fresh_server() {
    let (server, _) = harness();
    for (label, input) in corpus() {
        check(&server, &label, &input, false);
    }
    let ping = common::reply(&server, &request(json!(1), "ping", Value::Null));
    assert_eq!(ping["result"], json!({}));
    let after = common::reply(&server, &stateless(json!(2), "tools/list", json!({})));
    assert_eq!(
        after["result"]["tools"].as_array().unwrap().len(),
        pn_ultramemory_mcp::TOOL_NAMES.len()
    );
}

/// No hostile input panics once a session is open, for a batch-less and a batch-capable revision.
#[test]
fn hostile_inputs_inside_sessions() {
    for (version, batch_allowed) in [("2025-11-25", false), ("2025-03-26", true)] {
        let (server, _) = harness();
        common::reply(&server, &initialize(1, version));
        for (label, input) in corpus() {
            check(&server, &label, &input, batch_allowed);
        }
        let list = common::reply(&server, &request(json!(2), "tools/list", Value::Null));
        assert_eq!(
            list["result"]["tools"].as_array().unwrap().len(),
            pn_ultramemory_mcp::TOOL_NAMES.len(),
            "{version}"
        );
        let answer = common::reply(&server, &call(3, "recall", json!({ "q": "still alive" })));
        assert_eq!(answer["result"]["isError"], json!(false), "{version}");
    }
}

/// The whole corpus, glued into one byte stream, is survived by `serve`.
#[test]
fn hostile_stream_through_serve() {
    let (server, _) = harness();
    let mut input = Vec::new();
    for (_, text) in corpus() {
        input.extend_from_slice(text.as_bytes());
        input.push(b'\n');
    }
    input.extend_from_slice(b"\xff\xfe\xfd\n\xc3\x28\n");
    input.extend_from_slice(request(json!("last"), "ping", Value::Null).as_bytes());
    input.push(b'\n');
    let mut output = Vec::new();
    server.serve(input.as_slice(), &mut output).unwrap();
    let text = String::from_utf8(output).unwrap();
    let mut last = Value::Null;
    for line in text.lines() {
        last = parse_response(line);
    }
    assert_eq!(last["id"], json!("last"));
    assert_eq!(last["result"], json!({}));
}

/// A small deterministic pseudo-random generator (xorshift64*), so failures are reproducible.
struct Rng(u64);

impl Rng {
    /// Returns the next 64-bit value.
    fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Returns a value below `bound` (which must be positive).
    fn below(&mut self, bound: usize) -> usize {
        usize::try_from(self.next_u64() % u64::try_from(bound).unwrap()).unwrap()
    }
}

/// Damages `base` with a few random byte-level edits.
fn mutate(rng: &mut Rng, base: &[u8]) -> Vec<u8> {
    const STRUCTURAL: &[u8] = b"{}[]\",:\\ \n\r\t\0-0123456789eE.tfn";
    let mut bytes = base.to_vec();
    for _ in 0..=rng.below(4) {
        if bytes.is_empty() {
            bytes.push(b'{');
        }
        let at = rng.below(bytes.len());
        match rng.below(8) {
            0 => bytes[at] = u8::try_from(rng.below(256)).unwrap(),
            1 => {
                let end = (at + 1 + rng.below(12)).min(bytes.len());
                bytes.drain(at..end);
            }
            2 => {
                let end = (at + 1 + rng.below(12)).min(bytes.len());
                let copy = bytes[at..end].to_vec();
                bytes.splice(at..at, copy);
            }
            3 => bytes.insert(at, STRUCTURAL[rng.below(STRUCTURAL.len())]),
            4 => bytes.truncate(at),
            5 => {
                let other = rng.below(bytes.len());
                bytes.swap(at, other);
            }
            6 => bytes[at] = STRUCTURAL[rng.below(STRUCTURAL.len())],
            _ => bytes.extend_from_slice(b"\xf0\x9f\x92"),
        }
    }
    bytes
}

/// Valid messages of every kind, used as the fuzzer's starting points.
fn seeds() -> Vec<String> {
    vec![
        initialize(1, "2025-03-26"),
        initialize(2, "2025-11-25"),
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#.to_owned(),
        request(json!(3), "tools/list", Value::Null),
        request(json!("s"), "ping", Value::Null),
        call(4, "recall", json!({ "q": "parser", "budget": 900 })),
        call(5, "impact", json!({ "symbol": "a::b", "depth": 2 })),
        call(
            6,
            "remember",
            json!({ "kind": "lesson", "text": "t", "about": ["x", "y"] }),
        ),
        call(7, "expand", json!({ "id": "n1" })),
        stateless(json!(8), "server/discover", json!({})),
        stateless(json!(9), "tools/list", json!({})),
        stateless(
            json!(10),
            "tools/call",
            json!({ "name": "recall", "arguments": { "q": "x" } }),
        ),
        format!(
            "[{},{}]",
            request(json!(11), "ping", Value::Null),
            call(12, "expand", json!({ "id": "n" }))
        ),
    ]
}

/// Thirty thousand mutated messages, as lossy text through `handle_line`, never panic.
#[test]
fn mutation_fuzzing_through_handle_line() {
    let (server, _) = harness();
    let seeds = seeds();
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut answered = 0_usize;
    for round in 0..30_000 {
        let base = seeds[round % seeds.len()].as_bytes();
        let damaged = mutate(&mut rng, base);
        let text = String::from_utf8_lossy(&damaged);
        let label = format!("fuzz round {round}");
        check(&server, &label, &text, true);
        if server.handle_line(&text).is_some() {
            answered += 1;
        }
    }
    assert!(
        answered > 10_000,
        "the fuzzer should mostly produce answerable input: {answered}"
    );
    let ping = common::reply(&server, &request(json!(1), "ping", Value::Null));
    assert_eq!(ping["result"], json!({}));
}

/// Mutated raw bytes, including invalid UTF-8, through `serve` as one stream.
#[test]
fn mutation_fuzzing_through_serve() {
    let (server, _) = harness();
    let seeds = seeds();
    let mut rng = Rng(0xD1B5_4A32_D192_ED03);
    let mut input = Vec::new();
    for round in 0..5_000 {
        let base = seeds[round % seeds.len()].as_bytes();
        let damaged = mutate(&mut rng, base);
        input.extend(damaged.into_iter().filter(|byte| *byte != b'\n'));
        input.push(b'\n');
    }
    let mut output = Vec::new();
    server.serve(input.as_slice(), &mut output).unwrap();
    let text = String::from_utf8(output).expect("responses are always valid UTF-8");
    let responses = text.lines().map(parse_response).count();
    assert!(responses > 1_000, "{responses}");
}

/// Identical hostile inputs produce identical outputs from identical servers (determinism).
#[test]
fn hostile_answers_are_deterministic() {
    let (first, _) = harness();
    let (second, _) = harness();
    for (label, input) in corpus() {
        assert_eq!(
            first.handle_line(&input),
            second.handle_line(&input),
            "{label}"
        );
    }
}
