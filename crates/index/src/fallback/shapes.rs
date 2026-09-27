// SPDX-License-Identifier: Apache-2.0
//! Declaration shapes recognised by the lexical fallback.
//!
//! # Role in the architecture
//! Given the text of one line (without its indentation), [`detect`] decides whether the line
//! starts a declaration and, if so, reports its kind, name and visibility. It knows the keyword
//! vocabulary shared by the fallback languages (`fun`, `fn`, `func`, `def`, `class`, `object`,
//! `trait`, `struct`, `enum`, `interface`, `type`, `module`, `defmodule`, `defp`, `proc`, ...),
//! the modifiers that may precede a keyword (`pub`, `public`, `private`, `static`, `async`,
//! `suspend`, `override`, `final`, `open`, `data`, ...) and a few shapes that have no keyword
//! (`name() {` in shell, `name <- function(` in R, `name :: Type` in Haskell, `(defn name` in
//! Clojure, `CREATE FUNCTION name` in SQL, `const Name = struct` in Zig).
//!
//! # Invariants
//! Pure and linear in the length of the line; never panics.

use pn_ultramemory_core::{SymbolKind, Visibility};

/// A declaration recognised on a line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Shape {
    /// The kind of symbol.
    pub(super) kind: SymbolKind,
    /// The simple name (the last segment of `written`).
    pub(super) name: String,
    /// The name as written, with `.` between segments (`Foo::bar` becomes `Foo.bar`).
    pub(super) written: String,
    /// Visibility given by a keyword, `Unknown` otherwise.
    pub(super) visibility: Visibility,
    /// Byte offset in the trimmed line where the declaration text starts, after annotations.
    pub(super) header_start: usize,
}

/// Words that may precede the declaration keyword.
const MODIFIERS: &[&str] = &[
    "pub",
    "public",
    "private",
    "protected",
    "internal",
    "fileprivate",
    "export",
    "static",
    "async",
    "suspend",
    "override",
    "final",
    "open",
    "abstract",
    "sealed",
    "inline",
    "noinline",
    "crossinline",
    "tailrec",
    "operator",
    "infix",
    "external",
    "extern",
    "unsafe",
    "const",
    "lateinit",
    "lazy",
    "dynamic",
    "mutating",
    "nonmutating",
    "convenience",
    "required",
    "partial",
    "readonly",
    "local",
    "case",
    "inner",
    "annotation",
    "value",
    "mutable",
    "default",
    "expect",
    "actual",
    "virtual",
    "native",
    "synchronized",
    "transient",
    "volatile",
    "impl",
    "implicit",
    "lazy",
    "final",
    "nonisolated",
    "distributed",
    "indirect",
    "prefix",
    "postfix",
];

/// Reads an identifier-like word (letters, digits, `_`, `$`) from the start of `text`.
fn take_word(text: &str) -> &str {
    let end = text
        .char_indices()
        .find(|(_, c)| !(c.is_alphanumeric() || *c == '_' || *c == '$'))
        .map_or(text.len(), |(i, _)| i);
    &text[..end]
}

/// Skips a balanced group that starts with `open` at the beginning of `text`, returning the rest.
fn skip_group(text: &str, open: char, close: char) -> &str {
    let mut depth = 0i32;
    for (i, c) in text.char_indices() {
        if c == open {
            depth += 1;
        } else if c == close {
            depth -= 1;
            if depth == 0 {
                return &text[i + close.len_utf8()..];
            }
        }
    }
    ""
}

/// Removes leading annotations such as `@Override`, `@Foo(1)` and `@JvmStatic`, returning the
/// rest of the line and how many bytes were removed.
fn strip_annotations(text: &str) -> (&str, usize) {
    let mut rest = text;
    while let Some(after) = rest.strip_prefix('@') {
        let word_len = after
            .char_indices()
            .find(|(_, c)| !(c.is_alphanumeric() || matches!(c, '_' | '.' | '$' | ':')))
            .map_or(after.len(), |(i, _)| i);
        if word_len == 0 {
            break;
        }
        rest = after[word_len..].trim_start();
        if rest.starts_with('(') {
            rest = skip_group(rest, '(', ')').trim_start();
        }
    }
    (rest, text.len() - rest.len())
}

