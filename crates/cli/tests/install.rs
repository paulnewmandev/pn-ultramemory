// SPDX-License-Identifier: Apache-2.0
//! Integration tests of the installer: exact ownership, idempotency, hostile files and the
//! round trip that matters most -- install then uninstall must leave a file byte for byte as it was.
//!
//! The binary has no library target, so the modules under test are compiled into this test crate
//! with `#[path]`. Every test uses the *project* scope of an agent, whose file lives inside a
//! temporary repository, so nothing outside that directory is ever read or written.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests unwrap and panic to fail loudly"
)]

#[path = "../src/agents.rs"]
pub mod agents;
// `agents` resolves the per-user configuration directories through `crate::dirs`, so that module
// has to exist at this test crate's root under the same name the binary gives it.
#[path = "../src/dirs.rs"]
pub mod dirs;
#[path = "../src/doctor.rs"]
pub mod doctor;
#[path = "../src/error.rs"]
pub mod error;
#[path = "../src/hook.rs"]
pub mod hook;
#[path = "../src/install.rs"]
pub mod install;

#[allow(
    dead_code,
    reason = "each test binary uses a different subset of the helpers"
)]
mod common;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use agents::{Platform, Scope};
use common::Repo;
use install::{Action, InstallOptions, install, uninstall};

/// The command every test registers, so the text written is predictable.
const COMMAND: &str = "/opt/pn/bin/pn-ultramemory";

/// Options that configure one agent in the repository itself.
fn options(agent: &str) -> InstallOptions {
    InstallOptions {
        agents: vec![agent.to_owned()],
        scope: Scope::Project,
        command: Some(COMMAND.to_owned()),
        ..InstallOptions::default()
    }
}

/// Installs for one agent and returns the single change it produced.
fn install_one(repo: &Path, agent: &str) -> install::Change {
    let report = install(repo, &options(agent)).expect("a report");
    assert_eq!(report.changes.len(), 1, "{:?}", report.changes);
    report.changes.into_iter().next().expect("one change")
}

/// Uninstalls for one agent and returns the single change it produced.
fn uninstall_one(repo: &Path, agent: &str) -> install::Change {
    let report = uninstall(repo, &options(agent)).expect("a report");
    assert_eq!(report.changes.len(), 1, "{:?}", report.changes);
    report.changes.into_iter().next().expect("one change")
}

/// The file each agent reads inside a repository.
fn project_file(agent: &str) -> &'static str {
    match agent {
        "amp" => ".amp/settings.json",
        "claude-code" => ".mcp.json",
        "codex" => ".codex/config.toml",
        "crush" => ".crush.json",
        "cursor" => ".cursor/mcp.json",
        "gemini" => ".gemini/settings.json",
        "kiro" => ".kiro/settings/mcp.json",
        "opencode" => "opencode.json",
        "vscode-copilot" => ".vscode/mcp.json",
        "zed" => ".zed/settings.json",
        other => panic!("no project file is known for {other}"),
    }
}

/// Other servers, other keys and the exact formatting of the file all survive an install.
#[test]
fn other_servers_and_formatting_survive() {
    let repo = Repo::new();
    let original = "{\n  \"mcpServers\": {\n    \"other\": {\n      \"command\": \"other-server\",\n      \"args\": [\"--flag\"]\n    }\n  },\n  \"unrelated\": 7\n}\n";
    repo.write(".cursor/mcp.json", original);
    let change = install_one(repo.path(), "cursor");
    assert_eq!(change.action, Action::Update);
    let after = repo.read(".cursor/mcp.json");
    assert!(after.contains(
        "\"other\": {\n      \"command\": \"other-server\",\n      \"args\": [\"--flag\"]\n    }"
    ));
    assert!(after.contains("\"unrelated\": 7"));
    let parsed: serde_json::Value = serde_json::from_str(&after).expect("valid JSON");
    assert_eq!(parsed["mcpServers"]["pn-ultramemory"]["command"], COMMAND);
    assert_eq!(
        parsed["mcpServers"]["pn-ultramemory"]["args"],
        serde_json::json!(["serve"])
    );
    assert_eq!(parsed["mcpServers"]["other"]["command"], "other-server");
}

/// Comments and trailing commas of a JSONC file come out byte for byte as they went in.
#[test]
fn jsonc_comments_and_trailing_commas_survive() {
    let repo = Repo::new();
    let original = "// the settings of this project\n{\n  /* servers we trust */\n  \"context_servers\": {\n    \"other\": { \"command\": \"x\" }, // keep me\n  },\n  \"theme\": \"One Dark\", // and me\n}\n";
    repo.write(".zed/settings.json", original);
    assert_eq!(install_one(repo.path(), "zed").action, Action::Update);
    let after = repo.read(".zed/settings.json");
    for comment in [
        "// the settings of this project",
        "/* servers we trust */",
        "// keep me",
        "// and me",
    ] {
        assert!(after.contains(comment), "lost {comment} in\n{after}");
    }
    assert!(after.contains("\"theme\": \"One Dark\", //"), "{after}");
    assert!(after.contains("\"pn-ultramemory\""), "{after}");
    assert_eq!(uninstall_one(repo.path(), "zed").action, Action::Update);
    assert_eq!(repo.read(".zed/settings.json"), original);
}

