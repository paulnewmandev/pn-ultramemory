// SPDX-License-Identifier: Apache-2.0
//! Checks that everything this tool needs is in place, and says exactly what to run when it is not.
//!
//! # Role in the architecture
//! A read-only report, built from [`crate::agents`] and [`crate::install`] plus a few questions put
//! to the file system. It changes nothing except one temporary file, which it writes to prove the
//! data directory is writable and then deletes.
//!
//! # The checks
//! | Name | Passes when |
//! |---|---|
//! | `repository` | the repository exists and can be listed |
//! | `git` | it is a Git working tree (informational: not being one is fine) |
//! | `data directory` | it exists and a file can really be written in it |
//! | `data directory location` | it is *not* inside the repository |
//! | `index` | the index database is there; its size and age are reported |
//! | `executable` | the version is known; the path of this binary is reported |
//! | `agents` | at least one detected agent already has our entry |
//! | `hooks` | informational: whether the environment switches hooks off |
//! | `disk space` | there is room left where the data directory lives |
//!
//! # Invariants
//! * It never panics and never fails: a question that cannot be answered on this platform is
//!   reported as unanswered, which counts as a pass, not as a problem.
//! * Every check that does not pass carries a hint naming the command that fixes it.
//! * The output is deterministic: a fixed order of checks and a sorted list of agents.

use std::fmt::Write as _;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};

use crate::agents;
use crate::install::{DEFAULT_SERVER_NAME, entry_state};

/// The name of the index database inside the data directory.
const INDEX_FILE: &str = "index.db";

/// Below this much free space, indexing a large repository is likely to fail.
const LOW_SPACE_BYTES: u64 = 64 * 1024 * 1024;

/// One question and its answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    /// A short, stable name, lowercase.
    pub name: String,
    /// Whether everything is as it should be. A question that could not be answered passes.
    pub ok: bool,
    /// What was found.
    pub detail: String,
    /// What to run about it, when there is something to run.
    pub hint: Option<String>,
}

impl Check {
    /// A check that passed.
    fn good(name: &str, detail: impl Into<String>) -> Self {
        Self {
            name: name.to_owned(),
            ok: true,
            detail: detail.into(),
            hint: None,
        }
    }

    /// A check that failed, with the command that puts it right.
    fn bad(name: &str, detail: impl Into<String>, hint: impl Into<String>) -> Self {
        Self {
            name: name.to_owned(),
            ok: false,
            detail: detail.into(),
            hint: Some(hint.into()),
        }
    }
}

/// Every check, and whether they all passed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoctorReport {
    /// The checks, always in the same order.
    pub checks: Vec<Check>,
    /// Whether every check passed.
    pub ok: bool,
}

impl DoctorReport {
    /// The report as JSON, for `--format json`.
    #[must_use]
    pub fn to_value(&self) -> Value {
        let checks: Vec<Value> = self
            .checks
            .iter()
            .map(|check| {
                let mut entry = Map::new();
                entry.insert("name".to_owned(), json!(check.name));
                entry.insert("ok".to_owned(), json!(check.ok));
                entry.insert("detail".to_owned(), json!(check.detail));
                entry.insert("hint".to_owned(), json!(check.hint));
                Value::Object(entry)
            })
            .collect();
        json!({ "ok": self.ok, "checks": checks })
    }

    /// One line per check, then a verdict.
    #[must_use]
    pub fn summary(&self) -> String {
        let mut out = String::new();
        for check in &self.checks {
            let mark = if check.ok { "ok  " } else { "FAIL" };
            let _ = writeln!(out, "{mark} {:<22} {}", check.name, check.detail);
            if let Some(hint) = &check.hint {
                let _ = writeln!(out, "     {:<22} -> {hint}", "");
            }
        }
        let failed = self.checks.iter().filter(|check| !check.ok).count();
        let _ = write!(
            out,
            "{}",
            if failed == 0 {
                "everything is in place".to_owned()
            } else {
                format!("{failed} of {} checks need attention", self.checks.len())
            }
        );
        out
    }
}

/// Runs every check against one repository and one data directory.
#[must_use]
pub fn doctor(repo: &Path, data_dir: &Path) -> DoctorReport {
    let checks = vec![
        repository_check(repo),
        git_check(repo),
        data_dir_check(data_dir),
        location_check(repo, data_dir),
        index_check(data_dir),
        executable_check(),
        agents_check(repo),
        hooks_check(),
        space_check(data_dir),
    ];
    let ok = checks.iter().all(|check| check.ok);
    DoctorReport { checks, ok }
}

