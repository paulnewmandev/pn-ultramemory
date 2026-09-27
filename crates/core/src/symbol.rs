// SPDX-License-Identifier: Apache-2.0
//! What an extractor reports about one source file: its symbols and the references between them.
//!
//! These are *drafts*: plain values produced by parsing, before storage assigns identities. A
//! symbol draft refers to its parent by position in the same list, and a reference draft refers to
//! the symbol that contains it the same way.

use core::fmt;

use crate::Language;

/// The kind of a declared program element.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SymbolKind {
    /// A module, namespace or package.
    Module,
    /// A class.
    Class,
    /// A struct, record or similar aggregate.
    Struct,
    /// An interface, trait or protocol.
    Interface,
    /// An enumeration.
    Enum,
    /// A free function.
    Function,
    /// A function that belongs to a type.
    Method,
    /// A constant or static.
    Constant,
    /// A variable or field.
    Variable,
    /// A type alias or similar.
    Type,
    /// A macro.
    Macro,
    /// Anything else worth listing.
    Other,
}

impl SymbolKind {
    /// Every kind.
    pub const ALL: [Self; 12] = [
        Self::Module,
        Self::Class,
        Self::Struct,
        Self::Interface,
        Self::Enum,
        Self::Function,
        Self::Method,
        Self::Constant,
        Self::Variable,
        Self::Type,
        Self::Macro,
        Self::Other,
    ];

    /// Stable lowercase name used in storage and in output.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Module => "module",
            Self::Class => "class",
            Self::Struct => "struct",
            Self::Interface => "interface",
            Self::Enum => "enum",
            Self::Function => "function",
            Self::Method => "method",
            Self::Constant => "constant",
            Self::Variable => "variable",
            Self::Type => "type",
            Self::Macro => "macro",
            Self::Other => "other",
        }
    }

    /// Looks a kind up by the name returned from [`SymbolKind::as_str`].
    ///
    /// # Examples
    /// ```
    /// use pn_ultramemory_core::SymbolKind;
    ///
    /// for kind in SymbolKind::ALL {
    ///     assert_eq!(SymbolKind::from_name(kind.as_str()), Some(kind));
    /// }
    /// assert_eq!(SymbolKind::from_name("nonsense"), None);
    /// ```
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == name)
    }

    /// Returns `true` for kinds that can be called.
    #[must_use]
    pub const fn is_callable(self) -> bool {
        matches!(self, Self::Function | Self::Method | Self::Macro)
    }
}

impl fmt::Display for SymbolKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Whether a symbol is part of its module's public surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Visibility {
    /// Visible outside its module or file.
    Public,
    /// Internal to its module or file.
    Private,
    /// The language or the fallback could not tell.
    Unknown,
}

impl Visibility {
    /// Stable lowercase name used in storage and in output.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Private => "private",
            Self::Unknown => "unknown",
        }
    }

    /// Looks a visibility up by the name returned from [`Visibility::as_str`].
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        [Self::Public, Self::Private, Self::Unknown]
            .into_iter()
            .find(|v| v.as_str() == name)
    }
}

/// Where a symbol sits in its file. Lines are 1-based and inclusive; bytes are 0-based offsets
/// with an exclusive end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Span {
    /// First line of the declaration, including any attached documentation comment.
    pub start_line: u32,
    /// Last line of the declaration.
    pub end_line: u32,
    /// Byte offset where the declaration starts.
    pub start_byte: u32,
    /// Byte offset just after the declaration.
    pub end_byte: u32,
}

impl Span {
    /// Number of bytes the span covers.
    #[must_use]
    pub const fn len_bytes(&self) -> u32 {
        self.end_byte.saturating_sub(self.start_byte)
    }

    /// Number of lines the span covers.
    #[must_use]
    pub const fn line_count(&self) -> u32 {
        self.end_line.saturating_sub(self.start_line) + 1
    }
}

/// A declared program element, as reported by an extractor.
#[derive(Debug, Clone, PartialEq)]
pub struct SymbolDraft {
    /// The simple name, such as `parse`.
    pub name: String,
    /// The name qualified by its enclosing symbols, such as `Parser::parse` or `models.User.save`,
    /// using the separator the language uses.
    pub qualified_name: String,
    /// What kind of element it is.
    pub kind: SymbolKind,
    /// The declaration on a single line: whitespace collapsed, no body, at most 240 characters.
    pub signature: String,
    /// The documentation attached to the symbol, with comment markers removed, if any.
    pub doc: Option<String>,
    /// Whether the symbol is public.
    pub visibility: Visibility,
    /// Where the symbol is in the file.
    pub span: Span,
    /// Index of the enclosing symbol in the same list, if any.
    pub parent: Option<usize>,
    /// Distinct names called inside the symbol, in order of first appearance, at most 24.
    pub outline: Vec<String>,
    /// Whitespace-normalized hash of the signature.
    pub sig_hash: u64,
    /// Whitespace-normalized hash of the whole declaration, body included.
    pub body_hash: u64,
}

