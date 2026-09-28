// SPDX-License-Identifier: Apache-2.0
//! The nine tools of the server, the [`Backend`] port that serves them and argument validation.
//!
//! # Role in the architecture
//! The MCP server is an entry adapter. It owns the wire format and nothing else: what `brief`,
//! `recall`, `outline`, `expand`, `impact`, `map`, `remember`, `memories` and `feedback` actually
//! do is decided by whatever implements [`Backend`] (the engine, wired in by the binary). This
//! module turns a `tools/call` argument object into a plain request struct, calls the backend and
//! reports the outcome as tool text.
//!
//! # Invariants
//! * The tool list is fixed and its order is deterministic, and it is grouped by what an agent
//!   does rather than alphabetically: read the code (`brief`, `recall`, `outline`, `expand`),
//!   survey it (`impact`, `map`), then write to the memory (`remember`, `memories`, `feedback`).
//! * The whole list stays under [`MAX_LIST_BYTES`], because it is paid for in every session that
//!   connects.
//! * Argument problems and backend failures are *tool errors* (`isError: true`), never protocol
//!   errors. Only an unknown tool name or a backend panic escapes as a protocol error.
//! * Validation is total: any JSON value as `arguments` yields a tool error or a request, never
//!   a panic.

use std::error::Error;
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};

use serde_json::{Map, Value, json};

use crate::protocol::clip;

/// The largest `tools/list` payload this crate is willing to put on the wire, in bytes.
///
/// The list is read once per session, before anything useful happens, by a tool whose whole
/// purpose is spending fewer tokens. At roughly four bytes per token this ceiling is about a
/// thousand tokens of overhead — two `recall` calls' worth — so a tool that cannot pay for that
/// much every session does not belong in the list. `crates/mcp/tests/stateless.rs` fails the build
/// when the payload grows past it, which makes raising it a decision someone has to make on
/// purpose.
pub const MAX_LIST_BYTES: usize = 4_200;

/// Names of the nine tools, in the order `tools/list` returns them.
pub const TOOL_NAMES: [&str; 9] = [
    "brief", "recall", "outline", "expand", "impact", "map", "memories", "remember", "feedback",
];

/// The memory kinds `remember` accepts, in the order the schema lists them.
const MEMORY_KINDS: [&str; 9] = [
    "decision",
    "fact",
    "lesson",
    "dead_end",
    "error_fix",
    "convention",
    "requirement",
    "task",
    "session",
];

/// A failure reported by a [`Backend`]; its text is shown to the agent as a tool error.
///
/// # Examples
/// ```
/// use pn_ultramemory_mcp::ToolFailure;
///
/// let failure = ToolFailure("index is empty".to_owned());
/// assert_eq!(failure.to_string(), "index is empty");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolFailure(pub String);

impl fmt::Display for ToolFailure {
    /// Writes the failure text unchanged.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for ToolFailure {}

/// Arguments of the `recall` tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecallRequest {
    /// What to recall: a question, a symbol name or a path (the `q` argument).
    pub query: String,
    /// Token budget for the answer, when the agent gave one.
    pub budget: Option<u32>,
    /// Whether the agent asked for a short reason per result; `None` when it did not say.
    pub explain: Option<bool>,
}

/// Arguments of the `impact` tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImpactRequest {
    /// The symbol whose blast radius is wanted.
    pub symbol: String,
    /// How many hops of dependents to follow, when the agent gave a limit.
    pub depth: Option<u32>,
}

/// Arguments of the `remember` tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RememberRequest {
    /// One of `decision`, `fact`, `lesson`, `dead_end`, `error_fix`, `convention`, `task` or
    /// `session`; the server has already checked it.
    pub kind: String,
    /// The memory itself.
    pub text: String,
    /// Symbols the memory is anchored to; empty when the agent named none.
    pub about: Vec<String>,
}

/// How a recalled result can be reported to have turned out.
const SIGNALS: [&str; 5] = ["used", "useful", "ignored", "dead_end", "corrected"];

