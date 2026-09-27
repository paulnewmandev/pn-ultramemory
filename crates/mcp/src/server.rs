// SPDX-License-Identifier: Apache-2.0
//! The [`Server`]: request dispatch for both protocol eras, batching and the stdio loop.
//!
//! # Role in the architecture
//! This module ties the other three together. [`crate::protocol`] classifies a message,
//! [`crate::tools`] validates and runs a tool call, [`crate::framing`] cuts the byte stream into
//! lines, and the [`Server`] decides what each request means in the era it belongs to.
//!
//! # Invariants
//! * [`Server::handle_line`] is total: any text yields `None` or one single-line JSON value.
//! * Session state is one negotiated revision behind a mutex. The lock is taken for a moment to
//!   read or write it and is **never held while a backend runs**, and a poisoned lock is
//!   recovered, so a misbehaving backend cannot wedge the server.
//! * The stateless revision (2026-07-28) never reads or writes session state: every such
//!   request stands alone, as the specification requires.
//! * Nothing is written to any stream except protocol messages.

use std::io::{self, BufRead, Write};
use std::sync::{Mutex, PoisonError};

use serde_json::{Map, Value, json};

use crate::framing::{Frame, read_frame};
use crate::protocol::{
    BATCH_VERSION, INTERNAL_ERROR, INVALID_REQUEST, Incoming, LATEST_LEGACY_VERSION,
    LEGACY_VERSIONS, META_SERVER_INFO, PARSE_ERROR, Params, RpcError, SUPPORTED_PROTOCOL_VERSIONS,
    check_modern_meta, classify, clip, failure, modern_meta, success,
};
use crate::tools::{self, Backend, CallError};

/// Longest line the server accepts, in bytes (4 MiB). Longer lines are refused with an error and
/// skipped without being buffered.
pub const MAX_LINE_BYTES: usize = 4 * 1024 * 1024;

/// Most messages accepted in one JSON-RPC batch; larger batches are refused whole.
const MAX_BATCH_LEN: usize = 256;

/// How long clients may cache the discovery and tool-list results, in milliseconds (one hour).
const CACHE_TTL_MS: u64 = 3_600_000;

/// What the server reports about itself in `serverInfo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerInfo {
    /// Programmatic name of the server, for example `pn-ultramemory`.
    pub name: String,
    /// Version of the server software.
    pub version: String,
}

/// Which protocol era a request is served under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Era {
    /// Negotiated once by `initialize`; results carry no `resultType`.
    Legacy,
    /// Stateless: per-request `_meta`; results carry `resultType` and, where cacheable, hints.
    Modern,
}

/// A Model Context Protocol server: four tools over newline-delimited JSON-RPC.
///
/// See the [crate documentation](crate) for the protocol revisions it speaks. A server is cheap
/// to share: all its methods take `&self`.
///
/// # Examples
/// ```
/// use pn_ultramemory_mcp::{
///     Backend, ExpandRequest, ImpactRequest, OutlineRequest, RecallRequest, RememberRequest,
///     Server, ServerInfo,
///     ToolFailure,
/// };
///
/// struct Demo;
///
/// impl Backend for Demo {
///     fn recall(&self, request: RecallRequest) -> Result<String, ToolFailure> {
///         Ok(format!("capsule for {}", request.query))
///     }
///     fn impact(&self, _: ImpactRequest) -> Result<String, ToolFailure> {
///         Err(ToolFailure("no index yet".to_owned()))
///     }
///     fn remember(&self, _: RememberRequest) -> Result<String, ToolFailure> {
///         Ok("stored".to_owned())
///     }
///     fn outline(&self, _: OutlineRequest) -> Result<String, ToolFailure> {
///         Ok("file:\n  path: src/lib.rs".into())
///     }
///     fn expand(&self, _: ExpandRequest) -> Result<String, ToolFailure> {
///         Ok("fn main() {}".to_owned())
///     }
/// }
///
/// let server = Server::new(
///     Demo,
///     ServerInfo { name: "demo".to_owned(), version: "0.1.0".to_owned() },
/// );
/// let call = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{
///     "name":"recall","arguments":{"q":"parser"},
///     "_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28",
///              "io.modelcontextprotocol/clientCapabilities":{}}}}"#
///     .replace('\n', "");
/// let reply = server.handle_line(&call).unwrap();
/// assert!(reply.contains(r#""text":"capsule for parser""#));
/// assert!(reply.contains(r#""isError":false"#));
/// ```
pub struct Server<B: Backend> {
    /// The implementation of the four tools.
    backend: B,
    /// Identity reported to clients.
    info: ServerInfo,
    /// Revision negotiated by `initialize`, if a legacy client has connected.
    negotiated: Mutex<Option<&'static str>>,
}

