// SPDX-License-Identifier: Apache-2.0
//! Fixture for the refusal ratchet: the `world-action` by-design shape.
//!
//! The fix is outside the tool, so no command of ours would help. The guard must excuse the site.

use std::fmt;

/// An error raised when the disk holding the database has no space left.
pub struct DiskError;

impl fmt::Display for DiskError {
    /// Reports a condition of the machine rather than of this tool.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // refusal:by-design world-action: freeing disk space happens outside this tool
        write!(f, "the disk holding the database is full")
    }
}
