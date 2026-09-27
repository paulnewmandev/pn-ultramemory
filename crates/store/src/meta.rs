// SPDX-License-Identifier: Apache-2.0
//! Free-form key/value metadata: the repository root, the time of the last index and the like.
//!
//! Keys and values are opaque text. Setting a key replaces its value; reading a key that was
//! never set gives `None`.

use rusqlite::{Connection, params};

use crate::error::Result;
use crate::query::{execute, query_opt};

/// Reads a metadata value.
pub(crate) fn get_meta(conn: &Connection, key: &str) -> Result<Option<String>> {
    query_opt(
        conn,
        "SELECT value FROM meta WHERE key = ?1",
        params![key],
        |row| row.get(0),
    )
}

/// Writes a metadata value, replacing any previous one.
pub(crate) fn set_meta(conn: &Connection, key: &str, value: &str) -> Result<()> {
    execute(
        conn,
        "INSERT INTO meta (key, value) VALUES (?1, ?2) \
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}
