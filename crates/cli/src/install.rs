// SPDX-License-Identifier: Apache-2.0
//! Registers and unregisters this tool in the configuration files of coding agents.
//!
//! # Role in the architecture
//! An exit adapter that owns one narrow piece of the user's machine: the single entry named after
//! this server inside each agent's list of MCP servers. [`crate::agents`] says which file and which
//! key; this module does the editing.
//!
//! # Exact ownership
//! The only thing this module may change is `<servers key>.<server name>`. Everything else -- other
//! servers, other settings, the order of keys, the indentation, the comments, the trailing commas,
//! the line endings and a byte-order mark -- survives untouched, because the edit is *textual*: a
//! hand-written scanner ([`Scan`]) records where the entry is and only those bytes are spliced. The
//! document is never round-tripped through a serializer, which would drop comments and reorder
//! keys. TOML is treated the same way: only the `[<servers key>.<server name>]` table is replaced.
//! `serde_json` is used to decide whether a file is *valid* and what an entry *means*, never to
//! write one.
//!
//! # When a file is written
//! | Situation | Action |
//! |---|---|
//! | The file does not exist | `Create`, a small document holding only our entry |
//! | Our entry is absent | `Update`, inserted beside the other servers |
//! | Our entry already means what we would write | `Unchanged` |
//! | Our entry was edited by hand and still starts this executable | `Skip`, the edit is kept |
//! | Our entry was edited by hand and starts something else | `Update`, it is repointed |
//! | `force` | `Update`, whatever the entry says |
//! | The file is invalid for its format, is a directory, or is a symbolic link out of its directory | `Skip`, with the reason |
//!
//! The rule in the middle is the one that matters: an entry that still starts this executable but
//! says something more (extra arguments, an environment variable) is a deliberate customization and
//! is left alone; an entry that starts something else is stale and is repaired. `force` overrides
//! both, and a dry run decides all of this and writes nothing.
//!
//! # Uninstalling
//! Our entry is cut out, and the object that held it goes too when our entry was the only reason it
//! was there, so a file that read `{}` before the install reads `{}` again. Among the ways to cut
//! the entry out, the one chosen is the one this module would turn *back* into the file as it stands
//! now, which is what makes an install followed by an uninstall byte for byte a no-op. The several
//! ways of writing an empty object cannot be told apart once an entry has been inside one, so they
//! all come back as `{}`. No file is ever deleted.
//!
//! # Safety
//! A file whose content really changes is first copied to `<path><BACKUP_SUFFIX>`. The new content
//! is written to a temporary file in the same directory and renamed over the original, so a crash
//! cannot leave a half-written configuration. A symbolic link that points outside its own directory
//! is refused rather than followed, and parent directories are created only for a file that does not
//! exist yet.
//!
//! # Unverified agents
//! An agent whose paths could not be confirmed, or whose real configuration format cannot be edited
//! safely, carries `verified: false` in the table and is never written to unless the user names it.

use std::fmt::Write as _;
use std::ops::Range;
use std::path::{Component, Path, PathBuf};

use serde_json::{Map, Value, json};

use crate::agents::{self, EntryShape, Format, Resolved, Scope};
use crate::error::CliError;

/// The suffix of the backup written before a configuration file is changed.
pub const BACKUP_SUFFIX: &str = ".bak-pn-ultramemory";

/// The name this server is registered under when nothing else is asked for.
pub const DEFAULT_SERVER_NAME: &str = "pn-ultramemory";

/// The command written when the path of this executable cannot be read.
const FALLBACK_COMMAND: &str = "pn-ultramemory";

/// What to install, where, and how carefully.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallOptions {
    /// The agents to configure, by id. Empty means every agent that was detected.
    pub agents: Vec<String>,
    /// Whether to edit the user's files or the repository's.
    pub scope: Scope,
    /// Decide everything, write nothing.
    pub dry_run: bool,
    /// Replace an entry that was edited by hand.
    pub force: bool,
    /// The name of the server inside the agent's configuration.
    pub server_name: String,
    /// The command that starts the server. Defaults to the path of this executable.
    pub command: Option<String>,
    /// Add `--repo <repository>` to the arguments, so the server always serves this repository.
    pub pin_repo: bool,
}

impl Default for InstallOptions {
    fn default() -> Self {
        Self {
            agents: Vec::new(),
            scope: Scope::User,
            dry_run: false,
            force: false,
            server_name: DEFAULT_SERVER_NAME.to_owned(),
            command: None,
            pin_repo: false,
        }
    }
}

/// What happened, or would happen, to one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// The file did not exist and was written.
    Create,
    /// The file existed and its content changed.
    Update,
    /// The file already means exactly what we would write.
    Unchanged,
    /// The file was left alone, for the reason given.
    Skip(String),
}

impl Action {
    /// The lowercase name used in reports.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Update => "update",
            Self::Unchanged => "unchanged",
            Self::Skip(_) => "skip",
        }
    }

    /// Whether this action changed, or would change, the file.
    #[must_use]
    pub const fn is_change(&self) -> bool {
        matches!(self, Self::Create | Self::Update)
    }
}

/// One file, and what was done to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// The id of the agent that reads the file.
    pub agent: String,
    /// The file.
    pub path: PathBuf,
    /// What happened to it.
    pub action: Action,
    /// The region that changes, as it was. `None` when nothing was there.
    pub before: Option<String>,
    /// The region that changes, as it now is. `None` when it was removed.
    pub after: Option<String>,
}

/// Everything that happened, in a deterministic order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallReport {
    /// One entry per file considered, sorted by agent id and then by path.
    pub changes: Vec<Change>,
}

