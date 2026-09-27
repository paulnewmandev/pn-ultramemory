// SPDX-License-Identifier: Apache-2.0
//! Shared helpers for the integration tests: a recording backend, message builders and envelope
//! assertions.
//!
//! The backend records every request it receives, so a test can prove how wire arguments were
//! mapped, and it can be told to fail or panic through trigger words in the argument text.
#![allow(
    dead_code,
    reason = "each integration test crate uses a different subset of the helpers"
)]

use std::sync::{Arc, Mutex};

use pn_ultramemory_mcp::{
    Backend, ExpandRequest, ImpactRequest, OutlineRequest, RecallRequest, RememberRequest, Server,
    ServerInfo, ToolFailure,
};
use serde_json::{Map, Value, json};

/// One call as the backend received it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Call {
    /// A `recall` call.
    Recall(RecallRequest),
    /// An `impact` call.
    Impact(ImpactRequest),
    /// A `remember` call.
    Remember(RememberRequest),
    /// An `expand` call.
    Expand(ExpandRequest),
    /// An `outline` call.
    Outline(OutlineRequest),
}

/// The shared, ordered record of backend calls.
#[derive(Default)]
pub(crate) struct Log {
    /// Calls in arrival order.
    calls: Mutex<Vec<Call>>,
}

impl Log {
    /// Appends a call.
    fn push(&self, call: Call) {
        self.calls.lock().unwrap().push(call);
    }

    /// Returns a copy of every call so far.
    pub(crate) fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }
}

/// A backend that records calls and answers with a description of what it received.
///
/// The word `FAIL` in the main text argument makes the call fail with a [`ToolFailure`]; the word
/// `PANIC` makes the backend panic.
#[derive(Clone, Default)]
pub(crate) struct Recorder {
    /// The record shared with the test.
    log: Arc<Log>,
}

/// Turns the trigger words of `text` into a failure or a panic, else returns `reply`.
fn scripted(text: &str, reply: String) -> Result<String, ToolFailure> {
    assert!(!text.contains("PANIC"), "scripted backend panic");
    if text.contains("FAIL") {
        return Err(ToolFailure(format!(
            "failed: {}",
            text.chars().take(40).collect::<String>()
        )));
    }
    Ok(reply)
}

impl Backend for Recorder {
    /// Records and echoes the query and budget, plus the `explain` flag when it was given.
    fn recall(&self, request: RecallRequest) -> Result<String, ToolFailure> {
        self.log.push(Call::Recall(request.clone()));
        let explain = request
            .explain
            .map_or_else(String::new, |flag| format!(":explain={flag}"));
        scripted(
            &request.query,
            format!("recall:{}:{:?}{explain}", request.query, request.budget),
        )
    }

    /// Records and echoes the symbol and depth.
    fn impact(&self, request: ImpactRequest) -> Result<String, ToolFailure> {
        self.log.push(Call::Impact(request.clone()));
        scripted(
            &request.symbol,
            format!("impact:{}:{:?}", request.symbol, request.depth),
        )
    }

    /// Records and echoes the kind, text and anchors.
    fn remember(&self, request: RememberRequest) -> Result<String, ToolFailure> {
        self.log.push(Call::Remember(request.clone()));
        scripted(
            &request.text,
            format!(
                "remember:{}:{}:{}",
                request.kind,
                request.text,
                request.about.join("|")
            ),
        )
    }

    /// Records and echoes the id, plus the line window when one bound was given.
    fn expand(&self, request: ExpandRequest) -> Result<String, ToolFailure> {
        self.log.push(Call::Expand(request.clone()));
        let window = if request.from.is_some() || request.to.is_some() {
            format!(":{:?}..{:?}", request.from, request.to)
        } else {
            String::new()
        };
        scripted(&request.id, format!("expand:{}{window}", request.id))
    }

    /// Records and echoes the path, plus the budget when one was given.
    fn outline(&self, request: OutlineRequest) -> Result<String, ToolFailure> {
        self.log.push(Call::Outline(request.clone()));
        let budget = request
            .budget
            .map_or_else(String::new, |budget| format!(":{budget}"));
        scripted(&request.path, format!("outline:{}{budget}", request.path))
    }
}

/// Server identity used by the tests.
pub(crate) fn info() -> ServerInfo {
    ServerInfo {
        name: "test-server".to_owned(),
        version: "9.9.9".to_owned(),
    }
}

/// A fresh server with a recording backend, and the record.
pub(crate) fn harness() -> (Server<Recorder>, Arc<Log>) {
    let recorder = Recorder::default();
    let log = Arc::clone(&recorder.log);
    (Server::new(recorder, info()), log)
}

