// SPDX-License-Identifier: Apache-2.0
//! Scripted sessions for the stateless revision (2026-07-28), driven through
//! [`Server::handle_line`]: `server/discover`, per-request metadata, `resultType`, caching hints,
//! the removed methods, and the interplay with the `initialize` era.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code may unwrap and panic, as clippy.toml allows; helpers outside #[test] fns are not detected as tests"
)]

mod common;

use common::{
    Call, error_of, harness, initialize, meta, notification, reply, request, result_of, silent,
    stateless, tool_text,
};
use pn_ultramemory_mcp::{
    ExpandRequest, MAX_LIST_BYTES, RecallRequest, SUPPORTED_PROTOCOL_VERSIONS,
};
use serde_json::{Value, json};

/// The server identity as results report it in `_meta`.
fn server_info() -> Value {
    json!({ "io.modelcontextprotocol/serverInfo": { "name": "test-server", "version": "9.9.9" } })
}

/// `server/discover` answers without any handshake and describes the server.
#[test]
fn discover_describes_the_server() {
    let (server, _) = harness();
    let response = reply(
        &server,
        &stateless(json!("d1"), "server/discover", json!({})),
    );
    let result = result_of(&response, &json!("d1"));
    assert_eq!(result["resultType"], json!("complete"));
    assert_eq!(
        result["supportedVersions"],
        json!(SUPPORTED_PROTOCOL_VERSIONS)
    );
    assert_eq!(result["supportedVersions"][0], json!("2026-07-28"));
    assert_eq!(
        result["capabilities"],
        json!({ "tools": { "listChanged": false } })
    );
    assert_eq!(result["ttlMs"], json!(3_600_000));
    assert_eq!(result["cacheScope"], json!("public"));
    assert_eq!(result["_meta"], server_info());
}

/// The absent-params form of a stateless request is malformed: `_meta` is required.
#[test]
fn discover_without_meta_is_invalid_params() {
    let (server, _) = harness();
    error_of(
        &reply(&server, &request(json!(1), "server/discover", Value::Null)),
        -32602,
    );
    error_of(
        &reply(&server, &request(json!(1), "server/discover", json!({}))),
        -32602,
    );
    error_of(
        &reply(
            &server,
            &request(json!(1), "server/discover", json!({ "_meta": {} })),
        ),
        -32602,
    );
}

/// A whole stateless session with no `initialize`: list, every tool, errors.
#[test]
fn full_stateless_session() {
    let (server, log) = harness();

    let list = reply(&server, &stateless(json!(1), "tools/list", json!({})));
    let result = result_of(&list, &json!(1));
    assert_eq!(result["resultType"], json!("complete"));
    assert_eq!(
        result["tools"].as_array().unwrap().len(),
        pn_ultramemory_mcp::TOOL_NAMES.len()
    );
    assert_eq!(result["ttlMs"], json!(3_600_000));
    assert_eq!(result["cacheScope"], json!("public"));
    assert_eq!(result["_meta"], server_info());
    assert!(result.get("nextCursor").is_none());

    let cases = [
        (
            "recall",
            json!({ "q": "parser", "budget": 500 }),
            "recall:parser:Some(500)",
        ),
        (
            "impact",
            json!({ "symbol": "m::f", "depth": 3 }),
            "impact:m::f:Some(3)",
        ),
        (
            "remember",
            json!({ "kind": "fact", "text": "t", "about": ["a"] }),
            "remember:fact:t:a",
        ),
        ("expand", json!({ "id": "n1" }), "expand:n1"),
    ];
    for (index, (name, arguments, text)) in cases.into_iter().enumerate() {
        let id = json!(10 + index);
        let response = reply(
            &server,
            &stateless(
                id.clone(),
                "tools/call",
                json!({ "name": name, "arguments": arguments }),
            ),
        );
        let result = result_of(&response, &id);
        assert_eq!(result["resultType"], json!("complete"), "{name}");
        assert_eq!(result["isError"], json!(false));
        assert_eq!(tool_text(result), text);
        assert_eq!(result["_meta"], server_info());
        assert!(
            result.get("ttlMs").is_none(),
            "tool results are not cacheable"
        );
    }
    assert_eq!(log.calls().len(), 4);

    let failed = reply(
        &server,
        &stateless(
            json!(20),
            "tools/call",
            json!({ "name": "expand", "arguments": { "id": "FAIL" } }),
        ),
    );
    let result = result_of(&failed, &json!(20));
    assert_eq!(result["resultType"], json!("complete"));
    assert_eq!(result["isError"], json!(true));

    let invalid = reply(
        &server,
        &stateless(
            json!(21),
            "tools/call",
            json!({ "name": "recall", "arguments": {} }),
        ),
    );
    let result = result_of(&invalid, &json!(21));
    assert_eq!(result["isError"], json!(true));
    assert!(tool_text(result).contains("`q` is required"));

    let unknown = reply(
        &server,
        &stateless(json!(22), "tools/call", json!({ "name": "nope" })),
    );
    let error = error_of(&unknown, -32602);
    assert!(error["message"].as_str().unwrap().contains("nope"));
    assert!(unknown.get("result").is_none());
}