impl InstallReport {
    /// The report as JSON, for `--format json`.
    #[must_use]
    pub fn to_value(&self) -> Value {
        let changes: Vec<Value> = self
            .changes
            .iter()
            .map(|change| {
                let mut entry = Map::new();
                entry.insert("agent".to_owned(), json!(change.agent));
                entry.insert("path".to_owned(), json!(change.path.to_string_lossy()));
                entry.insert("action".to_owned(), json!(change.action.as_str()));
                if let Action::Skip(reason) = &change.action {
                    entry.insert("reason".to_owned(), json!(reason));
                }
                entry.insert("before".to_owned(), json!(change.before));
                entry.insert("after".to_owned(), json!(change.after));
                Value::Object(entry)
            })
            .collect();
        json!({ "changed": self.changed(), "changes": changes })
    }

    /// How many files changed, or would change.
    #[must_use]
    pub fn changed(&self) -> usize {
        self.changes
            .iter()
            .filter(|change| change.action.is_change())
            .count()
    }

    /// One line per file, then a total.
    #[must_use]
    pub fn summary(&self) -> String {
        let mut out = String::new();
        for change in &self.changes {
            let path = change.path.display();
            let _ = write!(
                out,
                "{:<10} {:<16} {path}",
                change.action.as_str(),
                change.agent
            );
            if let Action::Skip(reason) = &change.action {
                let _ = write!(out, "  ({reason})");
            }
            out.push('\n');
        }
        let _ = write!(
            out,
            "{} of {} file(s) changed",
            self.changed(),
            self.changes.len()
        );
        out
    }
}

/// Registers this tool with the chosen agents.
///
/// # Errors
/// Returns an invalid-request error when an agent id or the server name is not usable. A file that
/// cannot be read or written is reported as [`Action::Skip`] instead, so one bad file never stops
/// the others.
pub fn install(repo: &Path, options: &InstallOptions) -> Result<InstallReport, CliError> {
    run(repo, options, false)
}

/// Removes this tool from the chosen agents, and nothing else. See the module documentation for
/// exactly how much is removed.
///
/// # Errors
/// The same as [`install`].
pub fn uninstall(repo: &Path, options: &InstallOptions) -> Result<InstallReport, CliError> {
    run(repo, options, true)
}

/// Whether one file already holds our entry, without changing anything.
///
/// # Errors
/// Returns the reason the file could not be read or is not valid for its format.
pub fn entry_state(target: &Resolved, server_name: &str) -> Result<Option<bool>, String> {
    match read_existing(&target.path)? {
        None => Ok(None),
        Some(text) => {
            locate_entry(&text, target, server_name.trim()).map(|found| Some(found.is_some()))
        }
    }
}

/// The shared body of [`install`] and [`uninstall`].
fn run(repo: &Path, options: &InstallOptions, remove: bool) -> Result<InstallReport, CliError> {
    let name = options.server_name.trim();
    if name.is_empty() {
        return Err(CliError::invalid(
            "the server name cannot be empty; it is the name the agent shows for this server, as \
             in `--name pn-ultramemory`, and leaving it off uses that name",
        ));
    }
    let command = match &options.command {
        Some(command) => command.clone(),
        None => std::env::current_exe().map_or_else(
            |_| FALLBACK_COMMAND.to_owned(),
            |path| path.to_string_lossy().into_owned(),
        ),
    };
    let args = if options.pin_repo {
        vec![
            "--repo".to_owned(),
            repo.to_string_lossy().into_owned(),
            "serve".to_owned(),
        ]
    } else {
        vec!["serve".to_owned()]
    };
    let mut changes = Vec::new();
    for (agent, targets) in select(repo, options)? {
        if targets.is_empty() {
            changes.push(Change {
                agent: agent.to_owned(),
                path: PathBuf::new(),
                action: Action::Skip(format!("no {} configuration file", options.scope.as_str())),
                before: None,
                after: None,
            });
            continue;
        }
        for target in targets {
            changes.push(apply(&target, options, &command, &args, remove));
        }
    }
    changes.sort_by(|left, right| (&left.agent, &left.path).cmp(&(&right.agent, &right.path)));
    Ok(InstallReport { changes })
}

/// The agents to touch and, for each, the files of the requested scope.
///
/// Naming an agent opts in to an unverified one; detection never does.
///
/// # Errors
/// Returns an invalid-request error for an agent id that is not in the table.
fn select(
    repo: &Path,
    options: &InstallOptions,
) -> Result<Vec<(&'static str, Vec<Resolved>)>, CliError> {
    let in_scope = |agent: &'static agents::AgentDef| -> Vec<Resolved> {
        agents::resolved(agent, repo)
            .into_iter()
            .filter(|target| target.scope == options.scope)
            .collect()
    };
    if options.agents.is_empty() {
        return Ok(agents::detect(repo)
            .into_iter()
            .filter(|detected| detected.agent.verified)
            .map(|detected| (detected.agent.id, in_scope(detected.agent)))
            .filter(|(_, targets)| !targets.is_empty())
            .collect());
    }
    let mut names: Vec<&str> = options.agents.iter().map(String::as_str).collect();
    names.sort_unstable();
    names.dedup();
    let mut out = Vec::new();
    for name in names {
        let agent = agents::by_id(name).ok_or_else(|| {
            let known: Vec<&str> = agents::all().iter().map(|agent| agent.id).collect();
            CliError::invalid(format!(
                "unknown agent `{name}`; known agents: {}",
                known.join(", ")
            ))
        })?;
        out.push((agent.id, in_scope(agent)));
    }
    Ok(out)
}