impl<B: Backend> Server<B> {
    /// Creates a server that answers tool calls with `backend` and identifies itself as `info`.
    pub fn new(backend: B, info: ServerInfo) -> Self {
        Self {
            backend,
            info,
            negotiated: Mutex::new(None),
        }
    }

    /// Handles one line of input and returns the line to write back, if any.
    ///
    /// This is the whole protocol as a pure function of the line and the session state: requests
    /// yield a response, notifications and blank lines yield `None`. The returned text is compact
    /// JSON without a newline, so a caller only has to append one. A line may be a single message
    /// or, when the negotiated revision is 2025-03-26, a JSON-RPC batch.
    ///
    /// This method never panics, whatever the text.
    ///
    /// # Examples
    /// ```
    /// use pn_ultramemory_mcp::{
    ///     Backend, ExpandRequest, ImpactRequest, OutlineRequest, RecallRequest, RememberRequest, Server,
    ///     ServerInfo, ToolFailure,
    /// };
    ///
    /// struct Empty;
    ///
    /// impl Backend for Empty {
    ///     fn recall(&self, _: RecallRequest) -> Result<String, ToolFailure> {
    ///         Ok(String::new())
    ///     }
    ///     fn impact(&self, _: ImpactRequest) -> Result<String, ToolFailure> {
    ///         Ok(String::new())
    ///     }
    ///     fn remember(&self, _: RememberRequest) -> Result<String, ToolFailure> {
    ///         Ok(String::new())
    ///     }
    ///     fn outline(&self, _: OutlineRequest) -> Result<String, ToolFailure> {
    ///         Ok("file:\n  path: src/lib.rs".into())
    ///     }
    ///     fn expand(&self, _: ExpandRequest) -> Result<String, ToolFailure> {
    ///         Ok(String::new())
    ///     }
    /// }
    ///
    /// let server = Server::new(
    ///     Empty,
    ///     ServerInfo { name: "demo".to_owned(), version: "0.1.0".to_owned() },
    /// );
    /// let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#;
    /// let reply = server.handle_line(init).unwrap();
    /// assert!(reply.contains(r#""protocolVersion":"2025-06-18""#));
    /// // Notifications and blank lines produce no output.
    /// assert_eq!(server.handle_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#), None);
    /// assert_eq!(server.handle_line("   "), None);
    /// // Garbage produces a parse error with a null id.
    /// let error = server.handle_line("{nope").unwrap();
    /// assert!(error.contains(r#""code":-32700"#) && error.contains(r#""id":null"#));
    /// ```
    pub fn handle_line(&self, line: &str) -> Option<String> {
        let text = line.trim();
        if text.is_empty() {
            return None;
        }
        let response = match serde_json::from_str::<Value>(text) {
            Ok(message) => self.handle_value(&message)?,
            Err(error) => failure(
                &Value::Null,
                &RpcError::new(PARSE_ERROR, format!("Parse error: {error}")),
            ),
        };
        Some(encode(&response))
    }

    /// Reads newline-delimited messages from `input` until end of input, writing each response to
    /// `output` as one line and flushing after every one.
    ///
    /// Lines longer than [`MAX_LINE_BYTES`] are refused with a `-32600` error and skipped without
    /// being buffered, so a hostile peer cannot exhaust memory and the stream stays in sync.
    /// Lines that are not valid UTF-8 are refused with a `-32700` error. Blank lines are ignored
    /// and a trailing `\r` is stripped. The loop ends cleanly (`Ok`) at end of input.
    ///
    /// Only protocol messages are written to `output`; diagnostics are the caller's business and
    /// belong on standard error.
    ///
    /// # Errors
    /// Returns the first I/O error from reading `input` or writing and flushing `output`, for
    /// example a broken pipe when the client has gone away.
    ///
    /// # Examples
    /// ```
    /// use pn_ultramemory_mcp::{
    ///     Backend, ExpandRequest, ImpactRequest, OutlineRequest, RecallRequest, RememberRequest, Server,
    ///     ServerInfo, ToolFailure,
    /// };
    ///
    /// struct Empty;
    ///
    /// impl Backend for Empty {
    ///     fn recall(&self, _: RecallRequest) -> Result<String, ToolFailure> {
    ///         Ok(String::new())
    ///     }
    ///     fn impact(&self, _: ImpactRequest) -> Result<String, ToolFailure> {
    ///         Ok(String::new())
    ///     }
    ///     fn remember(&self, _: RememberRequest) -> Result<String, ToolFailure> {
    ///         Ok(String::new())
    ///     }
    ///     fn outline(&self, _: OutlineRequest) -> Result<String, ToolFailure> {
    ///         Ok("file:\n  path: src/lib.rs".into())
    ///     }
    ///     fn expand(&self, _: ExpandRequest) -> Result<String, ToolFailure> {
    ///         Ok(String::new())
    ///     }
    /// }
    ///
    /// let server = Server::new(
    ///     Empty,
    ///     ServerInfo { name: "demo".to_owned(), version: "0.1.0".to_owned() },
    /// );
    /// let input = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n\n";
    /// let mut output = Vec::new();
    /// server.serve(input.as_bytes(), &mut output).unwrap();
    /// assert_eq!(String::from_utf8(output).unwrap(), "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n");
    /// ```
    pub fn serve(&self, mut input: impl BufRead, mut output: impl Write) -> io::Result<()> {
        let mut buffer = Vec::new();
        loop {
            let response = match read_frame(&mut input, MAX_LINE_BYTES, &mut buffer)? {
                Frame::Eof => return Ok(()),
                Frame::TooLong => Some(encode(&failure(
                    &Value::Null,
                    &RpcError::new(
                        INVALID_REQUEST,
                        format!("Message exceeds the {MAX_LINE_BYTES} byte limit"),
                    ),
                ))),
                Frame::Line => {
                    if buffer.last() == Some(&b'\r') {
                        buffer.pop();
                    }
                    match std::str::from_utf8(&buffer) {
                        Ok(line) => self.handle_line(line),
                        Err(_) => Some(encode(&failure(
                            &Value::Null,
                            &RpcError::new(PARSE_ERROR, "Parse error: input is not valid UTF-8"),
                        ))),
                    }
                }
            };
            buffer.shrink_to(64 * 1024);
            if let Some(mut text) = response {
                text.push('\n');
                output.write_all(text.as_bytes())?;
                output.flush()?;
            }
        }
    }

