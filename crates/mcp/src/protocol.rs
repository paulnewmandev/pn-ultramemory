// SPDX-License-Identifier: Apache-2.0
//! JSON-RPC 2.0 envelope handling and the protocol constants of the MCP server.
//!
//! # Role in the architecture
//! This is the lowest layer of the entry adapter. It knows what a message looks like on the wire
//! (requests, notifications, responses, error objects), which protocol revisions exist and how a
//! request announces the revision it speaks. It knows nothing about tools, sessions or I/O; the
//! `server` module builds those on top of it.
//!
//! # Invariants
//! * Classification is total: every [`serde_json::Value`] maps to exactly one [`Incoming`] variant
//!   and no input can make it panic.
//! * A response id is either a copy of a valid request id (a string or an integer) or `null`.
//!   `null` is used exactly when the request id could not be read, as JSON-RPC 2.0 section 5
//!   requires.
//! * Notifications and stray responses never produce output.

use serde_json::{Map, Value, json};

/// Newest protocol revision this server implements. It is stateless: no `initialize` handshake,
/// every request carries its version and capabilities in `_meta`.
pub const LATEST_PROTOCOL_VERSION: &str = "2026-07-28";

/// Every protocol revision the server accepts, newest first.
///
/// The first entry is served with per-request metadata; the rest are served through the
/// `initialize` handshake.
pub const SUPPORTED_PROTOCOL_VERSIONS: [&str; 5] = [
    "2026-07-28",
    "2025-11-25",
    "2025-06-18",
    "2025-03-26",
    "2024-11-05",
];

/// Revisions that carry the protocol version and client capabilities in every request.
pub(crate) const MODERN_VERSIONS: [&str; 1] = ["2026-07-28"];

/// Revisions that are negotiated once with `initialize`, newest first.
pub(crate) const LEGACY_VERSIONS: [&str; 4] =
    ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// Newest revision that `initialize` can negotiate; the answer to any unknown requested version.
pub(crate) const LATEST_LEGACY_VERSION: &str = "2025-11-25";

/// The only revision that allows JSON-RPC batches (added in 2025-03-26, removed in 2025-06-18).
pub(crate) const BATCH_VERSION: &str = "2025-03-26";

/// `_meta` key that carries the protocol version of a request.
pub(crate) const META_PROTOCOL_VERSION: &str = "io.modelcontextprotocol/protocolVersion";

/// `_meta` key that carries the client capabilities of a request.
pub(crate) const META_CLIENT_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";

/// `_meta` key under which results identify the server.
pub(crate) const META_SERVER_INFO: &str = "io.modelcontextprotocol/serverInfo";

/// JSON-RPC: invalid JSON was received.
pub(crate) const PARSE_ERROR: i64 = -32700;

/// JSON-RPC: the message is not a valid request object.
pub(crate) const INVALID_REQUEST: i64 = -32600;

/// JSON-RPC: the method does not exist or is not available.
pub(crate) const METHOD_NOT_FOUND: i64 = -32601;

/// JSON-RPC: the method parameters are invalid.
pub(crate) const INVALID_PARAMS: i64 = -32602;

/// JSON-RPC: the receiver failed for a reason that is not the sender's fault.
pub(crate) const INTERNAL_ERROR: i64 = -32603;

/// MCP 2026-07-28: the requested protocol version is not supported.
pub(crate) const UNSUPPORTED_PROTOCOL_VERSION: i64 = -32022;

/// A JSON-RPC error object, ready to be placed in an error response.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RpcError {
    /// Numeric error code.
    pub(crate) code: i64,
    /// One-sentence description.
    pub(crate) message: String,
    /// Optional structured details.
    pub(crate) data: Option<Value>,
}

impl RpcError {
    /// Creates an error without extra data.
    pub(crate) fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    /// Creates a `-32602` (invalid params) error.
    pub(crate) fn invalid_params(message: impl Into<String>) -> Self {
        Self::new(INVALID_PARAMS, message)
    }

    /// Creates a `-32601` (method not found) error that names the method, shortened if needed.
    pub(crate) fn method_not_found(method: &str) -> Self {
        Self::new(
            METHOD_NOT_FOUND,
            format!("Method not found: {}", clip(method, 64)),
        )
    }

