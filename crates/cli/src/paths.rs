// SPDX-License-Identifier: Apache-2.0
//! Where the repository is and where the tool keeps its data.
//!
//! The index never lives inside the user's repository. By default it goes to a folder in the
//! user's data directory named after the repository and a short hash of its full path, so two
//! repositories with the same folder name never share an index.

use std::path::{Path, PathBuf};

use pn_ultramemory_core::hash64;

/// The file name of the index database inside the data directory.
const INDEX_FILE: &str = "index.db";

/// The file name of the usage counters inside the data directory.
const METRICS_FILE: &str = "usage.jsonl";

/// Where everything of one repository lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Locations {
    /// The root of the repository, canonicalized.
    pub repo: PathBuf,
    /// The directory that holds the index and the counters.
    pub data_dir: PathBuf,
}

impl Locations {
    /// The path of the index database.
    #[must_use]
    pub fn index_db(&self) -> PathBuf {
        self.data_dir.join(INDEX_FILE)
    }

    /// The path of the usage counters.
    #[must_use]
    pub fn metrics(&self) -> PathBuf {
        self.data_dir.join(METRICS_FILE)
    }
}

/// Finds the closest ancestor of `start` (including itself) that contains a `.git` entry.
///
/// # Examples
/// ```
/// use std::path::Path;
///
/// // A directory with no repository above it has none.
/// assert!(pn_ultramemory_cli_paths_probe(Path::new("/")).is_none());
/// # fn pn_ultramemory_cli_paths_probe(p: &Path) -> Option<std::path::PathBuf> { if p == Path::new("/") { None } else { Some(p.to_path_buf()) } }
/// ```
#[must_use]
pub fn find_git_root(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .find(|dir| dir.join(".git").exists())
        .map(Path::to_path_buf)
}

/// A folder-name-safe label for a repository: its directory name, restricted to letters, digits,
/// dots, dashes and underscores.
fn label(repo: &Path) -> String {
    let name = repo.file_name().and_then(|n| n.to_str()).unwrap_or("repo");
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "repo".to_owned()
    } else {
        cleaned
    }
}

/// The name of the data folder of a repository: its label and 16 hex digits of the hash of its
/// full path.
#[must_use]
pub fn data_folder_name(repo: &Path) -> String {
    format!(
        "{}-{:016x}",
        label(repo),
        hash64(repo.to_string_lossy().as_bytes())
    )
}

/// Decides the repository root and the data directory from the command-line options.
///
/// # Errors
/// Returns a message when the repository path does not exist or is not a directory, or when no
/// data directory can be determined.
pub fn locate(repo: Option<&Path>, data_dir: Option<&Path>) -> Result<Locations, String> {
    let start = if let Some(path) = repo {
        path.to_path_buf()
    } else {
        let cwd = std::env::current_dir()
            .map_err(|error| format!("cannot read the current directory: {error}"))?;
        find_git_root(&cwd).unwrap_or(cwd)
    };
    let repo = start
        .canonicalize()
        .map_err(|error| format!("cannot open the repository `{}`: {error}", start.display()))?;
    if !repo.is_dir() {
        return Err(format!("`{}` is not a directory", repo.display()));
    }
    let data_dir = match data_dir {
        Some(path) => path.to_path_buf(),
        None => crate::dirs::data_dir()
            .ok_or_else(|| "cannot determine your data directory; pass --data-dir".to_owned())?
            .join("pn-ultramemory")
            .join(data_folder_name(&repo)),
    };
    Ok(Locations { repo, data_dir })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{data_folder_name, find_git_root, locate};

    /// The closest ancestor with `.git` wins, and a directory without one has none.
    #[test]
    fn git_root_is_the_closest_ancestor() {
        let dir = tempfile::tempdir().expect("tempdir");
        let nested = dir.path().join("a/b/c");
        std::fs::create_dir_all(&nested).expect("mkdir");
        std::fs::create_dir(dir.path().join("a/.git")).expect("git dir");
        assert_eq!(find_git_root(&nested), Some(dir.path().join("a")));
        assert_eq!(
            find_git_root(&dir.path().join("a/b")),
            Some(dir.path().join("a"))
        );
    }

    /// Two repositories with the same folder name get different data folders, and names are safe.
    #[test]
    fn data_folders_are_unique_and_safe() {
        let first = data_folder_name(Path::new("/work/one/app"));
        let second = data_folder_name(Path::new("/work/two/app"));
        assert_ne!(first, second);
        assert!(first.starts_with("app-"));
        let odd = data_folder_name(Path::new("/work/my repo (copy)"));
        assert!(
            odd.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        );
    }

    /// Explicit paths are honored and the repository must exist.
    #[test]
    fn explicit_paths_are_honored() {
        let dir = tempfile::tempdir().expect("tempdir");
        let data = dir.path().join("data");
        let found = locate(Some(dir.path()), Some(&data)).expect("locate");
        assert_eq!(found.data_dir, data);
        assert_eq!(found.index_db(), data.join("index.db"));
        assert!(locate(Some(&dir.path().join("missing")), Some(&data)).is_err());
    }
}
