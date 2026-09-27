// SPDX-License-Identifier: Apache-2.0
//! Where every coding agent keeps its MCP server configuration.
//!
//! # Role in the architecture
//! Pure data plus pure path arithmetic, used by [`crate::install`] to know which file to edit and
//! by [`crate::doctor`] to report what is present. It performs no network access and, apart from
//! [`detect`], no I/O at all: [`resolve`] takes the base directories as an argument so it can be
//! tested for every platform from any platform.
//!
//! # The table of agents
//! Every entry records the documentation page it was taken from. A path that could not be found in
//! the agent's own documentation is never written to by default: the entry carries
//! `verified: false`, [`detect`] still reports it, and [`crate::install`] only touches it when the
//! user names that agent explicitly.
//!
//! | Id | Product | User-level file | Project-level file | Key | Format |
//! |---|---|---|---|---|---|
//! | `amp` | Amp | `~/.config/amp/settings.json` | `.amp/settings.json` | `amp.mcpServers` | JSONC |
//! | `claude-code` | Claude Code | `~/.claude.json` | `.mcp.json` | `mcpServers` | JSON |
//! | `cline` | Cline | `~/.cline/mcp.json` (unverified) | none | `mcpServers` | JSON |
//! | `codex` | Codex CLI | `~/.codex/config.toml` | `.codex/config.toml` | `mcp_servers` | TOML |
//! | `crush` | Crush | `~/.config/crush/crush.json` (opt-in) | `.crush.json` | `mcp` | JSONC |
//! | `cursor` | Cursor | `~/.cursor/mcp.json` | `.cursor/mcp.json` | `mcpServers` | JSON |
//! | `gemini` | Gemini CLI | `~/.gemini/settings.json` | `.gemini/settings.json` | `mcpServers` | JSON |
//! | `kiro` | Kiro | `~/.kiro/settings/mcp.json` | `.kiro/settings/mcp.json` | `mcpServers` | JSON |
//! | `opencode` | `opencode` | `~/.config/opencode/opencode.json` | `opencode.json` | `mcp` | JSON |
//! | `vscode-copilot` | Visual Studio Code | `<config>/Code/User/mcp.json` | `.vscode/mcp.json` | `servers` | JSONC |
//! | `windsurf` | Windsurf | `<xdg>/devin/mcp_config.json`, `~/.codeium/windsurf/mcp_config.json` | none | `mcpServers` | JSON |
//! | `zed` | Zed | `~/.config/zed/settings.json` | `.zed/settings.json` | `context_servers` | JSONC |
//!
//! `<config>` is the platform configuration directory: `%APPDATA%` on Windows,
//! `~/Library/Application Support` on macOS and `~/.config` elsewhere. `<xdg>` is
//! `$XDG_CONFIG_HOME` or `<home>/.config`.
//!
//! # Not supported
//! Continue reads YAML (`~/.continue/config.yaml`, key `mcpServers` holding a *list*) and takes
//! one file per server under `.continue/mcpServers/`; this tool edits JSON, JSONC and TOML only.
//! Aider documents no MCP client at all. Neither is in the table.
//!
//! # The shape of an entry
//! Agents disagree about the value: most take `{"command": ..., "args": [...]}`, `opencode` takes
//! `{"type": "local", "command": [...], "enabled": true}` and Crush wants an explicit
//! `"type": "stdio"`. [`EntryShape`] records which, so the installer never writes a shape the
//! agent would reject.
//!
//! # Invariants
//! * Ids are unique, lowercase and stable; they are what the user types.
//! * [`all`] returns the entries sorted by id, so every listing is deterministic.
//! * Every entry has a documentation URL.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Whether a configuration file belongs to the user or to one repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Scope {
    /// One file for the whole machine, under the user's home or configuration directory.
    User,
    /// One file per repository, committed with the code or ignored by it.
    Project,
}

impl Scope {
    /// The lowercase name used on the command line and in reports.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Project => "project",
        }
    }
}

/// The syntax of a configuration file, which decides how it is edited.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Format {
    /// Strict JSON: no comments, no trailing commas.
    Json,
    /// JSON with comments and trailing commas, as several editors accept.
    Jsonc,
    /// TOML tables.
    Toml,
}

