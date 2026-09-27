// SPDX-License-Identifier: Apache-2.0
//! Discovery and parsing of the Rust sources the guards analyse.
//!
//! # Role in the harness
//! Every guard needs the same three things: a deterministic list of files, the original source text
//! (so a finding can quote what a contributor actually wrote), and a way to tell library code from
//! test code. This module provides all three and nothing else.
//!
//! # Invariants
//! * Directory walks are sorted by byte order, so two runs on the same tree agree exactly.
//! * Paths are reported with forward slashes on every platform, so a baseline written on Linux
//!   compares equal on Windows.
//! * A file whose parent declares it as `#[cfg(test)] mod name;` is test code, even though the file
//!   itself carries no attribute. Guards that skip test code must consult [`TestOnly`].

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use proc_macro2::Span;

/// A parsed Rust source file, together with the text the parser saw.
pub(crate) struct SourceFile {
    /// Path relative to the repository root, with forward slashes (`crates/toon/src/error.rs`).
    pub(crate) rel: String,
    /// Directory name of the owning crate under `crates/` (`toon`).
    pub(crate) crate_name: String,
    /// Path relative to the crate directory, with forward slashes (`src/error.rs`).
    pub(crate) in_crate: String,
    /// The original text of the file.
    pub(crate) text: String,
    /// The parsed syntax tree.
    pub(crate) ast: syn::File,
    /// Byte offset at which each 1-based line begins; index 0 is unused.
    line_starts: Vec<usize>,
}

impl SourceFile {
    /// Reads and parses one file, recording the paths the baselines are keyed by.
    ///
    /// # Errors
    /// Returns a message naming the file when it cannot be read or does not parse as Rust.
    fn load(root: &Path, path: &Path) -> Result<Self, String> {
        let rel = relative(root, path);
        let text = std::fs::read_to_string(path)
            .map_err(|err| format!("{rel}: cannot read the file: {err}"))?;
        Self::from_text(rel, text)
    }

    /// Parses `text` as the file at the repository-relative path `rel`.
    ///
    /// # Errors
    /// Returns a message naming the file when the text does not parse as Rust.
    pub(crate) fn from_text(rel: String, text: String) -> Result<Self, String> {
        let ast = syn::parse_file(&text).map_err(|err| {
            format!("{rel}: does not parse as Rust: {err}; run `cargo check -p <crate>` first")
        })?;
        let (crate_name, in_crate) = split_crate(&rel);
        let mut line_starts = vec![0_usize, 0];
        for (offset, byte) in text.bytes().enumerate() {
            if byte == b'\n' {
                line_starts.push(offset + 1);
            }
        }
        Ok(Self {
            rel,
            crate_name,
            in_crate,
            text,
            ast,
            line_starts,
        })
    }

    /// Returns the source text covered by `span`, with runs of whitespace collapsed to one space.
    ///
    /// Spans are reported by `proc-macro2` as 1-based lines and 0-based character offsets, so the
    /// offsets are walked as characters rather than bytes. A span that cannot be resolved yields an
    /// empty string, which a caller must treat as "no text available" rather than as a match.
    pub(crate) fn span_text(&self, span: Span) -> String {
        let start = self.byte_offset(span.start().line, span.start().column);
        let end = self.byte_offset(span.end().line, span.end().column);
        match (start, end) {
            (Some(from), Some(to)) if to > from && to <= self.text.len() => self
                .text
                .get(from..to)
                .map_or_else(String::new, normalise_whitespace),
            _ => String::new(),
        }
    }

    /// Returns the 1-based line on which `span` begins, or `0` when it falls outside this file.
    ///
    /// A span that does not resolve gives `0`, which matches no comment marker, so an unresolvable
    /// site can never be excused by accident.
    pub(crate) fn span_line(&self, span: Span) -> usize {
        let line = span.start().line;
        if line < self.line_starts.len() {
            line
        } else {
            0
        }
    }

    /// Converts a 1-based line and a 0-based character column into a byte offset.
    fn byte_offset(&self, line: usize, column: usize) -> Option<usize> {
        let start = *self.line_starts.get(line)?;
        let rest = self.text.get(start..)?;
        let mut chars = rest.char_indices();
        for _ in 0..column {
            chars.next()?;
        }
        Some(
            chars
                .next()
                .map_or(self.text.len(), |(offset, _)| start + offset),
        )
    }
}

