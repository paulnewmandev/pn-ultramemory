// SPDX-License-Identifier: Apache-2.0
//! Fixture for the refusal ratchet: one message on each of the four carriers.
//!
//! None of the four names a way forward, so the guard must report four findings: one from the
//! `#[error]` attribute, one from the error `Display` implementation, one from a refusal constructor
//! and one from an explanatory field.

use std::fmt;

/// A refusal raised by the storage adapter.
pub enum StoreError {
    /// The database file is not readable.
    #[error("the database file cannot be opened")]
    Unreadable,
    /// A stored row does not satisfy an invariant.
    Corrupt(String),
}

impl fmt::Display for StoreError {
    /// Formats the variant that carries no payload.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "the storage adapter refused the write")
    }
}

/// A refusal handed back to the caller with an explanation attached.
pub struct Rejection {
    /// Why the request was refused.
    reason: String,
}

/// Builds the refusal raised when a stored row fails its invariant.
pub fn corrupt_row() -> StoreError {
    StoreError::Corrupt("a symbol row has no owning file".to_owned())
}

/// Builds the refusal raised when the budget is too small for any capsule.
pub fn too_small() -> Rejection {
    Rejection {
        reason: "the budget is smaller than the smallest capsule".to_owned(),
    }
}