/// Decides what to do with one file and, unless `dry_run` is set, does it.
fn apply(
    target: &Resolved,
    options: &InstallOptions,
    command: &str,
    args: &[String],
    remove: bool,
) -> Change {
    let change = |action: Action, before: Option<String>, after: Option<String>| Change {
        agent: target.agent_id.to_owned(),
        path: target.path.clone(),
        action,
        before,
        after,
    };
    match plan(target, options, command, args, remove) {
        Err(reason) | Ok(Plan::Skip(reason)) => change(Action::Skip(reason), None, None),
        Ok(Plan::Unchanged) => change(Action::Unchanged, None, None),
        Ok(Plan::Write {
            content,
            created,
            before,
            after,
        }) => {
            if !options.dry_run {
                if let Err(reason) = write_file(&target.path, &content, created) {
                    return change(Action::Skip(reason), None, None);
                }
            }
            change(
                if created {
                    Action::Create
                } else {
                    Action::Update
                },
                before,
                after,
            )
        }
    }
}

/// What one file needs.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Plan {
    /// Nothing to do.
    Unchanged,
    /// Do not touch it, and why.
    Skip(String),
    /// Write this content.
    Write {
        /// The whole new content of the file.
        content: String,
        /// Whether the file has to be created.
        created: bool,
        /// The region that changes, as it is now.
        before: Option<String>,
        /// The region that changes, as it will be.
        after: Option<String>,
    },
}

/// Works out the plan for one file without touching it.
///
/// # Errors
/// Returns the reason the file must be skipped when the problem is the path itself.
fn plan(
    target: &Resolved,
    options: &InstallOptions,
    command: &str,
    args: &[String],
    remove: bool,
) -> Result<Plan, String> {
    check_path(&target.path)?;
    let name = options.server_name.trim();
    let Some(text) = read_existing(&target.path)? else {
        if remove {
            return Ok(Plan::Skip("there is no such file".to_owned()));
        }
        let fresh = fresh_document(target, name, command, args);
        return Ok(Plan::Write {
            content: fresh.clone(),
            created: true,
            before: None,
            after: Some(fresh),
        });
    };
    if remove {
        remove_entry(&text, target, name, command, args)
    } else {
        write_entry(&text, target, name, command, args, options.force)
    }
}

/// Refuses a path this tool must not write to.
///
/// # Errors
/// Returns a reason when the path is a directory, or a symbolic link that leaves its own directory.
fn check_path(path: &Path) -> Result<(), String> {
    let Ok(meta) = path.symlink_metadata() else {
        return Ok(());
    };
    if meta.is_dir() {
        return Err(format!("`{}` is a directory", path.display()));
    }
    if meta.file_type().is_symlink() {
        let link = std::fs::read_link(path)
            .map_err(|error| format!("cannot read the link `{}`: {error}", path.display()))?;
        if link.is_absolute()
            || link
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
        {
            return Err(format!(
                "`{}` is a symbolic link that leaves its directory",
                path.display()
            ));
        }
    }
    Ok(())
}

/// Reads a file, returning `None` when it is not there.
///
/// # Errors
/// Returns a reason when the file exists but cannot be read as UTF-8.
fn read_existing(path: &Path) -> Result<Option<String>, String> {
    match std::fs::read(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("cannot read `{}`: {error}", path.display())),
        Ok(bytes) => String::from_utf8(bytes)
            .map(Some)
            .map_err(|_| format!("`{}` is not valid UTF-8", path.display())),
    }
}

/// Backs up, then writes through a temporary file in the same directory and renames it over the
/// original, which is atomic on every platform this tool supports.
///
/// # Errors
/// Returns a reason when any step fails.
fn write_file(path: &Path, content: &str, created: bool) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("`{}` has no directory", path.display()))?;
    if created {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create `{}`: {error}", parent.display()))?;
    } else {
        let backup = with_suffix(path, BACKUP_SUFFIX);
        std::fs::copy(path, &backup)
            .map_err(|error| format!("cannot write `{}`: {error}", backup.display()))?;
    }
    let temp = parent.join(format!(".pn-ultramemory-{}.tmp", std::process::id()));
    std::fs::write(&temp, content)
        .map_err(|error| format!("cannot write `{}`: {error}", temp.display()))?;
    std::fs::rename(&temp, path).map_err(|error| {
        let _ = std::fs::remove_file(&temp);
        format!("cannot replace `{}`: {error}", path.display())
    })
}

/// A path with a suffix appended to its file name.
fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

/// The value of our entry, in the shape the agent expects.
fn entry_value(shape: EntryShape, command: &str, args: &[String]) -> Value {
    match shape {
        EntryShape::CommandArgs => json!({ "command": command, "args": args }),
        EntryShape::StdioCommandArgs => {
            json!({ "type": "stdio", "command": command, "args": args })
        }
        EntryShape::LocalCommandList => {
            let mut parts = vec![command.to_owned()];
            parts.extend(args.iter().cloned());
            json!({ "type": "local", "command": parts, "enabled": true })
        }
    }
}

/// The command an entry starts, whichever shape it has.
fn entry_command(value: &Value) -> Option<String> {
    match value.get("command")? {
        Value::String(text) => Some(text.clone()),
        Value::Array(parts) => parts.first()?.as_str().map(ToOwned::to_owned),
        _ => None,
    }
}

/// The whole content of a configuration file that holds nothing but our entry.
fn fresh_document(target: &Resolved, name: &str, command: &str, args: &[String]) -> String {
    let value = entry_value(target.entry, command, args);
    if target.format == Format::Toml {
        return toml_table(target.servers_key, name, &value);
    }
    let mut servers = Map::new();
    servers.insert(name.to_owned(), value);
    let mut root = Map::new();
    root.insert(target.servers_key.to_owned(), Value::Object(servers));
    format!("{}\n", render_json(&Value::Object(root), "  ", "", "\n"))
}

