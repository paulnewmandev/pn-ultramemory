// SPDX-License-Identifier: Apache-2.0
//! The hook this tool installs into a coding agent: one short line, or nothing at all.
//!
//! # Role in the architecture
//! An entry adapter. An agent runs `pn-ultramemory hook <event>` before or after a tool call, hands
//! it JSON on standard input and reads JSON from standard output. [`run_hook`] is the whole
//! behaviour and is pure with respect to input and output, so every rule below is tested directly;
//! [`run_hook_stdio`] only adds the reading, the writing and the deadline.
//!
//! # Events
//! | Event | What it says |
//! |---|---|
//! | `session-start` | One line: this repository is indexed, how big the index is, and to recall instead of reading. When there is no index, one line saying how to build it. |
//! | `pre-tool` | For a repository-wide search or a very large whole-file read, once per session: a recall would cost fewer tokens. Never a denial. Nothing for anything else. |
//! | `post-tool` | Nothing. Reserved. |
//! | anything else | Nothing. |
//!
//! # The shape of the answer
//! A single line of JSON: `{"additionalContext": "..."}` when there is something to say, and `{}`
//! when there is not. The field name is [`CONTEXT_KEY`], defined once, because agents disagree
//! about it:
//!
//! | Agent | The field that reaches the model | Source |
//! |---|---|---|
//! | Claude Code | `additionalContext`, inside `hookSpecificOutput` beside `hookEventName` | <https://code.claude.com/docs/en/hooks> |
//! | Cursor | `additional_context`; `agent_message` when a call is denied | <https://cursor.com/docs/agent/hooks> |
//!
//! `additionalContext` at the top level is what this module writes, because it is the name most
//! agents understand and the one the flat shape is documented under. An agent that wants another
//! spelling needs one line changed here, not a new code path. Claude Code's nesting is deliberate:
//! a wrapper would have to name the event, and the hook does not know what the agent calls it.
//!
//! # Invariants
//! * It never fails: every error becomes `{}` and the process is expected to exit `0`.
//! * It never blocks: the work runs on its own thread and [`HookContext::deadline`] is enforced by
//!   the caller, which prints `{}` and returns if the thread is not finished in time.
//! * It writes exactly one line to standard output and nothing else, ever.
//! * It is silent when [`HookContext::enabled`] is false or [`DISABLE_ENV`] is set.
//! * No input makes it panic: standard input is bounded by [`MAX_INPUT_BYTES`], invalid UTF-8 is
//!   read leniently, and malformed JSON is simply not understood.
//! * The advice is given at most once per session, remembered by a marker file under the data
//!   directory. Deleting that file offers the advice again.

use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

use crate::error::CliError;

/// The field of the answer that carries text for the agent to read.
pub const CONTEXT_KEY: &str = "additionalContext";

/// The environment variable that turns every hook off.
pub const DISABLE_ENV: &str = "PN_ULTRAMEMORY_NO_HOOKS";

/// The environment variable that names the session when the agent does not.
pub const SESSION_ENV: &str = "PN_ULTRAMEMORY_SESSION";

/// The most standard input that is read: one mebibyte. Anything beyond it is ignored.
pub const MAX_INPUT_BYTES: u64 = 1024 * 1024;

/// The file the index command may leave next to the index with `{"files": N, "symbols": M}`. It is
/// optional: without it the session line simply gives no counts.
pub const STATS_FILE: &str = "stats.json";

/// The name of the index database inside the data directory.
const INDEX_FILE: &str = "index.db";

/// A whole-file read above this size is worth a word.
const LARGE_FILE_BYTES: u64 = 64 * 1024;

/// The answer that says nothing.
const NOTHING: &str = "{}";

/// The advice given at most once per session before an expensive read.
const ADVICE: &str = "pn-ultramemory: a recall call would answer this with fewer tokens.";

/// What the hook needs to know about the world.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookContext {
    /// The repository the agent is working in.
    pub repo: PathBuf,
    /// Where the index and the session markers live. It is passed in rather than worked out here,
    /// because the command line has already decided it.
    pub data_dir: PathBuf,
    /// Whether hooks are wanted at all.
    pub enabled: bool,
    /// How long the work may take before the caller gives up and says nothing.
    pub deadline: Duration,
}

impl HookContext {
    /// A context with a sensible deadline, for a repository and its data directory.
    #[must_use]
    pub fn new(repo: impl Into<PathBuf>, data_dir: impl Into<PathBuf>) -> Self {
        Self {
            repo: repo.into(),
            data_dir: data_dir.into(),
            enabled: true,
            deadline: Duration::from_millis(400),
        }
    }
}