impl Format {
    /// The lowercase name used in reports.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Jsonc => "jsonc",
            Self::Toml => "toml",
        }
    }
}

/// The operating-system families whose paths differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Platform {
    /// macOS.
    MacOs,
    /// Linux and the other Unix-like systems.
    Linux,
    /// Windows, where an agent's configuration lives under `%APPDATA%`.
    Windows,
}

impl Platform {
    /// The platform this binary was built for.
    #[must_use]
    pub const fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacOs
        } else if cfg!(windows) {
            Self::Windows
        } else {
            Self::Linux
        }
    }
}

/// The directory a configuration path starts from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Base {
    /// The user's home directory.
    Home,
    /// `$XDG_CONFIG_HOME`, or `<home>/.config` when it is not set, on every platform.
    XdgConfig,
    /// The platform configuration directory: `%APPDATA%` on Windows,
    /// `~/Library/Application Support` on macOS, `~/.config` elsewhere.
    Config,
    /// The root of the repository being configured.
    Repo,
}

/// One platform's location: a base directory and the path under it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Location {
    /// Where the path starts.
    pub base: Base,
    /// The path under the base directory, written with forward slashes.
    pub rel: &'static str,
}

/// Where a configuration file lives on each platform, resolved at runtime by [`resolve`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PathSpec {
    /// The location on macOS, or `None` when the agent has no such file there.
    pub macos: Option<Location>,
    /// The location on Linux and other Unix-like systems.
    pub linux: Option<Location>,
    /// The location on Windows.
    pub windows: Option<Location>,
}

/// One location, on every platform.
const fn same(base: Base, rel: &'static str) -> PathSpec {
    let at = Location { base, rel };
    PathSpec {
        macos: Some(at),
        linux: Some(at),
        windows: Some(at),
    }
}

/// A different location per platform.
const fn per_os(macos: Location, linux: Location, windows: Location) -> PathSpec {
    PathSpec {
        macos: Some(macos),
        linux: Some(linux),
        windows: Some(windows),
    }
}

/// Shorthand for one [`Location`].
const fn at(base: Base, rel: &'static str) -> Location {
    Location { base, rel }
}

/// The base directories [`resolve`] expands a [`PathSpec`] against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bases {
    /// The user's home directory.
    pub home: PathBuf,
    /// `$XDG_CONFIG_HOME` or `<home>/.config`.
    pub xdg_config: PathBuf,
    /// The platform configuration directory.
    pub config: PathBuf,
    /// The root of the repository.
    pub repo: PathBuf,
}

/// The shape of one server entry, because agents disagree about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryShape {
    /// `{"command": "<exe>", "args": ["serve"]}`, what almost every agent expects.
    CommandArgs,
    /// `{"type": "stdio", "command": "<exe>", "args": ["serve"]}`, for agents that require the
    /// transport to be named.
    StdioCommandArgs,
    /// `{"type": "local", "command": ["<exe>", "serve"], "enabled": true}`, the `opencode` shape.
    LocalCommandList,
}

/// One configuration file of one agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfigTarget {
    /// Whether the file is the user's or the repository's.
    pub scope: Scope,
    /// The syntax of the file.
    pub format: Format,
    /// Where the file is, per platform.
    pub path: PathSpec,
    /// The key of the object (or the TOML table) that holds the servers by name.
    pub servers_key: &'static str,
    /// The shape of the value written under `servers_key.<server name>`.
    pub entry: EntryShape,
}

/// One coding agent and every configuration file it reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentDef {
    /// The stable identifier the user types, lowercase.
    pub id: &'static str,
    /// The product name, as its makers write it.
    pub display: &'static str,
    /// The page the paths and the key below were taken from.
    pub doc_url: &'static str,
    /// Whether the paths below come from that page *and* writing them needs no permission. It is
    /// `false` when a path could not be confirmed, or when the agent's current configuration
    /// format is one this tool cannot edit safely. Such an agent is written to only when the user
    /// names it explicitly.
    pub verified: bool,
    /// The files that can be edited, user scope first.
    pub targets: Vec<ConfigTarget>,
}