/// Each stateless request stands alone: nothing is remembered between them.
#[test]
fn stateless_requests_leave_no_session() {
    let (server, _) = harness();
    reply(&server, &stateless(json!(1), "tools/list", json!({})));
    reply(&server, &stateless(json!(2), "server/discover", json!({})));
    let bare = reply(&server, &request(json!(3), "tools/list", Value::Null));
    error_of(&bare, -32602);
    let call_without_meta = reply(&server, &common::call(4, "recall", json!({ "q": "x" })));
    error_of(&call_without_meta, -32602);
}

/// A legacy session does not exempt later stateless requests from carrying `_meta`.
#[test]
fn a_legacy_session_does_not_leak_into_stateless_validation() {
    let (server, _) = harness();
    reply(&server, &initialize(1, "2025-11-25"));
    let partial = json!({ "_meta": { "io.modelcontextprotocol/protocolVersion": "2026-07-28" } });
    error_of(
        &reply(&server, &request(json!(2), "tools/list", partial)),
        -32602,
    );
    let response = reply(&server, &stateless(json!(3), "tools/list", json!({})));
    assert!(result_of(&response, &json!(3)).get("resultType").is_some());
    let legacy = reply(&server, &request(json!(4), "tools/list", Value::Null));
    assert!(result_of(&legacy, &json!(4)).get("resultType").is_none());
}

/// Per-request metadata problems: `-32602` for missing or mistyped fields, `-32022` for versions.
#[test]
fn validates_per_request_metadata() {
    let (server, _) = harness();
    let version = "io.modelcontextprotocol/protocolVersion";
    let caps = "io.modelcontextprotocol/clientCapabilities";
    let cases: [(Value, i64); 10] = [
        (json!({ version: "2026-07-28" }), -32602),
        (json!({ caps: {} }), -32602),
        (json!({ version: 20_260_728, caps: {} }), -32602),
        (json!({ version: null, caps: {} }), -32602),
        (json!({ version: "2026-07-28", caps: [] }), -32602),
        (json!({ version: "2026-07-28", caps: "none" }), -32602),
        (json!({ version: "2026-07-28", caps: null }), -32602),
        (json!({ version: "1900-01-01", caps: {} }), -32022),
        (json!({ version: "2025-11-25", caps: {} }), -32022),
        (json!({ version: "", caps: {} }), -32022),
    ];
    for (meta, code) in cases {
        for method in ["tools/list", "server/discover", "tools/call"] {
            let line = request(json!(5), method, json!({ "name": "recall", "_meta": meta }));
            let response = reply(&server, &line);
            error_of(&response, code);
            assert_eq!(response["id"], json!(5));
        }
    }
}

/// An unsupported version lists what is supported, so the client can retry.
#[test]
fn unsupported_version_error_lists_the_supported_versions() {
    let (server, _) = harness();
    let line = request(
        json!(1),
        "tools/list",
        json!({ "_meta": {
            "io.modelcontextprotocol/protocolVersion": "2099-01-01",
            "io.modelcontextprotocol/clientCapabilities": {}
        }}),
    );
    let response = reply(&server, &line);
    let error = error_of(&response, -32022);
    assert_eq!(error["message"], json!("Unsupported protocol version"));
    assert_eq!(error["data"]["requested"], json!("2099-01-01"));
    assert_eq!(
        error["data"]["supported"],
        json!(SUPPORTED_PROTOCOL_VERSIONS)
    );
}