/// Whether the repository is there and can be listed.
fn repository_check(repo: &Path) -> Check {
    let shown = repo.display();
    match std::fs::read_dir(repo) {
        Ok(_) => Check::good("repository", format!("{shown}")),
        Err(error) => Check::bad(
            "repository",
            format!("cannot read `{shown}`: {error}"),
            "check the path, or pass --repo <PATH>",
        ),
    }
}

/// Whether the repository is a Git working tree. Not being one is not a problem.
fn git_check(repo: &Path) -> Check {
    if repo.join(".git").exists() {
        Check::good("git", "a Git working tree")
    } else {
        Check::good("git", "not a Git working tree; every file will be indexed")
    }
}

/// Whether the data directory exists and a file can really be written in it.
fn data_dir_check(data_dir: &Path) -> Check {
    let shown = data_dir.display();
    if !data_dir.is_dir() {
        return Check::bad(
            "data directory",
            format!("`{shown}` does not exist yet"),
            "run `pn-ultramemory index` once; it creates the directory",
        );
    }
    let probe = data_dir.join(format!(".pn-ultramemory-write-test-{}", std::process::id()));
    match std::fs::write(&probe, b"ok") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            Check::good("data directory", format!("{shown} (writable)"))
        }
        Err(error) => Check::bad(
            "data directory",
            format!("cannot write in `{shown}`: {error}"),
            "fix the permissions, or pass --data-dir <PATH>",
        ),
    }
}

/// Whether the data directory sits outside the repository, where it belongs.
fn location_check(repo: &Path, data_dir: &Path) -> Check {
    let inside = match (repo.canonicalize(), data_dir.canonicalize()) {
        (Ok(repo), Ok(data)) => data.starts_with(&repo),
        _ => data_dir.starts_with(repo),
    };
    if inside {
        Check::bad(
            "data directory location",
            format!("`{}` is inside the repository", data_dir.display()),
            "move it out: pass --data-dir <PATH> outside the repository, or unset \
             PN_ULTRAMEMORY_HOME",
        )
    } else {
        Check::good("data directory location", "outside the repository")
    }
}

/// Whether the index is there, how large it is and when it was last written.
fn index_check(data_dir: &Path) -> Check {
    let path = data_dir.join(INDEX_FILE);
    let shown = path.display();
    match path.metadata() {
        Ok(meta) if meta.is_file() => {
            let written = meta.modified().map_or_else(
                |_| "an unknown time".to_owned(),
                |time| format!("at {}", utc(time)),
            );
            Check::good(
                "index",
                format!("{shown}, {}, last written {written}", bytes(meta.len())),
            )
        }
        _ => Check::bad(
            "index",
            format!("there is no index at `{shown}`"),
            "run `pn-ultramemory index`",
        ),
    }
}

/// The version of this build and, when it can be read, the path it runs from.
fn executable_check() -> Check {
    let version = env!("CARGO_PKG_VERSION");
    match std::env::current_exe() {
        Ok(path) => Check::good("executable", format!("{version} at {}", path.display())),
        Err(error) => Check::good(
            "executable",
            format!("{version}; the path of this executable could not be read: {error}"),
        ),
    }
}

/// Which agents were found, and which of them already start this server.
fn agents_check(repo: &Path) -> Check {
    let mut lines: Vec<String> = Vec::new();
    let mut configured = 0;
    for detected in agents::detect(repo) {
        let mut states: Vec<String> = Vec::new();
        for target in &detected.targets {
            let state = match entry_state(target, DEFAULT_SERVER_NAME) {
                Ok(Some(true)) => {
                    configured += 1;
                    "configured".to_owned()
                }
                Ok(Some(false)) => "not configured".to_owned(),
                Ok(None) => "no file yet".to_owned(),
                Err(reason) => reason,
            };
            states.push(format!("{} ({state})", target.path.display()));
        }
        states.sort_unstable();
        lines.push(format!("{}: {}", detected.agent.id, states.join(", ")));
    }
    lines.sort_unstable();
    if lines.is_empty() {
        return Check::good("agents", "no coding agent was detected on this machine");
    }
    let detail = lines.join("; ");
    if configured == 0 {
        return Check::bad("agents", detail, "run `pn-ultramemory install`");
    }
    Check::good("agents", detail)
}