    /// Creates the `-32022` error that lists the accepted revisions.
    pub(crate) fn unsupported_version(requested: &str) -> Self {
        Self {
            code: UNSUPPORTED_PROTOCOL_VERSION,
            message: "Unsupported protocol version".to_owned(),
            data: Some(json!({
                "supported": SUPPORTED_PROTOCOL_VERSIONS,
                "requested": clip(requested, 64),
            })),
        }
    }

    /// Renders the error as the `error` member of a response.
    pub(crate) fn to_value(&self) -> Value {
        let mut object = Map::new();
        object.insert("code".to_owned(), Value::from(self.code));
        object.insert("message".to_owned(), Value::from(self.message.as_str()));
        if let Some(data) = &self.data {
            object.insert("data".to_owned(), data.clone());
        }
        Value::Object(object)
    }
}

/// Builds a success response for the request `id`.
pub(crate) fn success(id: &Value, result: Value) -> Value {
    let mut response = Map::new();
    response.insert("jsonrpc".to_owned(), Value::from("2.0"));
    response.insert("id".to_owned(), id.clone());
    response.insert("result".to_owned(), result);
    Value::Object(response)
}

/// Builds an error response for the request `id`, which is `null` when it could not be read.
pub(crate) fn failure(id: &Value, error: &RpcError) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": error.to_value() })
}

/// Shortens `text` to at most `max_chars` characters, marking the cut with an ellipsis.
///
/// Used before echoing untrusted text into an error message, so a hostile peer cannot inflate
/// responses. The cut always falls on a character boundary.
pub(crate) fn clip(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let head: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{head}...")
    } else {
        head
    }
}

/// The `params` member of a request, as far as the server can use it.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Params<'a> {
    /// No `params`, or an explicit `null`.
    Absent,
    /// A JSON object, the only structured shape MCP uses.
    Object(&'a Map<String, Value>),
    /// Anything else: an array, a string, a number or a boolean.
    Malformed,
}

/// One inbound message after classification.
#[derive(Debug)]
pub(crate) enum Incoming<'a> {
    /// A request that expects exactly one response.
    Request {
        /// The request id, a string or an integer, copied verbatim.
        id: Value,
        /// The method name.
        method: &'a str,
        /// The parameters.
        params: Params<'a>,
    },
    /// A notification: it never gets a response.
    Notification,
    /// A response object. The server sends no requests, so it is ignored.
    Response,
    /// A message that is not valid JSON-RPC 2.0.
    Invalid {
        /// The id to answer with: the request id when readable, otherwise `null`.
        id: Value,
        /// What is wrong, as one short sentence.
        reason: &'static str,
    },
}

/// Tells whether `id` is a legal request id: a string or an integer (MCP forbids `null`).
fn is_valid_id(id: &Value) -> bool {
    match id {
        Value::String(_) => true,
        Value::Number(number) => number.is_i64() || number.is_u64(),
        _ => false,
    }
}

/// Classifies one JSON value received from the client.
pub(crate) fn classify(message: &Value) -> Incoming<'_> {
    let Value::Object(object) = message else {
        return Incoming::Invalid {
            id: Value::Null,
            reason: "a message must be a JSON object",
        };
    };
    let id = object.get("id");
    let readable_id = id
        .filter(|candidate| is_valid_id(candidate))
        .cloned()
        .unwrap_or(Value::Null);

    let Some(method) = object.get("method") else {
        if object.contains_key("result") || object.contains_key("error") {
            return Incoming::Response;
        }
        return Incoming::Invalid {
            id: readable_id,
            reason: "missing `method`",
        };
    };
    let Some(method) = method.as_str() else {
        return Incoming::Invalid {
            id: readable_id,
            reason: "`method` must be a string",
        };
    };
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Incoming::Invalid {
            id: readable_id,
            reason: "`jsonrpc` must be \"2.0\"",
        };
    }
    match id {
        None => Incoming::Notification,
        Some(candidate) if is_valid_id(candidate) => Incoming::Request {
            id: candidate.clone(),
            method,
            params: match object.get("params") {
                None | Some(Value::Null) => Params::Absent,
                Some(Value::Object(map)) => Params::Object(map),
                Some(_) => Params::Malformed,
            },
        },
        Some(_) => Incoming::Invalid {
            id: Value::Null,
            reason: "`id` must be a string or an integer",
        },
    }
}