/// Extra `_meta` members and client identity are accepted and never change behaviour.
#[test]
fn optional_meta_fields_are_tolerated() {
    let (server, _) = harness();
    for extra in [
        json!({}),
        json!({ "io.modelcontextprotocol/clientInfo": { "name": "c", "version": "1" } }),
        json!({ "io.modelcontextprotocol/clientInfo": 5 }),
        json!({ "io.modelcontextprotocol/logLevel": "debug" }),
        json!({ "progressToken": "t1" }),
        json!({ "traceparent": "00-0af7651916cd43dd8448eb211c80319c-00f067aa0ba902b7-01" }),
        json!({ "com.example/anything": [1, 2, 3] }),
    ] {
        let mut full = meta();
        full.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let line = request(json!(1), "tools/list", json!({ "_meta": full }));
        let response = reply(&server, &line);
        assert_eq!(
            result_of(&response, &json!(1))["tools"]
                .as_array()
                .unwrap()
                .len(),
            pn_ultramemory_mcp::TOOL_NAMES.len()
        );
    }
}

/// Methods removed or not implemented in this revision are unknown methods.
#[test]
fn removed_methods_are_method_not_found() {
    let (server, _) = harness();
    for method in [
        "ping",
        "logging/setLevel",
        "resources/list",
        "prompts/list",
        "subscriptions/listen",
        "tasks/get",
        "completion/complete",
        "notifications/initialized",
        "",
    ] {
        let response = reply(&server, &stateless(json!(1), method, json!({})));
        error_of(&response, -32601);
    }
}

/// `initialize` always selects the legacy handshake, even when the request carries stateless
/// metadata, and answers with a legacy version.
#[test]
fn initialize_selects_the_legacy_era_even_with_meta() {
    let (server, _) = harness();
    let line = request(
        json!(1),
        "initialize",
        json!({ "protocolVersion": "2026-07-28", "_meta": meta() }),
    );
    let response = reply(&server, &line);
    assert_eq!(
        result_of(&response, &json!(1))["protocolVersion"],
        json!("2025-11-25")
    );
    assert!(result_of(&response, &json!(1)).get("resultType").is_none());
}

/// Batches do not exist in this revision.
#[test]
fn batches_are_refused_in_the_stateless_era() {
    let (server, log) = harness();
    let line = format!(
        "[{},{}]",
        stateless(json!(1), "tools/list", json!({})),
        stateless(json!(2), "server/discover", json!({}))
    );
    let response = reply(&server, &line);
    error_of(&response, -32600);
    assert_eq!(response["id"], Value::Null);
    assert!(log.calls().is_empty());
}

/// Notifications in the stateless era are ignored as everywhere else.
#[test]
fn stateless_notifications_are_silent() {
    let (server, _) = harness();
    silent(&server, &notification("notifications/cancelled"));
    silent(
        &server,
        &json!({ "jsonrpc": "2.0", "method": "notifications/cancelled",
                 "params": { "requestId": 3, "reason": "user", "_meta": meta() } })
        .to_string(),
    );
}

/// The tool list is byte-for-byte deterministic and the same for both eras.
#[test]
fn tool_list_is_deterministic_and_era_independent() {
    let (server, _) = harness();
    let first = server
        .handle_line(&stateless(json!(1), "tools/list", json!({})))
        .unwrap();
    let second = server
        .handle_line(&stateless(json!(1), "tools/list", json!({})))
        .unwrap();
    assert_eq!(first, second);

    let (other, _) = harness();
    reply(&other, &initialize(1, "2025-11-25"));
    let legacy = reply(&other, &request(json!(1), "tools/list", Value::Null));
    let modern: Value = serde_json::from_str(&first).unwrap();
    assert_eq!(legacy["result"]["tools"], modern["result"]["tools"]);
}