/// Reads a possibly qualified name after a keyword: segments joined by `.` or `::`.
fn take_name(text: &str, dash: bool) -> Option<(String, usize)> {
    let mut out = String::new();
    let mut consumed = 0;
    loop {
        let part = &text[consumed..];
        let end = part
            .char_indices()
            .find(|(_, c)| {
                !(c.is_alphanumeric()
                    || matches!(c, '_' | '$' | '!' | '?' | '\'')
                    || (dash && *c == '-'))
            })
            .map_or(part.len(), |(i, _)| i);
        if end == 0 {
            break;
        }
        out.push_str(&part[..end]);
        consumed += end;
        let tail = &text[consumed..];
        if let Some(next) = tail.strip_prefix("::") {
            if next.chars().next().is_some_and(char::is_alphanumeric) {
                out.push('.');
                consumed += 2;
                continue;
            }
        } else if let Some(next) = tail.strip_prefix('.') {
            if next
                .chars()
                .next()
                .is_some_and(|c| c.is_alphabetic() || c == '_')
            {
                out.push('.');
                consumed += 1;
                continue;
            }
        } else if let Some(next) = tail.strip_prefix(':') {
            // Lua method syntax `function Class:method(`.
            if next
                .chars()
                .next()
                .is_some_and(|c| c.is_alphabetic() || c == '_')
                && !next.starts_with(':')
            {
                out.push('.');
                consumed += 1;
                continue;
            }
        }
        break;
    }
    let first = out.chars().next()?;
    if !(first.is_alphabetic() || first == '_' || first == '$') {
        return None;
    }
    Some((out, consumed))
}

/// Maps a keyword to the kind of symbol it declares in the given language.
fn keyword_kind(language: &str, word: &str) -> Option<SymbolKind> {
    let kind = match word {
        "fun" | "fn" | "func" | "def" | "defp" | "function" | "proc" | "sub" | "defn" | "defn-" => {
            SymbolKind::Function
        }
        "class" if language == "haskell" => SymbolKind::Interface,
        "class" | "object" | "actor" => SymbolKind::Class,
        "interface" | "trait" | "protocol" | "defprotocol" => SymbolKind::Interface,
        "struct" | "record" | "union" | "defrecord" => SymbolKind::Struct,
        "enum" => SymbolKind::Enum,
        "type" | "typealias" | "newtype" | "data" | "deftype" => SymbolKind::Type,
        "module" | "defmodule" | "namespace" | "ns" => SymbolKind::Module,
        "package" if language == "perl" => SymbolKind::Module,
        "macro" | "defmacro" | "defmacrop" => SymbolKind::Macro,
        "extension" | "defimpl" => SymbolKind::Other,
        _ => return None,
    };
    Some(kind)
}

/// Returns the visibility that a modifier or a keyword implies.
fn word_visibility(word: &str) -> Option<Visibility> {
    match word {
        "pub" | "public" | "export" => Some(Visibility::Public),
        "private" | "fileprivate" | "protected" | "internal" | "local" | "defp" | "defmacrop" => {
            Some(Visibility::Private)
        }
        _ => None,
    }
}

/// Detects a declaration on a line that has been trimmed of its indentation.
pub(super) fn detect(language: &str, head: &str) -> Option<Shape> {
    let (text, skipped) = strip_annotations(head);
    if text.is_empty() {
        return None;
    }
    let dash = matches!(language, "shell" | "clojure");
    special(language, text, dash)
        .or_else(|| keyword_shape(language, text, dash))
        .map(|mut shape| {
            shape.header_start += skipped;
            shape
        })
}

/// Detects the shapes that have no leading keyword or that need language knowledge.
fn special(language: &str, text: &str, dash: bool) -> Option<Shape> {
    match language {
        "shell" => shell_function(text),
        "r" => r_function(text),
        "clojure" => clojure_form(text),
        "sql" => sql_create(text),
        "haskell" => haskell_signature(text),
        "zig" => zig_const(text, dash),
        "elixir" => elixir_form(text),
        _ => None,
    }
}

/// Builds a shape from parts.
fn shape(kind: SymbolKind, written: String, visibility: Visibility, header_start: usize) -> Shape {
    let name = written.rsplit('.').next().unwrap_or(&written).to_owned();
    Shape {
        kind,
        name,
        written,
        visibility,
        header_start,
    }
}

