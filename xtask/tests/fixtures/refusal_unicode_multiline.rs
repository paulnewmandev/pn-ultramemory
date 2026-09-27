// SPDX-License-Identifier: Apache-2.0
//! Fixture for the refusal ratchet: non-ASCII text, a message split over several lines, and a
//! message carrying a tab.
//!
//! The guard must key findings by whitespace-normalised text, so a message written across three
//! source lines becomes one baseline line, and an embedded tab never creates a second field.

use std::fmt;

/// An error raised when a path cannot be encoded for the store.
pub struct PathError;

/// An error raised when another process holds the store open.
pub struct LockError;

/// An error raised when required fields are absent.
pub struct FieldError;

impl fmt::Display for PathError {
    /// Carries non-ASCII characters, which the guard must count by character, not by byte.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "the path holds a café — dash that the store cannot encode")
    }
}

impl fmt::Display for LockError {
    /// Spans three source lines and names a command, so it is not a finding.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "another process holds the store open.\n\
             Close it, then run `pn-ultramemory doctor` to check the database."
        )
    }
}

impl fmt::Display for FieldError {
    /// Carries a tab, which must be folded away before the finding becomes a baseline line.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "two\tfields\tare missing")
    }
}