    /// Returns the revision negotiated by `initialize`, if any.
    fn negotiated(&self) -> Option<&'static str> {
        *self
            .negotiated
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Handles one parsed JSON value: a single message or a batch.
    fn handle_value(&self, value: &Value) -> Option<Value> {
        match value {
            Value::Array(items) => self.handle_batch(items),
            single => self.handle_message(single, false),
        }
    }

    /// Handles a JSON-RPC batch, which only the 2025-03-26 revision allows.
    fn handle_batch(&self, items: &[Value]) -> Option<Value> {
        let refuse = |reason: String| {
            Some(failure(
                &Value::Null,
                &RpcError::new(INVALID_REQUEST, reason),
            ))
        };
        if self.negotiated() != Some(BATCH_VERSION) {
            return refuse(
                "JSON-RPC batches are only allowed under protocol version 2025-03-26".to_owned(),
            );
        }
        if items.is_empty() {
            return refuse("A batch must contain at least one message".to_owned());
        }
        if items.len() > MAX_BATCH_LEN {
            return refuse(format!(
                "A batch may contain at most {MAX_BATCH_LEN} messages"
            ));
        }
        let responses: Vec<Value> = items
            .iter()
            .filter_map(|item| self.handle_message(item, true))
            .collect();
        (!responses.is_empty()).then_some(Value::Array(responses))
    }

    /// Handles one message and returns its response, if it gets one.
    fn handle_message(&self, message: &Value, in_batch: bool) -> Option<Value> {
        match classify(message) {
            Incoming::Notification | Incoming::Response => None,
            Incoming::Invalid { id, reason } => {
                Some(failure(&id, &RpcError::new(INVALID_REQUEST, reason)))
            }
            Incoming::Request { id, method, params } => {
                Some(match self.dispatch(method, params, in_batch) {
                    Ok(result) => success(&id, result),
                    Err(error) => failure(&id, &error),
                })
            }
        }
    }

    /// Routes a request to its handler according to the era it belongs to.
    fn dispatch(
        &self,
        method: &str,
        params: Params<'_>,
        in_batch: bool,
    ) -> Result<Value, RpcError> {
        if method == "initialize" {
            if in_batch {
                return Err(RpcError::new(
                    INVALID_REQUEST,
                    "`initialize` must not be part of a batch",
                ));
            }
            return self.initialize(params);
        }
        if let Some(meta) = modern_meta(params) {
            check_modern_meta(meta)?;
            return match method {
                "server/discover" => Ok(self.stamp(Era::Modern, Self::discovery(), true)),
                "tools/list" => self.list_tools(Era::Modern, params),
                "tools/call" => self.call_tool(Era::Modern, params),
                other => Err(RpcError::method_not_found(other)),
            };
        }
        match method {
            "ping" => Ok(json!({})),
            "server/discover" => Err(RpcError::invalid_params(
                "`server/discover` needs the per-request `_meta` protocol fields",
            )),
            "tools/list" | "tools/call" if self.negotiated().is_none() => {
                Err(RpcError::invalid_params(
                    "Not initialized: send `initialize` first, or include the per-request \
                     `_meta` protocol fields",
                ))
            }
            "tools/list" => self.list_tools(Era::Legacy, params),
            "tools/call" => self.call_tool(Era::Legacy, params),
            other => Err(RpcError::method_not_found(other)),
        }
    }

