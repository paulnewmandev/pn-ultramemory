// SPDX-License-Identifier: Apache-2.0
//! Small helpers that run a cached statement and return owned results.
//!
//! Every query of the adapter goes through `prepare_cached`, so a hot statement is parsed once per
//! connection. These helpers keep that pattern in one place and convert errors as they go.
//!
//! Invariant: SQL text passed here is either a constant or assembled from constants and
//! placeholder counts; every caller-supplied value travels as a bound parameter.

use rusqlite::{Connection, OptionalExtension, Params, Row};

use crate::error::{DbResult, Result};

/// Runs a query and maps every row.
pub(crate) fn query_all<T, P, F>(conn: &Connection, sql: &str, params: P, map: F) -> Result<Vec<T>>
where
    P: Params,
    F: FnMut(&Row<'_>) -> rusqlite::Result<T>,
{
    let mut statement = conn.prepare_cached(sql).db()?;
    let rows = statement.query_map(params, map).db()?;
    rows.collect::<rusqlite::Result<Vec<T>>>().db()
}

/// Runs a query and maps the first row, if there is one.
pub(crate) fn query_opt<T, P, F>(
    conn: &Connection,
    sql: &str,
    params: P,
    map: F,
) -> Result<Option<T>>
where
    P: Params,
    F: FnOnce(&Row<'_>) -> rusqlite::Result<T>,
{
    let mut statement = conn.prepare_cached(sql).db()?;
    statement.query_row(params, map).optional().db()
}

/// Runs a statement that returns no rows and reports how many rows it changed.
pub(crate) fn execute<P: Params>(conn: &Connection, sql: &str, params: P) -> Result<usize> {
    let mut statement = conn.prepare_cached(sql).db()?;
    statement.execute(params).db()
}

/// Reads a single count.
pub(crate) fn count(conn: &Connection, sql: &str) -> Result<u64> {
    let mut statement = conn.prepare_cached(sql).db()?;
    let value: i64 = statement.query_row([], |row| row.get(0)).db()?;
    Ok(u64::try_from(value).unwrap_or(0))
}

/// Drops an index and returns the statement that creates it again, read from the schema so the
/// two can never drift apart; `None` (and nothing dropped) if the index does not exist.
///
/// Bulk writers use it to leave an index out while filling an empty table and build it at the
/// end from sorted data, which is far cheaper than maintaining it row by row.
pub(crate) fn drop_index(conn: &Connection, name: &str) -> Result<Option<String>> {
    let sql: Option<String> = query_opt(
        conn,
        "SELECT sql FROM sqlite_schema WHERE type = 'index' AND name = ?1",
        rusqlite::params![name],
        |row| row.get(0),
    )?;
    if sql.is_some() {
        conn.execute_batch(&format!("DROP INDEX {name};")).db()?;
    }
    Ok(sql)
}
