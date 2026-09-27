// SPDX-License-Identifier: Apache-2.0
//! The programming languages pn-ultramemory recognizes.
//!
//! Twelve languages have a full grammar behind them. Every other language listed in
//! [`Language::Other`] is handled by a lexical fallback that finds declarations from their
//! shape, so an unknown language still gets a card for each of its symbols, just with lower
//! confidence.

use core::fmt;
use std::path::Path;

/// A source language, either backed by a grammar or handled by the lexical fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    /// Rust.
    Rust,
    /// Python.
    Python,
    /// JavaScript, including JSX, ES modules and `CommonJS`.
    JavaScript,
    /// TypeScript without JSX.
    TypeScript,
    /// TypeScript with JSX.
    Tsx,
    /// Go.
    Go,
    /// Java.
    Java,
    /// C.
    C,
    /// C++.
    Cpp,
    /// C#.
    CSharp,
    /// Ruby.
    Ruby,
    /// PHP.
    Php,
    /// A language handled by the lexical fallback, identified by its lowercase name.
    Other(&'static str),
}

/// Languages that have no grammar yet, with the file extensions that identify them.
const OTHER_LANGUAGES: &[(&str, &[&str])] = &[
    ("kotlin", &["kt", "kts"]),
    ("swift", &["swift"]),
    ("scala", &["scala", "sc"]),
    ("dart", &["dart"]),
    ("lua", &["lua"]),
    ("shell", &["sh", "bash", "zsh"]),
    ("perl", &["pl", "pm"]),
    ("elixir", &["ex", "exs"]),
    ("haskell", &["hs"]),
    ("clojure", &["clj", "cljs"]),
    ("sql", &["sql"]),
    ("r", &["r"]),
    ("julia", &["jl"]),
    ("zig", &["zig"]),
];

impl Language {
    /// The languages that have a full grammar.
    pub const GRAMMAR_BACKED: [Self; 12] = [
        Self::Rust,
        Self::Python,
        Self::JavaScript,
        Self::TypeScript,
        Self::Tsx,
        Self::Go,
        Self::Java,
        Self::C,
        Self::Cpp,
        Self::CSharp,
        Self::Ruby,
        Self::Php,
    ];

    /// Guesses the language from a file extension, without the dot, ignoring case.
    ///
    /// # Examples
    /// ```
    /// use pn_ultramemory_core::Language;
    ///
    /// assert_eq!(Language::from_extension("RS"), Some(Language::Rust));
    /// assert_eq!(Language::from_extension("kt").map(Language::name), Some("kotlin"));
    /// assert_eq!(Language::from_extension("png"), None);
    /// ```
    #[must_use]
    pub fn from_extension(extension: &str) -> Option<Self> {
        let extension = extension.to_ascii_lowercase();
        let known = match extension.as_str() {
            "rs" => Self::Rust,
            "py" | "pyi" => Self::Python,
            "js" | "mjs" | "cjs" | "jsx" => Self::JavaScript,
            "ts" | "mts" | "cts" => Self::TypeScript,
            "tsx" => Self::Tsx,
            "go" => Self::Go,
            "java" => Self::Java,
            "c" | "h" => Self::C,
            "cc" | "cpp" | "cxx" | "hh" | "hpp" | "hxx" => Self::Cpp,
            "cs" => Self::CSharp,
            "rb" => Self::Ruby,
            "php" => Self::Php,
            other => {
                return OTHER_LANGUAGES
                    .iter()
                    .find(|(_, extensions)| extensions.contains(&other))
                    .map(|(name, _)| Self::Other(name));
            }
        };
        Some(known)
    }

    /// Guesses the language from a path, using its extension.
    #[must_use]
    pub fn from_path(path: &Path) -> Option<Self> {
        path.extension()
            .and_then(|e| e.to_str())
            .and_then(Self::from_extension)
    }