/// A TOML file keeps every other table, and ours is appended as a table of its own.
#[test]
fn toml_other_tables_survive() {
    let repo = Repo::new();
    let original = "# my settings\nmodel = \"fast\"\n\n[tui]\nnotifications = true\n\n[mcp_servers.other]\ncommand = \"other\"\nargs = []\n";
    repo.write(".codex/config.toml", original);
    assert_eq!(install_one(repo.path(), "codex").action, Action::Update);
    let after = repo.read(".codex/config.toml");
    assert!(after.starts_with(original), "{after}");
    assert!(after.contains("[mcp_servers.pn-ultramemory]"), "{after}");
    assert!(
        after.contains(&format!("command = \"{COMMAND}\"")),
        "{after}"
    );
    assert!(after.contains("args = [\"serve\"]"), "{after}");
    assert_eq!(uninstall_one(repo.path(), "codex").action, Action::Update);
    assert_eq!(repo.read(".codex/config.toml"), original);
}

/// A TOML table of ours that is not the last one is replaced in place, and its neighbours stay.
#[test]
fn a_toml_table_in_the_middle_is_replaced_in_place() {
    let repo = Repo::new();
    let original = "[mcp_servers.pn-ultramemory]\ncommand = \"/old/pn-ultramemory\"\nargs = [\"serve\"]\n\n[tui]\nnotifications = true\n";
    repo.write(".codex/config.toml", original);
    assert_eq!(install_one(repo.path(), "codex").action, Action::Update);
    let after = repo.read(".codex/config.toml");
    assert!(
        after.ends_with("\n[tui]\nnotifications = true\n"),
        "{after}"
    );
    assert!(!after.contains("/old/pn-ultramemory"), "{after}");
    assert_eq!(
        after.matches("[mcp_servers.pn-ultramemory]").count(),
        1,
        "{after}"
    );
}

/// An empty file becomes a whole small document; a file that is only whitespace does too.
#[test]
fn an_empty_file_gets_a_document() {
    for original in ["", "   \n\n", "\u{feff}"] {
        let repo = Repo::new();
        repo.write(".cursor/mcp.json", original);
        assert_eq!(install_one(repo.path(), "cursor").action, Action::Update);
        let after = repo.read(".cursor/mcp.json");
        let body = after.trim_start_matches('\u{feff}');
        let parsed: serde_json::Value = serde_json::from_str(body).expect("valid JSON");
        assert_eq!(parsed["mcpServers"]["pn-ultramemory"]["command"], COMMAND);
        assert_eq!(
            after.starts_with('\u{feff}'),
            original.starts_with('\u{feff}')
        );
    }
}

/// A file that is not there is created, together with its directory.
#[test]
fn a_missing_file_is_created() {
    let repo = Repo::new();
    repo.mkdir(".cursor");
    let change = install_one(repo.path(), "cursor");
    assert_eq!(change.action, Action::Create);
    assert!(change.before.is_none());
    assert!(change.after.is_some());
    let parsed: serde_json::Value =
        serde_json::from_str(&repo.read(".cursor/mcp.json")).expect("valid JSON");
    assert_eq!(parsed["mcpServers"]["pn-ultramemory"]["command"], COMMAND);
}

/// Installing twice is a no-op the second time.
#[test]
fn installing_twice_changes_nothing() {
    let repo = Repo::new();
    repo.write(".cursor/mcp.json", "{}\n");
    assert_eq!(install_one(repo.path(), "cursor").action, Action::Update);
    let once = repo.read(".cursor/mcp.json");
    assert_eq!(install_one(repo.path(), "cursor").action, Action::Unchanged);
    assert_eq!(repo.read(".cursor/mcp.json"), once);
}

/// A hand-edited entry that still starts this executable is kept, until `force` is passed.
#[test]
fn a_hand_edited_entry_is_kept_unless_forced() {
    let repo = Repo::new();
    let edited = format!(
        "{{\n  \"mcpServers\": {{\n    \"pn-ultramemory\": {{\n      \"command\": \"{COMMAND}\",\n      \"args\": [\"serve\"],\n      \"env\": {{\"PN_ULTRAMEMORY_BUDGET\": \"900\"}}\n    }}\n  }}\n}}\n"
    );
    repo.write(".cursor/mcp.json", &edited);
    let change = install_one(repo.path(), "cursor");
    assert!(
        matches!(change.action, Action::Skip(ref why) if why.contains("--force")),
        "{change:?}"
    );
    assert_eq!(repo.read(".cursor/mcp.json"), edited);

    let forced = InstallOptions {
        force: true,
        ..options("cursor")
    };
    let report = install(repo.path(), &forced).expect("a report");
    assert_eq!(report.changes[0].action, Action::Update);
    let after = repo.read(".cursor/mcp.json");
    assert!(!after.contains("PN_ULTRAMEMORY_BUDGET"), "{after}");
}

/// An entry that starts some other executable is stale and is repointed without `force`.
#[test]
fn a_stale_entry_is_repointed() {
    let repo = Repo::new();
    repo.write(
        ".cursor/mcp.json",
        "{\n  \"mcpServers\": {\n    \"pn-ultramemory\": {\n      \"command\": \"/gone/pn-ultramemory\",\n      \"args\": [\"serve\"]\n    }\n  }\n}\n",
    );
    assert_eq!(install_one(repo.path(), "cursor").action, Action::Update);
    let after = repo.read(".cursor/mcp.json");
    assert!(!after.contains("/gone/"), "{after}");
    assert!(after.contains(COMMAND), "{after}");
}

