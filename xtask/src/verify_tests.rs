// SPDX-License-Identifier: Apache-2.0
//! Proof that the tests named as release blockers still exist.
//!
//! # Why this guard exists
//! `cargo test <name>` filters by substring and **exits 0 when the filter matches nothing**. A
//! curated list of release-blocking tests is therefore worthless on its own: rename a test, or delete
//! it, and every command that named it keeps reporting success while proving nothing. This guard asks
//! cargo to list the tests it actually has and fails when a named test is not among them.
//!
//! # What this guard does not prove
//! * It does not run the tests, so it says nothing about whether they **pass**.
//! * It does not prove the list is **complete**. Nothing can: a release blocker is a judgement, and
//!   this guard only holds the judgement to account once it has been written down.
//! * Matching is by substring, exactly as `cargo test` matches, so a name that is a prefix of an
//!   unrelated test is satisfied by that unrelated test.
//! * A test behind a feature flag that is off, or a target that is not built on this platform, is
//!   not listed and so counts as missing.

use std::path::Path;
use std::process::Command;

/// Runs the guard, returning whether it passed.
///
/// # Errors
/// Returns a message when the manifest cannot be read, when it names nothing, or when cargo cannot
/// list the tests.
pub(crate) fn run(root: &Path, manifest: &Path) -> Result<bool, String> {
    let text = std::fs::read_to_string(manifest)
        .map_err(|err| format!("cannot read {}: {err}", manifest.display()))?;
    let required = required_names(&text);
    if required.is_empty() {
        return Err(format!(
            "{} names no test; a manifest that enumerates nothing cannot prove anything. \
             Add one test name per line, or delete the manifest and the guard together.",
            manifest.display()
        ));
    }

    let listed = list_tests(root)?;
    println!("release-blocker manifest");
    println!("  manifest:          {}", manifest.display());
    println!("  names required:    {}", required.len());
    println!("  tests listed:      {}", listed.len());

    let missing: Vec<&String> = required
        .iter()
        .filter(|name| !listed.iter().any(|listed| listed.contains(name.as_str())))
        .collect();
    if missing.is_empty() {
        println!("  every named test exists");
        return Ok(true);
    }
    println!("\nNamed tests that do not exist ({}):", missing.len());
    for name in &missing {
        println!("  ! {name}");
    }
    println!(
        "  `cargo test <name>` would exit 0 for each of these while running nothing. Restore the \
         test, or edit {} to name the test that replaced it.",
        manifest.display()
    );
    Ok(false)
}

/// Reads the test names out of a manifest, dropping comments and blank lines.
fn required_names(text: &str) -> Vec<String> {
    text.lines()
        .map(|line| line.split('#').next().unwrap_or("").trim())
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Asks cargo for every test in the workspace.
///
/// # Errors
/// Returns a message when cargo cannot be run, when it fails, or when it lists no test at all. A
/// guard that enumerates nothing must fail rather than pass.
fn list_tests(root: &Path) -> Result<Vec<String>, String> {
    let output = Command::new("cargo")
        .current_dir(root)
        .args(["test", "--workspace", "--locked", "--", "--list"])
        .output()
        .map_err(|err| {
            format!("cannot run cargo: {err}; install the toolchain pinned in rust-toolchain.toml")
        })?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let tail: Vec<&str> = stderr.lines().rev().take(20).collect();
        return Err(format!(
            "`cargo test --workspace --locked -- --list` failed, so no test name can be verified. \
             Fix the build first, then run `cargo run -p xtask -- verify-tests <manifest>` again.\n\
             Last lines of cargo's output:\n{}",
            tail.into_iter().rev().collect::<Vec<_>>().join("\n")
        ));
    }
    let names = parse_listing(&stdout);
    if names.is_empty() {
        return Err(
            "cargo listed no test at all. A guard that enumerates nothing cannot pass; check that \
             the workspace really has test targets."
                .to_owned(),
        );
    }
    Ok(names)
}

/// Pulls the test names out of the `--list` output of every test binary cargo ran.
///
/// Each harness prints one `name: test` line per test, plus headings and a trailing count that are
/// not names.
fn parse_listing(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim_end();
            for suffix in [": test", ": bench"] {
                if let Some(name) = trimmed.strip_suffix(suffix) {
                    if !name.is_empty() {
                        return Some(name.to_owned());
                    }
                }
            }
            None
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Comments and blank lines are dropped, and an inline comment ends the name.
    #[test]
    fn reads_the_manifest() {
        let text = "# heading\n\ntoon::roundtrip\n  packer::exact  # the oracle\n#\n";
        assert_eq!(
            required_names(text),
            vec!["toon::roundtrip", "packer::exact"]
        );
    }

    /// A manifest with only comments names nothing, which the caller must treat as a failure.
    #[test]
    fn an_empty_manifest_names_nothing() {
        assert!(required_names("# nothing here\n\n").is_empty());
    }

    /// Only the `name: test` lines of the listing are names.
    #[test]
    fn parses_a_cargo_listing() {
        let stdout = "\n   Running unittests src/lib.rs (target/debug/deps/toon-1)\n\
                      tests::roundtrip: test\n\
                      tests::hostile_input: test\n\
                      2 tests, 0 benchmarks\n\
                      src/lib.rs - encode (line 12): test\n";
        assert_eq!(
            parse_listing(stdout),
            vec![
                "tests::roundtrip",
                "tests::hostile_input",
                "src/lib.rs - encode (line 12)"
            ]
        );
    }

    /// An empty listing yields no names, which the caller must treat as a failure.
    #[test]
    fn an_empty_listing_yields_nothing() {
        assert!(parse_listing("0 tests, 0 benchmarks\n").is_empty());
    }
}
