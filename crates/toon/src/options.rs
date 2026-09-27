// SPDX-License-Identifier: Apache-2.0
//! Options shared by the TOON encoder and decoder.
//!
//! # Role in the architecture
//! `pn-ultramemory-toon` is a leaf crate: it turns [`serde_json::Value`] into TOON text and back,
//! with no I/O. This module holds the small, copyable configuration types that the public
//! [`encode`](crate::encode) and [`decode`](crate::decode) functions accept, mirroring the option
//! names of section 13 of the TOON 4.1 specification (`indentSize`, `delimiter`, `strict`).
//!
//! # Invariants
//! * Defaults follow the specification: indentation of 2 spaces, comma delimiter, strict decoding.
//! * An indentation of `0` is meaningless (levels could not be told apart), so both the encoder and
//!   the decoder treat it as `1`, and both cap it at [`MAX_INDENT`] so that an absurd value cannot
//!   exhaust memory.

/// Largest indentation size honoured by the encoder and the decoder (larger values are capped).
pub(crate) const MAX_INDENT: usize = 1024;

/// Returns the indentation size actually used for a requested `indent`: at least 1, at most
/// [`MAX_INDENT`].
pub(crate) fn effective_indent(indent: usize) -> usize {
    indent.clamp(1, MAX_INDENT)
}

/// The delimiter that separates field names, inline array values and row cells.
///
/// The encoder declares the chosen delimiter in every header it emits (`[3|]`, `{a|b}`), so a
/// decoder never has to guess it.
///
/// # Examples
///
/// ```
/// use pn_ultramemory_toon::Delimiter;
///
/// assert_eq!(Delimiter::default(), Delimiter::Comma);
/// assert_eq!(Delimiter::Pipe.as_char(), '|');
/// assert_eq!(Delimiter::Tab.as_char(), '\t');
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Delimiter {
    /// Comma (`,`), the default. The header carries no delimiter symbol.
    #[default]
    Comma,
    /// Horizontal tab (U+0009). The header carries a tab inside the brackets.
    Tab,
    /// Pipe (`|`). The header carries a pipe inside the brackets.
    Pipe,
}

impl Delimiter {
    /// Returns the character used to separate values.
    ///
    /// # Examples
    ///
    /// ```
    /// use pn_ultramemory_toon::Delimiter;
    ///
    /// assert_eq!(Delimiter::Comma.as_char(), ',');
    /// ```
    #[must_use]
    pub const fn as_char(self) -> char {
        match self {
            Self::Comma => ',',
            Self::Tab => '\t',
            Self::Pipe => '|',
        }
    }

    /// Returns the delimiter as a byte (all delimiters are ASCII).
    pub(crate) const fn as_byte(self) -> u8 {
        match self {
            Self::Comma => b',',
            Self::Tab => b'\t',
            Self::Pipe => b'|',
        }
    }

    /// Returns the delimiter symbol written inside a header's brackets (empty for comma).
    pub(crate) const fn header_symbol(self) -> &'static str {
        match self {
            Self::Comma => "",
            Self::Tab => "\t",
            Self::Pipe => "|",
        }
    }
}

/// Options for [`encode`](crate::encode).
///
/// # Examples
///
/// ```
/// use pn_ultramemory_toon::{Delimiter, EncodeOptions};
///
/// let defaults = EncodeOptions::default();
/// assert_eq!(defaults.indent, 2);
/// assert_eq!(defaults.delimiter, Delimiter::Comma);
///
/// let custom = EncodeOptions { indent: 4, delimiter: Delimiter::Pipe };
/// assert_eq!(custom.indent, 4);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncodeOptions {
    /// Number of spaces per indentation level. Default `2`; `0` is treated as `1` and values above
    /// 1024 are capped.
    pub indent: usize,
    /// Document delimiter, declared by every header the encoder emits. Default comma.
    pub delimiter: Delimiter,
}

impl Default for EncodeOptions {
    /// Returns the specification defaults: 2 spaces and the comma delimiter.
    fn default() -> Self {
        Self {
            indent: 2,
            delimiter: Delimiter::Comma,
        }
    }
}

/// Options for [`decode`](crate::decode).
///
/// # Examples
///
/// ```
/// use pn_ultramemory_toon::DecodeOptions;
///
/// let defaults = DecodeOptions::default();
/// assert_eq!(defaults.indent, 2);
/// assert!(defaults.strict);
///
/// let lenient = DecodeOptions { strict: false, ..DecodeOptions::default() };
/// assert!(!lenient.strict);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodeOptions {
    /// Number of spaces per indentation level. Default `2`; `0` is treated as `1` and values above
    /// 1024 are capped.
    pub indent: usize,
    /// Enforce the strict-mode checks of section 14 of the specification. Default `true`.
    pub strict: bool,
}

impl Default for DecodeOptions {
    /// Returns the specification defaults: 2 spaces and strict mode.
    fn default() -> Self {
        Self {
            indent: 2,
            strict: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The effective indentation is clamped to `1..=1024`.
    #[test]
    fn indentation_is_clamped() {
        assert_eq!(effective_indent(0), 1);
        assert_eq!(effective_indent(2), 2);
        assert_eq!(effective_indent(1024), 1024);
        assert_eq!(effective_indent(usize::MAX), 1024);
    }

    /// The defaults match the values fixed by the specification.
    #[test]
    fn defaults_follow_the_specification() {
        assert_eq!(EncodeOptions::default().indent, 2);
        assert_eq!(EncodeOptions::default().delimiter, Delimiter::Comma);
        assert_eq!(DecodeOptions::default().indent, 2);
        assert!(DecodeOptions::default().strict);
    }

    /// Each delimiter maps to its character and to the symbol written in headers.
    #[test]
    fn delimiter_characters_and_symbols() {
        assert_eq!(Delimiter::Comma.as_char(), ',');
        assert_eq!(Delimiter::Tab.as_char(), '\t');
        assert_eq!(Delimiter::Pipe.as_char(), '|');
        assert_eq!(Delimiter::Comma.as_byte(), b',');
        assert_eq!(Delimiter::Tab.as_byte(), b'\t');
        assert_eq!(Delimiter::Pipe.as_byte(), b'|');
        assert_eq!(Delimiter::Comma.header_symbol(), "");
        assert_eq!(Delimiter::Tab.header_symbol(), "\t");
        assert_eq!(Delimiter::Pipe.header_symbol(), "|");
    }
}