/// Install then uninstall leaves the file exactly as it was, whatever it looked like.
#[test]
fn uninstall_round_trips_byte_for_byte() {
    let cases: &[(&str, &str)] = &[
        ("cursor", "{}"),
        ("cursor", "{}\n"),
        ("cursor", "{\"mcpServers\":{}}"),
        (
            "cursor",
            "{\n\t\"mcpServers\": {\n\t\t\"other\": {\"command\": \"x\"}\n\t}\n}\n",
        ),
        (
            "cursor",
            "{\n    \"other\": true,\n    \"mcpServers\": {\n        \"a\": {\"command\": \"a\"}\n    }\n}\n",
        ),
        (
            "cursor",
            "\u{feff}{\n  \"mcpServers\": {\n    \"a\": {\"command\": \"a\"}\n  }\n}\n",
        ),
        (
            "cursor",
            "{\r\n  \"mcpServers\": {\r\n    \"a\": {\"command\": \"a\"}\r\n  }\r\n}\r\n",
        ),
        (
            "claude-code",
            "{\n  \"mcpServers\": {\n    \"a\": {\"command\": \"a\"}\n  }\n}\n",
        ),
        (
            "zed",
            "// top\n{\n  \"context_servers\": {\n    \"a\": {\"command\": \"a\"}, // trailing\n  },\n}\n",
        ),
        ("zed", "{\n  \"theme\": \"dark\"\n}\n"),
        (
            "vscode-copilot",
            "{\n  \"servers\": {\n    \"a\": {\"command\": \"a\"}\n  },\n  \"inputs\": []\n}\n",
        ),
        (
            "vscode-copilot",
            "{\n  // no servers yet\n  \"inputs\": []\n}\n",
        ),
        (
            "amp",
            "{\n  \"amp.mcpServers\": {\n    \"a\": {\"command\": \"a\"}\n  },\n  \"amp.url\": \"x\"\n}\n",
        ),
        (
            "opencode",
            "{\n  \"$schema\": \"https://opencode.ai/config.json\",\n  \"mcp\": {\n    \"a\": {\"type\": \"local\", \"command\": [\"a\"], \"enabled\": true}\n  }\n}\n",
        ),
        (
            "crush",
            "{\n  \"mcp\": {\n    \"a\": {\"type\": \"stdio\", \"command\": \"a\"}\n  }\n}\n",
        ),
        (
            "gemini",
            "{\n  \"theme\": \"Default\",\n  \"mcpServers\": {\n    \"a\": {\"command\": \"a\"}\n  }\n}\n",
        ),
        (
            "kiro",
            "{\n  \"mcpServers\": {\n    \"a\": {\"command\": \"a\", \"disabled\": false}\n  }\n}\n",
        ),
        ("codex", ""),
        ("codex", "model = \"fast\"\n"),
        ("codex", "model = \"fast\"\n\n[tui]\nx = 1\n"),
        ("codex", "[mcp_servers.other]\ncommand = \"o\"\n"),
        ("codex", "greeting = \"\"\"\n[not a table]\n\"\"\"\n"),
        ("codex", "model = \"fast\"\r\n\r\n[tui]\r\nx = 1\r\n"),
    ];
    for (agent, original) in cases {
        let repo = Repo::new();
        let file = project_file(agent);
        repo.write(file, original);
        let change = install_one(repo.path(), agent);
        assert!(
            change.action.is_change(),
            "{agent} {original:?}: {change:?}"
        );
        assert_ne!(
            &repo.read(file),
            original,
            "{agent} {original:?} was not changed"
        );
        let change = uninstall_one(repo.path(), agent);
        assert!(
            change.action.is_change(),
            "{agent} {original:?}: {change:?}"
        );
        assert_eq!(
            &repo.read(file),
            original,
            "{agent} {original:?} did not round trip"
        );
    }
}

/// A file the installer created is left holding an empty document, never deleted: nothing can tell
/// it apart from a file the user wrote that way.
#[test]
fn uninstall_empties_a_file_it_created_but_keeps_it() {
    let repo = Repo::new();
    repo.mkdir(".cursor");
    assert_eq!(install_one(repo.path(), "cursor").action, Action::Create);
    let change = uninstall_one(repo.path(), "cursor");
    assert_eq!(change.action, Action::Update);
    assert!(change.before.is_some());
    assert!(change.after.is_none());
    assert_eq!(repo.read(".cursor/mcp.json").trim(), "{}");

    for (agent, file, left) in [
        ("cursor", ".cursor/mcp.json", "{}"),
        ("codex", ".codex/config.toml", ""),
    ] {
        let repo = Repo::new();
        repo.write(file, "");
        assert!(install_one(repo.path(), agent).action.is_change());
        assert_eq!(uninstall_one(repo.path(), agent).action, Action::Update);
        assert_eq!(repo.read(file).trim(), left, "{agent}");
    }
}