/// A configuration file of an agent with its path filled in for this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    /// The agent this file belongs to.
    pub agent_id: &'static str,
    /// The product name of that agent.
    pub display: &'static str,
    /// Whether the agent's paths are documented.
    pub verified: bool,
    /// Whether this is the user's file or the repository's.
    pub scope: Scope,
    /// The syntax of the file.
    pub format: Format,
    /// The key that holds the servers.
    pub servers_key: &'static str,
    /// The shape of the entry this file wants.
    pub entry: EntryShape,
    /// The absolute path of the file.
    pub path: PathBuf,
    /// Whether the file itself exists.
    pub exists: bool,
    /// Whether the directory that would contain it exists.
    pub parent_exists: bool,
}

/// An agent that appears to be installed: at least one of its files, or the directory that would
/// hold it, exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detected {
    /// The agent that was found.
    pub agent: &'static AgentDef,
    /// Only the files that exist, or whose directory exists.
    pub targets: Vec<Resolved>,
}

/// Expands a [`PathSpec`] for one platform, or returns `None` when the agent has no such file
/// there.
///
/// # Examples
/// Cursor's user-level file, on a Linux machine whose home directory is `/home/ada`:
///
/// ```text
/// let bases = Bases {
///     home: "/home/ada".into(),
///     xdg_config: "/home/ada/.config".into(),
///     config: "/home/ada/.config".into(),
///     repo: "/work/app".into(),
/// };
/// let cursor = by_id("cursor")?;
/// resolve(&cursor.targets[0].path, Platform::Linux, &bases)
/// // => Some("/home/ada/.cursor/mcp.json")
/// ```
#[must_use]
pub fn resolve(spec: &PathSpec, platform: Platform, bases: &Bases) -> Option<PathBuf> {
    let location = match platform {
        Platform::MacOs => spec.macos,
        Platform::Linux => spec.linux,
        Platform::Windows => spec.windows,
    }?;
    let root = match location.base {
        Base::Home => &bases.home,
        Base::XdgConfig => &bases.xdg_config,
        Base::Config => &bases.config,
        Base::Repo => &bases.repo,
    };
    let mut path = root.clone();
    for part in location.rel.split('/').filter(|part| !part.is_empty()) {
        path.push(part);
    }
    Some(path)
}

/// The base directories of this machine.
///
/// # Errors
/// Returns `None` when neither the home directory nor the configuration directory can be found,
/// which happens only in an environment without a user.
#[must_use]
pub fn bases(repo: &Path) -> Option<Bases> {
    let home = crate::dirs::home_dir()?;
    let xdg_config = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(value) if !value.is_empty() && Path::new(&value).is_absolute() => PathBuf::from(value),
        _ => home.join(".config"),
    };
    let config = crate::dirs::config_dir().unwrap_or_else(|| xdg_config.clone());
    Some(Bases {
        home,
        xdg_config,
        config,
        repo: repo.to_path_buf(),
    })
}

/// Every agent this tool can configure, sorted by id.
#[must_use]
pub fn all() -> &'static [AgentDef] {
    /// Built once, then shared, because [`AgentDef`] owns a growable list of targets.
    static TABLE: OnceLock<Vec<AgentDef>> = OnceLock::new();
    TABLE.get_or_init(build_table)
}

/// The agent with this id, if it is known.
#[must_use]
pub fn by_id(id: &str) -> Option<&'static AgentDef> {
    all().iter().find(|agent| agent.id == id)
}

/// Every file of one agent, with its path filled in for this machine and this repository.
///
/// Files the agent does not have on this platform are left out.
#[must_use]
pub fn resolved(agent: &'static AgentDef, repo: &Path) -> Vec<Resolved> {
    let Some(bases) = bases(repo) else {
        return Vec::new();
    };
    let platform = Platform::current();
    agent
        .targets
        .iter()
        .filter_map(|target| {
            let path = resolve(&target.path, platform, &bases)?;
            let exists = path.symlink_metadata().is_ok();
            let parent_exists = path.parent().is_some_and(Path::is_dir);
            Some(Resolved {
                agent_id: agent.id,
                display: agent.display,
                verified: agent.verified,
                scope: target.scope,
                format: target.format,
                servers_key: target.servers_key,
                entry: target.entry,
                path,
                exists,
                parent_exists,
            })
        })
        .collect()
}

