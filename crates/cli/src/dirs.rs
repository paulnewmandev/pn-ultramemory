// SPDX-License-Identifier: Apache-2.0
//! Where the platform expects an application to keep its files.
//!
//! # Why this exists
//! Only three directories are ever needed, and the obvious crate for them pulls in a dependency
//! under a copyleft licence, which the project's dependency policy does not allow (see
//! `deny.toml`). The conventions are short and stable, so they are implemented here instead: the
//! whole module is a few environment-variable lookups, it adds no dependency, and it is testable
//! without touching the real environment.
//!
//! # The conventions
//! | Function | Linux | macOS | Windows |
//! |---|---|---|---|
//! | [`home_dir`] | `$HOME` | `$HOME` | `%USERPROFILE%` |
//! | [`config_dir`] | `$XDG_CONFIG_HOME`, else `$HOME/.config` | `$HOME/Library/Application Support` | `%APPDATA%` |
//! | [`data_dir`] | `$XDG_DATA_HOME`, else `$HOME/.local/share` | `$HOME/Library/Application Support` | `%APPDATA%` |
//! | [`xdg_config_dir`] | `$XDG_CONFIG_HOME`, else `$HOME/.config` | same | same |
//!
//! [`xdg_config_dir`] is separate because several tools follow the freedesktop layout on every
//! platform, including macOS, where [`config_dir`] points somewhere else.
//!
//! # Invariants
//! An environment variable that is empty, or that holds a relative path, is ignored: a relative
//! path here would place a user's data somewhere that depends on the working directory.

use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The operating system whose conventions to follow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    /// Linux, and other systems that follow the freedesktop layout.
    Unix,
    /// macOS.
    MacOs,
    /// Windows, where both directories are the roaming application-data folder.
    Windows,
}

impl Os {
    /// The system this binary was built for.
    #[must_use]
    pub const fn current() -> Self {
        if cfg!(target_os = "windows") {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::MacOs
        } else {
            Self::Unix
        }
    }
}

/// Whether a path is absolute under the conventions of `os`.
///
/// [`Path::is_absolute`] answers for the platform the binary was **built** for, not for the one
/// being resolved. That is right at run time, since [`Os::current`] is the build target, but it
/// makes the Windows rules untestable from anywhere else: `C:\Users\Paul` is not absolute to a
/// Unix build, so a test of Windows behaviour would fail on the machine running it rather than
/// report anything about Windows. Deciding it from `os` keeps the answer the same wherever the
/// test runs, which is the whole reason these functions take the platform as an argument.
///
/// A Windows path is absolute when it names a drive (`C:\`, `C:/`) or a UNC share (`\\host\share`).
fn is_absolute_for(os: Os, path: &Path) -> bool {
    match os {
        Os::Unix | Os::MacOs => path.starts_with("/"),
        Os::Windows => {
            let text = path.as_os_str().to_string_lossy();
            let bytes = text.as_bytes();
            let drive = matches!(bytes, [letter, b':', b'\\' | b'/', ..]
                if letter.is_ascii_alphabetic());
            drive || text.starts_with(r"\\")
        }
    }
}

/// Reads an environment variable, ignoring one that is empty or holds a relative path.
fn absolute_var(read: &dyn Fn(&str) -> Option<OsString>, name: &str, os: Os) -> Option<PathBuf> {
    let value = read(name)?;
    let path = PathBuf::from(value);
    is_absolute_for(os, &path).then_some(path)
}

/// Reads an environment variable from the real environment.
fn real_env(name: &str) -> Option<OsString> {
    env::var_os(name)
}

/// The home directory, given an environment.
fn home_with(read: &dyn Fn(&str) -> Option<OsString>, os: Os) -> Option<PathBuf> {
    match os {
        Os::Windows => absolute_var(read, "USERPROFILE", os),
        Os::Unix | Os::MacOs => absolute_var(read, "HOME", os),
    }
}