/// Uninstalling something that is not there is a skip, not a failure.
#[test]
fn uninstall_without_an_entry_is_skipped() {
    let repo = Repo::new();
    repo.write(
        ".cursor/mcp.json",
        "{\n  \"mcpServers\": {\n    \"a\": {\"command\": \"a\"}\n  }\n}\n",
    );
    let before = repo.read(".cursor/mcp.json");
    let change = uninstall_one(repo.path(), "cursor");
    assert!(matches!(change.action, Action::Skip(_)), "{change:?}");
    assert_eq!(repo.read(".cursor/mcp.json"), before);

    let repo = Repo::new();
    repo.mkdir(".cursor");
    let change = uninstall_one(repo.path(), "cursor");
    assert!(matches!(change.action, Action::Skip(_)), "{change:?}");
    assert!(!repo.exists(".cursor/mcp.json"));
}

/// Indentation is learned from the file: tabs stay tabs, four spaces stay four.
#[test]
fn indentation_is_preserved() {
    let cases: &[(&str, &str)] = &[
        (
            "{\n\t\"other\": 1\n}\n",
            "\n\t\"mcpServers\": {\n\t\t\"pn-ultramemory\"",
        ),
        (
            "{\n  \"other\": 1\n}\n",
            "\n  \"mcpServers\": {\n    \"pn-ultramemory\"",
        ),
        (
            "{\n    \"other\": 1\n}\n",
            "\n    \"mcpServers\": {\n        \"pn-ultramemory\"",
        ),
    ];
    for (original, expected) in cases {
        let repo = Repo::new();
        repo.write(".cursor/mcp.json", original);
        assert_eq!(install_one(repo.path(), "cursor").action, Action::Update);
        let after = repo.read(".cursor/mcp.json");
        assert!(after.contains(expected), "{original:?} produced\n{after}");
    }
}

/// Windows line endings are kept, and no lone newline is introduced.
#[test]
fn crlf_is_preserved() {
    let repo = Repo::new();
    repo.write(".cursor/mcp.json", "{\r\n  \"other\": 1\r\n}\r\n");
    assert_eq!(install_one(repo.path(), "cursor").action, Action::Update);
    let after = repo.read(".cursor/mcp.json");
    assert_eq!(
        after.matches('\n').count(),
        after.matches("\r\n").count(),
        "{after:?}"
    );
    assert!(after.contains("\"pn-ultramemory\""), "{after}");
}

/// A file that is not valid for its format is refused with a reason, and left alone.
#[test]
fn invalid_files_are_skipped() {
    let cases: &[(&str, &str)] = &[
        ("cursor", "{not json"),
        ("cursor", "[]\n"),
        ("cursor", "{\"mcpServers\": []}"),
        ("cursor", "{\"mcpServers\": {\"a\": {}},}"),
        ("cursor", "{\"mcpServers\": {} } trailing"),
        ("cursor", "{\"a\": /* comment */ 1}"),
        ("cursor", "{\"a\": 1e\n}"),
        ("zed", "{\"context_servers\": {\"a\": 1\n"),
        ("zed", "{\"context_servers\": \"no\"}"),
        ("zed", "{/* unterminated\n}"),
        ("codex", "[mcp_servers.pn-ultramemory\ncommand = \"x\"\n"),
    ];
    for (agent, original) in cases {
        let repo = Repo::new();
        let file = project_file(agent);
        repo.write(file, original);
        let change = install_one(repo.path(), agent);
        assert!(
            matches!(change.action, Action::Skip(_)),
            "{original:?} gave {change:?}"
        );
        assert_eq!(&repo.read(file), original, "{original:?} was changed");
    }
}

/// A directory where a file belongs is refused, not deleted.
#[test]
fn a_directory_is_skipped() {
    let repo = Repo::new();
    repo.mkdir(".cursor/mcp.json");
    let change = install_one(repo.path(), "cursor");
    assert!(
        matches!(change.action, Action::Skip(ref why) if why.contains("directory")),
        "{change:?}"
    );
    assert!(repo.at(".cursor/mcp.json").is_dir());
}

/// A symbolic link that leaves its own directory is refused; one that stays is followed.
#[cfg(unix)]
#[test]
fn symlinks_out_of_their_directory_are_refused() {
    let repo = Repo::new();
    repo.write("elsewhere/real.json", "{}\n");
    repo.mkdir(".cursor");
    std::os::unix::fs::symlink("../elsewhere/real.json", repo.at(".cursor/mcp.json"))
        .expect("a symbolic link");
    let change = install_one(repo.path(), "cursor");
    assert!(
        matches!(change.action, Action::Skip(ref why) if why.contains("symbolic")),
        "{change:?}"
    );
    assert_eq!(repo.read("elsewhere/real.json"), "{}\n");

    let repo = Repo::new();
    repo.write(".cursor/real.json", "{}\n");
    std::os::unix::fs::symlink("real.json", repo.at(".cursor/mcp.json")).expect("a link");
    assert_eq!(install_one(repo.path(), "cursor").action, Action::Update);
}

/// A directory that cannot be written to is reported, not a panic and not a failure of the run.
#[cfg(unix)]
#[test]
fn an_unwritable_file_is_skipped() {
    use std::os::unix::fs::PermissionsExt as _;
    let repo = Repo::new();
    repo.write(".cursor/mcp.json", "{}\n");
    let dir = repo.at(".cursor");
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).expect("read only");
    let change = install_one(repo.path(), "cursor");
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).expect("writable again");
    assert!(
        matches!(change.action, Action::Skip(ref why) if why.contains("cannot write")),
        "{change:?}"
    );
    assert_eq!(repo.read(".cursor/mcp.json"), "{}\n");
}