    /// Answers `initialize`: negotiates a legacy revision and records it for the session.
    ///
    /// The requested revision is echoed when it is a legacy one; anything else (an unknown
    /// revision, or the stateless one, which has no handshake) gets the newest legacy revision.
    fn initialize(&self, params: Params<'_>) -> Result<Value, RpcError> {
        let requested = match params {
            Params::Object(map) => match map.get("protocolVersion") {
                Some(Value::String(version)) => version.as_str(),
                _ => {
                    return Err(RpcError::invalid_params(
                        "`protocolVersion` must be a string",
                    ));
                }
            },
            _ => {
                return Err(RpcError::invalid_params(
                    "`initialize` needs params with `protocolVersion`",
                ));
            }
        };
        let agreed = LEGACY_VERSIONS
            .iter()
            .copied()
            .find(|version| *version == requested)
            .unwrap_or(LATEST_LEGACY_VERSION);
        *self
            .negotiated
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(agreed);
        Ok(json!({
            "protocolVersion": agreed,
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": self.identity(),
        }))
    }

    /// Builds the body of a `server/discover` result.
    fn discovery() -> Value {
        json!({
            "supportedVersions": SUPPORTED_PROTOCOL_VERSIONS,
            "capabilities": { "tools": { "listChanged": false } },
        })
    }

    /// Answers `tools/list`; the list is fixed, so no cursor is ever valid.
    fn list_tools(&self, era: Era, params: Params<'_>) -> Result<Value, RpcError> {
        match params {
            Params::Malformed => {
                return Err(RpcError::invalid_params("`params` must be an object"));
            }
            Params::Object(map) if map.get("cursor").is_some_and(|cursor| !cursor.is_null()) => {
                return Err(RpcError::invalid_params(
                    "Invalid cursor: the tool list is not paginated",
                ));
            }
            Params::Object(_) | Params::Absent => {}
        }
        Ok(self.stamp(era, json!({ "tools": tools::definitions() }), true))
    }

    /// Answers `tools/call`: protocol errors for a malformed call or unknown tool, a normal
    /// result (possibly with `isError`) for everything else.
    fn call_tool(&self, era: Era, params: Params<'_>) -> Result<Value, RpcError> {
        let map = match params {
            Params::Object(map) => map,
            Params::Absent => {
                return Err(RpcError::invalid_params("`params` with `name` is required"));
            }
            Params::Malformed => {
                return Err(RpcError::invalid_params("`params` must be an object"));
            }
        };
        let Some(Value::String(name)) = map.get("name") else {
            return Err(RpcError::invalid_params("`name` must be a string"));
        };
        let no_arguments = Map::new();
        let arguments = match map.get("arguments") {
            None | Some(Value::Null) => &no_arguments,
            Some(Value::Object(arguments)) => arguments,
            Some(_) => return Err(RpcError::invalid_params("`arguments` must be an object")),
        };
        match tools::call(&self.backend, name, arguments) {
            Ok(outcome) => Ok(self.stamp(
                era,
                json!({
                    "content": [{ "type": "text", "text": outcome.text }],
                    "isError": outcome.is_error,
                }),
                false,
            )),
            Err(CallError::UnknownTool) => Err(RpcError::invalid_params(format!(
                "Unknown tool: {}",
                clip(name, 64)
            ))),
            Err(CallError::BackendPanicked) => Err(RpcError::new(
                INTERNAL_ERROR,
                "Internal error: the tool backend failed",
            )),
        }
    }

    /// Returns the `serverInfo` object.
    fn identity(&self) -> Value {
        json!({ "name": self.info.name, "version": self.info.version })
    }

    /// Adds the fields the era requires around a result body.
    ///
    /// Legacy results are the body unchanged. Stateless results start with `resultType`, carry
    /// the caching hints when `cacheable`, and end with the `serverInfo` metadata.
    fn stamp(&self, era: Era, body: Value, cacheable: bool) -> Value {
        if era == Era::Legacy {
            return body;
        }
        let mut result = Map::new();
        result.insert("resultType".to_owned(), Value::from("complete"));
        if let Value::Object(fields) = body {
            result.extend(fields);
        }
        if cacheable {
            result.insert("ttlMs".to_owned(), Value::from(CACHE_TTL_MS));
            result.insert("cacheScope".to_owned(), Value::from("public"));
        }
        result.insert(
            "_meta".to_owned(),
            json!({ META_SERVER_INFO: self.identity() }),
        );
        Value::Object(result)
    }
}

/// The response used if a value cannot be serialised, which cannot happen for JSON values.
const FALLBACK_RESPONSE: &str =
    r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32603,"message":"Internal error"}}"#;

/// Serialises a response compactly. The output never contains a raw newline.
fn encode(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| FALLBACK_RESPONSE.to_owned())
}