/// The configuration directory, given an environment.
fn config_with(read: &dyn Fn(&str) -> Option<OsString>, os: Os) -> Option<PathBuf> {
    match os {
        Os::Windows => absolute_var(read, "APPDATA", os),
        Os::MacOs => Some(
            home_with(read, os)?
                .join("Library")
                .join("Application Support"),
        ),
        Os::Unix => absolute_var(read, "XDG_CONFIG_HOME", os)
            .or_else(|| Some(home_with(read, os)?.join(".config"))),
    }
}

/// The data directory, given an environment.
fn data_with(read: &dyn Fn(&str) -> Option<OsString>, os: Os) -> Option<PathBuf> {
    match os {
        Os::Windows => absolute_var(read, "APPDATA", os),
        Os::MacOs => Some(
            home_with(read, os)?
                .join("Library")
                .join("Application Support"),
        ),
        Os::Unix => absolute_var(read, "XDG_DATA_HOME", os)
            .or_else(|| Some(home_with(read, os)?.join(".local").join("share"))),
    }
}

/// The freedesktop configuration directory, given an environment, on every platform.
fn xdg_config_with(read: &dyn Fn(&str) -> Option<OsString>, os: Os) -> Option<PathBuf> {
    absolute_var(read, "XDG_CONFIG_HOME", os).or_else(|| Some(home_with(read, os)?.join(".config")))
}

/// The user's home directory, or `None` when the environment does not say where it is.
#[must_use]
pub fn home_dir() -> Option<PathBuf> {
    home_with(&real_env, Os::current())
}

/// The directory where applications keep their configuration, following the platform's
/// convention.
#[must_use]
pub fn config_dir() -> Option<PathBuf> {
    config_with(&real_env, Os::current())
}

/// The directory where applications keep their data, following the platform's convention.
#[must_use]
pub fn data_dir() -> Option<PathBuf> {
    data_with(&real_env, Os::current())
}

/// The freedesktop configuration directory, used by tools that follow that layout on every
/// platform.
#[must_use]
pub fn xdg_config_dir() -> Option<PathBuf> {
    xdg_config_with(&real_env, Os::current())
}