/// The per-request metadata of the stateless revision.
pub(crate) fn meta() -> Value {
    json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientInfo": { "name": "test-client", "version": "1.0.0" },
        "io.modelcontextprotocol/clientCapabilities": {}
    })
}

/// Builds a request line. `params` of `Value::Null` omits the member.
pub(crate) fn request(id: Value, method: &str, params: Value) -> String {
    let mut message = Map::new();
    message.insert("jsonrpc".to_owned(), json!("2.0"));
    message.insert("id".to_owned(), id);
    message.insert("method".to_owned(), json!(method));
    if !params.is_null() {
        message.insert("params".to_owned(), params);
    }
    Value::Object(message).to_string()
}

/// Builds a stateless-revision request line: `params` gets the per-request `_meta` merged in.
pub(crate) fn stateless(id: Value, method: &str, params: Value) -> String {
    let mut params = match params {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    params.insert("_meta".to_owned(), meta());
    request(id, method, Value::Object(params))
}

/// Builds a notification line.
pub(crate) fn notification(method: &str) -> String {
    json!({ "jsonrpc": "2.0", "method": method }).to_string()
}

/// Builds an `initialize` request line for the given protocol version.
pub(crate) fn initialize(id: i64, version: &str) -> String {
    request(
        json!(id),
        "initialize",
        json!({
            "protocolVersion": version,
            "capabilities": {},
            "clientInfo": { "name": "test-client", "version": "1.0.0" }
        }),
    )
}

/// Builds a `tools/call` request line.
pub(crate) fn call(id: i64, name: &str, arguments: Value) -> String {
    let mut params = Map::new();
    params.insert("name".to_owned(), json!(name));
    params.insert("arguments".to_owned(), arguments);
    request(json!(id), "tools/call", Value::Object(params))
}

/// Checks that `line` is one compact JSON-RPC response (or batch of them) and returns it parsed.
pub(crate) fn parse_response(line: &str) -> Value {
    assert!(
        !line.contains('\n') && !line.contains('\r'),
        "raw line break in {line:?}"
    );
    let value: Value = serde_json::from_str(line).unwrap_or_else(|e| panic!("{e}: {line:?}"));
    match &value {
        Value::Array(items) => items.iter().for_each(assert_envelope),
        single => assert_envelope(single),
    }
    value
}

/// Asserts the JSON-RPC 2.0 response envelope: version, id, and exactly one of result and error.
pub(crate) fn assert_envelope(response: &Value) {
    let object = response
        .as_object()
        .unwrap_or_else(|| panic!("not an object: {response}"));
    assert_eq!(object.get("jsonrpc"), Some(&json!("2.0")), "{response}");
    let id = object
        .get("id")
        .unwrap_or_else(|| panic!("no id: {response}"));
    assert!(
        id.is_null() || id.is_string() || id.is_i64() || id.is_u64(),
        "bad id type: {response}"
    );
    assert_eq!(
        u8::from(object.contains_key("result")) + u8::from(object.contains_key("error")),
        1,
        "need exactly one of result and error: {response}"
    );
    if let Some(error) = object.get("error") {
        assert!(error["code"].is_i64(), "{response}");
        assert!(error["message"].is_string(), "{response}");
    }
}

/// Sends one line and returns the parsed response, which must exist.
pub(crate) fn reply<B: Backend>(server: &Server<B>, line: &str) -> Value {
    let response = server
        .handle_line(line)
        .unwrap_or_else(|| panic!("expected a response to {line:?}"));
    parse_response(&response)
}

/// Sends one line that must not produce any output.
pub(crate) fn silent<B: Backend>(server: &Server<B>, line: &str) {
    assert_eq!(server.handle_line(line), None, "{line:?}");
}

/// Returns the `result` of a successful response with the given id.
pub(crate) fn result_of<'a>(response: &'a Value, id: &Value) -> &'a Value {
    assert_eq!(&response["id"], id, "{response}");
    response
        .get("result")
        .unwrap_or_else(|| panic!("expected a result: {response}"))
}

/// Returns the `error` of a failed response after checking its code.
pub(crate) fn error_of(response: &Value, code: i64) -> &Value {
    let error = response
        .get("error")
        .unwrap_or_else(|| panic!("expected an error: {response}"));
    assert_eq!(error["code"], json!(code), "{response}");
    error
}

/// Returns the text of the single text block of a tool result.
pub(crate) fn tool_text(result: &Value) -> &str {
    let content = result["content"].as_array().expect("content array");
    assert_eq!(content.len(), 1, "{result}");
    assert_eq!(content[0]["type"], json!("text"));
    content[0]["text"].as_str().expect("text")
}
