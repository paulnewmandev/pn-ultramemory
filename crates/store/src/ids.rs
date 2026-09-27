// SPDX-License-Identifier: Apache-2.0
//! Stable symbol identities.
//!
//! A [`pn_ultramemory_core::SymbolId`] must survive re-indexing: edges, memory anchors and learned
//! statistics point at it. It is therefore not a counter but a function of what the symbol *is*:
//! the path of its file, its qualified name, its kind and its ordinal among the symbols of that
//! file that share the same name and kind (overloads, redefinitions). The function is
//! `core::hash64` over a length-prefixed encoding of those four parts, masked to a positive
//! 63-bit integer so it fits SQLite's signed column. The encoding is fixed for good: changing it
//! would orphan every stored edge and anchor, and a test pins a known value.
//!
//! # Collisions
//! Two different symbols hashing to the same 63-bit value is astronomically unlikely (about
//! 1 in 10^10 for fifty thousand symbols), but it must still be handled deterministically. When
//! a *new* key derives an id that is already taken (by another symbol of the same file, by an id
//! the file used to have, or by any stored symbol, which the store reports as a primary key
//! conflict on insert), the derivation is repeated with an increasing attempt number, which is
//! part of the hashed bytes, and the first free value wins. A key that already has a stored id in
//! the same file always keeps that id, so a resolved collision is stable across re-indexing too.
//!
//! Position in the architecture: internal to the storage adapter; only file upserts use it.

use std::collections::HashSet;

use pn_ultramemory_core::StorageError;
use pn_ultramemory_core::hash64;

use crate::error::Result;

/// Keeps 63 bits, so the id is never negative.
const ID_MASK: u64 = 0x7FFF_FFFF_FFFF_FFFF;

/// How many derivation attempts are made before giving up. Reaching it means the store is
/// unusable, not that a collision is likely.
const MAX_ATTEMPTS: u32 = 1_000;

/// What identifies a symbol inside the index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct IdKey<'a> {
    /// Path of the file that declares the symbol.
    pub(crate) path: &'a str,
    /// The qualified name of the symbol.
    pub(crate) qualified_name: &'a str,
    /// The stable name of the symbol's kind.
    pub(crate) kind: &'a str,
    /// Position among the symbols of the same file with the same qualified name and kind.
    pub(crate) ordinal: u32,
}

/// Appends a length-prefixed string, so that concatenations of different parts never collide.
fn push_part(bytes: &mut Vec<u8>, part: &str) {
    bytes.extend_from_slice(&(part.len() as u64).to_le_bytes());
    bytes.extend_from_slice(part.as_bytes());
}

/// Derives the id of a key for a given attempt (zero for the first), using `bytes` as scratch
/// space.
///
/// # Examples
/// ```text
/// derive_id(("src/lib.rs", "Parser::parse", "method", 0), 0) -> a positive i64
/// ```
pub(crate) fn derive_id_with(bytes: &mut Vec<u8>, key: &IdKey<'_>, attempt: u32) -> i64 {
    bytes.clear();
    push_part(bytes, key.path);
    push_part(bytes, key.qualified_name);
    push_part(bytes, key.kind);
    bytes.extend_from_slice(&key.ordinal.to_le_bytes());
    bytes.extend_from_slice(&attempt.to_le_bytes());
    let masked = hash64(bytes) & ID_MASK;
    i64::from_ne_bytes(masked.to_ne_bytes())
}

/// Derives the id of a key for a given attempt (zero for the first).
#[cfg(test)]
pub(crate) fn derive_id(key: &IdKey<'_>, attempt: u32) -> i64 {
    derive_id_with(
        &mut Vec::with_capacity(key.path.len() + key.qualified_name.len() + 48),
        key,
        attempt,
    )
}

/// Hands out ids that no other key of the batch has and that are not reserved.
///
/// The allocator only knows what it was told: the ids reserved at the start (the file's stored
/// ids, which are never recycled for a different symbol) and the ones it handed out. Whether a
/// derived id is also free in the *store* is discovered when the row is inserted: a primary key
/// conflict makes the caller ask for the next attempt with [`Allocator::claim`]. That keeps the
/// common case, a fresh id, free of any lookup.
#[derive(Debug, Default)]
pub(crate) struct Allocator {
    /// Every id that is spoken for.
    taken: HashSet<i64>,
    /// Scratch bytes for [`derive_id_with`].
    bytes: Vec<u8>,
}

impl Allocator {
    /// An allocator with some ids already spoken for.
    pub(crate) fn new(reserved: impl IntoIterator<Item = i64>) -> Self {
        Self {
            taken: reserved.into_iter().collect(),
            bytes: Vec::new(),
        }
    }

