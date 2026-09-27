// SPDX-License-Identifier: Apache-2.0
//! Fixture for the refusal ratchet: a message that only exists in test code.
//!
//! A message a test prints cannot reach a user or an agent, so the guard must find nothing here.

use std::fmt;

#[cfg(test)]
mod tests {
    use super::*;

    /// A throwaway error used only to exercise a test helper.
    struct ProbeError;

    impl fmt::Display for ProbeError {
        /// Says nothing useful, and does not have to: no user ever sees it.
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "probe failed")
        }
    }

    /// The probe error formats as expected.
    #[test]
    fn formats() {
        assert_eq!(ProbeError.to_string(), "probe failed");
    }
}
