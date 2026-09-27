// SPDX-License-Identifier: Apache-2.0
//! Scripted sessions for the `initialize`-based protocol revisions (2024-11-05 to 2025-11-25),
//! driven through [`Server::handle_line`].
//!
//! Covers the handshake, version negotiation, refusal before `initialize`, the four tools,
//! protocol versus tool errors, ids of every legal kind, notifications and JSON-RPC batches.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code may unwrap and panic, as clippy.toml allows; helpers outside #[test] fns are not detected as tests"
)]

mod common;

use common::{
    Call, call, error_of, harness, initialize, notification, reply, request, result_of, silent,
    tool_text,
};
use pn_ultramemory_mcp::{ExpandRequest, ImpactRequest, RecallRequest, RememberRequest};
use serde_json::{Value, json};

/// A complete session: handshake, tool list, every tool, and the errors around them.
#[test]
fn full_session_with_every_tool() {
    let (server, log) = harness();

    let init = reply(&server, &initialize(1, "2025-11-25"));
    let result = result_of(&init, &json!(1));
    assert_eq!(result["protocolVersion"], json!("2025-11-25"));
    assert_eq!(
        result["capabilities"],
        json!({ "tools": { "listChanged": false } })
    );
    assert_eq!(
        result["serverInfo"],
        json!({ "name": "test-server", "version": "9.9.9" })
    );
    assert!(
        result.get("resultType").is_none(),
        "legacy results carry no resultType"
    );
    silent(&server, &notification("notifications/initialized"));

    let list = reply(&server, &request(json!(2), "tools/list", Value::Null));
    let tools = result_of(&list, &json!(2))["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, pn_ultramemory_mcp::TOOL_NAMES);
    assert!(result_of(&list, &json!(2)).get("ttlMs").is_none());

    let recall = reply(
        &server,
        &call(3, "recall", json!({ "q": "parser", "budget": 900 })),
    );
    let result = result_of(&recall, &json!(3));
    assert_eq!(tool_text(result), "recall:parser:Some(900)");
    assert_eq!(result["isError"], json!(false));
    assert!(result.get("resultType").is_none());

    let impact = reply(&server, &call(4, "impact", json!({ "symbol": "a::b" })));
    assert_eq!(tool_text(result_of(&impact, &json!(4))), "impact:a::b:None");

    let remember = reply(
        &server,
        &call(
            5,
            "remember",
            json!({ "kind": "lesson", "text": "cache it", "about": ["x", "y"] }),
        ),
    );
    assert_eq!(
        tool_text(result_of(&remember, &json!(5))),
        "remember:lesson:cache it:x|y"
    );

    let expand = reply(&server, &call(6, "expand", json!({ "id": "n7" })));
    assert_eq!(tool_text(result_of(&expand, &json!(6))), "expand:n7");

    assert_eq!(
        log.calls(),
        [
            Call::Recall(RecallRequest {
                query: "parser".into(),
                budget: Some(900),
                explain: None
            }),
            Call::Impact(ImpactRequest {
                symbol: "a::b".into(),
                depth: None
            }),
            Call::Remember(RememberRequest {
                kind: "lesson".into(),
                text: "cache it".into(),
                about: vec!["x".into(), "y".into()],
            }),
            Call::Expand(ExpandRequest {
                id: "n7".into(),
                from: None,
                to: None
            }),
        ]
    );

    let ping = reply(&server, &request(json!("p"), "ping", Value::Null));
    assert_eq!(result_of(&ping, &json!("p")), &json!({}));
}

/// Backend failures and argument problems are normal results with `isError: true`.
#[test]
fn tool_errors_are_results_not_protocol_errors() {
    let (server, log) = harness();
    reply(&server, &initialize(1, "2025-06-18"));

    let failed = reply(&server, &call(2, "recall", json!({ "q": "please FAIL" })));
    let result = result_of(&failed, &json!(2));
    assert_eq!(result["isError"], json!(true));
    assert_eq!(tool_text(result), "failed: please FAIL");

    let invalid = reply(
        &server,
        &call(3, "remember", json!({ "kind": "note", "text": "t" })),
    );
    let result = result_of(&invalid, &json!(3));
    assert_eq!(result["isError"], json!(true));
    assert!(tool_text(result).contains("`kind` must be one of decision"));

    let missing = reply(
        &server,
        &request(json!(4), "tools/call", json!({ "name": "recall" })),
    );
    let result = result_of(&missing, &json!(4));
    assert_eq!(result["isError"], json!(true));
    assert!(tool_text(result).contains("`q` is required"));

    assert_eq!(
        log.calls().len(),
        1,
        "only the FAIL call reached the backend"
    );
}

/// Protocol-level errors use the proper JSON-RPC codes.
#[test]
fn protocol_errors_use_standard_codes() {
    let (server, _) = harness();
    reply(&server, &initialize(1, "2025-11-25"));

    let unknown_tool = reply(&server, &call(2, "forget", json!({})));
    assert!(
        error_of(&unknown_tool, -32602)["message"]
            .as_str()
            .unwrap()
            .contains("forget")
    );

    let cases: [(&str, Value, i64); 12] = [
        ("tools/call", Value::Null, -32602),
        ("tools/call", json!([]), -32602),
        ("tools/call", json!("x"), -32602),
        ("tools/call", json!({}), -32602),
        ("tools/call", json!({ "name": 5 }), -32602),
        ("tools/call", json!({ "name": null }), -32602),
        (
            "tools/call",
            json!({ "name": "recall", "arguments": "q" }),
            -32602,
        ),
        (
            "tools/call",
            json!({ "name": "recall", "arguments": [1] }),
            -32602,
        ),
        ("tools/list", json!([]), -32602),
        ("tools/list", json!({ "cursor": "abc" }), -32602),
        ("nonsense", Value::Null, -32601),
        ("tools/list/extra", Value::Null, -32601),
    ];
    for (method, params, code) in cases {
        let response = reply(&server, &request(json!(9), method, params.clone()));
        error_of(&response, code);
        assert_eq!(response["id"], json!(9), "{method} {params}");
    }
}

/// A null cursor is the same as none, and a params-less list works.
#[test]
fn tools_list_tolerates_null_cursor_and_missing_params() {
    let (server, _) = harness();
    reply(&server, &initialize(1, "2025-11-25"));
    for params in [
        Value::Null,
        json!({}),
        json!({ "cursor": null }),
        json!({ "_meta": {} }),
    ] {
        let response = reply(&server, &request(json!(2), "tools/list", params));
        assert_eq!(
            result_of(&response, &json!(2))["tools"]
                .as_array()
                .unwrap()
                .len(),
            pn_ultramemory_mcp::TOOL_NAMES.len()
        );
    }
}

/// Capabilities the server does not have are unknown methods, not crashes.
#[test]
fn unsupported_features_are_method_not_found() {
    let (server, _) = harness();
    reply(&server, &initialize(1, "2025-11-25"));
    for method in [
        "resources/list",
        "resources/read",
        "resources/subscribe",
        "prompts/list",
        "prompts/get",
        "logging/setLevel",
        "completion/complete",
        "subscriptions/listen",
        "tasks/get",
        "roots/list",
        "sampling/createMessage",
        "",
        "Tools/List",
    ] {
        let response = reply(&server, &request(json!(1), method, Value::Null));
        error_of(&response, -32601);
    }
}

/// Version negotiation: the client's version when supported, otherwise the newest legacy one.
#[test]
fn negotiates_the_protocol_version() {
    let table = [
        ("2024-11-05", "2024-11-05"),
        ("2025-03-26", "2025-03-26"),
        ("2025-06-18", "2025-06-18"),
        ("2025-11-25", "2025-11-25"),
        ("2026-07-28", "2025-11-25"),
        ("2027-01-01", "2025-11-25"),
        ("2023-01-01", "2025-11-25"),
        ("1.0", "2025-11-25"),
        ("", "2025-11-25"),
        ("latest", "2025-11-25"),
        ("2025-11-25 ", "2025-11-25"),
    ];
    for (requested, agreed) in table {
        let (server, _) = harness();
        let response = reply(&server, &initialize(1, requested));
        assert_eq!(
            result_of(&response, &json!(1))["protocolVersion"],
            json!(agreed),
            "{requested}"
        );
    }
}

/// `initialize` needs a string `protocolVersion`, and a failed attempt does not open the session.
#[test]
fn invalid_initialize_does_not_start_a_session() {
    let (server, _) = harness();
    for params in [
        Value::Null,
        json!([]),
        json!("2025-11-25"),
        json!({}),
        json!({ "protocolVersion": 20_251_125 }),
        json!({ "protocolVersion": null }),
        json!({ "protocolVersion": ["2025-11-25"] }),
        json!({ "capabilities": {} }),
    ] {
        let response = reply(&server, &request(json!(1), "initialize", params.clone()));
        error_of(&response, -32602);
        let list = reply(&server, &request(json!(2), "tools/list", Value::Null));
        error_of(&list, -32602);
    }
}

/// Before `initialize`, tools are refused with a clear error while `ping` still works.
#[test]
fn tools_are_refused_before_initialize() {
    let (server, log) = harness();
    let list = reply(&server, &request(json!(1), "tools/list", Value::Null));
    assert!(
        error_of(&list, -32602)["message"]
            .as_str()
            .unwrap()
            .contains("initialize")
    );
    let called = reply(&server, &call(2, "recall", json!({ "q": "x" })));
    error_of(&called, -32602);
    assert!(log.calls().is_empty());

    let ping = reply(&server, &request(json!(3), "ping", Value::Null));
    assert_eq!(result_of(&ping, &json!(3)), &json!({}));
    error_of(
        &reply(&server, &request(json!(4), "nope", Value::Null)),
        -32601,
    );
    error_of(
        &reply(&server, &request(json!(5), "server/discover", json!({}))),
        -32602,
    );

    silent(&server, &notification("notifications/initialized"));
    error_of(
        &reply(&server, &request(json!(6), "tools/list", Value::Null)),
        -32602,
    );

    reply(&server, &initialize(7, "2025-11-25"));
    let list = reply(&server, &request(json!(8), "tools/list", Value::Null));
    assert_eq!(
        result_of(&list, &json!(8))["tools"]
            .as_array()
            .unwrap()
            .len(),
        pn_ultramemory_mcp::TOOL_NAMES.len()
    );
}

/// Repeating `initialize` renegotiates instead of failing.
#[test]
fn reinitialize_renegotiates() {
    let (server, _) = harness();
    let first = reply(&server, &initialize(1, "2025-03-26"));
    assert_eq!(
        result_of(&first, &json!(1))["protocolVersion"],
        json!("2025-03-26")
    );
    let second = reply(&server, &initialize(2, "2024-11-05"));
    assert_eq!(
        result_of(&second, &json!(2))["protocolVersion"],
        json!("2024-11-05")
    );
}

/// Notifications are never answered, known or not, in any state.
#[test]
fn notifications_never_get_a_response() {
    let (server, _) = harness();
    for method in [
        "notifications/initialized",
        "notifications/cancelled",
        "notifications/progress",
        "notifications/roots/list_changed",
        "notifications/does-not-exist",
        "totally/unknown",
        "",
    ] {
        silent(&server, &notification(method));
    }
    reply(&server, &initialize(1, "2025-11-25"));
    for method in [
        "notifications/initialized",
        "notifications/cancelled",
        "whatever",
    ] {
        silent(&server, &notification(method));
    }
    silent(
        &server,
        &json!({ "jsonrpc": "2.0", "method": "notifications/cancelled", "params": { "requestId": 1 } })
            .to_string(),
    );
    silent(
        &server,
        r#"{"jsonrpc":"2.0","method":"tools/call","params":[1,2,3]}"#,
    );
    silent(&server, r#"{"jsonrpc":"2.0","id":1,"result":{}}"#);
    silent(
        &server,
        r#"{"jsonrpc":"2.0","id":1,"error":{"code":1,"message":"x"}}"#,
    );
}

/// Every legal id kind is echoed unchanged; illegal ids are invalid requests with a null id.
#[test]
fn ids_are_echoed_and_validated() {
    let (server, _) = harness();
    for id in [
        json!(0),
        json!(-1),
        json!(42),
        json!(i64::MAX),
        json!(u64::MAX),
        json!(""),
        json!("abc"),
        json!("line\nbreak \"quoted\" \u{1F600}"),
        json!("x".repeat(1000)),
    ] {
        let response = reply(&server, &request(id.clone(), "ping", Value::Null));
        assert_eq!(result_of(&response, &id), &json!({}));
    }
    for text in [
        r#"{"jsonrpc":"2.0","id":null,"method":"ping"}"#,
        r#"{"jsonrpc":"2.0","id":1.5,"method":"ping"}"#,
        r#"{"jsonrpc":"2.0","id":true,"method":"ping"}"#,
        r#"{"jsonrpc":"2.0","id":[1],"method":"ping"}"#,
        r#"{"jsonrpc":"2.0","id":{},"method":"ping"}"#,
    ] {
        let response = reply(&server, text);
        error_of(&response, -32600);
        assert_eq!(response["id"], Value::Null, "{text}");
    }
}

/// Structurally invalid requests get `-32600`, with the id when it is readable.
#[test]
fn invalid_requests_are_reported() {
    let (server, _) = harness();
    for (text, id) in [
        (r#"{"jsonrpc":"2.0","id":3}"#, json!(3)),
        (r#"{"jsonrpc":"2.0","id":"a","method":7}"#, json!("a")),
        (r#"{"jsonrpc":"1.0","id":4,"method":"ping"}"#, json!(4)),
        (r#"{"id":5,"method":"ping"}"#, json!(5)),
        ("{}", Value::Null),
        ("42", Value::Null),
        (r#""ping""#, Value::Null),
        ("null", Value::Null),
        ("true", Value::Null),
    ] {
        let response = reply(&server, text);
        error_of(&response, -32600);
        assert_eq!(response["id"], id, "{text}");
    }
}

/// Malformed JSON is a parse error with a null id.
#[test]
fn malformed_json_is_a_parse_error() {
    let (server, _) = harness();
    for text in [
        "{",
        "}",
        "{\"jsonrpc\":\"2.0\",",
        "not json",
        "{'a':1}",
        "[1,2",
        "{\"a\":1}}",
        "\"unterminated",
    ] {
        let response = reply(&server, text);
        error_of(&response, -32700);
        assert_eq!(response["id"], Value::Null);
    }
}

/// Blank lines and whitespace are ignored; whitespace around a message is tolerated.
#[test]
fn blank_lines_are_ignored() {
    let (server, _) = harness();
    for text in ["", " ", "\t", "\r", "  \r  ", "\u{a0}"] {
        silent(&server, text);
    }
    let padded = format!("  \t{}  \r", request(json!(1), "ping", Value::Null));
    assert_eq!(result_of(&reply(&server, &padded), &json!(1)), &json!({}));
}

/// Builds a JSON-RPC batch line from message lines.
fn batch(messages: &[String]) -> String {
    format!("[{}]", messages.join(","))
}

/// Batches are refused before `initialize` and under every revision except 2025-03-26.
#[test]
fn batches_are_refused_unless_the_revision_allows_them() {
    let ping = request(json!(1), "ping", Value::Null);

    let (fresh, _) = harness();
    let response = reply(&fresh, &batch(std::slice::from_ref(&ping)));
    error_of(&response, -32600);
    assert_eq!(response["id"], Value::Null);

    for version in ["2024-11-05", "2025-06-18", "2025-11-25", "2026-07-28"] {
        let (server, _) = harness();
        reply(&server, &initialize(1, version));
        error_of(&reply(&server, &batch(std::slice::from_ref(&ping))), -32600);
    }
}

/// Under 2025-03-26 a batch yields one array with a response per request, in order.
#[test]
fn batches_work_under_2025_03_26() {
    let (server, log) = harness();
    reply(&server, &initialize(1, "2025-03-26"));
    silent(&server, &notification("notifications/initialized"));

    let messages = [
        request(json!(10), "ping", Value::Null),
        notification("notifications/cancelled"),
        call(11, "recall", json!({ "q": "batched" })),
        request(json!("s"), "tools/list", Value::Null),
        request(json!(12), "nope", Value::Null),
        "17".to_owned(),
        r#"{"jsonrpc":"2.0","id":13,"method":"tools/call","params":{"name":"forget"}}"#.to_owned(),
    ];
    let response = reply(&server, &batch(&messages));
    let items = response.as_array().unwrap();
    assert_eq!(items.len(), 6, "the notification has no response");
    assert_eq!(result_of(&items[0], &json!(10)), &json!({}));
    assert_eq!(
        tool_text(result_of(&items[1], &json!(11))),
        "recall:batched:None"
    );
    assert_eq!(
        result_of(&items[2], &json!("s"))["tools"]
            .as_array()
            .unwrap()
            .len(),
        pn_ultramemory_mcp::TOOL_NAMES.len()
    );
    error_of(&items[3], -32601);
    error_of(&items[4], -32600);
    assert_eq!(items[4]["id"], Value::Null);
    error_of(&items[5], -32602);
    assert_eq!(log.calls().len(), 1);
}

/// Batch edge cases: all notifications, empty, oversized, nested and containing `initialize`.
#[test]
fn batch_edge_cases_under_2025_03_26() {
    let (server, _) = harness();
    reply(&server, &initialize(1, "2025-03-26"));

    silent(&server, &batch(&[notification("a"), notification("b")]));

    for text in ["[]", "[[]]", "[[1]]"] {
        let response = reply(&server, text);
        if text == "[]" {
            error_of(&response, -32600);
        } else {
            let items = response.as_array().unwrap();
            assert_eq!(items.len(), 1, "{text}");
            error_of(&items[0], -32600);
        }
    }

    let too_many = batch(&vec![request(json!(1), "ping", Value::Null); 257]);
    let response = reply(&server, &too_many);
    error_of(&response, -32600);
    assert!(response.get("id").is_some());

    let at_limit = batch(&vec![request(json!(1), "ping", Value::Null); 256]);
    assert_eq!(reply(&server, &at_limit).as_array().unwrap().len(), 256);

    let with_init = batch(&[
        initialize(5, "2025-11-25"),
        request(json!(6), "ping", Value::Null),
    ]);
    let response = reply(&server, &with_init);
    let items = response.as_array().unwrap();
    error_of(&items[0], -32600);
    assert_eq!(items[0]["id"], json!(5));
    assert_eq!(result_of(&items[1], &json!(6)), &json!({}));
    let ping = reply(&server, &request(json!(7), "ping", Value::Null));
    assert_eq!(result_of(&ping, &json!(7)), &json!({}));
    let after = reply(&server, &request(json!(8), "tools/list", Value::Null));
    assert_eq!(
        result_of(&after, &json!(8))["tools"]
            .as_array()
            .unwrap()
            .len(),
        pn_ultramemory_mcp::TOOL_NAMES.len()
    );
}

/// A panicking backend yields `-32603` for that call and the session keeps working.
#[test]
fn a_panicking_backend_does_not_end_the_session() {
    let (server, _) = harness();
    reply(&server, &initialize(1, "2025-11-25"));
    let response = reply(&server, &call(2, "recall", json!({ "q": "PANIC now" })));
    error_of(&response, -32603);
    assert_eq!(response["id"], json!(2));
    let after = reply(&server, &call(3, "recall", json!({ "q": "fine" })));
    assert_eq!(tool_text(result_of(&after, &json!(3))), "recall:fine:None");
}

/// `explain` and the line window travel from the wire to the backend in a legacy session.
#[test]
fn explain_and_line_window_reach_the_backend() {
    let (server, log) = harness();
    reply(&server, &initialize(1, "2025-11-25"));

    let explained = reply(
        &server,
        &call(2, "recall", json!({ "q": "parser", "explain": true })),
    );
    let result = result_of(&explained, &json!(2));
    assert_eq!(tool_text(result), "recall:parser:None:explain=true");
    assert_eq!(result["isError"], json!(false));

    let window = reply(
        &server,
        &call(3, "expand", json!({ "id": "n7", "from": 10, "to": 20.0 })),
    );
    let result = result_of(&window, &json!(3));
    assert_eq!(tool_text(result), "expand:n7:Some(10)..Some(20)");
    assert_eq!(result["isError"], json!(false));

    let plain = reply(
        &server,
        &call(4, "expand", json!({ "id": "n7", "from": null })),
    );
    assert_eq!(tool_text(result_of(&plain, &json!(4))), "expand:n7");

    assert_eq!(
        log.calls(),
        [
            Call::Recall(RecallRequest {
                query: "parser".into(),
                budget: None,
                explain: Some(true),
            }),
            Call::Expand(ExpandRequest {
                id: "n7".into(),
                from: Some(10),
                to: Some(20),
            }),
            Call::Expand(ExpandRequest {
                id: "n7".into(),
                from: None,
                to: None,
            }),
        ]
    );
}

/// Invalid `explain` or line windows are tool errors, not protocol errors, and are not forwarded.
#[test]
fn invalid_explain_and_line_window_are_tool_errors() {
    let (server, log) = harness();
    reply(&server, &initialize(1, "2025-06-18"));
    let cases = [
        (
            "recall",
            json!({ "q": "x", "explain": "yes" }),
            "`explain` must be a boolean",
        ),
        (
            "expand",
            json!({ "id": "n", "from": 0 }),
            "`from` must be an integer from 1 to",
        ),
        (
            "expand",
            json!({ "id": "n", "from": -3 }),
            "`from` must be an integer from 1 to",
        ),
        (
            "expand",
            json!({ "id": "n", "to": 0 }),
            "`to` must be an integer from 1 to",
        ),
        (
            "expand",
            json!({ "id": "n", "from": 9, "to": 3 }),
            "must not be less than `from`",
        ),
        (
            "expand",
            json!({ "id": "n", "from": "1" }),
            "`from` must be an integer, not a string",
        ),
        (
            "expand",
            json!({ "id": "n", "from": 2.5 }),
            "`from` must be an integer from 1 to",
        ),
    ];
    for (offset, (tool, arguments, fragment)) in (10_i64..).zip(cases) {
        let id = json!(offset);
        let response = reply(&server, &call(offset, tool, arguments));
        let result = result_of(&response, &id);
        assert_eq!(result["isError"], json!(true), "{fragment}");
        assert!(
            tool_text(result).contains(fragment),
            "{}",
            tool_text(result)
        );
    }
    assert!(log.calls().is_empty());
}