/// Whether one file is evidence that its agent is installed.
///
/// The file itself existing is always evidence. Its directory existing is evidence only when that
/// directory belongs to the agent: the repository root and the home directory exist for everyone,
/// so `.mcp.json` or `~/.claude.json` count only once they are really there, while `~/.cursor` or
/// `.vscode` are proof on their own.
fn is_evidence(target: &Resolved, bases: &Bases) -> bool {
    if target.exists {
        return true;
    }
    let Some(parent) = target.path.parent() else {
        return false;
    };
    target.parent_exists && parent != bases.repo && parent != bases.home
}

/// The agents that appear to be installed: those with at least one configuration file that exists,
/// or that own a directory which exists.
///
/// The result is sorted by agent id and, within an agent, keeps the order of the table.
#[must_use]
pub fn detect(repo: &Path) -> Vec<Detected> {
    let Some(bases) = bases(repo) else {
        return Vec::new();
    };
    all()
        .iter()
        .filter_map(|agent| {
            let targets: Vec<Resolved> = resolved(agent, repo)
                .into_iter()
                .filter(|target| is_evidence(target, &bases))
                .collect();
            if targets.is_empty() {
                None
            } else {
                Some(Detected { agent, targets })
            }
        })
        .collect()
}

/// One configuration file, spelled out so the table below reads as a table.
const fn target(
    scope: Scope,
    format: Format,
    path: PathSpec,
    servers_key: &'static str,
) -> ConfigTarget {
    ConfigTarget {
        scope,
        format,
        path,
        servers_key,
        entry: EntryShape::CommandArgs,
    }
}

