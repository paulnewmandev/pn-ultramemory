// SPDX-License-Identifier: Apache-2.0
//! Fixture for the panic ratchet: panicking constructs that only exist in test code.
//!
//! Panicking is how a test reports a failed assertion, so the guard must find nothing here, whether
//! the construct sits in a `#[cfg(test)]` module or in a bare `#[test]` function.

/// Returns the answer, without panicking.
pub fn answer() -> u8 {
    42
}

/// The answer is the answer.
#[test]
fn bare_test_function() {
    let values = vec![answer()];
    assert_eq!(values[0], 42);
    let first = values.first().unwrap();
    assert_eq!(*first, 42);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Indexing, unwrapping and panicking are all normal inside a test module.
    #[test]
    fn asserts_with_panics() {
        let values = vec![answer()];
        assert_eq!(values[0], 42);
        let widened = values.len() as u32;
        assert_eq!(widened, 1);
        if widened != 1 {
            panic!("the vector changed size");
        }
    }
}
