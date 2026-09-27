// SPDX-License-Identifier: Apache-2.0
//! End-to-end tests of [`Server::serve`] with in-memory streams: framing, bounded lines, invalid
//! UTF-8, flushing, I/O errors and concurrent use.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code may unwrap and panic, as clippy.toml allows; helpers outside #[test] fns are not detected as tests"
)]

mod common;

use std::io::{self, BufRead, BufReader, Cursor, Read, Write};

use common::{
    Call, call, error_of, harness, initialize, notification, parse_response, request, result_of,
    stateless, tool_text,
};
use pn_ultramemory_mcp::{MAX_LINE_BYTES, RecallRequest};
use serde_json::{Value, json};

/// Runs `serve` over `input` and returns the parsed response lines.
fn run(server: &pn_ultramemory_mcp::Server<common::Recorder>, input: &[u8]) -> Vec<Value> {
    let mut output = Vec::new();
    server
        .serve(input, &mut output)
        .expect("serve must end cleanly");
    let text = String::from_utf8(output).expect("output must be UTF-8");
    assert!(
        text.is_empty() || text.ends_with('\n'),
        "output must end with a newline"
    );
    text.lines().map(parse_response).collect()
}

/// Joins message lines into one newline-terminated input stream.
fn stream(lines: &[String]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for line in lines {
        bytes.extend_from_slice(line.as_bytes());
        bytes.push(b'\n');
    }
    bytes
}

/// A complete legacy session over the stream: exactly one output line per request.
#[test]
fn scripted_legacy_session_over_serve() {
    let (server, log) = harness();
    let input = stream(&[
        initialize(1, "2025-06-18"),
        notification("notifications/initialized"),
        request(json!(2), "tools/list", Value::Null),
        call(3, "recall", json!({ "q": "parser", "budget": 100 })),
        call(4, "impact", json!({ "symbol": "s" })),
        call(5, "remember", json!({ "kind": "convention", "text": "t" })),
        call(6, "expand", json!({ "id": "n1" })),
        call(7, "expand", json!({ "id": "FAIL" })),
        call(8, "nope", json!({})),
        request(json!(9), "ping", Value::Null),
        request(json!(10), "unknown/method", Value::Null),
    ]);
    let out = run(&server, &input);
    assert_eq!(out.len(), 10, "the notification gets no line");
    assert_eq!(
        result_of(&out[0], &json!(1))["protocolVersion"],
        json!("2025-06-18")
    );
    assert_eq!(
        result_of(&out[1], &json!(2))["tools"]
            .as_array()
            .unwrap()
            .len(),
        pn_ultramemory_mcp::TOOL_NAMES.len()
    );
    assert_eq!(
        tool_text(result_of(&out[2], &json!(3))),
        "recall:parser:Some(100)"
    );
    assert_eq!(tool_text(result_of(&out[3], &json!(4))), "impact:s:None");
    assert_eq!(
        tool_text(result_of(&out[4], &json!(5))),
        "remember:convention:t:"
    );
    assert_eq!(tool_text(result_of(&out[5], &json!(6))), "expand:n1");
    assert_eq!(result_of(&out[6], &json!(7))["isError"], json!(true));
    error_of(&out[7], -32602);
    assert_eq!(result_of(&out[8], &json!(9)), &json!({}));
    error_of(&out[9], -32601);
    assert_eq!(log.calls().len(), 5);
}

/// A complete stateless session over the stream, with no handshake.
#[test]
fn scripted_stateless_session_over_serve() {
    let (server, _) = harness();
    let input = stream(&[
        stateless(json!("probe"), "server/discover", json!({})),
        stateless(json!(1), "tools/list", json!({})),
        stateless(
            json!(2),
            "tools/call",
            json!({ "name": "expand", "arguments": { "id": "x" } }),
        ),
        request(json!(3), "tools/list", Value::Null),
    ]);
    let out = run(&server, &input);
    assert_eq!(out.len(), 4);
    assert_eq!(
        result_of(&out[0], &json!("probe"))["resultType"],
        json!("complete")
    );
    assert_eq!(
        result_of(&out[1], &json!(1))["tools"]
            .as_array()
            .unwrap()
            .len(),
        pn_ultramemory_mcp::TOOL_NAMES.len()
    );
    assert_eq!(tool_text(result_of(&out[2], &json!(2))), "expand:x");
    error_of(&out[3], -32602);
}

