// SPDX-License-Identifier: Apache-2.0
//! The file-system implementation of the [`SourceTree`] port.
//!
//! # Role in the architecture
//! [`FsSourceTree`] is the exit adapter that lists, reads and writes the files of a repository
//! on disk. It is the only place in this crate that touches the file system.
//!
//! # Invariants
//! * **Paths that leave the root are refused.** Paths that are absolute or contain `..` are
//!   refused before any file-system call, and the real path (after resolving symbolic links)
//!   must stay under the canonical root, which defeats links that point out of the tree.
//! * The containment check happens just before the access and is not atomic with it: it defends
//!   against links and paths that already point out of the tree, not against another process that
//!   rewrites the tree concurrently with malicious timing.
//! * Listing is deterministic: a sorted list of forward-slash relative paths, whatever the
//!   platform and the order the operating system returns entries in.
//! * Writes are atomic: the new content goes to a temporary file in the same directory, which is
//!   then renamed over the original, so a crash leaves either the old or the new file, never a
//!   mix. File permissions are preserved.
//! * No network access, and no state shared between calls.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::UNIX_EPOCH;

use ignore::WalkBuilder;
use pn_ultramemory_core::{Language, SourceError, SourceFile, SourceTree};

/// Directories that are never listed, wherever they are.
const SKIPPED_DIRECTORIES: &[&str] = &["node_modules", "target", "dist", "build", "vendor", ".git"];

/// A first line longer than this many characters marks a file as minified.
const MINIFIED_LINE_CHARS: usize = 2000;

/// How many bytes are read to look at the first line of a file.
const FIRST_LINE_PROBE: usize = 8192;

/// Source of unique names for temporary files.
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A repository on disk.
///
/// # Examples
/// ```
/// use pn_ultramemory_core::SourceTree;
/// use pn_ultramemory_index::FsSourceTree;
///
/// let dir = tempfile::tempdir().unwrap();
/// std::fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();
/// std::fs::write(dir.path().join("notes.bin"), [0u8, 1, 2]).unwrap();
///
/// let tree = FsSourceTree::new(dir.path(), 1_000_000).unwrap();
/// let files = tree.list().unwrap();
/// assert_eq!(files.len(), 1);
/// assert_eq!(files[0].path, "main.rs");
///
/// tree.write("main.rs", "fn main() { run(); }\n").unwrap();
/// assert_eq!(tree.read("main.rs").unwrap(), "fn main() { run(); }\n");
/// assert!(tree.read("../secret").is_err());
/// ```
#[derive(Debug, Clone)]
pub struct FsSourceTree {
    /// The canonical root directory.
    root: PathBuf,
    /// Files larger than this many bytes are not listed.
    max_file_bytes: u64,
}

impl FsSourceTree {
    /// Opens the repository rooted at `root`, which is canonicalized so that later checks
    /// compare real paths.
    ///
    /// # Errors
    /// [`SourceError::Io`] when the root does not exist, cannot be resolved or is not a
    /// directory.
    pub fn new(root: impl Into<PathBuf>, max_file_bytes: u64) -> Result<Self, SourceError> {
        let root = root.into();
        let root = root
            .canonicalize()
            .map_err(|e| SourceError::Io(format!("{}: {e}", root.display())))?;
        if !root.is_dir() {
            return Err(SourceError::Io(format!(
                "{} is not a directory",
                root.display()
            )));
        }
        Ok(Self {
            root,
            max_file_bytes,
        })
    }

    /// The canonical root directory.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The size limit, in bytes, above which files are not listed.
    #[must_use]
    pub const fn max_file_bytes(&self) -> u64 {
        self.max_file_bytes
    }

    /// Resolves a relative path to a real path that is guaranteed to be under the root.
    ///
    /// # Errors
    /// [`SourceError::OutsideRoot`] for an absolute path, a `..` component or a link that leaves
    /// the tree, and [`SourceError::Io`] when the path does not exist.
    fn resolve(&self, path: &str) -> Result<PathBuf, SourceError> {
        if path.is_empty() || path.contains('\0') {
            return Err(SourceError::Io(format!(
                "`{}` is not a valid path",
                path.escape_debug()
            )));
        }
        let relative = Path::new(path);
        if relative.is_absolute() {
            return Err(SourceError::OutsideRoot(path.to_owned()));
        }
        for component in relative.components() {
            match component {
                Component::Normal(_) | Component::CurDir => {}
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                    return Err(SourceError::OutsideRoot(path.to_owned()));
                }
            }
        }
        let real = self
            .root
            .join(relative)
            .canonicalize()
            .map_err(|e| SourceError::Io(format!("{path}: {e}")))?;
        if real.starts_with(&self.root) {
            Ok(real)
        } else {
            Err(SourceError::OutsideRoot(path.to_owned()))
        }
    }
}

