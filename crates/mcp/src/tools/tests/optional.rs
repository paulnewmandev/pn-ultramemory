// SPDX-License-Identifier: Apache-2.0
//! Unit tests for the optional `recall.explain` and `expand.from` / `expand.to` arguments:
//! mapping to the request structs, boundary values and rejection of invalid ones.

use super::*;

/// Builds the expected `recall` request for the `explain` tests.
fn recall_seen(query: &str, budget: Option<u32>, explain: Option<bool>) -> Seen {
    Seen::Recall(RecallRequest {
        query: query.into(),
        budget,
        explain,
    })
}

/// Builds the expected `expand` request for the line-window tests.
fn expand_seen(id: &str, from: Option<u32>, to: Option<u32>) -> Seen {
    Seen::Expand(ExpandRequest {
        id: id.into(),
        from,
        to,
    })
}

/// Valid `explain` values reach the backend; `null` and absence both mean "not given".
#[test]
fn maps_the_explain_flag() {
    let backend = Recording::ok();
    let cases: [(&str, Seen); 6] = [
        (
            r#"{"q":"x","explain":true}"#,
            recall_seen("x", None, Some(true)),
        ),
        (
            r#"{"q":"x","explain":false}"#,
            recall_seen("x", None, Some(false)),
        ),
        (r#"{"q":"x","explain":null}"#, recall_seen("x", None, None)),
        (r#"{"q":"x"}"#, recall_seen("x", None, None)),
        (
            r#"{"q":"x","budget":50,"explain":true}"#,
            recall_seen("x", Some(50), Some(true)),
        ),
        (
            r#"{"explain":true,"budget":null,"q":"x"}"#,
            recall_seen("x", None, Some(true)),
        ),
    ];
    for (arguments, expected) in cases {
        let outcome = invoke(&backend, "recall", arguments).unwrap();
        assert!(!outcome.is_error, "{arguments}: {}", outcome.text);
        assert_eq!(backend.calls().last(), Some(&expected), "{arguments}");
    }
    assert_eq!(backend.calls().len(), 6);
}

/// Valid line windows reach the backend, including the boundaries, floats and nulls.
#[test]
fn maps_the_line_window() {
    let backend = Recording::ok();
    let max = u32::MAX;
    let cases: [(&str, Seen); 14] = [
        (r#"{"id":"n"}"#, expand_seen("n", None, None)),
        (r#"{"id":"n","from":1}"#, expand_seen("n", Some(1), None)),
        (r#"{"id":"n","to":1}"#, expand_seen("n", None, Some(1))),
        (
            r#"{"id":"n","from":1,"to":1}"#,
            expand_seen("n", Some(1), Some(1)),
        ),
        (
            r#"{"id":"n","from":2,"to":2}"#,
            expand_seen("n", Some(2), Some(2)),
        ),
        (
            r#"{"id":"n","from":10,"to":250}"#,
            expand_seen("n", Some(10), Some(250)),
        ),
        (r#"{"id":"n","from":5.0}"#, expand_seen("n", Some(5), None)),
        (r#"{"id":"n","to":9.0}"#, expand_seen("n", None, Some(9))),
        (
            r#"{"id":"n","from":5.0,"to":5.0}"#,
            expand_seen("n", Some(5), Some(5)),
        ),
        (
            r#"{"id":"n","from":null,"to":null}"#,
            expand_seen("n", None, None),
        ),
        (
            r#"{"id":"n","from":3,"to":null}"#,
            expand_seen("n", Some(3), None),
        ),
        (
            r#"{"id":"n","from":null,"to":7}"#,
            expand_seen("n", None, Some(7)),
        ),
        (
            r#"{"id":"n","from":4294967295,"to":4294967295}"#,
            expand_seen("n", Some(max), Some(max)),
        ),
        (
            r#"{"id":"n","from":1,"to":4294967295.0}"#,
            expand_seen("n", Some(1), Some(max)),
        ),
    ];
    for (arguments, expected) in cases {
        let outcome = invoke(&backend, "expand", arguments).unwrap();
        assert!(!outcome.is_error, "{arguments}: {}", outcome.text);
        assert_eq!(backend.calls().last(), Some(&expected), "{arguments}");
    }
    assert_eq!(backend.calls().len(), 14);
}

/// Invalid `explain` and line-window arguments as `(tool, arguments, message fragment)`.
const INVALID_NEW_ARGUMENTS: [(&str, &str, &str); 38] = [
    (
        "recall",
        r#"{"q":"x","explain":"true"}"#,
        "`explain` must be a boolean, not a string",
    ),
    (
        "recall",
        r#"{"q":"x","explain":"yes"}"#,
        "`explain` must be a boolean, not a string",
    ),
    (
        "recall",
        r#"{"q":"x","explain":1}"#,
        "`explain` must be a boolean, not a number",
    ),
    (
        "recall",
        r#"{"q":"x","explain":0}"#,
        "`explain` must be a boolean, not a number",
    ),
    (
        "recall",
        r#"{"q":"x","explain":[true]}"#,
        "`explain` must be a boolean, not an array",
    ),
    (
        "recall",
        r#"{"q":"x","explain":{}}"#,
        "`explain` must be a boolean, not an object",
    ),
    (
        "recall",
        r#"{"q":"x","explains":true}"#,
        "unknown argument `explains`; allowed: q, budget, explain",
    ),
    (
        "expand",
        r#"{"id":"n","from":0}"#,
        "`from` must be an integer from 1 to",
    ),
    (
        "expand",
        r#"{"id":"n","from":0.0}"#,
        "`from` must be an integer from 1 to",
    ),
    (
        "expand",
        r#"{"id":"n","from":-0.0}"#,
        "`from` must be an integer from 1 to",
    ),
    (
        "expand",
        r#"{"id":"n","from":-1}"#,
        "`from` must be an integer from 1 to",
    ),
    (
        "expand",
        r#"{"id":"n","from":-5.0}"#,
        "`from` must be an integer from 1 to",
    ),
    (
        "expand",
        r#"{"id":"n","from":1.5}"#,
        "`from` must be an integer from 1 to",
    ),
    (
        "expand",
        r#"{"id":"n","from":4294967296}"#,
        "`from` must be an integer from 1 to",
    ),
    (
        "expand",
        r#"{"id":"n","from":1e30}"#,
        "`from` must be an integer from 1 to",
    ),
    (
        "expand",
        r#"{"id":"n","from":99999999999999999999999}"#,
        "`from` must be an integer from 1 to",
    ),
    (
        "expand",
        r#"{"id":"n","from":"5"}"#,
        "`from` must be an integer, not a string",
    ),
    (
        "expand",
        r#"{"id":"n","from":true}"#,
        "`from` must be an integer, not a boolean",
    ),
    (
        "expand",
        r#"{"id":"n","from":[1]}"#,
        "`from` must be an integer, not an array",
    ),
    (
        "expand",
        r#"{"id":"n","from":{"a":1}}"#,
        "`from` must be an integer, not an object",
    ),
    (
        "expand",
        r#"{"id":"n","to":0}"#,
        "`to` must be an integer from 1 to",
    ),
    (
        "expand",
        r#"{"id":"n","to":-1}"#,
        "`to` must be an integer from 1 to",
    ),
    (
        "expand",
        r#"{"id":"n","to":2.5}"#,
        "`to` must be an integer from 1 to",
    ),
    (
        "expand",
        r#"{"id":"n","to":4294967296}"#,
        "`to` must be an integer from 1 to",
    ),
    (
        "expand",
        r#"{"id":"n","to":"9"}"#,
        "`to` must be an integer, not a string",
    ),
    (
        "expand",
        r#"{"id":"n","to":false}"#,
        "`to` must be an integer, not a boolean",
    ),
    (
        "expand",
        r#"{"id":"n","to":[]}"#,
        "`to` must be an integer, not an array",
    ),
    (
        "expand",
        r#"{"id":"n","to":{}}"#,
        "`to` must be an integer, not an object",
    ),
    (
        "expand",
        r#"{"id":"n","from":5,"to":4}"#,
        "`to` (4) must not be less than `from` (5)",
    ),
    (
        "expand",
        r#"{"id":"n","from":2,"to":1}"#,
        "`to` (1) must not be less than `from` (2)",
    ),
    (
        "expand",
        r#"{"id":"n","from":5.0,"to":4.0}"#,
        "`to` (4) must not be less than `from` (5)",
    ),
    (
        "expand",
        r#"{"id":"n","from":4294967295,"to":1}"#,
        "must not be less than `from`",
    ),
    (
        "expand",
        r#"{"id":"n","from":0,"to":5}"#,
        "`from` must be an integer from 1 to",
    ),
    (
        "expand",
        r#"{"id":"n","from":3,"to":0}"#,
        "`to` must be an integer from 1 to",
    ),
    (
        "expand",
        r#"{"id":"n","from":"a","to":"b"}"#,
        "`from` must be an integer, not a string",
    ),
    ("expand", r#"{"from":1,"to":2}"#, "`id` is required"),
    (
        "expand",
        r#"{"id":"n","line":3}"#,
        "unknown argument `line`; allowed: id, from, to",
    ),
    (
        "expand",
        r#"{"id":"n","lines":"1-5"}"#,
        "unknown argument `lines`",
    ),
];

/// Every invalid new argument is a tool error that never reaches the backend.
#[test]
fn rejects_invalid_explain_and_window_arguments() {
    let backend = Recording::ok();
    for (name, arguments, expected) in INVALID_NEW_ARGUMENTS {
        let outcome = invoke(&backend, name, arguments).unwrap();
        assert!(outcome.is_error, "{name} {arguments}");
        assert!(
            outcome.text.contains(expected),
            "{name} {arguments}: {}",
            outcome.text
        );
    }
    assert!(
        backend.calls().is_empty(),
        "invalid calls reached the backend"
    );
}

/// The schema lists the new properties with the right types and a few words each.
#[test]
fn schema_declares_the_new_properties() {
    let tools = definitions();
    let properties = |name: &str| -> Value {
        let tool = tools
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap();
        tool["inputSchema"]["properties"].clone()
    };
    let recall = properties("recall");
    let names: Vec<&str> = recall
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(names, ["q", "budget", "explain"]);
    assert_eq!(recall["explain"]["type"], json!("boolean"));
    let expand = properties("expand");
    let names: Vec<&str> = expand
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(names, ["id", "from", "to"]);
    assert_eq!(expand["from"]["type"], json!("integer"));
    assert_eq!(expand["to"]["type"], json!("integer"));
    for property in [&recall["explain"], &expand["from"], &expand["to"]] {
        let words = property["description"]
            .as_str()
            .unwrap()
            .split_whitespace()
            .count();
        assert!(words <= 5, "{property}");
    }
    for tool in tools.as_array().unwrap() {
        let required = tool["inputSchema"]["required"].as_array().unwrap();
        assert!(
            required
                .iter()
                .all(|key| !["explain", "from", "to"].contains(&key.as_str().unwrap()))
        );
    }
}
