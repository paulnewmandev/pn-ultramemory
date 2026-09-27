// SPDX-License-Identifier: Apache-2.0
//! Fixture for the refusal ratchet: a message that explains but names no way forward.
//!
//! The guard must count one message site, see no continuation and report exactly one finding.

use std::fmt;

/// An error raised when a capsule does not fit inside the token budget.
pub struct BudgetError;

impl fmt::Display for BudgetError {
    /// States the problem and stops there, which is the failure this guard exists to catch.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "the capsule exceeds the token budget")
    }
}