/// A JSON string literal, escaped by the serializer so no quoting rule is reinvented.
fn json_string(text: &str) -> String {
    Value::String(text.to_owned()).to_string()
}

/// Renders a value the way a configuration file is written by hand: objects over several lines,
/// arrays of plain values on one. `base` is the indentation of the line the value starts on.
fn render_json(value: &Value, unit: &str, base: &str, nl: &str) -> String {
    match value {
        Value::Object(map) if !map.is_empty() => {
            let inner = format!("{base}{unit}");
            let mut out = format!("{{{nl}");
            for (index, (key, item)) in map.iter().enumerate() {
                if index > 0 {
                    let _ = write!(out, ",{nl}");
                }
                let rendered = render_json(item, unit, &inner, nl);
                let _ = write!(out, "{inner}{}: {rendered}", json_string(key));
            }
            let _ = write!(out, "{nl}{base}}}");
            out
        }
        Value::Array(items)
            if items
                .iter()
                .all(|item| !item.is_object() && !item.is_array()) =>
        {
            let parts: Vec<String> = items.iter().map(ToString::to_string).collect();
            format!("[{}]", parts.join(", "))
        }
        other => other.to_string(),
    }
}

/// The string one indentation level adds in this file: a tab, or however many spaces the first
/// indented line uses. Two spaces when the file has no indented line to learn from.
fn indent_unit(text: &str) -> String {
    for line in text.lines() {
        let indent: String = line
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
        if indent.is_empty() || indent.len() == line.len() {
            continue;
        }
        return if indent.starts_with('\t') {
            "\t".to_owned()
        } else {
            indent
        };
    }
    "  ".to_owned()
}

/// The line ending this file uses.
fn newline_of(text: &str) -> &'static str {
    if text.contains("\r\n") { "\r\n" } else { "\n" }
}

/// The same text with the line endings the file uses.
fn with_newlines(text: &str, nl: &str) -> String {
    if nl == "\n" {
        text.to_owned()
    } else {
        text.replace('\n', nl)
    }
}

/// The indentation of the line that byte `at` is on, or nothing when text precedes it there.
fn line_indent(text: &str, at: usize) -> String {
    let start = text[..at].rfind('\n').map_or(0, |index| index + 1);
    text[start..at]
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect()
}

/// The length of a byte-order mark at the start of the text, which must be preserved.
fn bom_len(text: &str) -> usize {
    usize::from(text.starts_with('\u{feff}')) * '\u{feff}'.len_utf8()
}

/// Replaces `range` with `patch`, keeping every other byte.
fn splice(text: &str, range: &Range<usize>, patch: &str) -> String {
    let mut out = String::with_capacity(text.len() + patch.len());
    out.push_str(&text[..range.start]);
    out.push_str(patch);
    out.push_str(&text[range.end..]);
    out
}

/// A member of a JSON object, with the spans needed to splice it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Member {
    /// The key, unescaped.
    key: String,
    /// From the opening quote of the key to the last byte of the value.
    span: Range<usize>,
    /// Just the value.
    value: Range<usize>,
}

/// A JSON object found in the text.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Obj {
    /// The index of the `{`.
    open: usize,
    /// The index of the `}`.
    close: usize,
    /// Its members, in file order.
    members: Vec<Member>,
}

/// A hand-written scanner that records *where* things are instead of what they mean. It tracks
/// strings and, for JSONC, comments, and it is only ever run on text that [`read_json`] has already
/// found valid, so its errors are about a structure this tool cannot edit rather than about syntax.
struct Scan<'a> {
    /// The whole file.
    text: &'a str,
    /// The byte offset of the next character.
    pos: usize,
    /// Whether comments are allowed.
    jsonc: bool,
}

impl<'a> Scan<'a> {
    /// A scanner positioned at `pos`.
    const fn new(text: &'a str, pos: usize, jsonc: bool) -> Self {
        Self { text, pos, jsonc }
    }

    /// The byte at the cursor.
    fn peek(&self) -> Option<u8> {
        self.text.as_bytes().get(self.pos).copied()
    }

    /// The problem, named with the position it was found at.
    fn fail<T>(&self, what: &str) -> Result<T, String> {
        Err(format!("{what} at byte {}", self.pos))
    }

    /// Skips whitespace and, in JSONC, comments.
    ///
    /// # Errors
    /// Returns a message for an unterminated block comment or a stray slash.
    fn trivia(&mut self) -> Result<(), String> {
        loop {
            match self.peek() {
                Some(b' ' | b'\t' | b'\r' | b'\n') => self.pos += 1,
                Some(b'/') if self.jsonc => match self.text.as_bytes().get(self.pos + 1) {
                    Some(b'/') => {
                        let rest = &self.text[self.pos..];
                        self.pos += rest.find('\n').unwrap_or(rest.len());
                    }
                    Some(b'*') => {
                        let Some(at) = self.text[self.pos + 2..].find("*/") else {
                            return self.fail("an unterminated comment");
                        };
                        self.pos += at + 4;
                    }
                    _ => return self.fail("a stray `/`"),
                },
                _ => return Ok(()),
            }
        }
    }