/// Returns `true` when `path` is inside `root`, comparing the paths as they are written.
///
/// It is a lexical test, so it does not resolve symbolic links; callers that need that must
/// canonicalize first.
///
/// # Examples
/// ```text
/// assert!(is_inside(Path::new("/a/b/c"), Path::new("/a")));
/// assert!(!is_inside(Path::new("/a/bc"), Path::new("/a/b")));
/// ```
#[must_use]
pub fn is_inside(path: &Path, root: &Path) -> bool {
    path.starts_with(root)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::PathBuf;

    use super::{
        Os, config_with, data_with, home_with, is_absolute_for, is_inside, xdg_config_with,
    };

    /// Builds an environment lookup from a list of pairs.
    fn env_of(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<OsString> {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| OsString::from(*value))
        }
    }

    /// On Linux the freedesktop variables win, and the documented fallbacks apply without them.
    #[test]
    fn linux_follows_freedesktop() {
        let full = env_of(&[
            ("HOME", "/home/paul"),
            ("XDG_CONFIG_HOME", "/cfg"),
            ("XDG_DATA_HOME", "/data"),
        ]);
        assert_eq!(config_with(&full, Os::Unix), Some(PathBuf::from("/cfg")));
        assert_eq!(data_with(&full, Os::Unix), Some(PathBuf::from("/data")));

        let bare = env_of(&[("HOME", "/home/paul")]);
        assert_eq!(
            config_with(&bare, Os::Unix),
            Some(PathBuf::from("/home/paul/.config"))
        );
        assert_eq!(
            data_with(&bare, Os::Unix),
            Some(PathBuf::from("/home/paul/.local/share"))
        );
    }

    /// On macOS both directories are the standard application-support folder, and the
    /// freedesktop lookup still answers for the tools that use it there.
    #[test]
    fn macos_uses_application_support() {
        let env = env_of(&[("HOME", "/Users/paul")]);
        let support = PathBuf::from("/Users/paul/Library/Application Support");
        assert_eq!(config_with(&env, Os::MacOs), Some(support.clone()));
        assert_eq!(data_with(&env, Os::MacOs), Some(support));
        assert_eq!(
            xdg_config_with(&env, Os::MacOs),
            Some(PathBuf::from("/Users/paul/.config"))
        );

        let with_xdg = env_of(&[("HOME", "/Users/paul"), ("XDG_CONFIG_HOME", "/x")]);
        assert_eq!(
            xdg_config_with(&with_xdg, Os::MacOs),
            Some(PathBuf::from("/x"))
        );
    }

    /// On Windows the roaming application-data folder answers for both, from its own variable.
    #[test]
    fn windows_uses_appdata() {
        let env = env_of(&[
            ("USERPROFILE", r"C:\Users\Paul"),
            ("APPDATA", r"C:\Users\Paul\AppData\Roaming"),
        ]);
        let roaming = PathBuf::from(r"C:\Users\Paul\AppData\Roaming");
        assert_eq!(
            home_with(&env, Os::Windows),
            Some(PathBuf::from(r"C:\Users\Paul"))
        );
        assert_eq!(config_with(&env, Os::Windows), Some(roaming.clone()));
        assert_eq!(data_with(&env, Os::Windows), Some(roaming));
    }

    /// Absoluteness follows the platform being resolved, not the one running the test. Without
    /// this, every Windows case below would pass or fail for the wrong reason.
    #[test]
    fn absoluteness_follows_the_resolved_platform() {
        let windows = [r"C:\Users\Paul", "D:/data", r"\\server\share"];
        for path in windows {
            assert!(
                is_absolute_for(Os::Windows, &PathBuf::from(path)),
                "{path} is absolute on Windows"
            );
            assert!(
                !is_absolute_for(Os::Unix, &PathBuf::from(path)),
                "{path} is not absolute on Unix"
            );
        }
        for path in [r"Users\Paul", "C:", "C:relative", r"\single"] {
            assert!(
                !is_absolute_for(Os::Windows, &PathBuf::from(path)),
                "{path} is not absolute on Windows"
            );
        }
        assert!(is_absolute_for(Os::Unix, &PathBuf::from("/home/paul")));
        assert!(is_absolute_for(Os::MacOs, &PathBuf::from("/Users/paul")));
        assert!(!is_absolute_for(Os::Unix, &PathBuf::from("home/paul")));
    }

    /// An empty or relative variable is ignored, because it would put a user's data somewhere
    /// that depends on the working directory.
    #[test]
    fn empty_and_relative_values_are_ignored() {
        let empty = env_of(&[("HOME", "")]);
        assert_eq!(home_with(&empty, Os::Unix), None);

        let relative = env_of(&[("HOME", "/home/paul"), ("XDG_CONFIG_HOME", "relative/path")]);
        assert_eq!(
            config_with(&relative, Os::Unix),
            Some(PathBuf::from("/home/paul/.config"))
        );
    }

    /// With nothing in the environment every lookup answers `None` instead of guessing.
    #[test]
    fn an_empty_environment_yields_nothing() {
        let nothing = env_of(&[]);
        for os in [Os::Unix, Os::MacOs, Os::Windows] {
            assert_eq!(home_with(&nothing, os), None, "{os:?}");
            assert_eq!(config_with(&nothing, os), None, "{os:?}");
            assert_eq!(data_with(&nothing, os), None, "{os:?}");
        }
    }

    /// Containment compares whole path components, so a shared prefix is not containment.
    #[test]
    fn containment_compares_components() {
        assert!(is_inside(&PathBuf::from("/a/b/c"), &PathBuf::from("/a")));
        assert!(is_inside(&PathBuf::from("/a"), &PathBuf::from("/a")));
        assert!(!is_inside(&PathBuf::from("/a/bc"), &PathBuf::from("/a/b")));
        assert!(!is_inside(&PathBuf::from("/x"), &PathBuf::from("/a")));
    }

    /// The compiled-in platform is one of the three the module knows.
    #[test]
    fn the_current_platform_is_known() {
        assert!(matches!(Os::current(), Os::Unix | Os::MacOs | Os::Windows));
    }
}