/// The same, for a file that wants a different entry shape.
const fn shaped(mut target: ConfigTarget, entry: EntryShape) -> ConfigTarget {
    target.entry = entry;
    target
}
/// The agents that are command-line tools, with the page each entry was read from.
#[allow(
    clippy::too_many_lines,
    reason = "a flat data table is clearer than helpers"
)]
fn command_line_agents() -> Vec<AgentDef> {
    vec![
        // <https://ampcode.com/docs/cli/settings>: user settings are `~/.config/amp/settings.json`
        // or `.jsonc` on macOS and Linux and `%USERPROFILE%\.config\amp\settings.json` on Windows,
        // so the path is the same under the home directory everywhere; the workspace file is
        // `.amp/settings.json`. <https://ampcode.com/docs/customize/mcp>: the key is the flat,
        // dotted `amp.mcpServers`, because "all settings use the `amp.` prefix".
        AgentDef {
            id: "amp",
            display: "Amp",
            doc_url: "https://ampcode.com/docs/customize/mcp",
            verified: true,
            targets: vec![
                target(
                    Scope::User,
                    Format::Jsonc,
                    same(Base::Home, ".config/amp/settings.json"),
                    "amp.mcpServers",
                ),
                target(
                    Scope::Project,
                    Format::Jsonc,
                    same(Base::Repo, ".amp/settings.json"),
                    "amp.mcpServers",
                ),
            ],
        },
        // <https://code.claude.com/docs/en/mcp>: user scope is the top-level `mcpServers` object
        // of `~/.claude.json`; project scope is `.mcp.json` in the repository root.
        AgentDef {
            id: "claude-code",
            display: "Claude Code",
            doc_url: "https://code.claude.com/docs/en/mcp",
            verified: true,
            targets: vec![
                target(
                    Scope::User,
                    Format::Json,
                    same(Base::Home, ".claude.json"),
                    "mcpServers",
                ),
                target(
                    Scope::Project,
                    Format::Json,
                    same(Base::Repo, ".mcp.json"),
                    "mcpServers",
                ),
            ],
        },
        // <https://learn.chatgpt.com/docs/extend/mcp?surface=cli>: `~/.codex/config.toml` by
        // default and `.codex/config.toml` for a trusted project, with one TOML table per server,
        // `[mcp_servers.<name>]`, holding `command` and `args`.
        AgentDef {
            id: "codex",
            display: "Codex CLI",
            doc_url: "https://learn.chatgpt.com/docs/extend/mcp?surface=cli",
            verified: true,
            targets: vec![
                target(
                    Scope::User,
                    Format::Toml,
                    same(Base::Home, ".codex/config.toml"),
                    "mcp_servers",
                ),
                target(
                    Scope::Project,
                    Format::Toml,
                    same(Base::Repo, ".codex/config.toml"),
                    "mcp_servers",
                ),
            ],
        },
        // <https://github.com/charmbracelet/crush/blob/main/docs/config/README.md> and the schema
        // at <https://charm.land/crush.json>: the key is `mcp` (never `mcpServers`) and a local
        // server names its transport, `"type": "stdio"`. The *current* format is a shell script
        // (`crushrc`) that this tool must not rewrite, and only the deprecated JSON files are
        // editable, so this entry is opt-in: `--agents crush`.
        AgentDef {
            id: "crush",
            display: "Crush",
            doc_url: "https://github.com/charmbracelet/crush/blob/main/docs/config/README.md",
            verified: false,
            targets: vec![
                shaped(
                    target(
                        Scope::User,
                        Format::Jsonc,
                        same(Base::Home, ".config/crush/crush.json"),
                        "mcp",
                    ),
                    EntryShape::StdioCommandArgs,
                ),
                shaped(
                    target(
                        Scope::Project,
                        Format::Jsonc,
                        same(Base::Repo, ".crush.json"),
                        "mcp",
                    ),
                    EntryShape::StdioCommandArgs,
                ),
            ],
        },
        // <https://google-gemini.github.io/gemini-cli/docs/tools/mcp-server.html>: the global
        // `~/.gemini/settings.json` and the project `.gemini/settings.json`, key `mcpServers`.
        AgentDef {
            id: "gemini",
            display: "Gemini CLI",
            doc_url: "https://google-gemini.github.io/gemini-cli/docs/tools/mcp-server.html",
            verified: true,
            targets: vec![
                target(
                    Scope::User,
                    Format::Json,
                    same(Base::Home, ".gemini/settings.json"),
                    "mcpServers",
                ),
                target(
                    Scope::Project,
                    Format::Json,
                    same(Base::Repo, ".gemini/settings.json"),
                    "mcpServers",
                ),
            ],
        },
        // <https://opencode.ai/docs/mcp-servers/> and <https://opencode.ai/docs/config/>: the key
        // is `mcp`, a local server is `{"type": "local", "command": [...], "enabled": true}`, the
        // global file is `~/.config/opencode/opencode.json` and a project may hold `opencode.json`
        // in its root. Both JSON and JSONC are accepted. The documentation gives no user-level
        // path for Windows, so there is no Windows target rather than a guessed one.
        AgentDef {
            id: "opencode",
            display: "opencode",
            doc_url: "https://opencode.ai/docs/mcp-servers/",
            verified: true,
            targets: vec![
                shaped(
                    target(
                        Scope::User,
                        Format::Jsonc,
                        PathSpec {
                            macos: Some(at(Base::Home, ".config/opencode/opencode.json")),
                            linux: Some(at(Base::Home, ".config/opencode/opencode.json")),
                            windows: None,
                        },
                        "mcp",
                    ),
                    EntryShape::LocalCommandList,
                ),
                shaped(
                    target(
                        Scope::Project,
                        Format::Jsonc,
                        same(Base::Repo, "opencode.json"),
                        "mcp",
                    ),
                    EntryShape::LocalCommandList,
                ),
            ],
        },
    ]
}