    /// Reads a string literal and returns its unescaped text.
    ///
    /// # Errors
    /// Returns a message for anything that is not a complete, well-formed string.
    fn string(&mut self) -> Result<String, String> {
        if self.peek() != Some(b'"') {
            return self.fail("expected a string");
        }
        let start = self.pos;
        self.pos += 1;
        while let Some(byte) = self.peek() {
            match byte {
                b'"' => {
                    self.pos += 1;
                    return serde_json::from_str::<String>(&self.text[start..self.pos])
                        .map_err(|error| format!("a bad string at byte {start}: {error}"));
                }
                b'\\' => {
                    self.pos += 1;
                    let width = self.text[self.pos..]
                        .chars()
                        .next()
                        .map_or(1, char::len_utf8);
                    self.pos += width;
                }
                _ => {
                    let width = self.text[self.pos..]
                        .chars()
                        .next()
                        .map_or(1, char::len_utf8);
                    self.pos += width;
                }
            }
        }
        self.fail("an unterminated string")
    }

    /// Skips any value, leaving the cursor just after it.
    ///
    /// # Errors
    /// Returns a message when the text is not a value.
    fn value(&mut self) -> Result<(), String> {
        let rest = &self.text[self.pos..];
        for word in ["true", "false", "null"] {
            if rest.starts_with(word) {
                self.pos += word.len();
                return Ok(());
            }
        }
        match self.peek() {
            Some(b'{') => self.object().map(|_| ()),
            Some(b'[') => self.array(),
            Some(b'"') => self.string().map(|_| ()),
            Some(b'-' | b'0'..=b'9') => {
                let len = rest
                    .find(|c: char| !matches!(c, '-' | '+' | '.' | 'e' | 'E' | '0'..='9'))
                    .unwrap_or(rest.len());
                self.pos += len;
                Ok(())
            }
            _ => self.fail("expected a value"),
        }
    }

    /// Skips an array.
    ///
    /// # Errors
    /// Returns a message for an unterminated or malformed array.
    fn array(&mut self) -> Result<(), String> {
        self.pos += 1;
        loop {
            self.trivia()?;
            match self.peek() {
                Some(b']') => {
                    self.pos += 1;
                    return Ok(());
                }
                Some(b',') => self.pos += 1,
                Some(_) => self.value()?,
                None => return self.fail("an unterminated array"),
            }
        }
    }

    /// Reads an object and every one of its members, with their spans.
    ///
    /// # Errors
    /// Returns a message for an unterminated or malformed object.
    fn object(&mut self) -> Result<Obj, String> {
        let open = self.pos;
        self.pos += 1;
        let mut members = Vec::new();
        loop {
            self.trivia()?;
            if self.peek() == Some(b'}') {
                let close = self.pos;
                self.pos += 1;
                return Ok(Obj {
                    open,
                    close,
                    members,
                });
            }
            let start = self.pos;
            let key = self.string()?;
            self.trivia()?;
            if self.peek() != Some(b':') {
                return self.fail("expected `:`");
            }
            self.pos += 1;
            self.trivia()?;
            let value_start = self.pos;
            self.value()?;
            members.push(Member {
                key,
                span: start..self.pos,
                value: value_start..self.pos,
            });
            self.trivia()?;
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {}
                _ => return self.fail("expected `,` or `}`"),
            }
        }
    }
}

/// Turns JSONC into JSON a strict parser accepts: comments and trailing commas become whitespace,
/// on the same lines, so a parse error still names the line the user has to look at.
///
/// Comments go first, because a comma is trailing when nothing but a comment and a closing bracket
/// follows it.
fn strip_jsonc(text: &str) -> String {
    blank_trailing_commas(&blank_comments(text))
}

/// Where [`blank_comments`] is while reading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Blank {
    /// Outside a string and outside a comment.
    Code,
    /// Just after a slash, which may open a comment.
    Slash,
    /// Inside a string.
    Text,
    /// Just after a backslash inside a string.
    Escape,
    /// Inside a comment that ends with the line.
    Line,
    /// Inside a comment that ends with a star and a slash.
    Block,
    /// Inside such a comment, just after a star.
    Star,
}

/// A space, unless the character is a newline: the line numbering has to survive.
fn blanked(c: char) -> char {
    if c == '\n' { '\n' } else { ' ' }
}

/// Replaces every comment with whitespace and leaves every other character alone.
fn blank_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut state = Blank::Code;
    for c in text.chars() {
        state = match state {
            Blank::Code if c == '"' => {
                out.push(c);
                Blank::Text
            }
            Blank::Code if c == '/' => Blank::Slash,
            Blank::Code => {
                out.push(c);
                Blank::Code
            }
            Blank::Slash if c == '/' || c == '*' => {
                out.push_str("  ");
                if c == '/' { Blank::Line } else { Blank::Block }
            }
            Blank::Slash => {
                out.push('/');
                out.push(c);
                if c == '"' { Blank::Text } else { Blank::Code }
            }
            Blank::Text => {
                out.push(c);
                match c {
                    '\\' => Blank::Escape,
                    '"' => Blank::Code,
                    _ => Blank::Text,
                }
            }
            Blank::Escape => {
                out.push(c);
                Blank::Text
            }
            Blank::Line => {
                out.push(blanked(c));
                if c == '\n' { Blank::Code } else { Blank::Line }
            }
            Blank::Block | Blank::Star => {
                out.push(blanked(c));
                match c {
                    '/' if state == Blank::Star => Blank::Code,
                    '*' => Blank::Star,
                    _ => Blank::Block,
                }
            }
        };
    }
    if state == Blank::Slash {
        out.push('/');
    }
    out
}

/// Replaces a comma that nothing but a closing bracket follows with a space.
fn blank_trailing_commas(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut in_string = false;
    let mut escape = false;
    for (index, &c) in chars.iter().enumerate() {
        if in_string {
            out.push(c);
            if escape {
                escape = false;
            } else if c == '\\' {
                escape = true;
            } else if c == '"' {
                in_string = false;
            }
        } else if c == '"' {
            in_string = true;
            out.push(c);
        } else if c == ',' {
            let next = chars[index + 1..].iter().find(|c| !c.is_whitespace());
            out.push(if matches!(next, Some('}' | ']')) {
                ' '
            } else {
                ','
            });
        } else {
            out.push(c);
        }
    }
    out
}