/// Arguments of the `map` tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapRequest {
    /// A token budget for the map, when the agent gave one.
    pub budget: Option<u32>,
    /// Only files whose path starts with this, when the agent narrowed it.
    pub path: Option<String>,
}

/// Arguments of the `memories` tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoriesRequest {
    /// Only memories of this kind; the server has already checked the name.
    pub kind: Option<String>,
    /// Only memories whose anchored code changed since they were written.
    pub stale: bool,
    /// The most to return, when the agent gave a limit.
    pub limit: Option<u32>,
}

/// Arguments of the `feedback` tool.
///
/// Exactly one of `symbol` and `memory` is set; the server has already checked that.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedbackRequest {
    /// The symbol the feedback is about.
    pub symbol: Option<String>,
    /// The memory the feedback is about.
    pub memory: Option<i64>,
    /// One of `used`, `useful`, `ignored`, `dead_end`, `corrected`.
    pub signal: String,
}

/// Arguments of the `brief` tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BriefRequest {
    /// A token budget for the whole briefing, when the agent gave one.
    pub budget: Option<u32>,
}

/// Arguments of the `outline` tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineRequest {
    /// The file to describe, as a path inside the repository.
    pub path: String,
    /// A token budget. The detail falls to fit it; symbols are never dropped.
    pub budget: Option<u32>,
}

/// Arguments of the `expand` tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpandRequest {
    /// Identifier of the node to expand, as returned by an earlier result.
    pub id: String,
    /// First line of the requested window: 1-based, within the symbol's own source (line 1 is
    /// the first line of its declaration). `None` means from the first line. The server has
    /// already checked that it is at least 1.
    pub from: Option<u32>,
    /// Last line of the requested window, inclusive, numbered like `from`. `None` means up to
    /// the last line. The server has already checked that it is at least 1 and, when `from` is
    /// also given, not less than `from`.
    pub to: Option<u32>,
}

/// The four operations the server exposes; implemented by the layer that owns the data.
///
/// Every method returns the text shown to the agent, or a [`ToolFailure`] whose text is shown as
/// a tool error so the agent can react to it. Implementations must not panic: an unwinding panic
/// is caught and reported as a protocol-level internal error (`-32603`) and the session
/// continues, but under `panic = "abort"`, as in the release profile, the process ends.
///
/// # Examples
/// ```
/// use pn_ultramemory_mcp::{
///     Backend, FeedbackRequest, MemoriesRequest, MapRequest, BriefRequest, ExpandRequest, ImpactRequest, OutlineRequest, RecallRequest,
///     RememberRequest, ToolFailure,
/// };
///
/// struct Silent;
///
/// impl Backend for Silent {
///     fn recall(&self, request: RecallRequest) -> Result<String, ToolFailure> {
///         Ok(format!("nothing about {}", request.query))
///     }
///     fn impact(&self, _: ImpactRequest) -> Result<String, ToolFailure> {
///         Err(ToolFailure("no index".to_owned()))
///     }
///     fn remember(&self, _: RememberRequest) -> Result<String, ToolFailure> {
///         Ok("stored".to_owned())
///     }
///     fn brief(&self, _: BriefRequest) -> Result<String, ToolFailure> {
///         Ok("brief:\n  files: 0".into())
///     }
///     fn map(&self, _: MapRequest) -> Result<String, ToolFailure> {
///         Ok("files[0]{path}:".into())
///     }
///     fn memories(&self, _: MemoriesRequest) -> Result<String, ToolFailure> {
///         Ok("memories[0]{id}:".into())
///     }
///     fn feedback(&self, _: FeedbackRequest) -> Result<String, ToolFailure> {
///         Ok("recorded: true".into())
///     }
///     fn outline(&self, _: OutlineRequest) -> Result<String, ToolFailure> {
///         Ok("file:\n  path: src/lib.rs".into())
///     }
///     fn expand(&self, _: ExpandRequest) -> Result<String, ToolFailure> {
///         Ok(String::new())
///     }
/// }
///
/// let reply = Silent.recall(RecallRequest {
///     query: "parser".to_owned(),
///     budget: None,
///     explain: None,
/// });
/// assert_eq!(reply.unwrap(), "nothing about parser");
/// ```
pub trait Backend: Send + Sync {
    /// Returns a capsule of code and memories relevant to the query.
    ///
    /// # Errors
    /// Returns a [`ToolFailure`] when the query cannot be answered.
    fn recall(&self, request: RecallRequest) -> Result<String, ToolFailure>;

