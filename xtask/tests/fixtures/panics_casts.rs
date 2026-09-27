// SPDX-License-Identifier: Apache-2.0
//! Fixture for the panic ratchet: which `as` casts are recorded.
//!
//! A cast into an integer type is recorded, because the guard has no type information and cannot
//! tell a widening cast from a truncating one. A cast of a literal is not, because the compiler
//! already range-checks it, and a cast into a float is not, because it cannot truncate an integer
//! into a smaller integer.

/// Counters narrowed for a report.
pub struct Counters {
    /// How many symbols were seen.
    total: usize,
}

impl Counters {
    /// Narrows the count to 32 bits, which is recorded.
    pub fn narrowed(&self) -> u32 {
        self.total as u32
    }

    /// Widens the count to a float, which is not recorded.
    pub fn ratio(&self, of: usize) -> f64 {
        self.total as f64 / of as f64
    }

    /// Casts a literal, which the compiler already range-checks, so it is not recorded.
    pub fn marker() -> u8 {
        7 as u8
    }
}
