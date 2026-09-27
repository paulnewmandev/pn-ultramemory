// SPDX-License-Identifier: Apache-2.0
//! Unit tests for the tool list, argument validation and backend containment.
use std::sync::Mutex;

use super::*;

/// One backend call as the recording backend saw it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Seen {
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

/// A backend that records every request and echoes a fixed reply.
struct Recording {
    /// Requests in arrival order.
    seen: Mutex<Vec<Seen>>,
    /// When set, every call fails with this text.
    fail_with: Option<&'static str>,
    /// When set, every call panics.
    panics: bool,
}

impl Recording {
    /// A backend that succeeds.
    fn ok() -> Self {
        Self {
            seen: Mutex::new(Vec::new()),
            fail_with: None,
            panics: false,
        }
    }

    /// Records `seen` and produces the configured reply.
    fn reply(&self, seen: Seen) -> Result<String, ToolFailure> {
        assert!(!self.panics, "backend panic under test");
        self.seen.lock().unwrap().push(seen);
        match self.fail_with {
            Some(text) => Err(ToolFailure(text.to_owned())),
            None => Ok("done".to_owned()),
        }
    }

    /// Everything recorded so far.
    fn calls(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

impl Backend for Recording {
    /// Records the request.
    fn recall(&self, request: RecallRequest) -> Result<String, ToolFailure> {
        self.reply(Seen::Recall(request))
    }
    /// Records the request.
    fn impact(&self, request: ImpactRequest) -> Result<String, ToolFailure> {
        self.reply(Seen::Impact(request))
    }
    /// Records the request.
    fn remember(&self, request: RememberRequest) -> Result<String, ToolFailure> {
        self.reply(Seen::Remember(request))
    }
    /// Records the request.
    fn expand(&self, request: ExpandRequest) -> Result<String, ToolFailure> {
        self.reply(Seen::Expand(request))
    }
    /// Records the request.
    fn outline(&self, request: OutlineRequest) -> Result<String, ToolFailure> {
        self.reply(Seen::Outline(request))
    }
}

/// Calls a tool with arguments given as JSON text.
fn invoke(backend: &Recording, name: &str, arguments: &str) -> Result<Outcome, CallError> {
    let Value::Object(map) = serde_json::from_str::<Value>(arguments).unwrap() else {
        panic!("test arguments must be an object: {arguments}");
    };
    call(backend, name, &map)
}

/// The tool list is the four documented tools in order, each with a required-argument list.
#[test]
fn lists_exactly_four_tools_in_order() {
    let tools = definitions();
    let names: Vec<&str> = tools
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, TOOL_NAMES);
    let required = |name: &str| -> Vec<String> {
        let tool = tools
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == name)
            .unwrap();
        tool["inputSchema"]["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect()
    };
    assert_eq!(required("recall"), ["q"]);
    assert_eq!(required("impact"), ["symbol"]);
    assert_eq!(required("remember"), ["kind", "text"]);
    assert_eq!(required("expand"), ["id"]);
}

/// Descriptions stay terse (at most 20 words) and annotations follow the tool semantics.
#[test]
fn descriptions_are_terse_and_annotations_are_honest() {
    let tools = definitions();
    for tool in tools.as_array().unwrap() {
        let description = tool["description"].as_str().unwrap();
        assert!(
            description.split_whitespace().count() <= 20,
            "{description}"
        );
        assert_eq!(tool["annotations"]["openWorldHint"], json!(false));
        let read_only = tool["annotations"]["readOnlyHint"].as_bool().unwrap();
        let is_remember = tool["name"] == "remember";
        assert_eq!(read_only, !is_remember);
        assert_eq!(
            tool["annotations"]["destructiveHint"] == json!(false),
            is_remember
        );
    }
}

/// The schema enum and the validation list agree on the memory kinds.
#[test]
fn schema_enum_matches_accepted_kinds() {
    let tools = definitions();
    let remember = &tools[2];
    assert_eq!(
        remember["inputSchema"]["properties"]["kind"]["enum"],
        json!(MEMORY_KINDS)
    );
    // The count is asserted so that adding a kind is a deliberate act: the enum is part of the
    // tool schema an agent reads, so it must be updated in the crate documentation too.
    assert_eq!(MEMORY_KINDS.len(), 9);
    assert!(MEMORY_KINDS.contains(&"requirement"));
}

/// Valid calls reach the backend with exactly the mapped arguments.
#[test]
fn maps_arguments_to_requests() {
    let backend = Recording::ok();
    let cases: [(&str, &str, Seen); 9] = [
        (
            "recall",
            r#"{"q":"parser"}"#,
            Seen::Recall(RecallRequest {
                query: "parser".into(),
                budget: None,
                explain: None,
            }),
        ),
        (
            "recall",
            r#"{"q":"parser","budget":1200}"#,
            Seen::Recall(RecallRequest {
                query: "parser".into(),
                budget: Some(1200),
                explain: None,
            }),
        ),
        (
            "recall",
            r#"{"q":" padded ","budget":4000.0}"#,
            Seen::Recall(RecallRequest {
                query: " padded ".into(),
                budget: Some(4000),
                explain: None,
            }),
        ),
        (
            "recall",
            r#"{"q":"x","budget":null}"#,
            Seen::Recall(RecallRequest {
                query: "x".into(),
                budget: None,
                explain: None,
            }),
        ),
        (
            "impact",
            r#"{"symbol":"a::b","depth":0}"#,
            Seen::Impact(ImpactRequest {
                symbol: "a::b".into(),
                depth: Some(0),
            }),
        ),
        (
            "impact",
            r#"{"symbol":"a::b"}"#,
            Seen::Impact(ImpactRequest {
                symbol: "a::b".into(),
                depth: None,
            }),
        ),
        (
            "remember",
            r#"{"kind":"dead_end","text":"tried X"}"#,
            Seen::Remember(RememberRequest {
                kind: "dead_end".into(),
                text: "tried X".into(),
                about: vec![],
            }),
        ),
        (
            "remember",
            r#"{"kind":"fact","text":"t","about":["a","b::c"]}"#,
            Seen::Remember(RememberRequest {
                kind: "fact".into(),
                text: "t".into(),
                about: vec!["a".into(), "b::c".into()],
            }),
        ),
        (
            "expand",
            r#"{"id":"n42"}"#,
            Seen::Expand(ExpandRequest {
                id: "n42".into(),
                from: None,
                to: None,
            }),
        ),
    ];
    for (name, arguments, expected) in cases {
        let outcome = invoke(&backend, name, arguments).unwrap();
        assert_eq!(
            outcome,
            Outcome {
                text: "done".into(),
                is_error: false
            },
            "{arguments}"
        );
        assert_eq!(backend.calls().last(), Some(&expected), "{arguments}");
    }
    assert_eq!(backend.calls().len(), 9);
}

/// Every memory kind is accepted and forwarded verbatim.
#[test]
fn accepts_every_memory_kind() {
    let backend = Recording::ok();
    for kind in MEMORY_KINDS {
        let arguments = json!({ "kind": kind, "text": "t" }).to_string();
        assert!(
            !invoke(&backend, "remember", &arguments).unwrap().is_error,
            "{kind}"
        );
    }
    assert_eq!(backend.calls().len(), MEMORY_KINDS.len());
}

/// Invalid calls as `(tool, arguments, fragment of the expected message)`.
const INVALID_CALLS: [(&str, &str, &str); 44] = [
    ("recall", "{}", "`q` is required"),
    ("recall", r#"{"q":null}"#, "`q` is required"),
    ("recall", r#"{"q":""}"#, "`q` must not be empty"),
    ("recall", r#"{"q":"  \n\t"}"#, "`q` must not be empty"),
    ("recall", r#"{"q":5}"#, "`q` must be a string, not a number"),
    (
        "recall",
        r#"{"q":true}"#,
        "`q` must be a string, not a boolean",
    ),
    (
        "recall",
        r#"{"q":["a"]}"#,
        "`q` must be a string, not an array",
    ),
    (
        "recall",
        r#"{"q":{"a":1}}"#,
        "`q` must be a string, not an object",
    ),
    (
        "recall",
        r#"{"q":"x","budget":"100"}"#,
        "`budget` must be an integer, not a string",
    ),
    (
        "recall",
        r#"{"q":"x","budget":true}"#,
        "`budget` must be an integer, not a boolean",
    ),
    (
        "recall",
        r#"{"q":"x","budget":[1]}"#,
        "`budget` must be an integer, not an array",
    ),
    (
        "recall",
        r#"{"q":"x","budget":-1}"#,
        "`budget` must be an integer from 0 to",
    ),
    (
        "recall",
        r#"{"q":"x","budget":1.5}"#,
        "`budget` must be an integer from 0 to",
    ),
    (
        "recall",
        r#"{"q":"x","budget":-0.5}"#,
        "`budget` must be an integer from 0 to",
    ),
    (
        "recall",
        r#"{"q":"x","budget":4294967296}"#,
        "`budget` must be an integer from 0 to",
    ),
    (
        "recall",
        r#"{"q":"x","budget":1e30}"#,
        "`budget` must be an integer from 0 to",
    ),
    (
        "recall",
        r#"{"q":"x","budget":99999999999999999999999}"#,
        "`budget` must be",
    ),
    (
        "recall",
        r#"{"q":"x","query":"y"}"#,
        "unknown argument `query`; allowed: q, budget",
    ),
    ("recall", r#"{"query":"y"}"#, "unknown argument `query`"),
    ("impact", "{}", "`symbol` is required"),
    ("impact", r#"{"symbol":""}"#, "`symbol` must not be empty"),
    (
        "impact",
        r#"{"symbol":7}"#,
        "`symbol` must be a string, not a number",
    ),
    (
        "impact",
        r#"{"symbol":"a","depth":-3}"#,
        "`depth` must be an integer from 0 to",
    ),
    (
        "impact",
        r#"{"symbol":"a","depth":"2"}"#,
        "`depth` must be an integer, not a string",
    ),
    (
        "impact",
        r#"{"symbol":"a","depth":{}}"#,
        "`depth` must be an integer, not an object",
    ),
    (
        "impact",
        r#"{"symbol":"a","deep":2}"#,
        "unknown argument `deep`",
    ),
    ("remember", "{}", "`kind` is required; one of decision"),
    (
        "remember",
        r#"{"kind":null,"text":"t"}"#,
        "`kind` is required",
    ),
    (
        "remember",
        r#"{"kind":"Decision","text":"t"}"#,
        "`kind` must be one of decision",
    ),
    (
        "remember",
        r#"{"kind":"note","text":"t"}"#,
        "`kind` must be one of decision",
    ),
    (
        "remember",
        r#"{"kind":3,"text":"t"}"#,
        "`kind` must be one of decision",
    ),
    ("remember", r#"{"kind":"fact"}"#, "`text` is required"),
    (
        "remember",
        r#"{"kind":"fact","text":""}"#,
        "`text` must not be empty",
    ),
    (
        "remember",
        r#"{"kind":"fact","text":9}"#,
        "`text` must be a string, not a number",
    ),
    (
        "remember",
        r#"{"kind":"fact","text":"t","about":"a"}"#,
        "`about` must be an array",
    ),
    (
        "remember",
        r#"{"kind":"fact","text":"t","about":[1]}"#,
        "`about[0]` must be a non",
    ),
    (
        "remember",
        r#"{"kind":"fact","text":"t","about":["a",""]}"#,
        "`about[1]` must be",
    ),
    (
        "remember",
        r#"{"kind":"fact","text":"t","about":[null]}"#,
        "`about[0]` must be",
    ),
    (
        "remember",
        r#"{"kind":"fact","text":"t","about":{"a":1}}"#,
        "`about` must be an arr",
    ),
    (
        "remember",
        r#"{"kind":"fact","text":"t","tags":[]}"#,
        "unknown argument `tags`",
    ),
    ("expand", "{}", "`id` is required"),
    ("expand", r#"{"id":""}"#, "`id` must not be empty"),
    (
        "expand",
        r#"{"id":12}"#,
        "`id` must be a string, not a number",
    ),
    (
        "expand",
        r#"{"id":"n1","extra":1}"#,
        "unknown argument `extra`",
    ),
];

/// Argument validation table: each case is a tool error that never reaches the backend.
#[test]
fn rejects_invalid_arguments_without_calling_the_backend() {
    let backend = Recording::ok();
    for (name, arguments, expected) in INVALID_CALLS {
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

/// A backend failure becomes a tool error carrying its text.
#[test]
fn backend_failures_become_tool_errors() {
    let backend = Recording {
        fail_with: Some("index is empty"),
        ..Recording::ok()
    };
    let outcome = invoke(&backend, "recall", r#"{"q":"x"}"#).unwrap();
    assert_eq!(
        outcome,
        Outcome {
            text: "index is empty".into(),
            is_error: true
        }
    );
}

/// Unknown names, including near misses and odd characters, are not tools.
#[test]
fn unknown_tools_are_reported() {
    let backend = Recording::ok();
    for name in [
        "",
        "Recall",
        "recall ",
        "recall\n",
        "forget",
        "tools/list",
        "\u{0}",
        "réca",
    ] {
        assert_eq!(
            invoke(&backend, name, "{}"),
            Err(CallError::UnknownTool),
            "{name:?}"
        );
    }
    assert!(backend.calls().is_empty());
}

/// A panicking backend is contained and reported, not propagated.
#[test]
fn backend_panics_are_contained() {
    let backend = Recording {
        panics: true,
        ..Recording::ok()
    };
    let outcome = invoke(&backend, "expand", r#"{"id":"n1"}"#);
    assert_eq!(outcome, Err(CallError::BackendPanicked));
}

/// Long or odd argument names are clipped in the message and never break validation.
#[test]
fn unknown_argument_names_are_clipped() {
    let backend = Recording::ok();
    let long = "k".repeat(10_000);
    let arguments = json!({ "q": "x", long.as_str(): 1 }).to_string();
    let outcome = invoke(&backend, "recall", &arguments).unwrap();
    assert!(outcome.is_error);
    assert!(outcome.text.len() < 200, "{}", outcome.text.len());
}

/// The failure type prints its text and works as a boxed error.
#[test]
fn tool_failure_is_a_displayable_error() {
    let failure = ToolFailure("boom".to_owned());
    assert_eq!(failure.to_string(), "boom");
    let boxed: Box<dyn Error> = Box::new(failure.clone());
    assert_eq!(boxed.to_string(), "boom");
    assert_eq!(failure.clone(), failure);
}

/// Unit tests for the optional `explain`, `from` and `to` arguments.
mod optional;