/// Empty input ends cleanly with no output; so does input that is only blank lines.
#[test]
fn ends_cleanly_at_eof() {
    let (server, _) = harness();
    assert!(run(&server, b"").is_empty());
    assert!(run(&server, b"\n\n  \n\t\n\r\n").is_empty());
}

/// CRLF endings, blank lines and a last line without a newline are all handled.
#[test]
fn tolerates_crlf_blank_lines_and_missing_final_newline() {
    let (server, _) = harness();
    let ping = |id: i64| request(json!(id), "ping", Value::Null);
    let input = format!("{}\r\n\r\n\n{}\r\n   \n{}", ping(1), ping(2), ping(3));
    let out = run(&server, input.as_bytes());
    let ids: Vec<&Value> = out.iter().map(|r| &r["id"]).collect();
    assert_eq!(ids, [&json!(1), &json!(2), &json!(3)]);
}

/// A message pretty-printed over several lines is not one message: each line is judged alone.
#[test]
fn embedded_newlines_split_messages() {
    let (server, _) = harness();
    let out = run(
        &server,
        b"{\"jsonrpc\":\"2.0\",\n\"id\":1,\n\"method\":\"ping\"}\n",
    );
    assert_eq!(out.len(), 3, "three fragments, three parse errors");
    for response in &out {
        error_of(response, -32700);
        assert_eq!(response["id"], Value::Null);
    }
}

/// Pads a ping request with trailing spaces to exactly `length` bytes.
fn padded_ping(length: usize) -> Vec<u8> {
    let mut line = request(json!(1), "ping", Value::Null).into_bytes();
    assert!(line.len() <= length);
    line.resize(length, b' ');
    line
}

/// A line of exactly 4 MiB is served; one byte more is refused and the stream resynchronises.
#[test]
fn oversized_lines_are_refused_and_the_stream_resynchronises() {
    let (server, _) = harness();

    let mut input = padded_ping(MAX_LINE_BYTES);
    input.push(b'\n');
    let out = run(&server, &input);
    assert_eq!(out.len(), 1);
    assert_eq!(result_of(&out[0], &json!(1)), &json!({}));

    let mut input = padded_ping(MAX_LINE_BYTES + 1);
    input.push(b'\n');
    input.extend_from_slice(request(json!(2), "ping", Value::Null).as_bytes());
    input.push(b'\n');
    let out = run(&server, &input);
    assert_eq!(out.len(), 2);
    let error = error_of(&out[0], -32600);
    assert!(error["message"].as_str().unwrap().contains("4194304"));
    assert_eq!(out[0]["id"], Value::Null);
    assert_eq!(result_of(&out[1], &json!(2)), &json!({}));
}

/// A huge line with no newline at all is refused at end of input, without buffering it.
#[test]
fn a_huge_unterminated_line_is_refused_at_eof() {
    let (server, _) = harness();
    let input = vec![b'x'; 3 * MAX_LINE_BYTES];
    let out = run(&server, &input);
    assert_eq!(out.len(), 1);
    error_of(&out[0], -32600);
}

/// A reader that yields endless data in small chunks; `serve` must refuse it, not hang or grow.
struct Endless {
    /// How many bytes were served so far.
    served: usize,
    /// Stop after this many bytes (with end of input).
    stop_after: usize,
    /// The chunk handed out on every call.
    chunk: [u8; 8192],
}

impl Read for Endless {
    /// Not used: `serve` reads through `BufRead`.
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let n = out
            .len()
            .min(self.stop_after - self.served)
            .min(self.chunk.len());
        out[..n].copy_from_slice(&self.chunk[..n]);
        self.served += n;
        Ok(n)
    }
}