/// Returns the path relative to `root` with forward slashes, or `None` when a component is not
/// valid UTF-8.
fn relative_slash_path(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    let mut out = String::new();
    for component in relative.components() {
        if let Component::Normal(part) = component {
            if !out.is_empty() {
                out.push('/');
            }
            out.push_str(part.to_str()?);
        }
    }
    (!out.is_empty()).then_some(out)
}

/// Returns `true` when the file looks minified: a `.min.js` name, or a first line longer than
/// [`MINIFIED_LINE_CHARS`] characters.
fn is_minified(path: &Path, size: u64) -> bool {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if name.ends_with(".min.js") || name.ends_with(".min.mjs") {
        return true;
    }
    if size <= MINIFIED_LINE_CHARS as u64 {
        return false;
    }
    let mut buffer = vec![0u8; FIRST_LINE_PROBE];
    let Ok(mut file) = File::open(path) else {
        return false;
    };
    let mut filled = 0;
    while filled < buffer.len() {
        match file.read(&mut buffer[filled..]) {
            Ok(0) | Err(_) => break,
            Ok(n) => filled += n,
        }
    }
    let probe = &buffer[..filled];
    match probe.iter().position(|b| *b == b'\n') {
        Some(end) => String::from_utf8_lossy(&probe[..end]).chars().count() > MINIFIED_LINE_CHARS,
        None => {
            filled == FIRST_LINE_PROBE
                || String::from_utf8_lossy(probe).chars().count() > MINIFIED_LINE_CHARS
        }
    }
}

/// Returns the modification time in seconds since the Unix epoch, negative before it.
fn mtime_secs(metadata: &fs::Metadata) -> i64 {
    match metadata.modified() {
        Ok(time) => match time.duration_since(UNIX_EPOCH) {
            Ok(after) => i64::try_from(after.as_secs()).unwrap_or(i64::MAX),
            Err(before) => -i64::try_from(before.duration().as_secs()).unwrap_or(i64::MAX),
        },
        Err(_) => 0,
    }
}

impl SourceTree for FsSourceTree {
    fn list(&self) -> Result<Vec<SourceFile>, SourceError> {
        fs::read_dir(&self.root)
            .map_err(|e| SourceError::Io(format!("{}: {e}", self.root.display())))?;
        let walker = WalkBuilder::new(&self.root)
            .hidden(true)
            .ignore(true)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .parents(true)
            .require_git(false)
            .follow_links(false)
            .filter_entry(|entry| {
                let is_dir = entry.file_type().is_some_and(|t| t.is_dir());
                !(is_dir
                    && entry
                        .file_name()
                        .to_str()
                        .is_some_and(|n| SKIPPED_DIRECTORIES.contains(&n)))
            })
            .build();
        let mut files = Vec::new();
        for entry in walker.flatten() {
            if !entry.file_type().is_some_and(|t| t.is_file()) {
                continue;
            }
            let path = entry.path();
            if Language::from_path(path).is_none() {
                continue;
            }
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if metadata.len() > self.max_file_bytes || is_minified(path, metadata.len()) {
                continue;
            }
            let Some(relative) = relative_slash_path(&self.root, path) else {
                continue;
            };
            files.push(SourceFile {
                path: relative,
                size: metadata.len(),
                mtime_secs: mtime_secs(&metadata),
            });
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        files.dedup_by(|a, b| a.path == b.path);
        Ok(files)
    }

    fn read(&self, path: &str) -> Result<String, SourceError> {
        let real = self.resolve(path)?;
        let bytes = fs::read(&real).map_err(|e| SourceError::Io(format!("{path}: {e}")))?;
        String::from_utf8(bytes).map_err(|_| SourceError::NotText(path.to_owned()))
    }

    fn write(&self, path: &str, content: &str) -> Result<(), SourceError> {
        let real = self.resolve(path)?;
        let metadata = fs::metadata(&real).map_err(|e| SourceError::Io(format!("{path}: {e}")))?;
        if !metadata.is_file() {
            return Err(SourceError::Io(format!("{path} is not a file")));
        }
        let directory = real
            .parent()
            .ok_or_else(|| SourceError::Io(format!("{path} has no directory")))?;
        let name = real
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| SourceError::Io(format!("{path} has no usable file name")))?;
        let unique = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let temporary = directory.join(format!(".{name}.{}.{unique}.tmp", std::process::id()));
        let outcome = write_temporary(&temporary, content, &metadata)
            .and_then(|()| fs::rename(&temporary, &real));
        if let Err(error) = outcome {
            let _ = fs::remove_file(&temporary);
            return Err(SourceError::Io(format!("{path}: {error}")));
        }
        Ok(())
    }
}

/// Creates `temporary`, writes `content` to it, copies the permissions of the original and
/// flushes everything to disk.
fn write_temporary(
    temporary: &Path,
    content: &str,
    original: &fs::Metadata,
) -> std::io::Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temporary)?;
    file.write_all(content.as_bytes())?;
    file.set_permissions(original.permissions())?;
    file.sync_all()
}
