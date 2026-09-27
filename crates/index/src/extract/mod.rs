// SPDX-License-Identifier: Apache-2.0
//! Extraction of symbols, references and imports from source text.
//!
//! # Role in the architecture
//! This module is the exit adapter that implements the [`Extractor`] port of
//! `pn-ultramemory-core`. Twelve languages are parsed with tree-sitter grammars, one rules module
//! per language family, all driven by the shared iterative walker in [`engine`]. Every other
//! language goes through the lexical [`crate::fallback`].
//!
//! # Invariants
//! * Extraction is total: malformed input produces a best-effort result with a non-zero
//!   `parse_errors`, never a panic. Only an unsupported language or a parser that could not
//!   produce any tree (limit exceeded or timeout) is an error.
//! * A C file that only the C++ grammar reads better is reported as C++ (`FileExtract::language`
//!   says which grammar produced the result).
//! * Results are deterministic: no hash-map iteration order and no clock influences the output
//!   (the parse deadline only fires on pathological input).
//! * Traversal is iterative and every output size is bounded, see [`engine`].

mod c_family;
mod csharp;
mod ecmascript;
pub(crate) mod engine;
mod go;
mod java;
mod model;
mod nodes;
mod php;
mod python;
mod ruby;
mod rust;
pub(crate) mod text;

use std::ops::ControlFlow;
use std::time::{Duration, Instant};

use pn_ultramemory_core::{ExtractError, Extractor, FileExtract, Language};
use tree_sitter::{ParseOptions, Parser, Tree};

pub(crate) use engine::SymbolExtra;
use engine::{Rules, Walker};

/// Longest time one file may take to parse before the parser is stopped.
const PARSE_DEADLINE: Duration = Duration::from_secs(10);

/// Largest source, in bytes, that is handed to the parser (tree-sitter offsets are 32-bit).
const MAX_SOURCE_BYTES: usize = 1 << 30;

/// Extracts symbols, references and imports with tree-sitter grammars, and with a lexical
/// fallback for languages that have no grammar.
///
/// The value is stateless and cheap to copy: a parser is created for each call, so one
/// extractor can be shared between threads.
///
/// # Examples
/// ```
/// use pn_ultramemory_core::{Extractor, Language};
/// use pn_ultramemory_index::TreeSitterExtractor;
///
/// let extractor = TreeSitterExtractor::new();
/// let file = extractor
///     .extract(Language::Rust, "/// Adds.\npub fn add(a: i32, b: i32) -> i32 { a + b }\n")
///     .unwrap();
/// assert_eq!(file.symbols[0].name, "add");
/// assert_eq!(file.symbols[0].doc.as_deref(), Some("Adds."));
/// assert_eq!(file.parse_errors, 0);
/// ```
#[derive(Debug, Default, Clone, Copy)]
pub struct TreeSitterExtractor;

impl TreeSitterExtractor {
    /// Creates an extractor.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Extractor for TreeSitterExtractor {
    fn supports(&self, _language: Language) -> bool {
        true
    }

    fn extract(&self, language: Language, source: &str) -> Result<FileExtract, ExtractError> {
        match extract_detailed(language, source) {
            Ok((file, _extras, _tree)) => Ok(retry_as_cpp(language, source, file)),
            Err(ExtractError::Unsupported(Language::Other(_))) => {
                Ok(crate::fallback::extract_lexical(language, source))
            }
            Err(error) => Err(error),
        }
    }
}

/// Re-reads a C file that has syntax errors with the C++ grammar and keeps that result when it
/// has strictly fewer errors.
///
/// The extension `.h` is mapped to C, but many headers hold C++ (`class`, `namespace`,
/// templates), which the C grammar cannot read. The result then says
/// [`Language::Cpp`], the language the file was actually parsed as.
fn retry_as_cpp(language: Language, source: &str, file: FileExtract) -> FileExtract {
    if language != Language::C || file.parse_errors == 0 || !mentions_cpp_syntax(source) {
        return file;
    }
    match extract_detailed(Language::Cpp, source) {
        Ok((cpp, _, _)) if cpp.parse_errors < file.parse_errors => cpp,
        _ => file,
    }
}