/// Checks that a document is a valid object for its format and returns both what it means and where
/// its members are. `serde_json` decides validity; the scanner only finds the spans.
///
/// # Errors
/// Returns the reason the file cannot be edited.
fn read_json(text: &str, jsonc: bool) -> Result<(Map<String, Value>, Obj), String> {
    let body = &text[bom_len(text)..];
    let source = if jsonc {
        strip_jsonc(body)
    } else {
        body.to_owned()
    };
    let parsed: Value = serde_json::from_str(&source).map_err(|error| {
        format!(
            "this is not valid {}: {error}",
            if jsonc { "JSONC" } else { "JSON" }
        )
    })?;
    let Value::Object(map) = parsed else {
        return Err("the top level is not an object".to_owned());
    };
    let mut scan = Scan::new(text, bom_len(text), jsonc);
    scan.trivia()?;
    let root = scan.object()?;
    Ok((map, root))
}

/// The object that starts at `at`.
///
/// # Errors
/// Returns a message when the value there is not an object.
fn object_at(text: &str, at: usize, jsonc: bool, what: &str) -> Result<Obj, String> {
    let mut scan = Scan::new(text, at, jsonc);
    if scan.peek() != Some(b'{') {
        return Err(format!("`{what}` is not an object"));
    }
    scan.object()
}

/// Where our entry sits in a file.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Found {
    /// The whole entry: its key and its value, or for TOML the whole table.
    member: Range<usize>,
    /// Just the value, or for TOML the whole table again.
    value: Range<usize>,
}

/// Finds our entry without changing anything.
///
/// # Errors
/// Returns the reason the file cannot be edited: it is not valid for its format, or the key that
/// should hold the servers holds something else.
fn locate_entry(text: &str, target: &Resolved, name: &str) -> Result<Option<Found>, String> {
    if target.format == Format::Toml {
        return toml_locate(text, target.servers_key, name);
    }
    let jsonc = target.format == Format::Jsonc;
    if text[bom_len(text)..].trim().is_empty() {
        return Ok(None);
    }
    let (_, root) = read_json(text, jsonc)?;
    let Some(member) = root.members.iter().find(|m| m.key == target.servers_key) else {
        return Ok(None);
    };
    let servers = object_at(text, member.value.start, jsonc, target.servers_key)?;
    Ok(servers
        .members
        .iter()
        .find(|m| m.key == name)
        .map(|entry| Found {
            member: entry.span.clone(),
            value: entry.value.clone(),
        }))
}

/// Writes or refreshes our entry in a file that already exists.
///
/// # Errors
/// Returns the reason the file cannot be edited.
fn write_entry(
    text: &str,
    target: &Resolved,
    name: &str,
    command: &str,
    args: &[String],
    force: bool,
) -> Result<Plan, String> {
    let desired = entry_value(target.entry, command, args);
    if target.format == Format::Toml {
        return toml_write(text, target, name, &desired, force);
    }
    let jsonc = target.format == Format::Jsonc;
    let unit = indent_unit(text);
    let nl = newline_of(text);
    let head = &text[..bom_len(text)];
    if text[bom_len(text)..].trim().is_empty() {
        let fresh = fresh_document(target, name, command, args);
        let content = format!("{head}{fresh}");
        return Ok(Plan::Write {
            content,
            created: false,
            before: None,
            after: Some(fresh),
        });
    }
    let (parsed, root) = read_json(text, jsonc)?;
    let Some(member) = root.members.iter().find(|m| m.key == target.servers_key) else {
        let mut servers = Map::new();
        servers.insert(name.to_owned(), desired);
        let base = member_base(text, &root, &unit);
        let value = render_json(&Value::Object(servers), &unit, &base, nl);
        let patch = format!("{}: {value}", json_string(target.servers_key));
        let (range, insert) = insert_member(text, &root, &patch, &unit, nl);
        return Ok(Plan::Write {
            content: splice(text, &range, &insert),
            created: false,
            before: None,
            after: Some(patch),
        });
    };
    let servers = object_at(text, member.value.start, jsonc, target.servers_key)?;
    let existing = parsed
        .get(target.servers_key)
        .and_then(|servers| servers.get(name));
    let Some(entry) = servers.members.iter().find(|m| m.key == name) else {
        let base = member_base(text, &servers, &unit);
        let patch = format!(
            "{}: {}",
            json_string(name),
            render_json(&desired, &unit, &base, nl)
        );
        let (range, insert) = insert_member(text, &servers, &patch, &unit, nl);
        return Ok(Plan::Write {
            content: splice(text, &range, &insert),
            created: false,
            before: None,
            after: Some(patch),
        });
    };
    if existing == Some(&desired) {
        return Ok(Plan::Unchanged);
    }
    if !force {
        match existing {
            Some(found) if entry_command(found).as_deref() == Some(command) => {
                return Ok(Plan::Skip(kept_by_hand()));
            }
            None => {
                return Ok(Plan::Skip(
                    "the entry cannot be read; pass --force".to_owned(),
                ));
            }
            Some(_) => {}
        }
    }
    let rendered = render_json(&desired, &unit, &line_indent(text, entry.span.start), nl);
    let key = &text[entry.span.start..entry.value.start];
    Ok(Plan::Write {
        content: splice(text, &entry.value, &rendered),
        created: false,
        before: Some(text[entry.span.clone()].to_owned()),
        after: Some(format!("{key}{rendered}")),
    })
}