    /// Looks a language up by the name returned from [`Language::name`].
    ///
    /// # Examples
    /// ```
    /// use pn_ultramemory_core::Language;
    ///
    /// for language in Language::GRAMMAR_BACKED {
    ///     assert_eq!(Language::from_name(language.name()), Some(language));
    /// }
    /// assert_eq!(Language::from_name("swift"), Some(Language::Other("swift")));
    /// assert_eq!(Language::from_name("klingon"), None);
    /// ```
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::GRAMMAR_BACKED
            .into_iter()
            .find(|l| l.name() == name)
            .or_else(|| {
                OTHER_LANGUAGES
                    .iter()
                    .find(|(other, _)| *other == name)
                    .map(|(other, _)| Self::Other(other))
            })
    }

    /// Stable lowercase name used in storage and in output.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Python => "python",
            Self::JavaScript => "javascript",
            Self::TypeScript => "typescript",
            Self::Tsx => "tsx",
            Self::Go => "go",
            Self::Java => "java",
            Self::C => "c",
            Self::Cpp => "cpp",
            Self::CSharp => "csharp",
            Self::Ruby => "ruby",
            Self::Php => "php",
            Self::Other(name) => name,
        }
    }

    /// The family this language belongs to for the purpose of linking references to definitions.
    ///
    /// A reference can only refer to a definition written in a language of the same family:
    /// JavaScript, TypeScript and TSX share one family (they import each other freely), and C and
    /// C++ share another. Every other language is its own family. Linking across families would
    /// match, say, TypeScript's `Record` to an unrelated C# class with the same name.
    ///
    /// # Examples
    /// ```
    /// use pn_ultramemory_core::Language;
    ///
    /// assert_eq!(Language::TypeScript.family(), Language::JavaScript.family());
    /// assert_eq!(Language::C.family(), Language::Cpp.family());
    /// assert_ne!(Language::Rust.family(), Language::Python.family());
    /// ```
    #[must_use]
    pub const fn family(self) -> &'static str {
        match self {
            Self::JavaScript | Self::TypeScript | Self::Tsx => "ecmascript",
            Self::C | Self::Cpp => "c-family",
            other => other.name(),
        }
    }

    /// Returns `true` when a full grammar backs this language.
    #[must_use]
    pub const fn is_grammar_backed(self) -> bool {
        !matches!(self, Self::Other(_))
    }
}

impl fmt::Display for Language {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::{Language, OTHER_LANGUAGES};
    use std::path::Path;

    /// Every name round-trips, for grammar-backed and fallback languages alike.
    #[test]
    fn names_round_trip() {
        for language in Language::GRAMMAR_BACKED {
            assert_eq!(Language::from_name(language.name()), Some(language));
            assert!(language.is_grammar_backed());
        }
        for (name, _) in OTHER_LANGUAGES {
            let language = Language::from_name(name).expect("listed fallback language");
            assert_eq!(language.name(), *name);
            assert!(!language.is_grammar_backed());
        }
    }

    /// Extensions map to the expected languages regardless of case.
    #[test]
    fn extensions_are_recognized() {
        assert_eq!(Language::from_extension("TSX"), Some(Language::Tsx));
        assert_eq!(Language::from_extension("hpp"), Some(Language::Cpp));
        assert_eq!(Language::from_extension("h"), Some(Language::C));
        assert_eq!(
            Language::from_path(Path::new("src/app/main.py")),
            Some(Language::Python)
        );
        assert_eq!(Language::from_path(Path::new("Makefile")), None);
    }

    /// Related languages share a family and unrelated ones do not.
    #[test]
    fn families_group_related_languages() {
        assert_eq!(Language::Tsx.family(), "ecmascript");
        assert_eq!(Language::Cpp.family(), "c-family");
        assert_eq!(Language::Go.family(), "go");
        assert_eq!(Language::Other("kotlin").family(), "kotlin");
        assert_ne!(Language::Java.family(), Language::CSharp.family());
    }

    /// No extension is claimed by two languages.
    #[test]
    fn fallback_extensions_do_not_collide_with_grammar_backed_ones() {
        for (_, extensions) in OTHER_LANGUAGES {
            for extension in *extensions {
                let language = Language::from_extension(extension).expect("mapped");
                assert!(
                    !language.is_grammar_backed(),
                    "{extension} is claimed twice"
                );
            }
        }
    }
}
