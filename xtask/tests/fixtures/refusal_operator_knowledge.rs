// SPDX-License-Identifier: Apache-2.0
//! Fixture for the refusal ratchet: the `operator-knowledge` by-design shape.
//!
//! Only the operator knows which directory is the project root, so the tool cannot name the
//! continuation. The guard must excuse the site and report no finding.

use std::fmt;

/// An error raised when several directories could be the project root.
pub struct ProjectError;

impl fmt::Display for ProjectError {
    /// Explains the ambiguity without guessing which root the operator meant.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // refusal:by-design operator-knowledge: only the operator knows which root is intended
        write!(f, "several directories look like the project root")
    }
}