    /// Claims the first id of `key`, starting at attempt `first_attempt`, that is not spoken for,
    /// and returns it with the attempt that produced it.
    ///
    /// `derive` is a parameter only so that tests can force collisions; production code passes
    /// [`derive_id_with`].
    ///
    /// # Errors
    /// [`StorageError::Corrupt`] if no free id is found within the attempt limit.
    pub(crate) fn claim(
        &mut self,
        key: &IdKey<'_>,
        first_attempt: u32,
        derive: impl Fn(&mut Vec<u8>, &IdKey<'_>, u32) -> i64,
    ) -> Result<(i64, u32)> {
        for attempt in first_attempt..first_attempt.saturating_add(MAX_ATTEMPTS) {
            let candidate = derive(&mut self.bytes, key, attempt);
            if self.taken.insert(candidate) {
                return Ok((candidate, attempt));
            }
        }
        Err(StorageError::Corrupt(format!(
            "no free symbol id for `{}` in `{}` after {MAX_ATTEMPTS} attempts",
            key.qualified_name, key.path
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::{Allocator, IdKey, derive_id, derive_id_with};
    use crate::error::Result;

    /// A key used by several tests.
    fn key(ordinal: u32) -> IdKey<'static> {
        IdKey {
            path: "src/lib.rs",
            qualified_name: "Parser::parse",
            kind: "method",
            ordinal,
        }
    }

    /// The derivation never changes: this value is pinned because stored ids depend on it.
    #[test]
    fn derivation_is_pinned() {
        assert_eq!(derive_id(&key(0), 0), 5_762_637_690_985_864_666);
        assert_ne!(derive_id(&key(0), 0), derive_id(&key(1), 0));
        assert_ne!(derive_id(&key(0), 0), derive_id(&key(0), 1));
    }

    /// Ids are positive and depend on every part of the key.
    #[test]
    fn ids_are_positive_and_sensitive_to_every_part() {
        let base = derive_id(&key(0), 0);
        assert!(base >= 0);
        let other_path = IdKey {
            path: "src/main.rs",
            ..key(0)
        };
        let other_name = IdKey {
            qualified_name: "Parser::parse2",
            ..key(0)
        };
        let other_kind = IdKey {
            kind: "function",
            ..key(0)
        };
        for other in [other_path, other_name, other_kind] {
            assert_ne!(derive_id(&other, 0), base);
        }
        for ordinal in 0..2_000 {
            assert!(derive_id(&key(ordinal), 0) >= 0);
        }
    }

    /// Length prefixes keep `("ab", "c")` and `("a", "bc")` apart.
    #[test]
    fn parts_do_not_run_together() {
        let left = IdKey {
            path: "ab",
            qualified_name: "c",
            kind: "k",
            ordinal: 0,
        };
        let right = IdKey {
            path: "a",
            qualified_name: "bc",
            kind: "k",
            ordinal: 0,
        };
        assert_ne!(derive_id(&left, 0), derive_id(&right, 0));
    }

    /// Reserved ids are never handed out, and a fresh key gets its first derived id.
    #[test]
    fn claims_the_first_free_id() -> Result<()> {
        let mut allocator = Allocator::new([42]);
        let (id, attempt) = allocator.claim(&key(1), 0, derive_id_with)?;
        assert_eq!((id, attempt), (derive_id(&key(1), 0), 0));
        // The same key cannot be claimed twice: the second claim moves to the next attempt.
        let (again, attempt) = allocator.claim(&key(1), 0, derive_id_with)?;
        assert_eq!((again, attempt), (derive_id(&key(1), 1), 1));
        Ok(())
    }

    /// Collisions, inside the batch and against reserved ids, are resolved by moving on to the
    /// next attempt, deterministically; a retry after a store conflict starts past the failure.
    #[test]
    fn resolves_collisions_deterministically() -> Result<()> {
        // Every key derives the same id on attempt 0 and a distinct one afterwards.
        let forced = |_: &mut Vec<u8>, k: &IdKey<'_>, attempt: u32| -> i64 {
            if attempt == 0 {
                7
            } else {
                1_000 + i64::from(attempt) * 10 + i64::from(k.ordinal)
            }
        };
        let run = || -> Result<Vec<(i64, u32)>> {
            let mut allocator = Allocator::new([1_011]);
            (0..3)
                .map(|ordinal| allocator.claim(&key(ordinal), 0, forced))
                .collect()
        };
        // key 0 takes 7; key 1 collides with it and tries attempt 1 (1011 is reserved, so it
        // moves on to attempt 2: 1021); key 2 collides and gets attempt 1: 1012.
        let first = run()?;
        assert_eq!(first, vec![(7, 0), (1_021, 2), (1_012, 1)]);
        assert_eq!(first, run()?);
        // A store conflict on the id just handed out asks for the following attempt.
        let mut allocator = Allocator::new([]);
        let (id, attempt) = allocator.claim(&key(0), 0, forced)?;
        assert_eq!((id, attempt), (7, 0));
        let (next, attempt) = allocator.claim(&key(0), attempt + 1, forced)?;
        assert_eq!((next, attempt), (1_010, 1));
        Ok(())
    }

    /// Running out of attempts is reported, not looped on forever.
    #[test]
    fn gives_up_after_the_attempt_limit() {
        let mut allocator = Allocator::new([5]);
        let result = allocator.claim(&key(0), 0, |_, _, _| 5);
        assert!(result.is_err());
    }
}