    /// Returns the blast radius of a symbol.
    ///
    /// # Errors
    /// Returns a [`ToolFailure`] when the symbol is unknown or the analysis fails.
    fn impact(&self, request: ImpactRequest) -> Result<String, ToolFailure>;

    /// Stores a memory and returns a confirmation.
    ///
    /// # Errors
    /// Returns a [`ToolFailure`] when the memory cannot be stored.
    fn remember(&self, request: RememberRequest) -> Result<String, ToolFailure>;

    /// Returns the full source of one node.
    ///
    /// # Errors
    /// Returns a [`ToolFailure`] when the id is unknown or the source is unavailable.
    fn expand(&self, request: ExpandRequest) -> Result<String, ToolFailure>;

    /// Everything a session needs to know about this repository, inside a token budget.
    ///
    /// # Errors
    /// Returns a [`ToolFailure`] whose text is shown to the agent.
    fn brief(&self, request: BriefRequest) -> Result<String, ToolFailure>;

    /// A compact map of the repository, inside a token budget.
    ///
    /// # Errors
    /// Returns a [`ToolFailure`] whose text is shown to the agent.
    fn map(&self, request: MapRequest) -> Result<String, ToolFailure>;

    /// What is already known about this codebase, and what of it may have gone out of date.
    ///
    /// # Errors
    /// Returns a [`ToolFailure`] whose text is shown to the agent.
    fn memories(&self, request: MemoriesRequest) -> Result<String, ToolFailure>;

    /// Records how a result turned out, which is what lets ranking improve.
    ///
    /// # Errors
    /// Returns a [`ToolFailure`] whose text is shown to the agent.
    fn feedback(&self, request: FeedbackRequest) -> Result<String, ToolFailure>;

    /// Describes one whole file: every symbol it declares, at the richest detail that fits.
    ///
    /// # Errors
    /// Returns a [`ToolFailure`] whose text is shown to the agent.
    fn outline(&self, request: OutlineRequest) -> Result<String, ToolFailure>;
}

/// The text and status of one finished tool call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Outcome {
    /// Text shown to the agent.
    pub(crate) text: String,
    /// Whether the call ended in an error the agent should see as such.
    pub(crate) is_error: bool,
}

/// Why a tool call could not even produce an [`Outcome`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CallError {
    /// The tool name is not one of [`TOOL_NAMES`].
    UnknownTool,
    /// The backend panicked; the call is reported as an internal error.
    BackendPanicked,
}

/// Builds the `tools` array returned by `tools/list`.
///
/// Descriptions are deliberately terse: this array is sent to every client on every session, so
/// each byte is a token the agent pays for. The annotations are the hints defined by the
/// 2025-03-26 and later revisions; older clients ignore fields they do not know.
pub(crate) fn definitions() -> Value {
    let mut tools = reading_tools();
    if let Value::Array(all) = &mut tools {
        for group in [survey_tools(), writing_tools()] {
            if let Value::Array(rest) = group {
                all.extend(rest);
            }
        }
    }
    tools
}