/// The agents that are editors or editor extensions, with the page each entry was read from.
#[allow(
    clippy::too_many_lines,
    reason = "a flat data table is clearer than helpers"
)]
fn editor_agents() -> Vec<AgentDef> {
    vec![
        // <https://docs.cline.bot/mcp/configuring-mcp-servers> gives the key (`mcpServers`) and
        // "CLI: ~/.cline/mcp.json", while
        // <https://docs.cline.bot/cline-cli/configuration> lays the same data out at
        // `~/.cline/data/settings/cline_mcp_settings.json`. The two pages disagree and the editor
        // extension documents no path at all, so this entry is opt-in: `--agents cline`.
        AgentDef {
            id: "cline",
            display: "Cline",
            doc_url: "https://docs.cline.bot/mcp/configuring-mcp-servers",
            verified: false,
            targets: vec![target(
                Scope::User,
                Format::Json,
                same(Base::Home, ".cline/mcp.json"),
                "mcpServers",
            )],
        },
        // <https://cursor.com/docs/context/mcp>: "Create ~/.cursor/mcp.json in your home
        // directory" and "Create .cursor/mcp.json in your project"; the key is `mcpServers`.
        AgentDef {
            id: "cursor",
            display: "Cursor",
            doc_url: "https://cursor.com/docs/context/mcp",
            verified: true,
            targets: vec![
                target(
                    Scope::User,
                    Format::Json,
                    same(Base::Home, ".cursor/mcp.json"),
                    "mcpServers",
                ),
                target(
                    Scope::Project,
                    Format::Json,
                    same(Base::Repo, ".cursor/mcp.json"),
                    "mcpServers",
                ),
            ],
        },
        // <https://kiro.dev/docs/mcp/configuration/>: "Global MCP JSON - ~/.kiro/settings/mcp.json"
        // and "Workspace MCP JSON - .kiro/settings/mcp.json", key `mcpServers`, workspace wins.
        AgentDef {
            id: "kiro",
            display: "Kiro",
            doc_url: "https://kiro.dev/docs/mcp/configuration/",
            verified: true,
            targets: vec![
                target(
                    Scope::User,
                    Format::Json,
                    same(Base::Home, ".kiro/settings/mcp.json"),
                    "mcpServers",
                ),
                target(
                    Scope::Project,
                    Format::Json,
                    same(Base::Repo, ".kiro/settings/mcp.json"),
                    "mcpServers",
                ),
            ],
        },
        // <https://docs.trae.ai/ide/add-mcp-servers>: Trae documents MCP support, but its
        // documentation site renders in the browser and serves no readable text to a plain
        // request, so the configuration path below could not be read from the page itself. It
        // comes from secondary sources and is therefore NOT verified: the entry is opt-in
        // (`--agents trae`) and nothing is written to these paths by default. Replace this comment
        // with a quotation from the page, and set `verified: true`, once the path can be read
        // there directly.
        AgentDef {
            id: "trae",
            display: "Trae",
            doc_url: "https://docs.trae.ai/ide/add-mcp-servers",
            verified: false,
            targets: vec![
                target(
                    Scope::User,
                    Format::Json,
                    PathSpec {
                        macos: Some(Location {
                            base: Base::Config,
                            rel: "Trae/mcp.json",
                        }),
                        linux: Some(Location {
                            base: Base::XdgConfig,
                            rel: "Trae/mcp.json",
                        }),
                        windows: Some(Location {
                            base: Base::Config,
                            rel: "Trae/mcp.json",
                        }),
                    },
                    "mcpServers",
                ),
                target(
                    Scope::Project,
                    Format::Json,
                    same(Base::Repo, ".trae/mcp.json"),
                    "mcpServers",
                ),
            ],
        },
        // <https://code.visualstudio.com/docs/copilot/customization/mcp-servers>: the workspace
        // file is `.vscode/mcp.json` and "this format defines servers in a top-level `servers`
        // object"; the user file is `mcp.json` in the user profile folder, whose platform paths
        // are given in <https://code.visualstudio.com/docs/configure/settings>
        // (`%APPDATA%\Code\User`, `$HOME/Library/Application Support/Code/User`,
        // `$HOME/.config/Code/User`) -- exactly the platform configuration directory. Read as
        // JSONC, a superset, so a file that already carries comments is not refused. The same page
        // also accepts the portable `.mcp.json` with `mcpServers`, which is `claude-code`'s
        // project file, so configuring either one is enough.
        AgentDef {
            id: "vscode-copilot",
            display: "Visual Studio Code",
            doc_url: "https://code.visualstudio.com/docs/copilot/customization/mcp-servers",
            verified: true,
            targets: vec![
                target(
                    Scope::User,
                    Format::Jsonc,
                    same(Base::Config, "Code/User/mcp.json"),
                    "servers",
                ),
                target(
                    Scope::Project,
                    Format::Jsonc,
                    same(Base::Repo, ".vscode/mcp.json"),
                    "servers",
                ),
            ],
        },
        // <https://docs.devin.ai/windsurf/plugins/cascade/mcp>: the managed file is
        // `$XDG_CONFIG_HOME/devin/mcp_config.json` (`%AppData%/devin/mcp_config.json` on Windows)
        // and the editor also reads `~/.codeium/windsurf/mcp_config.json`. Key `mcpServers`; no
        // project scope is documented.
        AgentDef {
            id: "windsurf",
            display: "Windsurf",
            doc_url: "https://docs.devin.ai/windsurf/plugins/cascade/mcp",
            verified: true,
            targets: vec![
                target(
                    Scope::User,
                    Format::Json,
                    per_os(
                        at(Base::XdgConfig, "devin/mcp_config.json"),
                        at(Base::XdgConfig, "devin/mcp_config.json"),
                        at(Base::Config, "devin/mcp_config.json"),
                    ),
                    "mcpServers",
                ),
                target(
                    Scope::User,
                    Format::Json,
                    same(Base::Home, ".codeium/windsurf/mcp_config.json"),
                    "mcpServers",
                ),
            ],
        },
        // Key from <https://zed.dev/docs/ai/mcp> (`context_servers`, with `command`, `args` and
        // `env`); paths from <https://zed.dev/docs/configuring-zed>:
        // `$XDG_CONFIG_HOME/zed/settings.json` on macOS and Linux, `%APPDATA%\Zed\settings.json`
        // on Windows, `.zed/settings.json` in a project. The same page registers
        // `**/.zed/**/*.json` and `**/zed/**/*.json` as JSONC.
        AgentDef {
            id: "zed",
            display: "Zed",
            doc_url: "https://zed.dev/docs/ai/mcp",
            verified: true,
            targets: vec![
                target(
                    Scope::User,
                    Format::Jsonc,
                    per_os(
                        at(Base::XdgConfig, "zed/settings.json"),
                        at(Base::XdgConfig, "zed/settings.json"),
                        at(Base::Config, "Zed/settings.json"),
                    ),
                    "context_servers",
                ),
                target(
                    Scope::Project,
                    Format::Jsonc,
                    same(Base::Repo, ".zed/settings.json"),
                    "context_servers",
                ),
            ],
        },
    ]
}

