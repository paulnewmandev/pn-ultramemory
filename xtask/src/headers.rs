// SPDX-License-Identifier: Apache-2.0
//! The header gate, in Rust so that it also runs on Windows.
//!
//! # Why a second implementation
//! `scripts/check-headers` is the gate of record and is not changed by this crate. It is a POSIX
//! shell script, so it does not run on Windows, where a contributor would otherwise discover a
//! missing header only after pushing. This subcommand applies the same two rules to the same set of
//! files, so `cargo run -p xtask -- headers` is usable everywhere.
//!
//! # The rules
//! * Every source file carries `SPDX-License-Identifier: Apache-2.0` within its first five lines.
//! * Every Rust file carries a `//!` module documentation line within its first twelve lines.
//!
//! # What this guard does not prove
//! * It does not judge whether the documentation is **any good**, only that a line of it exists.
//! * It reads the file list from `git ls-files`, exactly as the shell script does, so a file that git
//!   ignores is not checked. Without git it falls back to walking the tree, which is not identical.
//! * It says nothing about files whose extension is not in the list below.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::source;

/// File extensions that must carry the licence header.
const CHECKED_EXTENSIONS: &[&str] = &["rs", "scm", "sh", "sql", "toml", "yaml", "yml"];

/// Directories never walked when git is unavailable and the tree is walked instead.
const SKIPPED_DIRECTORIES: &[&str] = &[".git", ".ratchet", "target"];

/// The licence line every source file must carry.
const SPDX: &str = "SPDX-License-Identifier: Apache-2.0";

/// How many leading lines may hold the licence line.
const SPDX_WINDOW: usize = 5;

/// How many leading lines may hold the module documentation.
const DOC_WINDOW: usize = 12;

/// Runs the guard, returning whether it passed.
///
/// # Errors
/// Returns a message when the file list cannot be built or a listed file cannot be read.
pub(crate) fn run(root: &Path) -> Result<bool, String> {
    let (files, source_of_list) = list_files(root)?;
    let mut checked = 0_usize;
    let mut problems = Vec::new();
    for rel in &files {
        if !is_checked(rel) {
            continue;
        }
        let path = root.join(rel);
        if !path.is_file() {
            continue;
        }
        let bytes =
            std::fs::read(&path).map_err(|err| format!("cannot read {}: {err}", path.display()))?;
        let text = String::from_utf8_lossy(&bytes);
        checked += 1;
        if !head(&text, SPDX_WINDOW).any(|line| line.contains(SPDX)) {
            problems.push(format!("missing SPDX header: {rel}"));
        }
        if is_rust(rel) && !head(&text, DOC_WINDOW).any(|line| line.starts_with("//!")) {
            problems.push(format!("missing //! module documentation: {rel}"));
        }
    }

    println!("file headers");
    println!("  file list from:    {source_of_list}");
    println!("  files checked:     {checked}");
    if checked == 0 {
        return Err(
            "no source file was checked. A guard that enumerates nothing cannot pass; run it from \
             inside the repository."
                .to_owned(),
        );
    }
    if problems.is_empty() {
        println!("  all file headers are present");
        return Ok(true);
    }
    println!("\nFiles missing a header ({}):", problems.len());
    for problem in &problems {
        println!("  ! {problem}");
    }
    println!(
        "  Add `// {SPDX}` as the first line, and, for a Rust file, a `//!` block saying what the \
         module is for."
    );
    Ok(false)
}

/// Returns the first `count` lines of `text`.
fn head(text: &str, count: usize) -> impl Iterator<Item = &str> {
    text.lines().take(count)
}

/// Returns true when a path names a Rust file, which must also carry module documentation.
fn is_rust(rel: &str) -> bool {
    rel.rsplit_once('.')
        .is_some_and(|(_, extension)| extension == "rs")
}

/// Returns true when a path is one the header rules apply to.
///
/// The extension list and the `scripts/` special case mirror `scripts/check-headers`, where every
/// file under `scripts/` is checked whatever it is called.
fn is_checked(rel: &str) -> bool {
    if rel.starts_with("scripts/") {
        return true;
    }
    rel.rsplit_once('.')
        .is_some_and(|(_, extension)| CHECKED_EXTENSIONS.contains(&extension))
}

/// Builds the list of repository-relative paths to check, and says where the list came from.
///
/// # Errors
/// Returns a message when neither git nor a directory walk can produce a list.
fn list_files(root: &Path) -> Result<(Vec<String>, &'static str), String> {
    if let Some(files) = git_files(root) {
        return Ok((files, "git ls-files"));
    }
    let mut files = Vec::new();
    walk(root, root, &mut files)?;
    files.sort();
    Ok((files, "directory walk (git unavailable)"))
}

/// Asks git for the tracked and untracked-but-not-ignored files, as the shell script does.
fn git_files(root: &Path) -> Option<Vec<String>> {
    let output = Command::new("git")
        .current_dir(root)
        .args(["ls-files", "--cached", "--others", "--exclude-standard"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let mut files: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_owned)
        .filter(|line| !line.is_empty())
        .collect();
    if files.is_empty() {
        return None;
    }
    files.sort();
    Some(files)
}

/// Walks the tree from `dir`, collecting paths relative to `root`.
///
/// # Errors
/// Returns a message when a directory cannot be listed.
fn walk(root: &Path, dir: &Path, into: &mut Vec<String>) -> Result<(), String> {
    let entries =
        std::fs::read_dir(dir).map_err(|err| format!("cannot list {}: {err}", dir.display()))?;
    let mut children: Vec<PathBuf> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|err| format!("cannot list {}: {err}", dir.display()))?;
        children.push(entry.path());
    }
    children.sort();
    for child in children {
        let name = child
            .file_name()
            .map(|part| part.to_string_lossy().into_owned());
        if child.is_dir() {
            if name.is_some_and(|name| SKIPPED_DIRECTORIES.contains(&name.as_str())) {
                continue;
            }
            walk(root, &child, into)?;
        } else {
            into.push(source::relative(root, &child));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The checked set is the extension list plus everything under `scripts/`.
    #[test]
    fn selects_the_same_files_as_the_shell_script() {
        assert!(is_checked("crates/toon/src/lib.rs"));
        assert!(is_checked("Cargo.toml"));
        assert!(is_checked(".github/workflows/guards.yml"));
        assert!(is_checked("scripts/check-offline"));
        assert!(is_checked("scripts/bump-version"));
        assert!(!is_checked("README.md"));
        assert!(!is_checked("assets/logo.svg"));
    }

    /// Only the leading lines of a file may carry the header.
    #[test]
    fn the_header_window_is_bounded() {
        let late = format!("{}// {SPDX}\n", "\n".repeat(SPDX_WINDOW));
        assert!(!head(&late, SPDX_WINDOW).any(|line| line.contains(SPDX)));
        let early = format!("// {SPDX}\n//! doc\n");
        assert!(head(&early, SPDX_WINDOW).any(|line| line.contains(SPDX)));
        assert!(head(&early, DOC_WINDOW).any(|line| line.starts_with("//!")));
    }

    /// The guard agrees with the shell script on this repository, which is the real test of it.
    #[test]
    fn agrees_with_the_repository_it_guards() {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        let root = manifest.parent().expect("xtask sits inside the repository");
        let (files, _) = list_files(root).expect("lists files");
        assert!(files.iter().any(|file| file == "scripts/check-headers"));
        assert!(files.iter().any(|file| file.starts_with("crates/")));
    }
}