/// The tools that only read. Split from the rest so that neither list outgrows a screen.
fn reading_tools() -> Value {
    let read_only = json!({
        "readOnlyHint": true,
        "destructiveHint": false,
        "openWorldHint": false
    });
    json!([
        {
            "name": "brief",
            "description": "What this repository is: its shape, modules, busiest symbols and what \
                            is already decided. Call this first.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "budget": { "type": "integer", "description": "Token budget", "minimum": 1 }
                }
            },
            "annotations": read_only
        },
        {
            "name": "recall",
            "description": "Recall code and memories relevant to a query, packed within a token \
                            budget.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "q": { "type": "string", "description": "Question, symbol or path" },
                    "budget": { "type": "integer", "description": "Token budget" },
                    "explain": { "type": "boolean", "description": "Why each result was chosen" }
                },
                "required": ["q"]
            },
            "annotations": read_only
        },
        {
            "name": "outline",
            "description": "List every symbol a file declares, with signatures and docs. Never \
                            drops one: a tight budget lowers the detail instead.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path in the repository" },
                    "budget": { "type": "integer", "description": "Token budget", "minimum": 1 }
                },
                "required": ["path"]
            },
            "annotations": read_only
        },
        {
            "name": "expand",
            "description": "Return the full source of one node id from an earlier result.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Node id" },
                    "from": { "type": "integer", "description": "First line, 1-based" },
                    "to": { "type": "integer", "description": "Last line, inclusive" }
                },
                "required": ["id"]
            },
            "annotations": read_only
        },
        {
            "name": "impact",
            "description": "Show what depends on a symbol, with confidence.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "symbol": { "type": "string", "description": "Qualified symbol name" },
                    "depth": { "type": "integer", "description": "Hops to follow" }
                },
                "required": ["symbol"]
            },
            "annotations": read_only
        },
    ])
}

/// The tools that survey the repository as a whole, rather than answering about one place in it.
fn survey_tools() -> Value {
    let read_only = json!({
        "readOnlyHint": true,
        "destructiveHint": false,
        "openWorldHint": false
    });
    json!([
        {
            "name": "map",
            "description": "A compact map of the repository within a budget: every file, its \
                            language, size and busiest symbols.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "budget": { "type": "integer", "description": "Token budget", "minimum": 1 },
                    "path": { "type": "string", "description": "Only paths starting with this" }
                }
            },
            "annotations": read_only
        },
        {
            "name": "memories",
            "description": "What is already known about this codebase. Use stale to see what may \
                            have gone out of date.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "kind": { "type": "string", "enum": MEMORY_KINDS },
                    "stale": {
                        "type": "boolean",
                        "description": "Only memories whose code changed"
                    },
                    "limit": { "type": "integer", "description": "How many at most", "minimum": 1 }
                }
            },
            "annotations": read_only
        }
    ])
}

/// The tools that write: one stores a memory, the other records how a result turned out.
fn writing_tools() -> Value {
    let writes = json!({
        "readOnlyHint": false,
        "destructiveHint": false,
        "openWorldHint": false
    });
    json!([
        {
            "name": "remember",
            "description": "Save a memory, optionally anchored to symbols, for later recall.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "kind": { "type": "string", "enum": MEMORY_KINDS },
                    "text": { "type": "string", "description": "What to remember" },
                    "about": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Symbols it concerns"
                    }
                },
                "required": ["kind", "text"]
            },
            "annotations": writes
        },
        {
            "name": "feedback",
            "description": "Say how a recalled symbol or memory turned out. This is what makes \
                            later answers better.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "symbol": { "type": "string", "description": "The symbol it is about" },
                    "memory": { "type": "integer", "description": "The memory id it is about" },
                    "signal": {
                        "type": "string",
                        "description": "How it turned out",
                        "enum": ["used", "useful", "ignored", "dead_end", "corrected"]
                    }
                },
                "required": ["signal"]
            },
            "annotations": writes
        }
    ])
}

/// Names the JSON type of `value` for error messages.
fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