/// Whether the environment switches the hooks off.
fn hooks_check() -> Check {
    if std::env::var_os(crate::hook::DISABLE_ENV).is_some_and(|value| !value.is_empty()) {
        Check::good(
            "hooks",
            format!("switched off by {}", crate::hook::DISABLE_ENV),
        )
    } else {
        Check::good("hooks", "enabled")
    }
}

/// How much room is left where the data directory lives, when that can be found out.
fn space_check(data_dir: &Path) -> Check {
    let Some(free) = free_space(data_dir) else {
        return Check::good("disk space", "could not be determined on this platform");
    };
    if free < LOW_SPACE_BYTES {
        return Check::bad(
            "disk space",
            format!("only {} free where the index lives", bytes(free)),
            "free some space, or pass --data-dir <PATH> on another disk",
        );
    }
    Check::good("disk space", format!("{} free", bytes(free)))
}

/// The free space of the file system that holds `path`, asked of `df`, or nothing when that cannot
/// be done. It is best effort by design: no platform library and no failure of its own.
#[cfg(unix)]
fn free_space(path: &Path) -> Option<u64> {
    let mut probe = path;
    while !probe.exists() {
        probe = probe.parent()?;
    }
    let output = std::process::Command::new("df")
        .arg("-Pk")
        .arg(probe)
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let row = text.lines().nth(1)?;
    let available: u64 = row.split_whitespace().nth(3)?.parse().ok()?;
    available.checked_mul(1024)
}

/// There is no way to ask this question here without a platform library, so it is not asked.
#[cfg(not(unix))]
fn free_space(_path: &Path) -> Option<u64> {
    None
}

/// A byte count a person can read, in binary units.
fn bytes(count: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut unit = 0;
    let mut scale: u64 = 1;
    while count / scale >= 1024 && unit + 1 < UNITS.len() {
        scale *= 1024;
        unit += 1;
    }
    if unit == 0 {
        return format!("{count} B");
    }
    format!(
        "{}.{} {}",
        count / scale,
        count % scale * 10 / scale,
        UNITS[unit]
    )
}