/// Wire arguments reach the backend unchanged in the stateless era too.
#[test]
fn stateless_calls_map_arguments_verbatim() {
    let (server, log) = harness();
    let odd = "  spaced \u{1F600}\ttab\nline  ";
    reply(
        &server,
        &stateless(
            json!(1),
            "tools/call",
            json!({ "name": "recall", "arguments": { "q": odd, "budget": 0 } }),
        ),
    );
    reply(
        &server,
        &stateless(
            json!(2),
            "tools/call",
            json!({ "name": "expand", "arguments": { "id": odd } }),
        ),
    );
    assert_eq!(
        log.calls(),
        [
            Call::Recall(RecallRequest {
                query: odd.into(),
                budget: Some(0),
                explain: None
            }),
            Call::Expand(ExpandRequest {
                id: odd.into(),
                from: None,
                to: None
            }),
        ]
    );
}

/// Measures the tool-list payload and enforces the 3 000 byte budget for every era.
#[test]
fn tool_list_payload_stays_tiny() {
    let (server, _) = harness();
    let stateless_line = server
        .handle_line(&stateless(json!(1), "tools/list", json!({})))
        .unwrap();
    let parsed: Value = serde_json::from_str(&stateless_line).unwrap();
    let tools_bytes = serde_json::to_string(&parsed["result"]["tools"])
        .unwrap()
        .len();

    let (legacy_server, _) = harness();
    reply(&legacy_server, &initialize(1, "2025-11-25"));
    let legacy_line = legacy_server
        .handle_line(&request(json!(1), "tools/list", Value::Null))
        .unwrap();

    println!("tools array: {tools_bytes} bytes");
    println!(
        "tools/list response, legacy era: {} bytes",
        legacy_line.len()
    );
    println!(
        "tools/list response, stateless era: {} bytes",
        stateless_line.len()
    );
    // MAX_LIST_BYTES carries the reasoning; this is where it is enforced. The list grew from six
    // tools to nine when brief, map, memories and feedback were added — feedback in particular was
    // the only way an agent could reach the learning subsystem at all.
    assert!(
        tools_bytes < MAX_LIST_BYTES,
        "tools array is {tools_bytes} bytes"
    );
    assert!(
        legacy_line.len() < MAX_LIST_BYTES,
        "legacy response is {} bytes",
        legacy_line.len()
    );
    assert!(
        stateless_line.len() < MAX_LIST_BYTES,
        "stateless response is {} bytes",
        stateless_line.len()
    );
}

/// `explain` and the line window travel from the wire to the backend in the stateless era.
#[test]
fn stateless_explain_and_line_window_reach_the_backend() {
    let (server, log) = harness();
    let explained = reply(
        &server,
        &stateless(
            json!(1),
            "tools/call",
            json!({ "name": "recall", "arguments": { "q": "parser", "budget": 300, "explain": false } }),
        ),
    );
    let result = result_of(&explained, &json!(1));
    assert_eq!(result["resultType"], json!("complete"));
    assert_eq!(tool_text(result), "recall:parser:Some(300):explain=false");

    let window = reply(
        &server,
        &stateless(
            json!(2),
            "tools/call",
            json!({ "name": "expand", "arguments": { "id": "n7", "from": 1, "to": 1 } }),
        ),
    );
    assert_eq!(
        tool_text(result_of(&window, &json!(2))),
        "expand:n7:Some(1)..Some(1)"
    );

    assert_eq!(
        log.calls(),
        [
            Call::Recall(RecallRequest {
                query: "parser".into(),
                budget: Some(300),
                explain: Some(false),
            }),
            Call::Expand(ExpandRequest {
                id: "n7".into(),
                from: Some(1),
                to: Some(1),
            }),
        ]
    );
}

/// Invalid new arguments are complete results with `isError`, never protocol errors.
#[test]
fn stateless_invalid_explain_and_line_window_are_tool_errors() {
    let (server, log) = harness();
    for (index, (tool, arguments)) in [
        ("recall", json!({ "q": "x", "explain": 1 })),
        ("expand", json!({ "id": "n", "from": 0 })),
        ("expand", json!({ "id": "n", "from": 5, "to": 4 })),
        ("expand", json!({ "id": "n", "to": -1 })),
    ]
    .into_iter()
    .enumerate()
    {
        let id = json!(index);
        let response = reply(
            &server,
            &stateless(
                id.clone(),
                "tools/call",
                json!({ "name": tool, "arguments": arguments }),
            ),
        );
        let result = result_of(&response, &id);
        assert_eq!(result["resultType"], json!("complete"));
        assert_eq!(result["isError"], json!(true), "{tool} {arguments}");
    }
    assert!(log.calls().is_empty());
}