/// Chooses the grammar for a file: C++ for a C file that has syntax errors, mentions C++ syntax
/// and has strictly fewer errors as C++. Any other file keeps its language.
///
/// This is the rule [`retry_as_cpp`] applies to extraction results, for callers that need the
/// tree itself (the documentation inserter).
pub(crate) fn choose_language(language: Language, source: &str) -> Language {
    if language != Language::C || !mentions_cpp_syntax(source) {
        return language;
    }
    let (Ok(as_c), Ok(as_cpp)) = (parse(Language::C, source), parse(Language::Cpp, source)) else {
        return language;
    };
    if count_errors(&as_cpp) < count_errors(&as_c) {
        Language::Cpp
    } else {
        language
    }
}

/// Returns `true` when a source contains a keyword or token that only C++ has, so that a second
/// parse is worth its time.
fn mentions_cpp_syntax(source: &str) -> bool {
    [
        "namespace ",
        "class ",
        "template<",
        "template <",
        "::",
        "public:",
        "private:",
        "typename ",
    ]
    .iter()
    .any(|needle| source.contains(needle))
}

/// Returns the tree-sitter grammar and the walker rules of a grammar-backed language.
fn grammar(language: Language) -> Option<(tree_sitter::Language, &'static dyn Rules)> {
    let pair: (tree_sitter::Language, &'static dyn Rules) = match language {
        Language::Rust => (tree_sitter_rust::LANGUAGE.into(), &rust::RUST),
        Language::Python => (tree_sitter_python::LANGUAGE.into(), &python::PYTHON),
        Language::JavaScript => (
            tree_sitter_javascript::LANGUAGE.into(),
            &ecmascript::JAVASCRIPT,
        ),
        Language::TypeScript => (
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            &ecmascript::TYPESCRIPT,
        ),
        Language::Tsx => (
            tree_sitter_typescript::LANGUAGE_TSX.into(),
            &ecmascript::TYPESCRIPT,
        ),
        Language::Go => (tree_sitter_go::LANGUAGE.into(), &go::GO),
        Language::Java => (tree_sitter_java::LANGUAGE.into(), &java::JAVA),
        Language::CSharp => (tree_sitter_c_sharp::LANGUAGE.into(), &csharp::CSHARP),
        Language::C => (tree_sitter_c::LANGUAGE.into(), &c_family::C),
        Language::Cpp => (tree_sitter_cpp::LANGUAGE.into(), &c_family::CPP),
        Language::Ruby => (tree_sitter_ruby::LANGUAGE.into(), &ruby::RUBY),
        Language::Php => (tree_sitter_php::LANGUAGE_PHP.into(), &php::PHP),
        Language::Other(_) => return None,
    };
    Some(pair)
}