/// What a reference to another name means.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RefKind {
    /// A call or invocation.
    Call,
    /// Use of a type name, for example in an annotation or a `new` expression.
    Type,
    /// Inheritance, implementation or extension.
    Inherit,
}

impl RefKind {
    /// Stable lowercase name used in storage.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Call => "call",
            Self::Type => "type",
            Self::Inherit => "inherit",
        }
    }

    /// Looks a kind up by the name returned from [`RefKind::as_str`].
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        [Self::Call, Self::Type, Self::Inherit]
            .into_iter()
            .find(|k| k.as_str() == name)
    }
}

/// A mention of another symbol's name inside a file, before it is resolved to a definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceDraft {
    /// The simple name that is referenced, such as `parse`.
    pub name: String,
    /// What kind of reference it is.
    pub kind: RefKind,
    /// The 1-based line of the reference.
    pub line: u32,
    /// Index of the symbol that contains the reference, or `None` at file level.
    pub owner: Option<usize>,
    /// The receiver or namespace written before the name, such as `self` or `models`, if any.
    pub qualifier: Option<String>,
}

/// Everything an extractor reports about one file.
#[derive(Debug, Clone, PartialEq)]
pub struct FileExtract {
    /// The language the file was parsed as.
    pub language: Language,
    /// Symbols in source order. A parent always comes before its children.
    pub symbols: Vec<SymbolDraft>,
    /// References to other names, in source order.
    pub references: Vec<ReferenceDraft>,
    /// Raw import or include targets as written, such as `std::fmt` or `./util`.
    pub imports: Vec<String>,
    /// Total number of lines in the file.
    pub line_count: u32,
    /// Number of syntax errors the parser recovered from. Zero for a clean parse.
    pub parse_errors: u32,
}

/// Returns the first sentence of a documentation text, on one line.
///
/// The text is cut at the first `. ` (or at the end of the first paragraph if there is none
/// sooner) and whitespace is collapsed. The trailing period is kept.
///
/// # Examples
/// ```
/// use pn_ultramemory_core::first_sentence;
///
/// assert_eq!(first_sentence("Parses input.\nSecond sentence."), "Parses input.");
/// assert_eq!(first_sentence("No period here\n\nNext paragraph"), "No period here");
/// assert_eq!(first_sentence(""), "");
/// ```
#[must_use]
pub fn first_sentence(doc: &str) -> String {
    let paragraph = doc.trim().split("\n\n").next().unwrap_or("");
    let flat = paragraph.split_whitespace().collect::<Vec<_>>().join(" ");
    match flat.find(". ") {
        Some(end) => flat[..=end].to_owned(),
        None => flat,
    }
}

#[cfg(test)]
mod tests {
    use super::{Span, SymbolKind, Visibility, first_sentence};

    /// Kind and visibility names round-trip and kinds are unique.
    #[test]
    fn names_round_trip() {
        for kind in SymbolKind::ALL {
            assert_eq!(SymbolKind::from_name(kind.as_str()), Some(kind));
        }
        for visibility in [Visibility::Public, Visibility::Private, Visibility::Unknown] {
            assert_eq!(Visibility::from_name(visibility.as_str()), Some(visibility));
        }
    }

    /// Only functions, methods and macros are callable.
    #[test]
    fn callable_kinds() {
        let callable: Vec<_> = SymbolKind::ALL
            .into_iter()
            .filter(|k| k.is_callable())
            .collect();
        assert_eq!(
            callable,
            [SymbolKind::Function, SymbolKind::Method, SymbolKind::Macro]
        );
    }

    /// Span measurements are inclusive for lines and never underflow.
    #[test]
    fn span_measures() {
        let span = Span {
            start_line: 3,
            end_line: 3,
            start_byte: 10,
            end_byte: 4,
        };
        assert_eq!(span.line_count(), 1);
        assert_eq!(span.len_bytes(), 0);
        let span = Span {
            start_line: 3,
            end_line: 7,
            start_byte: 10,
            end_byte: 90,
        };
        assert_eq!(span.line_count(), 5);
        assert_eq!(span.len_bytes(), 80);
    }

    /// The first sentence stops at a period followed by a space, or at the paragraph end.
    #[test]
    fn first_sentence_cases() {
        assert_eq!(
            first_sentence("  Adds two numbers. Really.  "),
            "Adds two numbers."
        );
        assert_eq!(
            first_sentence("Version 1.2 is fine. More."),
            "Version 1.2 is fine."
        );
        assert_eq!(
            first_sentence("Multi\nline   text\n\nother"),
            "Multi line text"
        );
    }
}