/// The whole table, sorted by id.
fn build_table() -> Vec<AgentDef> {
    let mut table = command_line_agents();
    table.extend(editor_agents());
    table.sort_by(|left, right| left.id.cmp(right.id));
    table
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{all, by_id};

    /// Ids are unique, lowercase, and every entry is documented and has at least one file.
    #[test]
    fn the_table_is_well_formed() {
        let mut ids = BTreeSet::new();
        for agent in all() {
            assert!(ids.insert(agent.id), "duplicate id {}", agent.id);
            assert!(
                agent
                    .id
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || matches!(c, '-' | '0'..='9')),
                "{} is not a plain lowercase id",
                agent.id
            );
            assert!(
                agent.doc_url.starts_with("https://"),
                "{} has no documentation",
                agent.id
            );
            assert!(
                !agent.display.is_empty(),
                "{} has no display name",
                agent.id
            );
            assert!(
                !agent.targets.is_empty(),
                "{} has no configuration file",
                agent.id
            );
            for target in &agent.targets {
                assert!(
                    !target.servers_key.is_empty(),
                    "{} has an empty key",
                    agent.id
                );
                let spec = target.path;
                assert!(
                    spec.macos.is_some() || spec.linux.is_some() || spec.windows.is_some(),
                    "{} has a file on no platform",
                    agent.id
                );
            }
        }
        assert!(
            all().len() >= 8,
            "only {} agents are supported",
            all().len()
        );
    }

    /// The table is sorted by id, so every listing is deterministic.
    #[test]
    fn the_table_is_sorted() {
        let ids: Vec<&str> = all().iter().map(|agent| agent.id).collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        assert_eq!(ids, sorted);
    }

    /// Lookup by id finds known agents and refuses anything else.
    #[test]
    fn lookup_by_id_works() {
        assert_eq!(by_id("cursor").map(|agent| agent.display), Some("Cursor"));
        assert!(by_id("Cursor").is_none());
        assert!(by_id("").is_none());
        assert!(by_id("no-such-agent").is_none());
    }
}
