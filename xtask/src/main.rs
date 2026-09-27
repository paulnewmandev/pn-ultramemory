// SPDX-License-Identifier: Apache-2.0
//! `xtask`: the quality harness of pn-ultramemory, run from the repository with `cargo run -p xtask`.
//!
//! # Role in the project
//! The compiler, clippy and the test suite each hold one kind of promise. The guards here hold the
//! ones no compiler can: that an error message names a way forward, that library code does not
//! panic, that a docstring says more than the item's name, that the tests a release depends on still
//! exist, and that every file carries its licence header. Each guard keeps a baseline under
//! `.ratchet/` that may only shrink, so it is useful on a codebase that already has findings.
//!
//! # This is a development tool
//! `xtask` is never published and never shipped. Nothing a user installs contains it, so its
//! dependencies — `syn` and `proc-macro2` — do not affect the footprint of the product. They exist
//! only so the guards read Rust the way the compiler does rather than by matching text.
//!
//! # Exit codes
//! * `0` — the guard passed, or `--update` rewrote a baseline.
//! * `1` — the guard failed, or could not do its job.
//! * `2` — the command line was wrong.
//!
//! # What this harness does not prove
//! Each guard documents its own limits in its module and in `docs/quality.md`. Together they prove
//! nothing about correctness, performance or security: they only hold promises a reviewer would
//! otherwise have to check by hand, every time, for ever.

mod docs;
mod headers;
mod panics;
mod ratchet;
mod refusal;
mod source;
mod verify_tests;

use std::path::PathBuf;
use std::process::ExitCode;

/// Exit code reported when a guard fails.
const FAILED: u8 = 1;

/// Exit code reported when the command line is wrong.
const MISUSED: u8 = 2;

/// The usage text, printed on a wrong command line and by `--help`.
const USAGE: &str = "\
xtask: the quality harness of pn-ultramemory.

Usage:
  cargo run -p xtask -- <command> [options]

Commands:
  headers                     SPDX header and module documentation in every source file.
  refusal [--update]          Every error message must name a way forward.
  panics [--update]           Library code must not panic; also covers indexing and integer casts.
  docs [--update]             A public item's first documented sentence must add to its name.
  verify-tests <manifest>     Every test named in <manifest> must still exist.

Options:
  --update                    Rewrite this guard's baseline under .ratchet/ and exit 0.
  -h, --help                  Print this text.

Exit codes:
  0 passed, 1 guard failed, 2 wrong command line.

Each guard states what it does not prove, in its own output and in docs/quality.md.";

/// Parses the command line, runs the guard and turns its verdict into an exit code.
fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match dispatch(&arguments) {
        Outcome::Passed => ExitCode::SUCCESS,
        Outcome::Failed(message) => {
            if let Some(message) = message {
                eprintln!("\nxtask: {message}");
            }
            ExitCode::from(FAILED)
        }
        Outcome::Misused(message) => {
            eprintln!("xtask: {message}\n\n{USAGE}");
            ExitCode::from(MISUSED)
        }
        Outcome::Helped => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
    }
}

/// What one run of `xtask` concluded.
enum Outcome {
    /// The guard passed, or a baseline was rewritten.
    Passed,
    /// The guard failed. The message, when there is one, explains why it could not do its job.
    Failed(Option<String>),
    /// The command line was wrong.
    Misused(String),
    /// Help was asked for.
    Helped,
}

/// Chooses the guard named on the command line and runs it.
fn dispatch(arguments: &[String]) -> Outcome {
    let Some(command) = arguments.first() else {
        return Outcome::Misused("no command given".to_owned());
    };
    let rest = arguments.get(1..).unwrap_or_default();
    if command == "-h" || command == "--help" || command == "help" {
        return Outcome::Helped;
    }

    let root = match working_root() {
        Ok(root) => root,
        Err(message) => return Outcome::Failed(Some(message)),
    };

    match command.as_str() {
        "headers" => {
            if let Some(unexpected) = rest.first() {
                return Outcome::Misused(format!("`headers` takes no options, got `{unexpected}`"));
            }
            finish(headers::run(&root))
        }
        "refusal" | "panics" | "docs" => match update_flag(rest) {
            Ok(update) => finish(match command.as_str() {
                "refusal" => refusal::run(&root, update),
                "panics" => panics::run(&root, update),
                _ => docs::run(&root, update),
            }),
            Err(message) => Outcome::Misused(message),
        },
        "verify-tests" => match rest {
            [manifest] => finish(verify_tests::run(&root, &PathBuf::from(manifest))),
            [] => Outcome::Misused("`verify-tests` needs the path of a manifest".to_owned()),
            _ => Outcome::Misused("`verify-tests` takes exactly one manifest path".to_owned()),
        },
        other => Outcome::Misused(format!("unknown command `{other}`")),
    }
}

/// Reads the optional `--update` flag, rejecting anything else.
///
/// # Errors
/// Returns a message naming the unexpected argument.
fn update_flag(rest: &[String]) -> Result<bool, String> {
    match rest {
        [] => Ok(false),
        [flag] if flag == "--update" => Ok(true),
        [unexpected, ..] => Err(format!(
            "expected `--update` or nothing, got `{unexpected}`"
        )),
    }
}

/// Turns a guard's result into an outcome.
fn finish(result: Result<bool, String>) -> Outcome {
    match result {
        Ok(true) => Outcome::Passed,
        Ok(false) => Outcome::Failed(None),
        Err(message) => Outcome::Failed(Some(message)),
    }
}

/// Finds the repository root, starting from the current directory.
///
/// # Errors
/// Returns a message when the current directory cannot be read or has no workspace above it.
fn working_root() -> Result<PathBuf, String> {
    let current = std::env::current_dir()
        .map_err(|err| format!("cannot read the current directory: {err}"))?;
    source::find_root(&current)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Turns string literals into the owned arguments `dispatch` expects.
    fn arguments(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|part| (*part).to_owned()).collect()
    }

    /// No command, an unknown command and a stray option are all misuse, not failure.
    #[test]
    fn misuse_is_distinct_from_failure() {
        assert!(matches!(dispatch(&arguments(&[])), Outcome::Misused(_)));
        assert!(matches!(
            dispatch(&arguments(&["nonsense"])),
            Outcome::Misused(_)
        ));
        assert!(matches!(
            dispatch(&arguments(&["refusal", "--rewrite"])),
            Outcome::Misused(_)
        ));
        assert!(matches!(
            dispatch(&arguments(&["verify-tests"])),
            Outcome::Misused(_)
        ));
        assert!(matches!(
            dispatch(&arguments(&["verify-tests", "a", "b"])),
            Outcome::Misused(_)
        ));
    }

    /// Help is not misuse, so asking for it exits successfully.
    #[test]
    fn help_is_not_misuse() {
        assert!(matches!(dispatch(&arguments(&["--help"])), Outcome::Helped));
        assert!(matches!(dispatch(&arguments(&["help"])), Outcome::Helped));
    }

    /// The update flag is read only in its exact spelling.
    #[test]
    fn reads_the_update_flag() {
        assert_eq!(update_flag(&[]), Ok(false));
        assert_eq!(update_flag(&arguments(&["--update"])), Ok(true));
        assert!(update_flag(&arguments(&["-u"])).is_err());
    }

    /// The usage text names every command, so a wrong command line teaches the right one.
    #[test]
    fn usage_names_every_command() {
        for command in ["headers", "refusal", "panics", "docs", "verify-tests"] {
            assert!(USAGE.contains(command), "{command}");
        }
    }
}