/// Why a hand-edited entry is left alone.
fn kept_by_hand() -> String {
    "the entry was edited by hand and still starts this executable; pass --force to replace it"
        .to_owned()
}

/// The indentation the members of `obj` sit at: the one they already use, or one level in from the
/// line the object opens on.
fn member_base(text: &str, obj: &Obj, unit: &str) -> String {
    obj.members.first().map_or_else(
        || format!("{}{unit}", line_indent(text, obj.open)),
        |first| line_indent(text, first.span.start),
    )
}

/// The range to replace and the text to put there so that `patch` becomes a member of `obj`.
fn insert_member(
    text: &str,
    obj: &Obj,
    patch: &str,
    unit: &str,
    nl: &str,
) -> (Range<usize>, String) {
    let base = member_base(text, obj, unit);
    if let Some(last) = obj.members.last() {
        return (last.span.end..last.span.end, format!(",{nl}{base}{patch}"));
    }
    let closing = line_indent(text, obj.open);
    if text[obj.open + 1..obj.close].trim().is_empty() {
        return (
            obj.open..obj.close + 1,
            format!("{{{nl}{base}{patch}{nl}{closing}}}"),
        );
    }
    (obj.close..obj.close, format!("{base}{patch}{nl}{closing}"))
}

/// Removes our entry, choosing the way of doing it that [`write_entry`] would turn back into the
/// text we have, which is what makes an install followed by an uninstall a no-op.
///
/// # Errors
/// Returns the reason the file cannot be edited.
fn remove_entry(
    text: &str,
    target: &Resolved,
    name: &str,
    command: &str,
    args: &[String],
) -> Result<Plan, String> {
    let Some(found) = locate_entry(text, target, name)? else {
        return Ok(Plan::Skip("the entry is not there".to_owned()));
    };
    let candidates = removal_candidates(text, target, name)?;
    let exact = candidates.iter().find(|candidate| {
        matches!(
            write_entry(candidate, target, name, command, args, true),
            Ok(Plan::Write { ref content, .. }) if content == text
        )
    });
    let Some(content) = exact.or_else(|| candidates.last()).cloned() else {
        return Ok(Plan::Skip("the entry is not there".to_owned()));
    };
    Ok(Plan::Write {
        content,
        created: false,
        before: Some(text[found.member].to_owned()),
        after: None,
    })
}

/// The ways our entry can be cut out, from the boldest to the most careful. The boldest also drops
/// the object, or the blank line, that exists only because our entry was put there.
///
/// # Errors
/// Returns the reason the file cannot be edited.
fn removal_candidates(text: &str, target: &Resolved, name: &str) -> Result<Vec<String>, String> {
    if target.format == Format::Toml {
        return Ok(toml_removals(text, target.servers_key, name));
    }
    let jsonc = target.format == Format::Jsonc;
    let (_, root) = read_json(text, jsonc)?;
    let Some((key_index, member)) = root
        .members
        .iter()
        .enumerate()
        .find(|(_, m)| m.key == target.servers_key)
    else {
        return Ok(Vec::new());
    };
    let servers = object_at(text, member.value.start, jsonc, target.servers_key)?;
    let Some(entry_index) = servers.members.iter().position(|m| m.key == name) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    if servers.members.len() == 1 {
        let (range, patch) = cut_member(text, &root, key_index);
        out.push(splice(text, &range, &patch));
    }
    let (range, patch) = cut_member(text, &servers, entry_index);
    out.push(splice(text, &range, &patch));
    Ok(out)
}

/// The range to replace and the text to put there so that member `index` of `obj` disappears.
///
/// The only member of an object leaves `{}` behind, so a file that read `{}` before reads `{}`
/// again. Otherwise the comma that joined the member to its neighbour goes with it.
fn cut_member(text: &str, obj: &Obj, index: usize) -> (Range<usize>, String) {
    let member = &obj.members[index];
    if obj.members.len() == 1 {
        let inside = &text[obj.open + 1..obj.close];
        let cut = member.span.start - obj.open - 1..member.span.end - obj.open - 1;
        let rest = format!("{}{}", &inside[..cut.start], &inside[cut.end..]);
        if rest.trim().is_empty() {
            return (obj.open..obj.close + 1, "{}".to_owned());
        }
        return (member.span.clone(), String::new());
    }
    if let Some(next) = obj.members.get(index + 1) {
        return (member.span.start..next.span.start, String::new());
    }
    (
        obj.members[index - 1].span.end..member.span.end,
        String::new(),
    )
}

/// One top-level table of a TOML file.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TomlTable {
    /// The dotted key of the header, with any quotes removed.
    key: Vec<String>,
    /// Whether the header was `[[an array of tables]]`, which is never one of ours.
    array: bool,
    /// From the first byte of the header line to the first byte of the next header, or the end.
    span: Range<usize>,
}

/// Splits a TOML file into its top-level tables.
///
/// Multi-line strings are followed by counting their delimiters on each line, which is enough for a
/// configuration file written by hand; a file that defeats it is refused rather than mangled,
/// because a header line without its closing bracket is an error.
///
/// # Errors
/// Returns a message when a table header has no closing bracket, or cannot be read.
fn toml_tables(text: &str) -> Result<Vec<TomlTable>, String> {
    let mut tables: Vec<TomlTable> = Vec::new();
    let mut offset = 0;
    let mut open: Option<&str> = None;
    for line in text.split_inclusive('\n') {
        let start = offset;
        offset += line.len();
        if let Some(delimiter) = open {
            if line.matches(delimiter).count() % 2 == 1 {
                open = None;
            }
            continue;
        }
        let trimmed = line.trim_start();
        if trimmed.starts_with('[') {
            let array = trimmed.starts_with("[[");
            let (from, close) = if array { (2, "]]") } else { (1, "]") };
            let Some(end) = trimmed.find(close) else {
                return Err(format!("a table header without `{close}` at byte {start}"));
            };
            let key = toml_dotted(&trimmed[from..end])
                .ok_or_else(|| format!("a table header that cannot be read at byte {start}"))?;
            if let Some(previous) = tables.last_mut() {
                previous.span.end = start;
            }
            tables.push(TomlTable {
                key,
                array,
                span: start..text.len(),
            });
        }
        for delimiter in ["\"\"\"", "'''"] {
            if line.matches(delimiter).count() % 2 == 1 {
                open = Some(delimiter);
            }
        }
    }
    Ok(tables)
}

