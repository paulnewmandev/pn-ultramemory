// SPDX-License-Identifier: Apache-2.0
//! Non-cryptographic 64-bit hashing used to detect that a symbol changed.
//!
//! The hashes are only used to answer "is this text still the same?" for signatures and bodies,
//! so speed and stability matter and collision resistance against an adversary does not. The
//! algorithm is FNV-1a with 64 bits, which is fixed forever so stored hashes stay comparable
//! across releases.

/// FNV-1a 64-bit offset basis.
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

/// FNV-1a 64-bit prime.
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Hashes raw bytes with FNV-1a (64 bits).
///
/// # Examples
/// ```
/// use pn_ultramemory_core::hash64;
///
/// assert_eq!(hash64(b""), 0xcbf2_9ce4_8422_2325);
/// assert_eq!(hash64(b"a"), 0xaf63_dc4c_8601_ec8c);
/// ```
#[must_use]
pub fn hash64(bytes: &[u8]) -> u64 {
    let mut hash = FNV_OFFSET;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// Hashes text after collapsing every run of whitespace into a single space and trimming both
/// ends, so that re-indenting or re-wrapping code does not count as a change.
///
/// # Examples
/// ```
/// use pn_ultramemory_core::hash_normalized;
///
/// assert_eq!(hash_normalized("fn  f( )\n{ }"), hash_normalized("fn f( ) { }"));
/// assert_ne!(hash_normalized("fn f()"), hash_normalized("fn g()"));
/// ```
#[must_use]
pub fn hash_normalized(text: &str) -> u64 {
    let mut hash = FNV_OFFSET;
    let mut pending_space = false;
    let mut started = false;
    for byte in text.bytes() {
        if byte.is_ascii_whitespace() {
            pending_space = started;
            continue;
        }
        if pending_space {
            hash ^= u64::from(b' ');
            hash = hash.wrapping_mul(FNV_PRIME);
            pending_space = false;
        }
        started = true;
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::{hash_normalized, hash64};

    /// Published FNV-1a 64-bit test vectors.
    #[test]
    fn matches_known_fnv_vectors() {
        assert_eq!(hash64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(hash64(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(hash64(b"foobar"), 0x8594_4171_f739_67e8);
    }

    /// Whitespace differences never change the normalized hash, content differences do.
    #[test]
    fn normalization_ignores_whitespace_only() {
        assert_eq!(hash_normalized("  a \t b\n\nc  "), hash_normalized("a b c"));
        assert_ne!(hash_normalized("a b c"), hash_normalized("a b d"));
        assert_ne!(hash_normalized("ab"), hash_normalized("a b"));
    }

    /// Empty and whitespace-only text hash like the empty string.
    #[test]
    fn blank_text_hashes_like_empty() {
        assert_eq!(hash_normalized(""), hash64(b""));
        assert_eq!(hash_normalized(" \n\t "), hash64(b""));
    }
}