impl BufRead for Endless {
    /// Returns the next chunk until the byte budget is spent.
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        let n = self.chunk.len().min(self.stop_after - self.served);
        Ok(&self.chunk[..n])
    }

    /// Accounts for consumed bytes.
    fn consume(&mut self, amount: usize) {
        self.served += amount;
    }
}

/// Memory stays bounded and the loop terminates on a 64 MiB stream with no newline.
#[test]
fn an_endless_line_does_not_exhaust_memory() {
    let (server, _) = harness();
    let reader = Endless {
        served: 0,
        stop_after: 64 * 1024 * 1024,
        chunk: [b'a'; 8192],
    };
    let mut output = Vec::new();
    server.serve(reader, &mut output).unwrap();
    let text = String::from_utf8(output).unwrap();
    assert_eq!(text.lines().count(), 1);
    error_of(&parse_response(text.trim_end()), -32600);
}

/// Invalid UTF-8 is a parse error for that line only, and never repaired silently.
#[test]
fn invalid_utf8_is_refused_per_line() {
    let (server, log) = harness();
    let mut input = Vec::new();
    input.extend_from_slice(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"recall\",\"arguments\":{\"q\":\"bad \xff\xfe bytes\"}}}\n");
    input.extend_from_slice(b"\xc3\x28\n");
    input.extend_from_slice(b"\xed\xa0\x80\n");
    input.extend_from_slice(b"\xf0\x28\x8c\x28\n");
    input.extend_from_slice(request(json!(2), "ping", Value::Null).as_bytes());
    input.push(b'\n');
    let out = run(&server, &input);
    assert_eq!(out.len(), 5);
    for response in &out[..4] {
        let error = error_of(response, -32700);
        assert!(error["message"].as_str().unwrap().contains("UTF-8"));
    }
    assert_eq!(result_of(&out[4], &json!(2)), &json!({}));
    assert!(
        log.calls().is_empty(),
        "corrupted text must not reach the backend"
    );
}

/// Valid multi-byte UTF-8 survives the stream byte for byte.
#[test]
fn multibyte_text_round_trips() {
    let (server, log) = harness();
    let text = "caf\u{e9} \u{65e5}\u{672c}\u{8a9e} \u{1F980} \u{202e}rtl";
    let line = stateless(
        json!(1),
        "tools/call",
        json!({ "name": "recall", "arguments": { "q": text } }),
    );
    let out = run(&server, &stream(&[line]));
    assert_eq!(
        tool_text(result_of(&out[0], &json!(1))),
        format!("recall:{text}:None")
    );
    assert_eq!(
        log.calls(),
        [Call::Recall(RecallRequest {
            query: text.into(),
            budget: None,
            explain: None
        })]
    );
}

/// Records writes and flushes so a test can check the flush discipline.
#[derive(Default)]
struct Spy {
    /// Everything written.
    bytes: Vec<u8>,
    /// Number of flush calls.
    flushes: usize,
    /// Number of flushes that happened while the last byte written was not a newline.
    flushes_mid_line: usize,
}

impl Write for Spy {
    /// Appends to the record.
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.bytes.extend_from_slice(data);
        Ok(data.len())
    }

    /// Counts the flush and notes whether it fell on a line boundary.
    fn flush(&mut self) -> io::Result<()> {
        self.flushes += 1;
        if self.bytes.last() != Some(&b'\n') {
            self.flushes_mid_line += 1;
        }
        Ok(())
    }
}

/// Output is flushed after every response, on a line boundary.
#[test]
fn flushes_after_every_response() {
    let (server, _) = harness();
    let input = stream(&[
        request(json!(1), "ping", Value::Null),
        notification("notifications/initialized"),
        request(json!(2), "ping", Value::Null),
        "garbage".to_owned(),
    ]);
    let mut spy = Spy::default();
    server.serve(input.as_slice(), &mut spy).unwrap();
    assert_eq!(spy.flushes, 3);
    assert_eq!(spy.flushes_mid_line, 0);
    assert_eq!(String::from_utf8(spy.bytes).unwrap().lines().count(), 3);
}