/// `name() {` and `function name` in shell; the second form is left to the keyword scan.
fn shell_function(text: &str) -> Option<Shape> {
    let (name, used) = take_name(text, true)?;
    let rest = text[used..].trim_start().strip_prefix('(')?.trim_start();
    rest.starts_with(')')
        .then(|| shape(SymbolKind::Function, name, Visibility::Unknown, 0))
}

/// `name <- function(` and `name = function(` in R.
fn r_function(text: &str) -> Option<Shape> {
    let (name, used) = take_name(text, false)?;
    let rest = text[used..].trim_start();
    let rest = rest
        .strip_prefix("<-")
        .or_else(|| rest.strip_prefix("<<-"))
        .or_else(|| rest.strip_prefix('='))?
        .trim_start();
    rest.starts_with("function")
        .then(|| shape(SymbolKind::Function, name, Visibility::Unknown, 0))
}

/// `(defn name`, `(def name`, `(defmacro name`, `(ns name`, ... in Clojure.
fn clojure_form(text: &str) -> Option<Shape> {
    let rest = text.strip_prefix('(')?;
    let full = rest.split(char::is_whitespace).next().unwrap_or("");
    let (kind, visibility) = match full {
        "defn" | "defmulti" | "defmethod" => (SymbolKind::Function, Visibility::Public),
        "defn-" => (SymbolKind::Function, Visibility::Private),
        "def" | "defonce" => (SymbolKind::Constant, Visibility::Public),
        "defmacro" => (SymbolKind::Macro, Visibility::Public),
        "defprotocol" => (SymbolKind::Interface, Visibility::Public),
        "defrecord" | "deftype" => (SymbolKind::Struct, Visibility::Public),
        "ns" => (SymbolKind::Module, Visibility::Public),
        _ => return None,
    };
    let after = rest[full.len()..]
        .trim_start()
        .trim_start_matches(['^', ':']);
    let (name, _) = take_name(after, true)?;
    Some(shape(kind, name, visibility, 0))
}

/// `CREATE [OR REPLACE] FUNCTION name` and the other `CREATE` statements in SQL.
fn sql_create(text: &str) -> Option<Shape> {
    let lower = text.to_ascii_lowercase();
    let mut words = lower.split_whitespace().peekable();
    if words.next()? != "create" {
        return None;
    }
    let mut consumed = 6;
    let mut kind = None;
    for word in words {
        consumed += lower[consumed..].find(word).map_or(0, |i| i + word.len());
        match word {
            "or" | "replace" | "temp" | "temporary" | "unique" | "materialized" | "global"
            | "local" | "if" | "not" | "exists" => {}
            "function" | "procedure" => {
                kind = Some(SymbolKind::Function);
                break;
            }
            "table" | "view" => {
                kind = Some(SymbolKind::Struct);
                break;
            }
            "type" | "domain" => {
                kind = Some(SymbolKind::Type);
                break;
            }
            "index" | "trigger" | "sequence" | "schema" => {
                kind = Some(SymbolKind::Other);
                break;
            }
            _ => return None,
        }
    }
    let kind = kind?;
    let mut rest = text[consumed..].trim_start();
    for filler in ["if not exists", "IF NOT EXISTS"] {
        if let Some(after) = rest.strip_prefix(filler) {
            rest = after.trim_start();
        }
    }
    let (name, _) = take_name(rest.trim_start_matches(['"', '`', '[']), false)?;
    Some(shape(kind, name, Visibility::Unknown, 0))
}

/// `name :: Type` at the start of a Haskell line.
fn haskell_signature(text: &str) -> Option<Shape> {
    let word = take_word(text);
    let first = word.chars().next()?;
    if !(first.is_lowercase() || first == '_') {
        return None;
    }
    let rest = text[word.len()..].trim_start();
    if !rest.starts_with("::") || matches!(word, "let" | "where" | "in" | "do" | "case" | "of") {
        return None;
    }
    Some(shape(
        SymbolKind::Function,
        word.to_owned(),
        Visibility::Unknown,
        0,
    ))
}