/// A large file is edited as surgically as a small one.
#[test]
fn a_large_file_is_edited_surgically() {
    let repo = Repo::new();
    let mut original = String::from("{\n  \"mcpServers\": {\n");
    for index in 0..4000 {
        let _ = write!(
            original,
            "    \"server-{index:04}\": {{\n      \"command\": \"s{index}\",\n      \"args\": []\n    }},\n"
        );
    }
    original.push_str("    \"last\": {\"command\": \"l\"}\n  }\n}\n");
    assert!(original.len() > 250_000, "{}", original.len());
    repo.write(".cursor/mcp.json", &original);
    assert_eq!(install_one(repo.path(), "cursor").action, Action::Update);
    let after = repo.read(".cursor/mcp.json");
    assert!(after.contains("\"server-3999\""), "a server went missing");
    let parsed: serde_json::Value = serde_json::from_str(&after).expect("valid JSON");
    assert_eq!(
        parsed["mcpServers"].as_object().map(serde_json::Map::len),
        Some(4002)
    );
    assert_eq!(uninstall_one(repo.path(), "cursor").action, Action::Update);
    assert_eq!(repo.read(".cursor/mcp.json"), original);
}

/// A dry run computes everything and writes nothing at all.
#[test]
fn a_dry_run_writes_nothing() {
    let repo = Repo::new();
    repo.write(".cursor/mcp.json", "{}\n");
    let dry = InstallOptions {
        dry_run: true,
        ..options("cursor")
    };
    let report = install(repo.path(), &dry).expect("a report");
    assert_eq!(report.changes[0].action, Action::Update);
    assert!(report.changes[0].after.is_some());
    assert_eq!(repo.read(".cursor/mcp.json"), "{}\n");
    assert!(!repo.exists(".cursor/mcp.json.bak-pn-ultramemory"));

    let repo = Repo::new();
    repo.mkdir(".cursor");
    let report = install(repo.path(), &dry).expect("a report");
    assert_eq!(report.changes[0].action, Action::Create);
    assert!(!repo.exists(".cursor/mcp.json"));
}

/// A backup appears only when a file that exists is about to change.
#[test]
fn a_backup_is_written_only_for_a_real_change() {
    let repo = Repo::new();
    let backup = ".cursor/mcp.json.bak-pn-ultramemory";
    repo.mkdir(".cursor");
    assert_eq!(install_one(repo.path(), "cursor").action, Action::Create);
    assert!(!repo.exists(backup), "a created file needs no backup");
    let created = repo.read(".cursor/mcp.json");
    assert_eq!(install_one(repo.path(), "cursor").action, Action::Unchanged);
    assert!(
        !repo.exists(backup),
        "nothing changed, so nothing to back up"
    );
    let forced = InstallOptions {
        force: true,
        pin_repo: true,
        ..options("cursor")
    };
    assert_eq!(
        install(repo.path(), &forced).expect("a report").changes[0].action,
        Action::Update
    );
    assert_eq!(repo.read(backup), created);
}

/// Pinning the repository puts it in the arguments of the server.
#[test]
fn pinning_the_repository_is_written_into_the_arguments() {
    let repo = Repo::new();
    repo.mkdir(".cursor");
    let pinned = InstallOptions {
        pin_repo: true,
        ..options("cursor")
    };
    install(repo.path(), &pinned).expect("a report");
    let parsed: serde_json::Value =
        serde_json::from_str(&repo.read(".cursor/mcp.json")).expect("valid JSON");
    assert_eq!(
        parsed["mcpServers"]["pn-ultramemory"]["args"],
        serde_json::json!(["--repo", repo.path().to_string_lossy(), "serve"])
    );
}

/// Every agent gets the entry shape its own documentation asks for.
#[test]
fn each_agent_gets_the_shape_it_expects() {
    let repo = Repo::new();
    repo.write("opencode.json", "{}\n");
    install_one(repo.path(), "opencode");
    let parsed: serde_json::Value =
        serde_json::from_str(&repo.read("opencode.json")).expect("valid JSON");
    let entry = &parsed["mcp"]["pn-ultramemory"];
    assert_eq!(entry["type"], "local");
    assert_eq!(entry["command"], serde_json::json!([COMMAND, "serve"]));
    assert_eq!(entry["enabled"], true);

    let repo = Repo::new();
    repo.write(".crush.json", "{}\n");
    install_one(repo.path(), "crush");
    let parsed: serde_json::Value =
        serde_json::from_str(&repo.read(".crush.json")).expect("valid JSON");
    assert_eq!(parsed["mcp"]["pn-ultramemory"]["type"], "stdio");
    assert_eq!(parsed["mcp"]["pn-ultramemory"]["command"], COMMAND);

    let repo = Repo::new();
    repo.write(".amp/settings.json", "{}\n");
    install_one(repo.path(), "amp");
    let text = repo.read(".amp/settings.json");
    assert!(text.contains("\"amp.mcpServers\""), "{text}");
    let parsed: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");
    assert_eq!(
        parsed["amp.mcpServers"]["pn-ultramemory"]["command"],
        COMMAND
    );
}