/// Parses `source` with the grammar of `language`.
///
/// # Errors
/// [`ExtractError::Unsupported`] for a language without a grammar and [`ExtractError::Parse`]
/// when the source is too large, the grammar cannot be loaded or the deadline passes.
pub(crate) fn parse(language: Language, source: &str) -> Result<Tree, ExtractError> {
    let (grammar, _) = grammar(language).ok_or(ExtractError::Unsupported(language))?;
    if source.len() > MAX_SOURCE_BYTES {
        return Err(ExtractError::Parse("the source is too large".to_owned()));
    }
    let mut parser = Parser::new();
    parser
        .set_language(&grammar)
        .map_err(|e| ExtractError::Parse(e.to_string()))?;
    let deadline = Instant::now() + PARSE_DEADLINE;
    let mut progress = |_: &tree_sitter::ParseState| {
        if Instant::now() > deadline {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    };
    let bytes = source.as_bytes();
    let options = ParseOptions::new().progress_callback(&mut progress);
    parser
        .parse_with_options(
            &mut |offset, _| bytes.get(offset..).unwrap_or(&[]),
            None,
            Some(options),
        )
        .ok_or_else(|| ExtractError::Parse("the parser gave up".to_owned()))
}

/// Counts the lines of a source text: line breaks, plus one for an unterminated last line.
pub(crate) fn count_lines(source: &str) -> u32 {
    if source.is_empty() {
        return 0;
    }
    let breaks = source.bytes().filter(|b| *b == b'\n').count();
    let unterminated = usize::from(!source.ends_with('\n'));
    text::to_u32(breaks + unterminated)
}

/// Extracts a grammar-backed language and also returns the internal facts about each symbol
/// and the syntax tree, which the documentation inserter reuses.
///
/// # Errors
/// See [`parse`].
pub(crate) fn extract_detailed(
    language: Language,
    source: &str,
) -> Result<(FileExtract, Vec<SymbolExtra>, Tree), ExtractError> {
    let (_, rules) = grammar(language).ok_or(ExtractError::Unsupported(language))?;
    let tree = parse(language, source)?;
    let outcome = {
        let root = tree.root_node();
        let mut walker = Walker::new(source, rules, root);
        walker.run(root);
        walker.finish()
    };
    let file = FileExtract {
        language,
        symbols: outcome.symbols,
        references: outcome.references,
        imports: outcome.imports,
        line_count: count_lines(source),
        parse_errors: outcome.errors,
    };
    Ok((file, outcome.extras, tree))
}

/// Counts the syntax errors (`ERROR` and `MISSING` nodes) of a parsed tree.
pub(crate) fn count_errors(tree: &Tree) -> u32 {
    let root = tree.root_node();
    if !root.has_error() {
        return 0;
    }
    let mut count = 0u32;
    let mut cursor = root.walk();
    loop {
        let node = cursor.node();
        if node.is_error() || node.is_missing() {
            count = count.saturating_add(1);
        }
        if node.has_error() && cursor.goto_first_child() {
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return count;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use pn_ultramemory_core::{ExtractError, Language};

    use super::{count_errors, count_lines, extract_detailed, parse};

    /// Lines are counted by their terminators, plus one for an unterminated last line.
    #[test]
    fn line_counting() {
        assert_eq!(count_lines(""), 0);
        assert_eq!(count_lines("a"), 1);
        assert_eq!(count_lines("a\n"), 1);
        assert_eq!(count_lines("a\n\n"), 2);
        assert_eq!(count_lines("a\r\nb"), 2);
        assert_eq!(count_lines("\n"), 1);
    }

    /// Valid source has no errors; broken source has some, and both counters agree.
    #[test]
    fn error_counting_matches_the_walk() {
        let valid = parse(Language::Rust, "fn f() {}").unwrap();
        assert_eq!(count_errors(&valid), 0);
        let source = "fn f( {\nstruct }\nlet = ;";
        let broken = parse(Language::Rust, source).unwrap();
        let counted = count_errors(&broken);
        assert!(counted > 0);
        let (file, _, _) = extract_detailed(Language::Rust, source).unwrap();
        assert_eq!(file.parse_errors, counted);
    }

    /// A language without a grammar is reported as unsupported by the tree-sitter path.
    #[test]
    fn grammarless_languages_are_unsupported_here() {
        let error = parse(Language::Other("lua"), "x").unwrap_err();
        assert_eq!(error, ExtractError::Unsupported(Language::Other("lua")));
        assert!(extract_detailed(Language::Other("lua"), "x").is_err());
    }

    /// The extra facts line up with the symbols, one entry each.
    #[test]
    fn extras_are_parallel_to_symbols() {
        let source = "/// doc\n#[inline]\nfn f() {}\nfn g() {}\n";
        let (file, extras, _) = extract_detailed(Language::Rust, source).unwrap();
        assert_eq!(file.symbols.len(), extras.len());
        assert!(extras[0].has_doc && !extras[1].has_doc);
        assert_eq!((extras[0].decl_line, extras[0].attach_line), (3, 2));
        assert_eq!(
            &source[extras[0].core_start..extras[0].core_end],
            "fn f() {}"
        );
        assert_eq!(extras[1].name_line, 4);
    }
}