/// The set of files that are only compiled for tests, so guards can skip them.
pub(crate) struct TestOnly {
    /// Repository-relative paths of files reachable only through a test-gated module.
    paths: BTreeSet<String>,
}

impl TestOnly {
    /// Returns true when `rel` is only compiled under `cfg(test)`.
    pub(crate) fn contains(&self, rel: &str) -> bool {
        self.paths.contains(rel)
    }

    /// Returns how many files were classified as test-only, for the guard summaries.
    pub(crate) fn len(&self) -> usize {
        self.paths.len()
    }
}

/// Walks `crates/*/src` and parses every Rust file found there, in byte order of the path.
///
/// # Errors
/// Returns a message when `crates/` is missing, when a file cannot be read, or when a file does not
/// parse. A guard that enumerated nothing must fail rather than pass, so an empty result is an
/// error here as well.
pub(crate) fn collect_crate_sources(root: &Path) -> Result<Vec<SourceFile>, String> {
    let crates_dir = root.join("crates");
    if !crates_dir.is_dir() {
        return Err(format!(
            "no `crates` directory under {}; run the guard from inside the repository",
            root.display()
        ));
    }
    let mut files = Vec::new();
    for crate_dir in sorted_children(&crates_dir)? {
        if !crate_dir.is_dir() {
            continue;
        }
        let src = crate_dir.join("src");
        if !src.is_dir() {
            continue;
        }
        for path in collect_rust_files(&src)? {
            files.push(SourceFile::load(root, &path)?);
        }
    }
    if files.is_empty() {
        return Err(format!(
            "found no Rust files under {}; a guard that enumerates nothing cannot pass",
            crates_dir.display()
        ));
    }
    Ok(files)
}

/// Collects every `.rs` file below `dir`, sorted by path in byte order.
///
/// # Errors
/// Returns a message when a directory cannot be listed.
pub(crate) fn collect_rust_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        for child in sorted_children(&current)? {
            if child.is_dir() {
                stack.push(child);
            } else if child.extension().is_some_and(|ext| ext == "rs") {
                out.push(child);
            }
        }
    }
    out.sort();
    Ok(out)
}

/// Lists the children of `dir`, sorted by path in byte order so walks are reproducible.
///
/// # Errors
/// Returns a message naming the directory when it cannot be read.
fn sorted_children(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let entries =
        std::fs::read_dir(dir).map_err(|err| format!("cannot list {}: {err}", dir.display()))?;
    let mut children = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|err| format!("cannot list {}: {err}", dir.display()))?;
        children.push(entry.path());
    }
    children.sort();
    Ok(children)
}

/// Works out which files are reachable only from a `#[cfg(test)]` module declaration.
///
/// The rule is applied until it stops changing, so a test-only module that declares further
/// submodules marks those as test-only too.
pub(crate) fn classify_test_only(files: &[SourceFile]) -> TestOnly {
    let known: BTreeSet<&str> = files.iter().map(|file| file.rel.as_str()).collect();
    let mut paths: BTreeSet<String> = BTreeSet::new();
    loop {
        let before = paths.len();
        for file in files {
            let inherited = paths.contains(&file.rel);
            for item in &file.ast.items {
                if let syn::Item::Mod(module) = item {
                    if module.content.is_some() {
                        continue;
                    }
                    if !inherited && !is_test_gated(&module.attrs) {
                        continue;
                    }
                    for candidate in child_module_files(&file.rel, &module.ident.to_string()) {
                        if known.contains(candidate.as_str()) {
                            paths.insert(candidate);
                        }
                    }
                }
            }
        }
        if paths.len() == before {
            break;
        }
    }
    TestOnly { paths }
}

/// Returns the two file paths a `mod name;` declaration inside `owner` can resolve to.
fn child_module_files(owner: &str, name: &str) -> Vec<String> {
    let (dir, stem) = match owner.rsplit_once('/') {
        Some((dir, file)) => (dir.to_owned(), file.trim_end_matches(".rs").to_owned()),
        None => (String::new(), owner.trim_end_matches(".rs").to_owned()),
    };
    let base = if matches!(stem.as_str(), "lib" | "main" | "mod") {
        dir
    } else if dir.is_empty() {
        stem
    } else {
        format!("{dir}/{stem}")
    };
    if base.is_empty() {
        vec![format!("{name}.rs"), format!("{name}/mod.rs")]
    } else {
        vec![format!("{base}/{name}.rs"), format!("{base}/{name}/mod.rs")]
    }
}

