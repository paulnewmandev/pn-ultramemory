// SPDX-License-Identifier: Apache-2.0
//! Minimal Model Context Protocol (MCP) server over stdio for pn-ultramemory.
//!
//! # Role in the architecture
//! This crate is an **entry adapter** (see `docs/architecture.md`). It owns the wire protocol and
//! nothing else: it depends on no other workspace crate, performs no network access and knows
//! nothing about indexes or memories. The nine things an agent can do are behind the [`Backend`]
//! trait, which the binary implements with the engine and hands to [`Server::new`]. The agent-facing
//! surface is kept small on purpose: every tool's schema is read once per session, so a tool that
//! cannot pay for its own description does not belong in the list.
//!
//! # Tools
//! Nine tools, listed in this order by `tools/list` (about 4 KB of JSON in total, or roughly a
//! thousand tokens of per-session overhead — [`MAX_LIST_BYTES`] is the ceiling a test
//! enforces):
//!
//! | Tool | Arguments | Purpose |
//! |---|---|---|
//! | `brief` | `budget` | What the repository is: size, languages, modules, busiest symbols and recorded memories. What a session that has lost its history reads first |
//! | `recall` | `q` (required), `budget`, `explain` | A capsule of code and memories for a query, within a token budget; `explain: true` adds a short reason per result |
//! | `outline` | `path` (required), `budget` | Every symbol in one file. Detail falls to fit the budget; symbols are never dropped |
//! | `expand` | `id` (required), `from`, `to` | The source of one node, optionally only lines `from` to `to` (1-based, inclusive, counted within the symbol's own source) so a very large function can be paged |
//! | `impact` | `symbol` (required), `depth` | What depends on a symbol, with confidence |
//! | `map` | `budget`, `path` | The repository's files and what each holds, optionally under one path prefix |
//! | `remember` | `kind` (required), `text` (required), `about` | Store a memory, optionally anchored to symbols |
//! | `memories` | `kind`, `stale`, `limit` | What is already known, and which memories the code has moved out from under |
//! | `feedback` | `signal` (required), `symbol` or `memory` (one of them) | Report that a result was `used`, `useful`, `ignored`, a `dead_end` or `corrected`, and get the updated utility back. The only way the learning subsystem hears from the agent that actually calls `recall` |
//!
//! `kind` is one of `decision`, `fact`, `lesson`, `dead_end`, `error_fix`, `convention`,
//! `requirement`, `task`
//! or `session`. Successful calls return `{content: [{type: "text", text}], isError: false}`.
//! Argument validation problems and [`ToolFailure`]s from the backend also return a normal
//! result, with `isError: true` and the message, so the agent can correct itself. Only an unknown
//! tool name (`-32602`), a malformed request or an unknown method is a JSON-RPC error.
//!
//! # Protocol revisions
//! **Targeted revision: `2026-07-28`**, the latest published at the time of writing
//! (<https://modelcontextprotocol.io/specification/2026-07-28>). **Accepted revisions:**
//! `2026-07-28`, `2025-11-25`, `2025-06-18`, `2025-03-26` and `2024-11-05`
//! ([`SUPPORTED_PROTOCOL_VERSIONS`]).
//!
//! Revision 2026-07-28 removed the `initialize` handshake, sessions and `ping`: every request
//! is self-contained and carries its protocol version and client capabilities in
//! `params._meta`. The earlier revisions negotiate once with `initialize`. The specification
//! allows a *dual-era* server and this crate is one, choosing per request:
//!
//! | Request | Served as |
//! |---|---|
//! | `initialize` | Legacy handshake. The client's version is echoed when it is `2025-11-25`, `2025-06-18`, `2025-03-26` or `2024-11-05`; any other version (including `2026-07-28`, which has no handshake) is answered with `2025-11-25`. |
//! | `params._meta` has `io.modelcontextprotocol/protocolVersion` or `io.modelcontextprotocol/clientCapabilities` | Stateless. Both fields are required (`-32602` otherwise); an unsupported version gets `-32022` with `data.supported` and `data.requested`. Results carry `resultType: "complete"`, `_meta` with the server identity and, for `server/discover` and `tools/list`, `ttlMs` and `cacheScope`. Methods: `server/discover`, `tools/list`, `tools/call`. `ping` no longer exists in this era (`-32601`). |
//! | anything else | Legacy session. `ping` always works. `tools/list` and `tools/call` are refused with `-32602` until `initialize` has been answered. |
//!
//! The `notifications/initialized` notification is accepted (and, like every other notification,
//! never answered), but the server does not wait for it: on one ordered stream it always
//! arrives before the client's next request. JSON-RPC batches are accepted only when the
//! negotiated revision is `2025-03-26`, the only one that has them, and are capped at 256
//! messages; in every other case an array is answered with `-32600`. The tool list is
//! identical for every era (annotations, which 2024-11-05 lacks, are extra fields that its
//! clients ignore). Unknown notifications are ignored silently.
//!
//! # Framing
//! Stdio carries one JSON-RPC message per line (UTF-8, no embedded newlines) and stdout carries
//! nothing else; logs belong on stderr. [`Server::serve`] enforces a bounded line: anything above
//! [`MAX_LINE_BYTES`] (4 MiB) is refused with `-32600` and skipped without being buffered.
//! Invalid UTF-8 is refused with `-32700` rather than silently repaired, because a repaired
//! memory would be a corrupted memory. Blank lines are ignored, and a response id is `null`
//! only when the request id could not be read (JSON-RPC 2.0 section 5).
//!
//! # Concurrency
//! [`Server::serve`] handles one message at a time, in arrival order, so a slow tool call delays
//! the messages behind it and `notifications/cancelled` cannot interrupt it. [`Server`] itself is
//! `Send + Sync` and [`Server::handle_line`] can be called from several threads at once.
//!
//! # Invariants
//! * No input, however hostile, makes the server panic: parsing is depth-limited by
//!   `serde_json`, sizes are bounded, and untrusted text echoed into errors is clipped.
//! * Output is deterministic: fixed tool order, insertion-ordered JSON objects, no clocks.
//! * The protocol core, [`Server::handle_line`], is pure with respect to I/O and is what the
//!   tests drive; [`Server::serve`] only adds framing.
//!
//! # Examples
//! A complete stateless exchange, with a backend that answers every tool call:
//!
//! ```
//! use pn_ultramemory_mcp::{
//!     Backend, BriefRequest, ExpandRequest, FeedbackRequest, ImpactRequest, MAX_LIST_BYTES,
//!     MapRequest, MemoriesRequest, OutlineRequest, RecallRequest, RememberRequest, Server,
//!     ServerInfo, ToolFailure,
//! };
//!
//! struct Notes;
//!
//! impl Backend for Notes {
//!     fn recall(&self, request: RecallRequest) -> Result<String, ToolFailure> {
//!         Ok(format!("recalled: {}", request.query))
//!     }
//!     fn impact(&self, request: ImpactRequest) -> Result<String, ToolFailure> {
//!         Ok(format!("impact of {}", request.symbol))
//!     }
//!     fn remember(&self, request: RememberRequest) -> Result<String, ToolFailure> {
//!         Ok(format!("remembered a {}", request.kind))
//!     }
//!     fn brief(&self, _: BriefRequest) -> Result<String, ToolFailure> {
//!         Ok("brief:\n  files: 0".into())
//!     }
//!     fn map(&self, _: MapRequest) -> Result<String, ToolFailure> {
//!         Ok("files[0]{path}:".into())
//!     }
//!     fn memories(&self, _: MemoriesRequest) -> Result<String, ToolFailure> {
//!         Ok("memories[0]{id}:".into())
//!     }
//!     fn feedback(&self, _: FeedbackRequest) -> Result<String, ToolFailure> {
//!         Ok("recorded: true".into())
//!     }
//!     fn outline(&self, _: OutlineRequest) -> Result<String, ToolFailure> {
//!         Ok("file:\n  path: src/lib.rs".into())
//!     }
//!     fn expand(&self, request: ExpandRequest) -> Result<String, ToolFailure> {
//!         Err(ToolFailure(format!("unknown node {}", request.id)))
//!     }
//! }
//!
//! let server = Server::new(
//!     Notes,
//!     ServerInfo { name: "notes".to_owned(), version: "1.0.0".to_owned() },
//! );
//!
//! // Legacy clients start with `initialize`.
//! let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"c","version":"1"}}}"#;
//! assert!(server.handle_line(init).unwrap().contains(r#""protocolVersion":"2025-11-25""#));
//! assert_eq!(server.handle_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#), None);
//!
//! let list = server.handle_line(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#).unwrap();
//! assert!(list.len() < MAX_LIST_BYTES);
//!
//! let call = r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"expand","arguments":{"id":"n1"}}}"#;
//! let reply = server.handle_line(call).unwrap();
//! assert!(reply.contains(r#""isError":true"#)); // a backend failure is a tool error, not a protocol error
//! ```

mod framing;
mod protocol;
mod server;
mod tools;

pub use protocol::{LATEST_PROTOCOL_VERSION, SUPPORTED_PROTOCOL_VERSIONS};
pub use server::{MAX_LINE_BYTES, Server, ServerInfo};
pub use tools::{
    Backend, BriefRequest, ExpandRequest, FeedbackRequest, ImpactRequest, MAX_LIST_BYTES,
    MapRequest, MemoriesRequest, OutlineRequest, RecallRequest, RememberRequest, TOOL_NAMES,
    ToolFailure,
};