/// A moment as `YYYY-MM-DDTHH:MM:SSZ`.
///
/// The date is worked out with the usual civil-from-days arithmetic rather than a calendar library,
/// because one timestamp is not worth a dependency. Anything before 1970 is said to be so.
fn utc(time: SystemTime) -> String {
    let Ok(since) = time.duration_since(UNIX_EPOCH) else {
        return "a time before 1970".to_owned();
    };
    let seconds = since.as_secs();
    let (days, rest) = (seconds / 86_400, seconds % 86_400);
    let (hour, minute, second) = (rest / 3_600, (rest % 3_600) / 60, rest % 60);
    let z = i64::try_from(days).unwrap_or(0) + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = u64::try_from(z - era * 146_097).unwrap_or(0);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = i64::try_from(yoe).unwrap_or(0) + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use super::{DEFAULT_SERVER_NAME, bytes, doctor, utc};

    /// A repository and a data directory beside each other, in a temporary place.
    fn setup() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let repo = dir.path().join("repo");
        let data = dir.path().join("data");
        std::fs::create_dir_all(&repo).expect("a repository");
        std::fs::create_dir_all(&data).expect("a data directory");
        (dir, repo, data)
    }

    /// A healthy setup passes the checks that can be decided, and every check keeps its name.
    #[test]
    fn a_healthy_setup_is_reported_as_such() {
        let (_dir, repo, data) = setup();
        std::fs::create_dir(repo.join(".git")).expect("a git directory");
        std::fs::write(data.join("index.db"), b"an index").expect("an index");
        let report = doctor(&repo, &data);
        let names: Vec<&str> = report
            .checks
            .iter()
            .map(|check| check.name.as_str())
            .collect();
        assert_eq!(
            names,
            [
                "repository",
                "git",
                "data directory",
                "data directory location",
                "index",
                "executable",
                "agents",
                "hooks",
                "disk space"
            ]
        );
        let index = &report.checks[4];
        assert!(index.ok, "{index:?}");
        assert!(index.detail.contains("8 B"), "{index:?}");
        assert!(
            index.detail.contains('Z'),
            "the time should be stamped: {index:?}"
        );
        assert!(
            report.checks[1].detail.contains("Git"),
            "{:?}",
            report.checks[1]
        );
        assert_eq!(report.to_value()["checks"][0]["name"], "repository");
        assert!(report.summary().contains("repository"));
    }

    /// Every check that does not pass says what to run, and the report knows it failed.
    #[test]
    fn failures_always_carry_a_hint() {
        let (_dir, repo, _data) = setup();
        let inside = repo.join("nested").join("data");
        std::fs::create_dir_all(&inside).expect("a nested data directory");
        let report = doctor(&repo, &inside);
        assert!(!report.ok);
        for check in &report.checks {
            assert_eq!(check.ok, check.hint.is_none(), "{check:?}");
        }
        let location = report
            .checks
            .iter()
            .find(|check| check.name == "data directory location");
        assert!(location.is_some_and(|check| !check.ok), "{location:?}");
        assert!(report.summary().contains("need attention"));

        let missing = doctor(Path::new("/no/such/repository"), Path::new("/no/such/data"));
        assert!(!missing.ok);
        assert!(missing.checks.iter().filter(|check| !check.ok).count() >= 2);
        assert!(
            missing
                .checks
                .iter()
                .all(|check| check.ok == check.hint.is_none())
        );
    }

    /// A detected agent that already starts this server passes the check; one that does not asks
    /// for the command that fixes it.
    #[test]
    fn the_agents_check_reads_the_files_it_finds() {
        let (_dir, repo, data) = setup();
        std::fs::create_dir_all(repo.join(".cursor")).expect("a cursor directory");
        std::fs::write(repo.join(".cursor/mcp.json"), "{}\n").expect("a configuration");
        let before = doctor(&repo, &data);
        let check = before
            .checks
            .iter()
            .find(|check| check.name == "agents")
            .expect("a check");
        assert!(check.detail.contains("cursor"), "{check:?}");
        assert!(
            check.ok || check.hint.as_deref() == Some("run `pn-ultramemory install`"),
            "{check:?}"
        );

        std::fs::write(
            repo.join(".cursor/mcp.json"),
            format!("{{\n  \"mcpServers\": {{\n    \"{DEFAULT_SERVER_NAME}\": {{\n      \"command\": \"x\"\n    }}\n  }}\n}}\n"),
        )
        .expect("a configuration with our entry");
        let after = doctor(&repo, &data);
        let check = after
            .checks
            .iter()
            .find(|check| check.name == "agents")
            .expect("a check");
        assert!(check.ok, "{check:?}");
        assert!(check.detail.contains("configured"), "{check:?}");
        assert!(check.hint.is_none(), "{check:?}");
    }

    /// A file that cannot be read as its format is reported in the detail, never as a panic.
    #[test]
    fn a_broken_agent_file_is_reported() {
        let (_dir, repo, data) = setup();
        std::fs::create_dir_all(repo.join(".cursor")).expect("a cursor directory");
        std::fs::write(repo.join(".cursor/mcp.json"), "{not json").expect("a broken file");
        let report = doctor(&repo, &data);
        let check = report
            .checks
            .iter()
            .find(|check| check.name == "agents")
            .expect("a check");
        assert!(check.detail.contains("cursor"), "{check:?}");
        assert!(check.detail.contains("not valid"), "{check:?}");
    }

    /// Timestamps and sizes are formatted without a calendar library and without a panic.
    #[test]
    fn timestamps_and_sizes_are_formatted() {
        assert_eq!(utc(UNIX_EPOCH), "1970-01-01T00:00:00Z");
        assert_eq!(
            utc(UNIX_EPOCH + Duration::from_secs(1_700_000_000)),
            "2023-11-14T22:13:20Z"
        );
        assert_eq!(
            utc(UNIX_EPOCH + Duration::from_secs(951_782_400)),
            "2000-02-29T00:00:00Z"
        );
        assert_eq!(
            utc(UNIX_EPOCH - Duration::from_secs(1)),
            "a time before 1970"
        );
        assert_eq!(utc(SystemTime::now()).len(), 20);
        assert_eq!(bytes(0), "0 B");
        assert_eq!(bytes(1_023), "1023 B");
        assert_eq!(bytes(1_024), "1.0 KiB");
        assert_eq!(bytes(1_536), "1.5 KiB");
        assert_eq!(bytes(4 * 1024 * 1024 * 1024), "4.0 GiB");
        assert!(
            bytes(u64::MAX).ends_with(" TiB"),
            "the largest unit is used, never a panic"
        );
    }
}