/// Returns true when these attributes mark an item as test-only.
///
/// Recognises `#[test]`, `#[cfg(test)]` and any `cfg` predicate that mentions the bare word `test`,
/// such as `cfg(any(test, feature = "x"))`. A predicate that begins with `not` is left alone, because
/// `cfg(not(test))` marks code that exists *outside* tests.
pub(crate) fn is_test_gated(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        let path = attr.path();
        if path.is_ident("test") {
            return true;
        }
        if !path.is_ident("cfg") {
            return false;
        }
        match &attr.meta {
            syn::Meta::List(list) => {
                let mut tokens = list.tokens.clone().into_iter().peekable();
                if tokens
                    .peek()
                    .is_some_and(|first| first.to_string() == "not")
                {
                    return false;
                }
                mentions_test(list.tokens.clone())
            }
            _ => false,
        }
    })
}

/// Returns true when `tokens` holds the bare identifier `test`, however deeply grouped.
///
/// The identifier must stand alone: the string literal in `feature = "test"` does not count.
fn mentions_test(tokens: proc_macro2::TokenStream) -> bool {
    let mut stack = vec![tokens];
    while let Some(stream) = stack.pop() {
        for token in stream {
            match token {
                proc_macro2::TokenTree::Group(group) => stack.push(group.stream()),
                proc_macro2::TokenTree::Ident(ident) if ident == "test" => return true,
                _ => {}
            }
        }
    }
    false
}

/// Collapses every run of whitespace in `text` into a single space and trims the ends.
pub(crate) fn normalise_whitespace(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut pending_space = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            pending_space = !out.is_empty();
        } else {
            if pending_space {
                out.push(' ');
                pending_space = false;
            }
            out.push(ch);
        }
    }
    out
}

/// Shortens `text` to at most `limit` characters, marking a cut with a trailing ellipsis.
pub(crate) fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let kept: String = text.chars().take(limit).collect();
    format!("{kept} ...")
}

/// Returns the last path segment of a type, or a placeholder for a type that has no name.
///
/// Used to name the scope a finding sits in, so a key reads `DecodeError::new` rather than `new`.
pub(crate) fn type_name(ty: &syn::Type) -> String {
    match ty {
        syn::Type::Path(path) => path
            .path
            .segments
            .last()
            .map_or_else(|| "<impl>".to_owned(), |segment| segment.ident.to_string()),
        syn::Type::Reference(reference) => type_name(&reference.elem),
        _ => "<impl>".to_owned(),
    }
}

