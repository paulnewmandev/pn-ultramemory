// SPDX-License-Identifier: Apache-2.0
//! Fixture for the refusal ratchet: a message that names a way forward.
//!
//! The guard must count one message site, see one continuation and report no finding.

use std::fmt;

/// An error raised when the index has not been built yet.
pub struct IndexError {
    /// The repository the index is missing for.
    root: String,
}

impl fmt::Display for IndexError {
    /// Names the subcommand that builds the missing index.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "no index for {}; run pn-ultramemory index to build one", self.root)
    }
}
