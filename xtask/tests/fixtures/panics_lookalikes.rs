// SPDX-License-Identifier: Apache-2.0
//! Fixture for the panic ratchet: methods and macros whose names merely resemble the panicking ones.
//!
//! `unwrap_or`, `unwrap_or_default` and `expect_err` do not panic, `write!` is not `panic!`, and a
//! slice pattern is not an index expression. The guard must find nothing here.

/// A setting with a fallback.
pub struct Setting {
    /// The value, when one was given.
    value: Option<u32>,
}

impl Setting {
    /// Falls back instead of panicking.
    pub fn or_zero(&self) -> u32 {
        self.value.unwrap_or(0)
    }

    /// Falls back to the default instead of panicking.
    pub fn or_default(&self) -> u32 {
        self.value.unwrap_or_default()
    }

    /// Maps the value instead of unwrapping it.
    pub fn doubled(&self) -> Option<u32> {
        self.value.map(|value| value.saturating_mul(2))
    }
}