/// Renders `path` relative to `root` with forward slashes, falling back to the full path.
pub(crate) fn relative(root: &Path, path: &Path) -> String {
    let stripped = path.strip_prefix(root).unwrap_or(path);
    stripped
        .components()
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// Splits `crates/<name>/<rest>` into the crate name and the path inside the crate.
fn split_crate(rel: &str) -> (String, String) {
    let tail = rel.strip_prefix("crates/").unwrap_or(rel);
    match tail.split_once('/') {
        Some((name, rest)) => (name.to_owned(), rest.to_owned()),
        None => (String::new(), tail.to_owned()),
    }
}

/// Finds the repository root by walking up from `start` looking for the workspace manifest.
///
/// # Errors
/// Returns a message when no ancestor holds a `Cargo.toml` with a `[workspace]` table next to a
/// `crates` directory.
pub(crate) fn find_root(start: &Path) -> Result<PathBuf, String> {
    let mut current = Some(start);
    while let Some(dir) = current {
        let manifest = dir.join("Cargo.toml");
        if dir.join("crates").is_dir() {
            if let Ok(text) = std::fs::read_to_string(&manifest) {
                if text.contains("[workspace]") {
                    return Ok(dir.to_path_buf());
                }
            }
        }
        current = dir.parent();
    }
    Err(format!(
        "cannot find the workspace root above {}; run the guard inside the repository",
        start.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whitespace normalisation folds newlines and tabs into single spaces and trims the ends.
    #[test]
    fn normalises_whitespace() {
        assert_eq!(normalise_whitespace("  a\n\tb   c  "), "a b c");
        assert_eq!(normalise_whitespace(""), "");
        assert_eq!(
            normalise_whitespace("caf\u{e9}\n\u{2014}"),
            "caf\u{e9} \u{2014}"
        );
    }

    /// Truncation counts characters, not bytes, so a multi-byte message is not cut mid-character.
    #[test]
    fn truncates_by_characters() {
        assert_eq!(truncate("abcdef", 10), "abcdef");
        assert_eq!(truncate("\u{e9}\u{e9}\u{e9}\u{e9}", 2), "\u{e9}\u{e9} ...");
    }

    /// A `mod name;` in `lib.rs` resolves beside it; in `guard.rs` it resolves inside `guard/`.
    #[test]
    fn resolves_child_module_paths() {
        assert_eq!(
            child_module_files("crates/engine/src/lib.rs", "guard"),
            vec![
                "crates/engine/src/guard.rs",
                "crates/engine/src/guard/mod.rs"
            ]
        );
        assert_eq!(
            child_module_files("crates/engine/src/guard.rs", "tests"),
            vec![
                "crates/engine/src/guard/tests.rs",
                "crates/engine/src/guard/tests/mod.rs"
            ]
        );
    }

    /// The crate name and the in-crate path are split off the repository-relative path.
    #[test]
    fn splits_crate_and_path() {
        let (name, rest) = split_crate("crates/toon/src/error.rs");
        assert_eq!(name, "toon");
        assert_eq!(rest, "src/error.rs");
    }

    /// `cfg(test)`, `cfg(any(test, ...))` and `#[test]` all count as test gating; `cfg(unix)` does
    /// not.
    #[test]
    fn detects_test_gating() {
        let gated: syn::ItemMod = syn::parse_str("#[cfg(test)] mod tests;").expect("parses");
        assert!(is_test_gated(&gated.attrs));
        let combined: syn::ItemMod =
            syn::parse_str("#[cfg(any(test, feature = \"x\"))] mod tests;").expect("parses");
        assert!(is_test_gated(&combined.attrs));
        let plain: syn::ItemMod = syn::parse_str("#[cfg(unix)] mod platform;").expect("parses");
        assert!(!is_test_gated(&plain.attrs));
        let inverted: syn::ItemMod = syn::parse_str("#[cfg(not(test))] mod real;").expect("parses");
        assert!(
            !is_test_gated(&inverted.attrs),
            "cfg(not(test)) is not test code"
        );
        let named: syn::ItemMod =
            syn::parse_str("#[cfg(feature = \"test\")] mod helper;").expect("parses");
        assert!(
            !is_test_gated(&named.attrs),
            "a feature called test is not cfg(test)"
        );
    }

    /// A span resolves to the exact source text even when the line holds multi-byte characters,
    /// because `proc-macro2` reports character columns while Rust strings are indexed by byte.
    #[test]
    fn span_text_handles_multibyte_lines() {
        let text = "//! doc\nfn f() {\n    let x = \"caf\u{e9} \u{2014} bad\";\n}\n";
        let file = SourceFile::from_text("probe.rs".to_owned(), text.to_owned()).expect("parses");
        let mut found = Vec::new();
        for item in &file.ast.items {
            if let syn::Item::Fn(function) = item {
                for stmt in &function.block.stmts {
                    found.push(file.span_text(syn::spanned::Spanned::span(stmt)));
                }
            }
        }
        assert_eq!(found, vec!["let x = \"caf\u{e9} \u{2014} bad\";"]);
    }

    /// A file reachable only through `#[cfg(test)] mod tests;` is classified as test code, and the
    /// classification is transitive through the modules that file declares.
    #[test]
    fn classifies_test_only_files_transitively() {
        let files = vec![
            SourceFile::from_text(
                "crates/a/src/lib.rs".to_owned(),
                "//! doc\nmod tools;\n".to_owned(),
            )
            .expect("parses"),
            SourceFile::from_text(
                "crates/a/src/tools.rs".to_owned(),
                "//! doc\n#[cfg(test)]\nmod tests;\n".to_owned(),
            )
            .expect("parses"),
            SourceFile::from_text(
                "crates/a/src/tools/tests.rs".to_owned(),
                "//! doc\nmod optional;\n".to_owned(),
            )
            .expect("parses"),
            SourceFile::from_text(
                "crates/a/src/tools/tests/optional.rs".to_owned(),
                "//! doc\n".to_owned(),
            )
            .expect("parses"),
        ];
        let test_only = classify_test_only(&files);
        assert!(!test_only.contains("crates/a/src/tools.rs"));
        assert!(test_only.contains("crates/a/src/tools/tests.rs"));
        assert!(test_only.contains("crates/a/src/tools/tests/optional.rs"));
        assert_eq!(test_only.len(), 2);
    }
}