/// An unknown agent is an invalid request, and the message lists what is known.
#[test]
fn an_unknown_agent_is_refused() {
    let repo = Repo::new();
    let mut opts = options("not-an-agent");
    opts.agents.push("cursor".to_owned());
    let error = install(repo.path(), &opts).expect_err("an error");
    assert_eq!(error.code, 2);
    assert!(error.message.contains("not-an-agent"), "{error}");
    assert!(error.message.contains("cursor"), "{error}");
    let empty = InstallOptions {
        server_name: "  ".to_owned(),
        ..options("cursor")
    };
    assert_eq!(install(repo.path(), &empty).expect_err("an error").code, 2);
}

/// An agent with no file in the chosen scope is reported, not silently dropped.
#[test]
fn an_agent_without_a_file_in_this_scope_is_reported() {
    let repo = Repo::new();
    let change = install_one(repo.path(), "windsurf");
    assert!(
        matches!(change.action, Action::Skip(ref why) if why.contains("project")),
        "{change:?}"
    );
}

/// Detection never picks an agent whose paths are not documented; naming it does.
#[test]
fn unverified_agents_must_be_named() {
    let repo = Repo::new();
    repo.write(".crush.json", "{}\n");
    let detected = InstallOptions {
        scope: Scope::Project,
        ..InstallOptions::default()
    };
    let report = install(repo.path(), &detected).expect("a report");
    assert!(
        report.changes.iter().all(|change| change.agent != "crush"),
        "an unverified agent was configured without being named: {:?}",
        report.changes
    );
    assert_eq!(repo.read(".crush.json"), "{}\n");
    assert!(install_one(repo.path(), "crush").action.is_change());
}

/// Installing with no agent named configures every agent the repository shows evidence of.
#[test]
fn detection_drives_an_install_with_no_names() {
    let repo = Repo::new();
    repo.mkdir(".cursor");
    repo.mkdir(".vscode");
    repo.write(".mcp.json", "{}\n");
    let detected = InstallOptions {
        scope: Scope::Project,
        command: Some(COMMAND.to_owned()),
        ..InstallOptions::default()
    };
    let report = install(repo.path(), &detected).expect("a report");
    let touched: Vec<&str> = report
        .changes
        .iter()
        .map(|change| change.agent.as_str())
        .collect();
    assert!(touched.contains(&"cursor"), "{touched:?}");
    assert!(touched.contains(&"vscode-copilot"), "{touched:?}");
    assert!(touched.contains(&"claude-code"), "{touched:?}");
    let mut sorted = touched.clone();
    sorted.sort_unstable();
    assert_eq!(touched, sorted, "the report must be sorted");
}

/// The repository root existing is not evidence that an agent is installed, or every repository
/// would grow an `.mcp.json` and an `opencode.json`. A file that is really there is evidence.
#[test]
fn the_repository_root_alone_is_not_evidence() {
    let repo = Repo::new();
    for detected in agents::detect(repo.path()) {
        for target in detected.targets {
            assert!(
                target.exists || target.path.parent() != Some(repo.path()),
                "{} was detected from the bare repository root",
                target.path.display()
            );
        }
    }
    repo.write(".mcp.json", "{}\n");
    let found = agents::detect(repo.path());
    let claude = found
        .iter()
        .find(|detected| detected.agent.id == "claude-code");
    assert!(
        claude.is_some_and(|detected| detected
            .targets
            .iter()
            .any(|target| target.path == repo.at(".mcp.json"))),
        "a file that exists is evidence"
    );
}

/// The report can be read as JSON, and says what changed.
#[test]
fn the_report_is_machine_readable() {
    let repo = Repo::new();
    repo.write(".cursor/mcp.json", "{}\n");
    let report = install(repo.path(), &options("cursor")).expect("a report");
    let value = report.to_value();
    assert_eq!(value["changed"], 1);
    assert_eq!(value["changes"][0]["agent"], "cursor");
    assert_eq!(value["changes"][0]["action"], "update");
    assert!(value["changes"][0]["after"].is_string());
    assert!(report.summary().contains("update"));
    assert!(report.summary().contains("1 of 1 file(s) changed"));
    let skipped = uninstall(repo.path(), &options("zed")).expect("a report");
    assert!(skipped.to_value()["changes"][0]["reason"].is_string());
}

/// The read-only inspection finds our entry, and says so without changing anything.
#[test]
fn inspection_is_read_only() {
    let repo = Repo::new();
    repo.mkdir(".cursor");
    install_one(repo.path(), "cursor");
    let before = repo.read(".cursor/mcp.json");
    let cursor = agents::by_id("cursor").expect("cursor");
    let targets = agents::resolved(cursor, repo.path());
    let project = targets
        .iter()
        .find(|target| target.scope == Scope::Project)
        .expect("a project file");
    assert_eq!(
        install::entry_state(project, "pn-ultramemory"),
        Ok(Some(true))
    );
    assert_eq!(repo.read(".cursor/mcp.json"), before);
}