/// Answers one hook event. This is the whole behaviour of the hook, as one pure function of the
/// event, the agent's JSON and the context.
///
/// # Examples
/// ```text
/// run_hook("post-tool", "{}", &context)                  // => "{}"
/// run_hook("session-start", "", &context)                // => {"additionalContext": "..."}
/// run_hook("pre-tool", r#"{"tool_name":"Grep"}"#, &ctx)  // => advice, once per session
/// ```
#[must_use]
pub fn run_hook(event: &str, input: &str, ctx: &HookContext) -> String {
    if !ctx.enabled || std::env::var_os(DISABLE_ENV).is_some_and(|value| !value.is_empty()) {
        return NOTHING.to_owned();
    }
    let message = match event {
        "session-start" => Some(session_line(ctx)),
        "pre-tool" => pre_tool_line(input, ctx),
        _ => None,
    };
    match message {
        Some(text) if !text.is_empty() => {
            let mut answer = serde_json::Map::new();
            answer.insert(CONTEXT_KEY.to_owned(), Value::String(text));
            Value::Object(answer).to_string()
        }
        _ => NOTHING.to_owned(),
    }
}

/// Reads standard input, answers, and writes the one line the agent expects.
///
/// The work happens on its own thread so that a slow disk can never hold the agent up: when the
/// deadline passes, `{}` is printed and this returns. The thread is abandoned, which is safe because
/// it only reads.
///
/// # Errors
/// Returns a failure only when standard output cannot be written, which the caller is expected to
/// turn into exit code `0` all the same: a hook must never fail the tool call it was called from.
pub fn run_hook_stdio(event: &str, ctx: &HookContext) -> Result<(), CliError> {
    let (sender, receiver) = std::sync::mpsc::channel();
    let event = event.to_owned();
    let context = ctx.clone();
    std::thread::spawn(move || {
        let input = read_input();
        let _ = sender.send(run_hook(&event, &input, &context));
    });
    let answer = receiver
        .recv_timeout(ctx.deadline)
        .unwrap_or_else(|_| NOTHING.to_owned());
    let mut out = std::io::stdout().lock();
    out.write_all(answer.as_bytes())?;
    out.write_all(b"\n")?;
    out.flush()?;
    Ok(())
}

/// Reads at most [`MAX_INPUT_BYTES`] of standard input, repairing invalid UTF-8 rather than
/// refusing it: a hook that fails on a stray byte is worse than a hook that misreads one.
fn read_input() -> String {
    let mut buffer = Vec::new();
    let _ = std::io::stdin()
        .lock()
        .take(MAX_INPUT_BYTES)
        .read_to_end(&mut buffer);
    String::from_utf8_lossy(&buffer).into_owned()
}

/// The line offered when a session starts: about forty tokens, and never a failure.
fn session_line(ctx: &HookContext) -> String {
    if !ctx.data_dir.join(INDEX_FILE).is_file() {
        return "pn-ultramemory: this repository has no index yet. Run `pn-ultramemory index` once, \
                then recall instead of reading files."
            .to_owned();
    }
    match counts(&ctx.data_dir.join(STATS_FILE)) {
        Some((files, symbols)) => format!(
            "pn-ultramemory: this repository is indexed ({files} files, {symbols} symbols). \
             Use the recall tool instead of reading files; expand for full source."
        ),
        None => "pn-ultramemory: this repository is indexed. Use the recall tool instead of \
                 reading files; expand for full source."
            .to_owned(),
    }
}

/// The counts in the optional statistics file, if it is there and says what it should.
fn counts(path: &Path) -> Option<(u64, u64)> {
    let text = std::fs::read_to_string(path).ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    Some((
        value.get("files")?.as_u64()?,
        value.get("symbols")?.as_u64()?,
    ))
}

/// The advice before an expensive tool call, or nothing.
fn pre_tool_line(input: &str, ctx: &HookContext) -> Option<String> {
    let call: Value = serde_json::from_str(input).ok()?;
    let tool = first_string(&call, &["tool_name", "toolName", "tool", "name"])?;
    let empty = Value::Object(serde_json::Map::new());
    let args = [
        "tool_input",
        "toolInput",
        "input",
        "arguments",
        "params",
        "args",
    ]
    .iter()
    .find_map(|key| call.get(*key))
    .filter(|value| value.is_object())
    .unwrap_or(&empty);
    if !is_expensive(&tool, args, ctx) {
        return None;
    }
    if !claim_once(ctx, &session_key(&call)) {
        return None;
    }
    Some(ADVICE.to_owned())
}

/// The first of these keys whose value is a string.
fn first_string(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .map(str::to_ascii_lowercase)
}

