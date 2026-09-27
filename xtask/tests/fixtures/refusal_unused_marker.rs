// SPDX-License-Identifier: Apache-2.0
//! Fixture for the refusal ratchet: a by-design marker that excuses nothing.
//!
//! The message beside the marker already names a way forward, so the suppression is dead. A
//! suppression that suppresses nothing is the same failure as a baseline entry that matches nothing,
//! and the guard must fail on it.

use std::fmt;

/// An error raised when the store schema is older than this build expects.
pub struct SchemaError;

impl fmt::Display for SchemaError {
    /// Names a command, so the marker above it has nothing to excuse.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // refusal:by-design world-action: left behind after the message was improved
        write!(f, "the store schema is out of date; run pn-ultramemory migrate")
    }
}