/// Fails on the first argument name that is not in `allowed`.
///
/// Rejecting unknown names turns a misspelt optional argument into feedback the agent can act on
/// instead of a silently ignored value.
fn check_keys(arguments: &Map<String, Value>, allowed: &[&str]) -> Result<(), String> {
    match arguments
        .keys()
        .find(|key| !allowed.contains(&key.as_str()))
    {
        Some(key) => Err(format!(
            "unknown argument `{}`; allowed: {}",
            clip(key, 40),
            allowed.join(", ")
        )),
        None => Ok(()),
    }
}

/// Reads a required string argument that is not blank.
fn required_text(arguments: &Map<String, Value>, key: &str) -> Result<String, String> {
    match arguments.get(key) {
        None | Some(Value::Null) => Err(format!("`{key}` is required")),
        Some(Value::String(text)) if text.trim().is_empty() => {
            Err(format!("`{key}` must not be empty"))
        }
        Some(Value::String(text)) => Ok(text.clone()),
        Some(other) => Err(format!(
            "`{key}` must be a string, not {}",
            type_name(other)
        )),
    }
}

/// Converts a JSON float to `u32` when it is a whole number in range.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the value is checked to be whole and within the u32 range right before the cast"
)]
fn whole_f64_to_u32(value: f64) -> Option<u32> {
    (value.fract() == 0.0 && (0.0..=f64::from(u32::MAX)).contains(&value)).then_some(value as u32)
}

/// Reads an optional integer argument between `min` and `u32::MAX`.
///
/// `null` counts as absent. Whole floats such as `4000.0` are accepted because JSON Schema treats
/// them as integers.
fn optional_count_from(
    arguments: &Map<String, Value>,
    key: &str,
    min: u32,
) -> Result<Option<u32>, String> {
    let out_of_range = || format!("`{key}` must be an integer from {min} to {}", u32::MAX);
    match arguments.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(number)) => {
            let parsed = match number.as_u64() {
                Some(unsigned) => u32::try_from(unsigned).ok(),
                None if number.is_i64() => None,
                None => number.as_f64().and_then(whole_f64_to_u32),
            };
            parsed
                .filter(|value| *value >= min)
                .map(Some)
                .ok_or_else(out_of_range)
        }
        Some(other) => Err(format!(
            "`{key}` must be an integer, not {}",
            type_name(other)
        )),
    }
}

/// Reads an optional non-negative integer argument that fits in 32 bits.
fn optional_count(arguments: &Map<String, Value>, key: &str) -> Result<Option<u32>, String> {
    optional_count_from(arguments, key, 0)
}

/// Reads an optional boolean argument; `null` counts as absent.
fn optional_flag(arguments: &Map<String, Value>, key: &str) -> Result<Option<bool>, String> {
    match arguments.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(flag)) => Ok(Some(*flag)),
        Some(other) => Err(format!(
            "`{key}` must be a boolean, not {}",
            type_name(other)
        )),
    }
}

/// Reads the optional `about` list of non-blank strings; `null` counts as absent.
fn optional_texts(arguments: &Map<String, Value>, key: &str) -> Result<Vec<String>, String> {
    match arguments.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .enumerate()
            .map(|(index, item)| match item {
                Value::String(text) if !text.trim().is_empty() => Ok(text.clone()),
                _ => Err(format!("`{key}[{index}]` must be a non-empty string")),
            })
            .collect(),
        Some(other) => Err(format!(
            "`{key}` must be an array of strings, not {}",
            type_name(other)
        )),
    }
}

/// Validates the arguments of `recall`.
fn parse_recall(arguments: &Map<String, Value>) -> Result<RecallRequest, String> {
    check_keys(arguments, &["q", "budget", "explain"])?;
    Ok(RecallRequest {
        query: required_text(arguments, "q")?,
        budget: optional_count(arguments, "budget")?,
        explain: optional_flag(arguments, "explain")?,
    })
}

/// Validates the arguments of `impact`.
fn parse_impact(arguments: &Map<String, Value>) -> Result<ImpactRequest, String> {
    check_keys(arguments, &["symbol", "depth"])?;
    Ok(ImpactRequest {
        symbol: required_text(arguments, "symbol")?,
        depth: optional_count(arguments, "depth")?,
    })
}

