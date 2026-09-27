// SPDX-License-Identifier: Apache-2.0
//! Fixture for the refusal ratchet: the `human-authority` by-design shape.
//!
//! A person must decide whether to overwrite the stored memories; naming a command here would push
//! them past the decision. The guard must excuse the site.

use std::fmt;

/// An error raised when an operation would discard memories a person may still want.
pub struct OverwriteError;

impl fmt::Display for OverwriteError {
    /// Stops short of naming the destructive command on purpose.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // refusal:by-design human-authority: discarding stored memories is a person's decision
        write!(f, "this would discard 412 stored memories")
    }
}