/// Splits a dotted TOML key on the dots that are not inside quotes, and unquotes each part.
fn toml_dotted(header: &str) -> Option<Vec<String>> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    for c in header.chars() {
        match (quote, c) {
            (Some(open), _) if c == open => quote = None,
            (None, '"' | '\'') => quote = Some(c),
            (None, '.') => parts.push(std::mem::take(&mut current).trim().to_owned()),
            _ => current.push(c),
        }
    }
    if quote.is_some() {
        return None;
    }
    parts.push(current.trim().to_owned());
    Some(parts)
}

/// The TOML table of our entry, ready to be spliced in.
fn toml_table(servers_key: &str, name: &str, value: &Value) -> String {
    let mut out = format!("[{servers_key}.{}]\n", toml_key(name));
    if let Value::Object(map) = value {
        for (key, item) in map {
            let _ = writeln!(out, "{key} = {}", toml_value(item));
        }
    }
    out
}

/// A TOML key: bare when it may be, quoted when it may not.
fn toml_key(name: &str) -> String {
    let bare = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'));
    if bare {
        name.to_owned()
    } else {
        json_string(name)
    }
}

/// A TOML value. Only the shapes this tool writes are handled; anything else becomes a string.
fn toml_value(value: &Value) -> String {
    match value {
        Value::Array(items) => {
            let parts: Vec<String> = items.iter().map(toml_value).collect();
            format!("[{}]", parts.join(", "))
        }
        Value::String(text) => json_string(text),
        Value::Bool(flag) => flag.to_string(),
        Value::Number(number) => number.to_string(),
        other => json_string(&other.to_string()),
    }
}

/// Finds the `[<servers key>.<name>]` table.
///
/// # Errors
/// Returns a message when the file cannot be read as TOML tables.
fn toml_locate(text: &str, servers_key: &str, name: &str) -> Result<Option<Found>, String> {
    let wanted = [servers_key.to_owned(), name.to_owned()];
    Ok(toml_tables(text)?
        .into_iter()
        .find(|table| !table.array && table.key == wanted)
        .map(|table| Found {
            member: table.span.clone(),
            value: table.span,
        }))
}

/// The keys of a TOML region, sorted, with its header first: what two tables must share to mean the
/// same thing whatever order they were written in.
fn toml_shape(region: &str) -> Vec<String> {
    let mut lines: Vec<String> = region
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(ToOwned::to_owned)
        .collect();
    if !lines.is_empty() {
        lines[1..].sort_unstable();
    }
    lines
}

/// The command a TOML table starts, if it names one.
fn toml_command(region: &str) -> Option<String> {
    for line in toml_shape(region) {
        let Some(rest) = line.strip_prefix("command") else {
            continue;
        };
        let value = rest.trim_start().strip_prefix('=')?.trim();
        if let Some(inner) = value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')) {
            return Some(inner.to_owned());
        }
        return serde_json::from_str::<String>(value).ok();
    }
    None
}

/// Writes or refreshes the `[<servers key>.<name>]` table, leaving every other byte alone.
///
/// # Errors
/// Returns the reason the file cannot be edited.
fn toml_write(
    text: &str,
    target: &Resolved,
    name: &str,
    desired: &Value,
    force: bool,
) -> Result<Plan, String> {
    let nl = newline_of(text);
    let rendered = with_newlines(&toml_table(target.servers_key, name, desired), nl);
    let Some(found) = toml_locate(text, target.servers_key, name)? else {
        let mut content = text.to_owned();
        if !content.is_empty() {
            if !content.ends_with('\n') {
                content.push_str(nl);
            }
            if !content.ends_with(&format!("{nl}{nl}")) {
                content.push_str(nl);
            }
        }
        content.push_str(&rendered);
        return Ok(Plan::Write {
            content,
            created: false,
            before: None,
            after: Some(rendered),
        });
    };
    let region = &text[found.value.clone()];
    if toml_shape(region) == toml_shape(&rendered) {
        return Ok(Plan::Unchanged);
    }
    if !force && toml_command(region) == entry_command(desired) {
        return Ok(Plan::Skip(kept_by_hand()));
    }
    let kept = region.len() - region.trim_end().len();
    let patch = format!("{}{}", rendered.trim_end(), &region[region.len() - kept..]);
    Ok(Plan::Write {
        content: splice(text, &found.value, &patch),
        created: false,
        before: Some(region.to_owned()),
        after: Some(patch),
    })
}

/// The ways the table can be cut out: with the blank line that separates it, or without.
fn toml_removals(text: &str, servers_key: &str, name: &str) -> Vec<String> {
    let Ok(Some(found)) = toml_locate(text, servers_key, name) else {
        return Vec::new();
    };
    let nl = newline_of(text);
    let mut out = Vec::new();
    if text[..found.value.start].ends_with(&format!("{nl}{nl}")) {
        let range = (found.value.start - nl.len())..found.value.end;
        out.push(splice(text, &range, ""));
    }
    out.push(splice(text, &found.value, ""));
    out
}