/// Returns the `_meta` object of a request when it carries per-request protocol fields.
///
/// A request that has either the protocol version or the client capabilities in its `_meta` is
/// a request of the stateless revision; [`check_modern_meta`] then validates it.
pub(crate) fn modern_meta(params: Params<'_>) -> Option<&Map<String, Value>> {
    let Params::Object(map) = params else {
        return None;
    };
    let Some(Value::Object(meta)) = map.get("_meta") else {
        return None;
    };
    (meta.contains_key(META_PROTOCOL_VERSION) || meta.contains_key(META_CLIENT_CAPABILITIES))
        .then_some(meta)
}

/// Validates the per-request protocol fields of a stateless-revision request.
///
/// The version is checked first, because it decides which fields are required at all.
///
/// # Errors
/// Returns `-32602` when the version is absent or not a string, or the client capabilities are
/// absent or not an object; returns `-32022` (with the accepted revisions) when the version is not
/// a stateless revision this server implements.
pub(crate) fn check_modern_meta(meta: &Map<String, Value>) -> Result<(), RpcError> {
    let version = match meta.get(META_PROTOCOL_VERSION) {
        Some(Value::String(version)) => version.as_str(),
        Some(_) => {
            return Err(RpcError::invalid_params(format!(
                "`_meta` field `{META_PROTOCOL_VERSION}` must be a string"
            )));
        }
        None => {
            return Err(RpcError::invalid_params(format!(
                "`_meta` field `{META_PROTOCOL_VERSION}` is required"
            )));
        }
    };
    if !MODERN_VERSIONS.contains(&version) {
        return Err(RpcError::unsupported_version(version));
    }
    match meta.get(META_CLIENT_CAPABILITIES) {
        Some(Value::Object(_)) => Ok(()),
        Some(_) => Err(RpcError::invalid_params(format!(
            "`_meta` field `{META_CLIENT_CAPABILITIES}` must be an object"
        ))),
        None => Err(RpcError::invalid_params(format!(
            "`_meta` field `{META_CLIENT_CAPABILITIES}` is required"
        ))),
    }
}

/// Unit tests for classification, error objects and per-request metadata validation.
#[cfg(test)]
mod tests {
    use super::*;

    /// Parses a JSON literal used by the tests.
    fn parse(text: &str) -> Value {
        serde_json::from_str(text).unwrap()
    }

    /// The accepted-revision list is exactly the stateless revisions followed by the legacy ones.
    #[test]
    fn supported_versions_are_modern_then_legacy() {
        let joined: Vec<&str> = MODERN_VERSIONS
            .iter()
            .chain(LEGACY_VERSIONS.iter())
            .copied()
            .collect();
        assert_eq!(joined, SUPPORTED_PROTOCOL_VERSIONS);
        assert_eq!(SUPPORTED_PROTOCOL_VERSIONS[0], LATEST_PROTOCOL_VERSION);
        assert_eq!(LEGACY_VERSIONS[0], LATEST_LEGACY_VERSION);
        assert!(LEGACY_VERSIONS.contains(&BATCH_VERSION));
    }