/// Validates the arguments of `remember`.
fn parse_remember(arguments: &Map<String, Value>) -> Result<RememberRequest, String> {
    check_keys(arguments, &["kind", "text", "about"])?;
    let kind = match arguments.get("kind") {
        None | Some(Value::Null) => {
            return Err(format!(
                "`kind` is required; one of {}",
                MEMORY_KINDS.join(", ")
            ));
        }
        Some(Value::String(kind)) if MEMORY_KINDS.contains(&kind.as_str()) => kind.clone(),
        Some(_) => return Err(format!("`kind` must be one of {}", MEMORY_KINDS.join(", "))),
    };
    Ok(RememberRequest {
        kind,
        text: required_text(arguments, "text")?,
        about: optional_texts(arguments, "about")?,
    })
}

/// Validates the arguments of `outline`.
/// Validates the arguments of `map`.
fn parse_map(arguments: &Map<String, Value>) -> Result<MapRequest, String> {
    check_keys(arguments, &["budget", "path"])?;
    Ok(MapRequest {
        budget: optional_count_from(arguments, "budget", 1)?,
        path: optional_string(arguments, "path")?,
    })
}

/// Validates the arguments of `memories`.
fn parse_memories(arguments: &Map<String, Value>) -> Result<MemoriesRequest, String> {
    check_keys(arguments, &["kind", "stale", "limit"])?;
    let kind = optional_string(arguments, "kind")?;
    if let Some(name) = &kind {
        if !MEMORY_KINDS.contains(&name.as_str()) {
            return Err(format!(
                "`kind` is `{name}`, which is not one of {}",
                MEMORY_KINDS.join(", ")
            ));
        }
    }
    Ok(MemoriesRequest {
        kind,
        stale: optional_flag(arguments, "stale")?.unwrap_or(false),
        limit: optional_count_from(arguments, "limit", 1)?,
    })
}

/// Validates the arguments of `feedback`.
///
/// Exactly one of `symbol` and `memory` must be given: feedback about nothing in particular cannot
/// be recorded, and feedback about two things at once means the caller is confused about which.
fn parse_feedback(arguments: &Map<String, Value>) -> Result<FeedbackRequest, String> {
    check_keys(arguments, &["symbol", "memory", "signal"])?;
    let signal = required_text(arguments, "signal")?;
    if !SIGNALS.contains(&signal.as_str()) {
        return Err(format!(
            "`signal` is `{signal}`, which is not one of: {}",
            SIGNALS.join(", ")
        ));
    }
    let symbol = optional_string(arguments, "symbol")?;
    let memory =
        match arguments.get("memory") {
            None | Some(Value::Null) => None,
            Some(Value::Number(n)) => Some(n.as_i64().ok_or_else(|| {
                "`memory` must be a whole number, which is what an id is".to_owned()
            })?),
            Some(other) => {
                return Err(format!(
                    "`memory` must be a number, not {}",
                    type_name(other)
                ));
            }
        };
    match (&symbol, memory) {
        (Some(_), Some(_)) => {
            Err("give `symbol` or `memory`, not both: feedback is about one thing".to_owned())
        }
        (None, None) => {
            Err("give `symbol` or `memory` to say what the feedback is about".to_owned())
        }
        _ => Ok(FeedbackRequest {
            symbol,
            memory,
            signal,
        }),
    }
}

/// The value of `key` when it is a string, or `None` when it is absent or null.
fn optional_string(arguments: &Map<String, Value>, key: &str) -> Result<Option<String>, String> {
    match arguments.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) if text.trim().is_empty() => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.trim().to_owned())),
        Some(other) => Err(format!(
            "`{key}` must be a string, not {}",
            type_name(other)
        )),
    }
}

