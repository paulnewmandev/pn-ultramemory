// SPDX-License-Identifier: Apache-2.0
//! Fixture for the refusal ratchet: a by-design marker with a shape outside the vocabulary.
//!
//! The shape is misspelled. The guard must fail on the marker itself rather than quietly accept a
//! suppression nobody can read.

use std::fmt;

/// An error raised when the configuration file cannot be located.
pub struct LocateError;

impl fmt::Display for LocateError {
    /// Carries a marker whose shape is not one of the three accepted names.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // refusal:by-design operator-knowlege: misspelled shape, must be rejected
        write!(f, "the configuration file cannot be located")
    }
}
