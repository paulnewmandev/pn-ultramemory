// SPDX-License-Identifier: Apache-2.0
//! The error type returned by the TOON decoder.
//!
//! # Role in the architecture
//! Decoding is the only fallible direction of this crate (encoding is total). Every failure,
//! whether a strict-mode violation of section 14 of the specification or an error that applies in
//! every mode (bad escape, missing colon, ...), is reported as a [`DecodeError`] that carries the
//! 1-based line number of the offending line and a human-readable message.
//!
//! # Invariants
//! * The line number is 1-based and refers to the original input, counting comment and blank lines.
//! * Messages are stable enough to read but are not an API: match on [`DecodeError::line`], not on
//!   the message text.

use std::fmt;

/// A failure to decode TOON text, with the line where it was detected.
///
/// # Examples
///
/// ```
/// use pn_ultramemory_toon::from_str;
///
/// let err = from_str("tags[3]: a,b").unwrap_err();
/// assert_eq!(err.line(), 1);
/// assert!(err.to_string().starts_with("line 1: "));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodeError {
    /// 1-based line number in the original input.
    line: usize,
    /// Description of what is wrong.
    message: String,
}

impl DecodeError {
    /// Creates an error for `line` (1-based) with the given message.
    pub(crate) fn new(line: usize, message: impl Into<String>) -> Self {
        Self {
            line,
            message: message.into(),
        }
    }

    /// Returns the 1-based line number of the offending line.
    ///
    /// # Examples
    ///
    /// ```
    /// use pn_ultramemory_toon::from_str;
    ///
    /// // The comment and the blank line count, so the bad line is line 4.
    /// let err = from_str("# note\n\na: 1\nb: \"bad\\q\"").unwrap_err();
    /// assert_eq!(err.line(), 4);
    /// ```
    #[must_use]
    pub fn line(&self) -> usize {
        self.line
    }

    /// Returns the description of the problem, without the line prefix.
    ///
    /// # Examples
    ///
    /// ```
    /// use pn_ultramemory_toon::from_str;
    ///
    /// let err = from_str("a: 1\na: 2").unwrap_err();
    /// assert!(err.message().contains("duplicate key"));
    /// ```
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for DecodeError {
    /// Formats the error as `line N: message`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for DecodeError {}

#[cfg(test)]
mod tests {
    use super::*;

    /// The display form is `line N: message` and the accessors return the parts.
    #[test]
    fn display_and_accessors() {
        let err = DecodeError::new(7, "bad thing");
        assert_eq!(err.line(), 7);
        assert_eq!(err.message(), "bad thing");
        assert_eq!(err.to_string(), "line 7: bad thing");
    }

    /// The error can be used as a boxed `std::error::Error`.
    #[test]
    fn works_as_std_error() {
        let boxed: Box<dyn std::error::Error> = Box::new(DecodeError::new(1, "x"));
        assert_eq!(boxed.to_string(), "line 1: x");
    }
}