/// What an uninstall cannot reproduce, and does not pretend to: the several ways of writing an
/// empty object, and a servers object that was already empty, all come back as the one shape an
/// install would have produced them from.
#[test]
fn empty_objects_are_normalized() {
    let cases: &[(&str, &str, &str)] = &[
        ("cursor", "{\n}\n", "{}\n"),
        ("cursor", "{ }", "{}"),
        ("cursor", "{}\n", "{}\n"),
        ("cursor", "{\n  \"mcpServers\": {}\n}\n", "{}\n"),
        (
            "gemini",
            "{\n  \"theme\": \"Default\",\n  \"mcpServers\": {}\n}\n",
            "{\n  \"theme\": \"Default\"\n}\n",
        ),
    ];
    for (agent, original, expected) in cases {
        let repo = Repo::new();
        let file = project_file(agent);
        repo.write(file, original);
        assert!(install_one(repo.path(), agent).action.is_change());
        assert!(uninstall_one(repo.path(), agent).action.is_change());
        assert_eq!(&repo.read(file), expected, "{agent} {original:?}");
    }
}

/// The base directories of an imaginary Linux machine.
fn linux_bases() -> agents::Bases {
    agents::Bases {
        home: PathBuf::from("/home/ada"),
        xdg_config: PathBuf::from("/home/ada/.config"),
        config: PathBuf::from("/home/ada/.config"),
        repo: PathBuf::from("/work/app"),
    }
}

/// The base directories of an imaginary macOS machine.
fn macos_bases() -> agents::Bases {
    agents::Bases {
        home: PathBuf::from("/Users/ada"),
        xdg_config: PathBuf::from("/Users/ada/.config"),
        config: PathBuf::from("/Users/ada/Library/Application Support"),
        repo: PathBuf::from("/work/app"),
    }
}

/// The base directories of an imaginary Windows machine.
fn windows_bases() -> agents::Bases {
    agents::Bases {
        home: PathBuf::from(r"C:\Users\ada"),
        xdg_config: PathBuf::from(r"C:\Users\ada\.config"),
        config: PathBuf::from(r"C:\Users\ada\AppData\Roaming"),
        repo: PathBuf::from(r"C:\work\app"),
    }
}

/// The only file of one agent in one scope.
fn only_target(id: &str, scope: Scope) -> agents::ConfigTarget {
    let agent = agents::by_id(id).unwrap_or_else(|| panic!("{id} is in the table"));
    *agent
        .targets
        .iter()
        .find(|target| target.scope == scope)
        .unwrap_or_else(|| panic!("{id} has no {} file", scope.as_str()))
}

/// A home-relative path resolves against the home directory of each platform.
#[test]
fn home_paths_resolve_per_platform() {
    let codex = only_target("codex", Scope::User).path;
    assert_eq!(
        agents::resolve(&codex, Platform::Linux, &linux_bases()),
        Some(PathBuf::from("/home/ada/.codex/config.toml"))
    );
    assert_eq!(
        agents::resolve(&codex, Platform::MacOs, &macos_bases()),
        Some(PathBuf::from("/Users/ada/.codex/config.toml"))
    );
    assert_eq!(
        agents::resolve(&codex, Platform::Windows, &windows_bases()),
        Some(
            PathBuf::from(r"C:\Users\ada")
                .join(".codex")
                .join("config.toml")
        )
    );
}

/// The configuration base differs per platform, which is how Visual Studio Code is found.
#[test]
fn config_paths_follow_the_platform() {
    let code = only_target("vscode-copilot", Scope::User);
    assert_eq!(
        agents::resolve(&code.path, Platform::MacOs, &macos_bases()),
        Some(PathBuf::from(
            "/Users/ada/Library/Application Support/Code/User/mcp.json"
        ))
    );
    assert_eq!(
        agents::resolve(&code.path, Platform::Linux, &linux_bases()),
        Some(PathBuf::from("/home/ada/.config/Code/User/mcp.json"))
    );
    assert_eq!(
        agents::resolve(&code.path, Platform::Windows, &windows_bases()),
        Some(PathBuf::from(r"C:\Users\ada\AppData\Roaming").join("Code/User/mcp.json"))
    );
    assert_eq!(code.format, agents::Format::Jsonc);
    assert_eq!(code.servers_key, "servers");
}

/// Zed keeps its settings under `~/.config` on macOS too, and under `%APPDATA%` on Windows.
#[test]
fn zed_paths_are_not_the_platform_default() {
    let zed = only_target("zed", Scope::User);
    assert_eq!(
        agents::resolve(&zed.path, Platform::MacOs, &macos_bases()),
        Some(PathBuf::from("/Users/ada/.config/zed/settings.json"))
    );
    assert_eq!(
        agents::resolve(&zed.path, Platform::Windows, &windows_bases()),
        Some(PathBuf::from(r"C:\Users\ada\AppData\Roaming").join("Zed/settings.json"))
    );
    assert_eq!(zed.servers_key, "context_servers");
}

/// Project files hang off the repository root, wherever that is.
#[test]
fn project_paths_hang_off_the_repository() {
    let code = only_target("vscode-copilot", Scope::Project).path;
    assert_eq!(
        agents::resolve(&code, Platform::Linux, &linux_bases()),
        Some(PathBuf::from("/work/app/.vscode/mcp.json"))
    );
    assert_eq!(
        agents::resolve(&code, Platform::Windows, &windows_bases()),
        Some(
            PathBuf::from(r"C:\work\app")
                .join(".vscode")
                .join("mcp.json")
        )
    );
}