    /// Valid ids of both kinds make a request and are copied verbatim.
    #[test]
    fn classifies_requests_with_string_and_integer_ids() {
        for (text, expected) in [
            (r#"{"jsonrpc":"2.0","id":7,"method":"ping"}"#, json!(7)),
            (
                r#"{"jsonrpc":"2.0","id":"a-1","method":"ping"}"#,
                json!("a-1"),
            ),
            (r#"{"jsonrpc":"2.0","id":-3,"method":"ping"}"#, json!(-3)),
            (
                r#"{"jsonrpc":"2.0","id":18446744073709551615,"method":"ping"}"#,
                json!(u64::MAX),
            ),
            (r#"{"jsonrpc":"2.0","id":"","method":"ping"}"#, json!("")),
        ] {
            let value = parse(text);
            match classify(&value) {
                Incoming::Request { id, method, params } => {
                    assert_eq!(id, expected, "{text}");
                    assert_eq!(method, "ping");
                    assert!(matches!(params, Params::Absent));
                }
                other => panic!("{text}: {other:?}"),
            }
        }
    }

    /// Ids that are not strings or integers make an invalid request answered with a null id.
    #[test]
    fn rejects_illegal_ids() {
        for text in [
            r#"{"jsonrpc":"2.0","id":null,"method":"ping"}"#,
            r#"{"jsonrpc":"2.0","id":1.5,"method":"ping"}"#,
            r#"{"jsonrpc":"2.0","id":1.0,"method":"ping"}"#,
            r#"{"jsonrpc":"2.0","id":true,"method":"ping"}"#,
            r#"{"jsonrpc":"2.0","id":[1],"method":"ping"}"#,
            r#"{"jsonrpc":"2.0","id":{"a":1},"method":"ping"}"#,
            r#"{"jsonrpc":"2.0","id":1e3,"method":"ping"}"#,
        ] {
            let value = parse(text);
            match classify(&value) {
                Incoming::Invalid { id, .. } => assert_eq!(id, Value::Null, "{text}"),
                other => panic!("{text}: {other:?}"),
            }
        }
    }

    /// A message without an id is a notification, whatever its method.
    #[test]
    fn classifies_notifications() {
        for text in [
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            r#"{"jsonrpc":"2.0","method":"anything/at/all","params":{"x":1}}"#,
            r#"{"jsonrpc":"2.0","method":"","params":[1,2]}"#,
        ] {
            assert!(
                matches!(classify(&parse(text)), Incoming::Notification),
                "{text}"
            );
        }
    }

    /// Response objects are recognised and ignored.
    #[test]
    fn classifies_responses() {
        for text in [
            r#"{"jsonrpc":"2.0","id":1,"result":{}}"#,
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":1,"message":"x"}}"#,
            r#"{"result":1}"#,
        ] {
            assert!(
                matches!(classify(&parse(text)), Incoming::Response),
                "{text}"
            );
        }
    }

    /// Structural problems are reported with the id when it is readable.
    #[test]
    fn reports_invalid_messages() {
        let cases = [
            ("[]", Value::Null),
            ("null", Value::Null),
            ("42", Value::Null),
            (r#""text""#, Value::Null),
            ("{}", Value::Null),
            (r#"{"jsonrpc":"2.0","id":5}"#, json!(5)),
            (r#"{"jsonrpc":"2.0","id":"x"}"#, json!("x")),
            (r#"{"jsonrpc":"2.0","id":5,"method":7}"#, json!(5)),
            (r#"{"jsonrpc":"1.0","id":5,"method":"ping"}"#, json!(5)),
            (r#"{"id":5,"method":"ping"}"#, json!(5)),
            (r#"{"jsonrpc":2,"id":5,"method":"ping"}"#, json!(5)),
        ];
        for (text, expected) in cases {
            let value = parse(text);
            match classify(&value) {
                Incoming::Invalid { id, .. } => assert_eq!(id, expected, "{text}"),
                other => panic!("{text}: {other:?}"),
            }
        }
    }

    /// `params` shapes map to the three [`Params`] variants.
    #[test]
    fn classifies_params_shapes() {
        for (text, absent, object) in [
            (r#"{"jsonrpc":"2.0","id":1,"method":"m"}"#, true, false),
            (
                r#"{"jsonrpc":"2.0","id":1,"method":"m","params":null}"#,
                true,
                false,
            ),
            (
                r#"{"jsonrpc":"2.0","id":1,"method":"m","params":{}}"#,
                false,
                true,
            ),
            (
                r#"{"jsonrpc":"2.0","id":1,"method":"m","params":[]}"#,
                false,
                false,
            ),
            (
                r#"{"jsonrpc":"2.0","id":1,"method":"m","params":"x"}"#,
                false,
                false,
            ),
            (
                r#"{"jsonrpc":"2.0","id":1,"method":"m","params":5}"#,
                false,
                false,
            ),
        ] {
            let value = parse(text);
            let Incoming::Request { params, .. } = classify(&value) else {
                panic!("{text}");
            };
            assert_eq!(matches!(params, Params::Absent), absent, "{text}");
            assert_eq!(matches!(params, Params::Object(_)), object, "{text}");
            assert_eq!(
                matches!(params, Params::Malformed),
                !absent && !object,
                "{text}"
            );
        }
    }

    /// Success and failure responses have the JSON-RPC shape and keep a null id as `null`.
    #[test]
    fn builds_responses() {
        assert_eq!(
            success(&json!(1), json!({"a": 1})),
            json!({"jsonrpc": "2.0", "id": 1, "result": {"a": 1}})
        );
        let error = RpcError::new(INVALID_REQUEST, "bad");
        assert_eq!(
            failure(&Value::Null, &error),
            json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32600, "message": "bad"}})
        );
        let with_data = RpcError::unsupported_version("1900-01-01");
        let value = with_data.to_value();
        assert_eq!(value["code"], json!(-32022));
        assert_eq!(value["data"]["requested"], json!("1900-01-01"));
        assert_eq!(
            value["data"]["supported"],
            json!(SUPPORTED_PROTOCOL_VERSIONS)
        );
    }

    /// Clipping keeps short text, cuts long text on a character boundary and never panics.
    #[test]
    fn clips_on_character_boundaries() {
        assert_eq!(clip("abc", 3), "abc");
        assert_eq!(clip("abcd", 3), "abc...");
        assert_eq!(clip("", 3), "");
        assert_eq!(
            clip("\u{1F600}\u{1F600}\u{1F600}", 2),
            "\u{1F600}\u{1F600}..."
        );
        assert_eq!(clip("abc", 0), "...");
    }

    /// Views a JSON value as request parameters.
    fn as_params(value: &Value) -> Params<'_> {
        match value {
            Value::Object(map) => Params::Object(map),
            _ => Params::Malformed,
        }
    }

    /// Only requests whose `_meta` has a per-request protocol key are treated as stateless.
    #[test]
    fn detects_the_stateless_era() {
        let with = parse(&format!(r#"{{"_meta":{{"{META_PROTOCOL_VERSION}":"x"}}}}"#));
        let caps_only = parse(&format!(
            r#"{{"_meta":{{"{META_CLIENT_CAPABILITIES}":{{}}}}}}"#
        ));
        let progress = parse(r#"{"_meta":{"progressToken":1}}"#);
        let scalar_meta = parse(r#"{"_meta":5}"#);
        let plain = parse(r#"{"name":"x"}"#);
        assert!(modern_meta(as_params(&with)).is_some());
        assert!(modern_meta(as_params(&caps_only)).is_some());
        assert!(modern_meta(as_params(&progress)).is_none());
        assert!(modern_meta(as_params(&scalar_meta)).is_none());
        assert!(modern_meta(as_params(&plain)).is_none());
        assert!(modern_meta(Params::Absent).is_none());
        assert!(modern_meta(Params::Malformed).is_none());
    }

    /// Per-request metadata validation table: the version is judged before the capabilities.
    #[test]
    fn validates_modern_meta() {
        let version = META_PROTOCOL_VERSION;
        let caps = META_CLIENT_CAPABILITIES;
        let cases: [(Value, Option<i64>); 9] = [
            (json!({ version: "2026-07-28", caps: {} }), None),
            (json!({ version: "2026-07-28", caps: {"roots": {}} }), None),
            (json!({ version: "2026-07-28" }), Some(-32602)),
            (json!({ version: "2026-07-28", caps: [] }), Some(-32602)),
            (json!({ version: "2026-07-28", caps: null }), Some(-32602)),
            (json!({ version: 5, caps: {} }), Some(-32602)),
            (json!({ caps: {} }), Some(-32602)),
            (json!({ version: "1900-01-01" }), Some(-32022)),
            (json!({ version: "2025-11-25", caps: {} }), Some(-32022)),
        ];
        for (meta, expected) in cases {
            let Value::Object(map) = &meta else {
                panic!("{meta}")
            };
            let got = check_modern_meta(map).err().map(|error| error.code);
            assert_eq!(got, expected, "{meta}");
        }
    }
}