/// A writer that always fails.
struct FailingWriter;

impl Write for FailingWriter {
    /// Fails as a closed pipe would.
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            "client went away",
        ))
    }

    /// Never reached.
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// A writer error ends `serve` with that error.
#[test]
fn write_errors_are_returned() {
    let (server, _) = harness();
    let input = stream(&[request(json!(1), "ping", Value::Null)]);
    let error = server.serve(input.as_slice(), FailingWriter).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
}

/// A reader that delivers its data and then fails instead of reporting end of input.
struct FailAfterData(Cursor<Vec<u8>>);

impl Read for FailAfterData {
    /// Not used: `serve` reads through `BufRead`.
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.0.read(out)
    }
}

impl BufRead for FailAfterData {
    /// Serves the data, then errors once it is exhausted.
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if self.0.fill_buf()?.is_empty() {
            return Err(io::Error::new(io::ErrorKind::ConnectionReset, "reset"));
        }
        self.0.fill_buf()
    }

    /// Delegates to the cursor.
    fn consume(&mut self, amount: usize) {
        self.0.consume(amount);
    }
}

/// A reader error ends `serve` with that error, after the responses already produced.
#[test]
fn read_errors_are_returned_after_earlier_responses() {
    let (server, _) = harness();
    let data = stream(&[request(json!(1), "ping", Value::Null)]);
    let mut output = Vec::new();
    let error = server
        .serve(FailAfterData(Cursor::new(data)), &mut output)
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::ConnectionReset);
    assert_eq!(String::from_utf8(output).unwrap().lines().count(), 1);
}

/// `serve` works through a small `BufReader` too, where lines span many refills.
#[test]
fn works_with_tiny_read_buffers() {
    let (server, _) = harness();
    let input = stream(&[
        initialize(1, "2025-11-25"),
        call(2, "recall", json!({ "q": "a".repeat(5000) })),
        request(json!(3), "tools/list", Value::Null),
    ]);
    let reader = BufReader::with_capacity(7, input.as_slice());
    let mut output = Vec::new();
    server.serve(reader, &mut output).unwrap();
    let out: Vec<Value> = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(parse_response)
        .collect();
    assert_eq!(out.len(), 3);
    assert_eq!(
        tool_text(result_of(&out[1], &json!(2))).len(),
        "recall:".len() + 5000 + ":None".len()
    );
}

/// Every output line is a compact JSON-RPC message: nothing else ever reaches the stream.
#[test]
fn output_carries_only_protocol_messages() {
    let (server, _) = harness();
    let mut lines = vec![
        initialize(1, "2025-03-26"),
        call(
            2,
            "remember",
            json!({ "kind": "fact", "text": "multi\nline\r\ntext\u{2028}with\u{0}nul" }),
        ),
        "[1,2,3]".to_owned(),
        "{".to_owned(),
        request(json!(3), "tools/list", Value::Null),
    ];
    lines.push(stateless(json!(4), "tools/list", json!({})));
    let out = run(&server, &stream(&lines));
    assert_eq!(out.len(), lines.len());
}

/// The server is `Send + Sync`: concurrent callers get correct, independent answers.
#[test]
fn concurrent_callers_do_not_interfere() {
    let (server, log) = harness();
    std::thread::scope(|scope| {
        for worker in 0..8 {
            let server = &server;
            scope.spawn(move || {
                for turn in 0..50 {
                    let id = worker * 1000 + turn;
                    let line = stateless(
                        json!(id),
                        "tools/call",
                        json!({ "name": "recall", "arguments": { "q": format!("w{worker}t{turn}") } }),
                    );
                    let response = parse_response(&server.handle_line(&line).unwrap());
                    assert_eq!(
                        tool_text(result_of(&response, &json!(id))),
                        format!("recall:w{worker}t{turn}:None")
                    );
                }
            });
        }
    });
    assert_eq!(log.calls().len(), 400);
}