/// A platform the agent does not run on resolves to nothing instead of to a guessed path.
#[test]
fn a_missing_platform_resolves_to_nothing() {
    let opencode = only_target("opencode", Scope::User).path;
    assert!(
        opencode.windows.is_none(),
        "no Windows path is documented for opencode"
    );
    assert!(agents::resolve(&opencode, Platform::Windows, &windows_bases()).is_none());
    assert!(agents::resolve(&opencode, Platform::Linux, &linux_bases()).is_some());
    let spec = agents::PathSpec {
        macos: Some(agents::Location {
            base: agents::Base::Home,
            rel: "a/b.json",
        }),
        linux: None,
        windows: None,
    };
    assert!(agents::resolve(&spec, Platform::Linux, &linux_bases()).is_none());
    assert_eq!(
        agents::resolve(&spec, Platform::MacOs, &macos_bases()),
        Some(PathBuf::from("/Users/ada/a/b.json"))
    );
}

/// A relative path with odd separators stays under its base directory.
#[test]
fn relative_paths_stay_relative() {
    let spec = agents::PathSpec {
        macos: Some(agents::Location {
            base: agents::Base::Home,
            rel: "/a//b/",
        }),
        linux: Some(agents::Location {
            base: agents::Base::Home,
            rel: "/a//b/",
        }),
        windows: None,
    };
    assert_eq!(
        agents::resolve(&spec, Platform::Linux, &linux_bases()),
        Some(PathBuf::from("/home/ada/a/b"))
    );
}

/// The platform of this build is one of the three the table knows, and the names in reports are
/// the lowercase ones.
#[test]
fn the_current_platform_is_known() {
    assert!(matches!(
        Platform::current(),
        Platform::MacOs | Platform::Linux | Platform::Windows
    ));
    assert_eq!(Scope::User.as_str(), "user");
    assert_eq!(Scope::Project.as_str(), "project");
    assert_eq!(agents::Format::Toml.as_str(), "toml");
    assert_eq!(agents::Format::Json.as_str(), "json");
    assert_eq!(agents::Format::Jsonc.as_str(), "jsonc");
}

/// Escapes, multi-byte characters and awkward keys never derail the scanner.
#[test]
fn escapes_and_multibyte_text_are_safe() {
    let cases: &[&str] = &[
        r#"{"a\"b": 1, "mcpServers": {"other": {"command": "café"}}}"#,
        r#"{"mcpServers": {"über": {"command": "x\\y"}}}"#,
        r#"{"mcpServers": {"other": {"command": "\/usr\/bin\/x"}}, "note": "→ ✓"}"#,
        "{\"mcpServers\": {\"other\": {\"command\": \"日本語\"}}}",
        r#"{"mcpServers": {"other": {"args": ["--flag=\"quoted\""]}}}"#,
        r#"{"mcpServers": {"other": {"n": -1.5e-3, "t": true, "z": null}}}"#,
    ];
    for original in cases {
        let repo = Repo::new();
        repo.write(".cursor/mcp.json", original);
        let change = install_one(repo.path(), "cursor");
        assert_eq!(change.action, Action::Update, "{original}");
        let after = repo.read(".cursor/mcp.json");
        let parsed: serde_json::Value = serde_json::from_str(&after).expect("valid JSON");
        assert_eq!(parsed["mcpServers"]["pn-ultramemory"]["command"], COMMAND);
        assert_eq!(uninstall_one(repo.path(), "cursor").action, Action::Update);
        assert_eq!(&repo.read(".cursor/mcp.json"), original, "{original}");
    }
}

/// A server name that is not a bare word is quoted where the format needs it.
#[test]
fn an_awkward_server_name_is_quoted() {
    let repo = Repo::new();
    repo.write(".codex/config.toml", "model = \"fast\"\n");
    let odd = InstallOptions {
        server_name: "pn ultra.memory".to_owned(),
        ..options("codex")
    };
    assert!(
        install(repo.path(), &odd).expect("a report").changes[0]
            .action
            .is_change()
    );
    let after = repo.read(".codex/config.toml");
    assert!(
        after.contains("[mcp_servers.\"pn ultra.memory\"]"),
        "{after}"
    );
    assert_eq!(
        uninstall(repo.path(), &odd).expect("a report").changes[0].action,
        Action::Update
    );
    assert_eq!(repo.read(".codex/config.toml"), "model = \"fast\"\n");

    let repo = Repo::new();
    repo.write(".cursor/mcp.json", "{}\n");
    let quoted = InstallOptions {
        server_name: "a\"b".to_owned(),
        ..options("cursor")
    };
    assert!(
        install(repo.path(), &quoted).expect("a report").changes[0]
            .action
            .is_change()
    );
    let parsed: serde_json::Value =
        serde_json::from_str(&repo.read(".cursor/mcp.json")).expect("valid JSON");
    assert_eq!(parsed["mcpServers"]["a\"b"]["command"], COMMAND);
    assert_eq!(
        uninstall(repo.path(), &quoted).expect("a report").changes[0].action,
        Action::Update
    );
    assert_eq!(repo.read(".cursor/mcp.json"), "{}\n");
}
