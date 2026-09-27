// SPDX-License-Identifier: Apache-2.0
//! Translation of SQLite failures into the [`StorageError`] of the storage port.
//!
//! Every function of the adapter returns `Result<_, StorageError>`. SQLite reports its own error
//! type, so this module is the single place that decides which failures mean "the stored data is
//! not usable" ([`StorageError::Corrupt`]) and which mean "the backend failed"
//! ([`StorageError::Backend`]). The foreign error types cannot implement `From` for each other,
//! so an extension trait, [`DbResult::db`], does the conversion at the call site.
//!
//! Invariant: no SQLite error escapes the crate unconverted, and no message ever contains
//! user-supplied query text beyond what SQLite itself reports.

use pn_ultramemory_core::StorageError;
use rusqlite::ErrorCode;
use rusqlite::types::Type;

/// The result type used inside the crate.
pub(crate) type Result<T> = std::result::Result<T, StorageError>;

/// Converts a SQLite error into a [`StorageError`].
///
/// Damaged or foreign files and values that cannot be decoded are reported as
/// [`StorageError::Corrupt`]; every other failure, including a database that stays locked past
/// the busy timeout, is a [`StorageError::Backend`].
pub(crate) fn from_sqlite(error: &rusqlite::Error) -> StorageError {
    match error {
        rusqlite::Error::SqliteFailure(failure, _)
            if matches!(
                failure.code,
                ErrorCode::NotADatabase | ErrorCode::DatabaseCorrupt
            ) =>
        {
            StorageError::Corrupt(error.to_string())
        }
        rusqlite::Error::FromSqlConversionFailure(..)
        | rusqlite::Error::InvalidColumnType(..)
        | rusqlite::Error::IntegralValueOutOfRange(..)
        | rusqlite::Error::Utf8Error(..) => StorageError::Corrupt(error.to_string()),
        other => StorageError::Backend(other.to_string()),
    }
}

/// Builds the error a row-mapping closure returns when a stored value cannot be decoded.
///
/// It surfaces as [`StorageError::Corrupt`] once it crosses [`DbResult::db`].
pub(crate) fn bad_value(column: usize, message: String) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(column, Type::Text, message.into())
}

/// Adds [`DbResult::db`] to SQLite results.
pub(crate) trait DbResult<T> {
    /// Converts the error side into a [`StorageError`].
    fn db(self) -> Result<T>;
}

impl<T> DbResult<T> for rusqlite::Result<T> {
    fn db(self) -> Result<T> {
        self.map_err(|error| from_sqlite(&error))
    }
}

#[cfg(test)]
mod tests {
    use super::{DbResult, bad_value, from_sqlite};
    use pn_ultramemory_core::StorageError;

    /// A failed decode is corruption, and a plain SQL error is a backend failure.
    #[test]
    fn classifies_errors() {
        let decode = bad_value(3, "unknown kind".into());
        assert!(matches!(from_sqlite(&decode), StorageError::Corrupt(_)));
        let backend = rusqlite::Error::InvalidQuery;
        assert!(matches!(from_sqlite(&backend), StorageError::Backend(_)));
    }

    /// The extension trait converts through the same mapping.
    #[test]
    fn extension_trait_converts() {
        let failed: rusqlite::Result<()> = Err(rusqlite::Error::InvalidQuery);
        assert!(matches!(failed.db(), Err(StorageError::Backend(_))));
        let fine: rusqlite::Result<u8> = Ok(4);
        assert_eq!(fine.db(), Ok(4));
    }
}