/// Whether this call is one a recall would answer for fewer tokens: a search across the whole
/// repository, or a whole very large file.
fn is_expensive(tool: &str, args: &Value, ctx: &HookContext) -> bool {
    let searches = ["grep", "glob", "search", "find", "ripgrep", "rg"];
    if searches.iter().any(|name| tool.contains(name)) {
        return is_wide_search(args, ctx);
    }
    let reads = ["read", "cat", "view", "open_file"];
    if reads.iter().any(|name| tool.contains(name)) {
        return is_whole_large_file(args);
    }
    false
}

/// Whether a search covers the whole repository: no narrowing path, or a pattern that walks
/// everything.
fn is_wide_search(args: &Value, ctx: &HookContext) -> bool {
    let pattern =
        first_string(args, &["pattern", "query", "glob", "regex", "q"]).unwrap_or_default();
    if pattern.contains("**") {
        return true;
    }
    let Some(path) = first_string(args, &["path", "dir", "directory", "cwd", "root"]) else {
        return true;
    };
    let repo = ctx.repo.to_string_lossy().to_ascii_lowercase();
    matches!(path.as_str(), "" | "." | "./" | "/") || path == repo
}

/// Whether a read asks for the whole of a file that is large enough to be worth mentioning.
fn is_whole_large_file(args: &Value) -> bool {
    let windows = [
        "limit",
        "offset",
        "from",
        "to",
        "lines",
        "start_line",
        "end_line",
        "head",
    ];
    if windows
        .iter()
        .any(|key| args.get(*key).is_some_and(|value| !value.is_null()))
    {
        return false;
    }
    let Some(file) = first_string(args, &["file_path", "path", "file", "filename", "target"])
    else {
        return false;
    };
    std::fs::metadata(file).is_ok_and(|meta| meta.is_file() && meta.len() > LARGE_FILE_BYTES)
}

/// The name of this session: what the agent calls it, what the environment calls it, or nothing in
/// particular. Only characters that are safe in a file name are kept.
fn session_key(call: &Value) -> String {
    let named = first_string(
        call,
        &["session_id", "sessionId", "session", "conversation_id"],
    )
    .or_else(|| std::env::var(SESSION_ENV).ok())
    .unwrap_or_default();
    let safe: String = named
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        .take(64)
        .collect();
    if safe.is_empty() {
        "session".to_owned()
    } else {
        safe
    }
}

/// Claims the one piece of advice this session is allowed, returning whether it was still unclaimed.
///
/// The claim is a marker file. When it cannot be written the advice is still given, because saying
/// something useful once too often is better than a hook that fails.
fn claim_once(ctx: &HookContext, key: &str) -> bool {
    let dir = ctx.data_dir.join("hooks");
    let marker = dir.join(format!("advised-{key}"));
    if marker.exists() {
        return false;
    }
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(&marker, key);
    true
}

#[cfg(test)]
mod tests {
    use super::{CONTEXT_KEY, HookContext, NOTHING, run_hook};

    /// A context in a temporary directory, with no index in it.
    fn context() -> (tempfile::TempDir, HookContext) {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let ctx = HookContext::new(dir.path().join("repo"), dir.path().join("data"));
        (dir, ctx)
    }

    /// The session line always says something, and it is short.
    #[test]
    fn a_session_always_gets_one_short_line() {
        let (_dir, ctx) = context();
        let answer = run_hook("session-start", "", &ctx);
        let value: serde_json::Value = serde_json::from_str(&answer).expect("one JSON object");
        let text = value[CONTEXT_KEY].as_str().expect("a string");
        assert!(text.contains("pn-ultramemory"), "{text}");
        assert!(text.len() < 240, "{} characters is too many", text.len());
        assert!(!answer.contains('\n'), "the answer must be one line");
    }

    /// Reserved and unknown events say nothing at all.
    #[test]
    fn unknown_events_say_nothing() {
        let (_dir, ctx) = context();
        for event in [
            "post-tool",
            "",
            "pre_tool",
            "session-end",
            "../../etc/passwd",
        ] {
            assert_eq!(run_hook(event, "{}", &ctx), NOTHING, "{event}");
        }
    }

    /// Hooks that are switched off say nothing, whatever is asked of them.
    #[test]
    fn disabled_hooks_say_nothing() {
        let (_dir, mut ctx) = context();
        ctx.enabled = false;
        assert_eq!(run_hook("session-start", "", &ctx), NOTHING);
        assert_eq!(
            run_hook("pre-tool", r#"{"tool_name":"Grep"}"#, &ctx),
            NOTHING
        );
    }
}