/// `const Name = struct {` (also `enum`, `union`) in Zig, with optional `pub`.
fn zig_const(text: &str, dash: bool) -> Option<Shape> {
    let (rest, visibility) = match text.strip_prefix("pub ") {
        Some(rest) => (rest.trim_start(), Visibility::Public),
        None => (text, Visibility::Unknown),
    };
    let after = rest.strip_prefix("const ")?.trim_start();
    let (name, used) = take_name(after, dash)?;
    let value = after[used..].trim_start().strip_prefix('=')?.trim_start();
    let value = value
        .strip_prefix("packed ")
        .or_else(|| value.strip_prefix("extern "))
        .unwrap_or(value);
    let kind = if value.starts_with("struct") || value.starts_with("union") {
        SymbolKind::Struct
    } else if value.starts_with("enum") {
        SymbolKind::Enum
    } else {
        return None;
    };
    Some(shape(kind, name, visibility, 0))
}

/// `defmodule Name do`, `def name`, `defp name`, `defmacro name` in Elixir.
fn elixir_form(text: &str) -> Option<Shape> {
    let word = take_word(text);
    if !matches!(
        word,
        "defmodule"
            | "defprotocol"
            | "defimpl"
            | "def"
            | "defp"
            | "defmacro"
            | "defmacrop"
            | "defguard"
    ) {
        return None;
    }
    let rest = text[word.len()..].strip_prefix(' ')?.trim_start();
    let (name, _) = take_name(rest, false)?;
    let kind = match word {
        "defmodule" => SymbolKind::Module,
        "defprotocol" => SymbolKind::Interface,
        "defimpl" => SymbolKind::Other,
        "defmacro" | "defmacrop" | "defguard" => SymbolKind::Macro,
        _ => SymbolKind::Function,
    };
    let visibility = match word {
        "defp" | "defmacrop" => Visibility::Private,
        "def" | "defmacro" | "defmodule" | "defprotocol" => Visibility::Public,
        _ => Visibility::Unknown,
    };
    Some(shape(kind, name, visibility, 0))
}

/// Recognises `[modifiers] keyword name` at the start of `text`.
fn keyword_shape(language: &str, text: &str, dash: bool) -> Option<Shape> {
    let mut rest = text;
    let mut visibility = Visibility::Unknown;
    let mut enum_class = false;
    for _ in 0..12 {
        let word = take_word(rest);
        if word.is_empty() {
            return None;
        }
        let after = &rest[word.len()..];
        let followed_by_space = after.starts_with(char::is_whitespace);
        let next_word = take_word(after.trim_start());
        if !(followed_by_space || (word == "pub" && after.starts_with('('))) {
            return None;
        }
        if let Some(v) = word_visibility(word) {
            if MODIFIERS.contains(&word) {
                visibility = v;
                rest = skip_modifier_group(after);
                continue;
            }
        }
        if word == "enum" && next_word == "class" {
            enum_class = true;
            rest = after.trim_start();
            continue;
        }
        if word == "data" && matches!(next_word, "class" | "object" | "interface") {
            rest = after.trim_start();
            continue;
        }
        if word == "fun" && next_word == "interface" {
            rest = after.trim_start();
            continue;
        }
        if MODIFIERS.contains(&word) && keyword_kind(language, word).is_none() {
            rest = skip_modifier_group(after);
            continue;
        }
        let mut kind = keyword_kind(language, word)?;
        if let Some(v) = word_visibility(word) {
            visibility = v;
        }
        if enum_class && word == "class" {
            kind = SymbolKind::Enum;
        }
        let name_text = skip_generics(after.trim_start());
        let (written, _) = take_name(name_text, dash)?;
        return Some(shape(kind, written, visibility, 0));
    }
    None
}

/// Skips the parenthesised or bracketed group that can follow `pub` or `private` (`pub(crate)`,
/// `private[this]`), returning the rest with leading whitespace removed.
fn skip_modifier_group(after: &str) -> &str {
    let trimmed = after.trim_start();
    let group = after.starts_with('(') || after.starts_with('[');
    if group {
        let (open, close) = if after.starts_with('(') {
            ('(', ')')
        } else {
            ('[', ']')
        };
        return skip_group(after, open, close).trim_start();
    }
    trimmed
}