/// Validates the arguments of `brief`.
fn parse_brief(arguments: &Map<String, Value>) -> Result<BriefRequest, String> {
    check_keys(arguments, &["budget"])?;
    Ok(BriefRequest {
        budget: optional_count_from(arguments, "budget", 1)?,
    })
}

/// Validates the arguments of `outline`.
fn parse_outline(arguments: &Map<String, Value>) -> Result<OutlineRequest, String> {
    check_keys(arguments, &["path", "budget"])?;
    let path = required_text(arguments, "path")?;
    let budget = optional_count_from(arguments, "budget", 1)?;
    Ok(OutlineRequest { path, budget })
}

/// Validates the arguments of `expand`.
fn parse_expand(arguments: &Map<String, Value>) -> Result<ExpandRequest, String> {
    check_keys(arguments, &["id", "from", "to"])?;
    let id = required_text(arguments, "id")?;
    let from = optional_count_from(arguments, "from", 1)?;
    let to = optional_count_from(arguments, "to", 1)?;
    if let (Some(first), Some(last)) = (from, to) {
        if last < first {
            return Err(format!(
                "`to` ({last}) must not be less than `from` ({first})"
            ));
        }
    }
    Ok(ExpandRequest { id, from, to })
}

/// Runs one backend call, converting its result into an [`Outcome`] and containing panics.
fn run(call: impl FnOnce() -> Result<String, ToolFailure>) -> Result<Outcome, CallError> {
    match catch_unwind(AssertUnwindSafe(call)) {
        Ok(Ok(text)) => Ok(Outcome {
            text,
            is_error: false,
        }),
        Ok(Err(ToolFailure(text))) => Ok(Outcome {
            text,
            is_error: true,
        }),
        Err(_) => Err(CallError::BackendPanicked),
    }
}

/// Wraps a validation message as a tool error outcome.
fn rejected(message: String) -> Outcome {
    Outcome {
        text: message,
        is_error: true,
    }
}

/// Validates `arguments` for the tool `name` and calls the backend.
///
/// # Errors
/// Returns [`CallError::UnknownTool`] for a name outside [`TOOL_NAMES`] and
/// [`CallError::BackendPanicked`] when the backend panics. Every other problem is reported inside
/// the [`Outcome`] as a tool error.
pub(crate) fn call<B: Backend + ?Sized>(
    backend: &B,
    name: &str,
    arguments: &Map<String, Value>,
) -> Result<Outcome, CallError> {
    match name {
        "recall" => match parse_recall(arguments) {
            Ok(request) => run(|| backend.recall(request)),
            Err(message) => Ok(rejected(message)),
        },
        "impact" => match parse_impact(arguments) {
            Ok(request) => run(|| backend.impact(request)),
            Err(message) => Ok(rejected(message)),
        },
        "remember" => match parse_remember(arguments) {
            Ok(request) => run(|| backend.remember(request)),
            Err(message) => Ok(rejected(message)),
        },
        "brief" => match parse_brief(arguments) {
            Ok(request) => run(|| backend.brief(request)),
            Err(message) => Ok(rejected(message)),
        },
        "map" => match parse_map(arguments) {
            Ok(request) => run(|| backend.map(request)),
            Err(message) => Ok(rejected(message)),
        },
        "memories" => match parse_memories(arguments) {
            Ok(request) => run(|| backend.memories(request)),
            Err(message) => Ok(rejected(message)),
        },
        "feedback" => match parse_feedback(arguments) {
            Ok(request) => run(|| backend.feedback(request)),
            Err(message) => Ok(rejected(message)),
        },
        "outline" => match parse_outline(arguments) {
            Ok(request) => run(|| backend.outline(request)),
            Err(message) => Ok(rejected(message)),
        },
        "expand" => match parse_expand(arguments) {
            Ok(request) => run(|| backend.expand(request)),
            Err(message) => Ok(rejected(message)),
        },
        _ => Err(CallError::UnknownTool),
    }
}

/// Unit tests for the tool list, argument validation and backend containment.
#[cfg(test)]
mod tests;