/// Skips a leading `<...>` or `[...]` generic parameter list after a keyword.
fn skip_generics(text: &str) -> &str {
    if text.starts_with('<') {
        skip_group(text, '<', '>').trim_start()
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::detect;
    use pn_ultramemory_core::{SymbolKind, Visibility};

    /// Returns the kind, written name and visibility of a detected declaration.
    fn parts(language: &str, line: &str) -> Option<(SymbolKind, String, Visibility)> {
        detect(language, line).map(|s| (s.kind, s.written, s.visibility))
    }

    /// Keyword declarations with modifiers are recognised in the C-like languages.
    #[test]
    fn keyword_declarations() {
        assert_eq!(
            parts("kotlin", "private suspend fun load(id: Int): User {"),
            Some((SymbolKind::Function, "load".into(), Visibility::Private))
        );
        assert_eq!(
            parts("kotlin", "data class Point(val x: Int)"),
            Some((SymbolKind::Class, "Point".into(), Visibility::Unknown))
        );
        assert_eq!(
            parts("kotlin", "enum class Color { RED }"),
            Some((SymbolKind::Enum, "Color".into(), Visibility::Unknown))
        );
        assert_eq!(
            parts("swift", "public protocol Shape {"),
            Some((SymbolKind::Interface, "Shape".into(), Visibility::Public))
        );
        assert_eq!(
            parts("swift", "@MainActor final class Model: Base {"),
            Some((SymbolKind::Class, "Model".into(), Visibility::Unknown))
        );
        assert_eq!(
            parts("zig", "pub fn main() void {"),
            Some((SymbolKind::Function, "main".into(), Visibility::Public))
        );
        assert_eq!(
            parts("scala", "case class User(name: String)"),
            Some((SymbolKind::Class, "User".into(), Visibility::Unknown))
        );
        assert_eq!(
            parts("kotlin", "fun <T> String.second(): T = this[1]"),
            Some((
                SymbolKind::Function,
                "String.second".into(),
                Visibility::Unknown
            ))
        );
    }

    /// Language-specific shapes are recognised.
    #[test]
    fn special_shapes() {
        assert_eq!(
            parts("lua", "local function helper(a, b)"),
            Some((SymbolKind::Function, "helper".into(), Visibility::Private))
        );
        assert_eq!(
            parts("lua", "function M.run(x)"),
            Some((SymbolKind::Function, "M.run".into(), Visibility::Unknown))
        );
        assert_eq!(
            parts("shell", "deploy_app() {"),
            Some((
                SymbolKind::Function,
                "deploy_app".into(),
                Visibility::Unknown
            ))
        );
        assert_eq!(
            parts("elixir", "defp secret(x) do"),
            Some((SymbolKind::Function, "secret".into(), Visibility::Private))
        );
        assert_eq!(
            parts("elixir", "defmodule MyApp.Worker do"),
            Some((
                SymbolKind::Module,
                "MyApp.Worker".into(),
                Visibility::Public
            ))
        );
        assert_eq!(
            parts("r", "square <- function(x) {"),
            Some((SymbolKind::Function, "square".into(), Visibility::Unknown))
        );
        assert_eq!(
            parts("haskell", "parse :: String -> Int"),
            Some((SymbolKind::Function, "parse".into(), Visibility::Unknown))
        );
        assert_eq!(
            parts("clojure", "(defn- helper [x]"),
            Some((SymbolKind::Function, "helper".into(), Visibility::Private))
        );
        assert_eq!(
            parts("sql", "CREATE OR REPLACE FUNCTION total(a int)"),
            Some((SymbolKind::Function, "total".into(), Visibility::Unknown))
        );
        assert_eq!(
            parts("zig", "const Point = struct {"),
            Some((SymbolKind::Struct, "Point".into(), Visibility::Unknown))
        );
    }

    /// Ordinary statements that merely contain a keyword are not declarations.
    #[test]
    fn statements_are_not_declarations() {
        assert_eq!(parts("kotlin", "val classes = listOf(1)"), None);
        assert_eq!(parts("lua", "type = 5"), None);
        assert_eq!(parts("lua", "object.method(1)"), None);
        assert_eq!(parts("kotlin", "return fun()"), None);
        assert_eq!(parts("kotlin", ""), None);
    }
}
