// SPDX-License-Identifier: Apache-2.0
//! Reading symbols back: by identity, by file, by name, by centrality and by missing
//! documentation.
//!
//! Every read query selects the same 18 columns, in the order [`symbol_from_row`] expects, so a
//! [`SymbolRecord`] is always built the same way. Queries that join further columns (a score, an
//! edge) append them after those 18.
//!
//! # Ordering
//! Every list has a total order that ends in the symbol id, so results never depend on the
//! storage order of rows or on hash-map iteration.
//!
//! Position in the architecture: the read side of the storage adapter for symbols. Writing them
//! is the `upsert` module, and text search is the `search` module.

use pn_ultramemory_core::{Confidence, FileId, Span, SymbolId, SymbolRecord};
use rusqlite::{Connection, Row, params};

use crate::convert::{
    confidence_to_sql, decode_names, language, limit_to_sql, prefix_upper_bound, symbol_kind,
    u64_from_sql, visibility,
};
use crate::error::Result;
use crate::query::{query_all, query_opt};

/// The columns of a symbol joined with its file, in the order [`symbol_from_row`] reads them.
macro_rules! symbol_columns {
    () => {
        "s.id, s.file_id, f.path, f.language, s.name, s.qualified_name, s.kind, s.signature, \
         s.doc, s.visibility, s.start_line, s.end_line, s.start_byte, s.end_byte, s.parent_id, \
         s.outline, s.sig_hash, s.body_hash"
    };
}

/// A constant `SELECT` of the symbol columns from `symbols s` joined with `files f`, followed by
/// the given clauses.
macro_rules! symbol_select {
    ($($tail:literal),+ $(,)?) => {
        concat!(
            "SELECT ",
            $crate::symbols::symbol_columns!(),
            " FROM symbols s JOIN files f ON f.id = s.file_id ",
            $($tail),+
        )
    };
}

pub(crate) use symbol_columns;

/// Builds a [`SymbolRecord`] from a row that starts with [`symbol_columns!`].
pub(crate) fn symbol_from_row(row: &Row<'_>) -> rusqlite::Result<SymbolRecord> {
    let language_name: String = row.get(3)?;
    let kind_name: String = row.get(6)?;
    let visibility_name: String = row.get(9)?;
    let outline: String = row.get(15)?;
    let parent: Option<i64> = row.get(14)?;
    Ok(SymbolRecord {
        id: SymbolId(row.get(0)?),
        file_id: FileId(row.get(1)?),
        path: row.get(2)?,
        language: language(&language_name, 3)?,
        name: row.get(4)?,
        qualified_name: row.get(5)?,
        kind: symbol_kind(&kind_name, 6)?,
        signature: row.get(7)?,
        doc: row.get(8)?,
        visibility: visibility(&visibility_name, 9)?,
        span: Span {
            start_line: row.get(10)?,
            end_line: row.get(11)?,
            start_byte: row.get(12)?,
            end_byte: row.get(13)?,
        },
        parent: parent.map(SymbolId),
        outline: decode_names(&outline),
        sig_hash: u64_from_sql(row.get(16)?),
        body_hash: u64_from_sql(row.get(17)?),
    })
}

/// Fetches one symbol by identity.
pub(crate) fn symbol(conn: &Connection, id: SymbolId) -> Result<Option<SymbolRecord>> {
    query_opt(
        conn,
        symbol_select!("WHERE s.id = ?1"),
        params![id.0],
        symbol_from_row,
    )
}

/// Lists the symbols of one file in the extractor's source order.
pub(crate) fn symbols_in_file(conn: &Connection, path: &str) -> Result<Vec<SymbolRecord>> {
    query_all(
        conn,
        symbol_select!("WHERE f.path = ?1 ORDER BY s.seq, s.id"),
        params![path],
        symbol_from_row,
    )
}

/// Finds symbols whose simple or qualified name equals `name`: exact matches first, then those
/// that differ only in (ASCII) case, each group ordered by path, line and id.
pub(crate) fn find_symbols(
    conn: &Connection,
    name: &str,
    limit: usize,
) -> Result<Vec<SymbolRecord>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    query_all(
        conn,
        symbol_select!(
            "WHERE s.name = ?1 COLLATE NOCASE OR s.qualified_name = ?1 COLLATE NOCASE ",
            "ORDER BY (s.name = ?1 OR s.qualified_name = ?1) DESC, f.path, s.start_line, s.id ",
            "LIMIT ?2"
        ),
        params![name, limit_to_sql(limit)],
        symbol_from_row,
    )
}

/// The path range `[lower, upper)` that selects the paths starting with `prefix`, as SQL
/// parameters: both `NULL` when there is no prefix, and only the upper bound `NULL` when no
/// string sorts after the prefix.
pub(crate) fn path_bounds(prefix: Option<&str>) -> (Option<String>, Option<String>) {
    match prefix {
        None | Some("") => (None, None),
        Some(prefix) => (Some(prefix.to_owned()), prefix_upper_bound(prefix)),
    }
}

/// The most referenced symbols with their in-degree, counting edges that are at least
/// heuristic. Ties break by path, then simple name, then id.
pub(crate) fn central_symbols(
    conn: &Connection,
    limit: usize,
    path_prefix: Option<&str>,
) -> Result<Vec<(SymbolRecord, u32)>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let (lower, upper) = path_bounds(path_prefix);
    let heuristic = confidence_to_sql(Confidence::Heuristic);
    query_all(
        conn,
        concat!(
            "SELECT ",
            symbol_columns!(),
            ", d.degree FROM (SELECT dst, COUNT(*) AS degree FROM edges ",
            "WHERE confidence >= ?1 GROUP BY dst) d ",
            "JOIN symbols s ON s.id = d.dst JOIN files f ON f.id = s.file_id ",
            "WHERE (?2 IS NULL OR f.path >= ?2) AND (?3 IS NULL OR f.path < ?3) ",
            "ORDER BY d.degree DESC, f.path, s.name, s.id LIMIT ?4"
        ),
        params![heuristic, lower, upper, limit_to_sql(limit)],
        |row| {
            let record = symbol_from_row(row)?;
            let degree: i64 = row.get(18)?;
            Ok((record, u32::try_from(degree).unwrap_or(u32::MAX)))
        },
    )
}

/// Public symbols without documentation, ordered by path and line. Modules are skipped, and a
/// documentation text that is only whitespace counts as missing.
pub(crate) fn undocumented_public(
    conn: &Connection,
    limit: usize,
    path_prefix: Option<&str>,
) -> Result<Vec<SymbolRecord>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let (lower, upper) = path_bounds(path_prefix);
    query_all(
        conn,
        symbol_select!(
            "WHERE s.visibility = 'public' AND s.kind <> 'module' ",
            "AND (s.doc IS NULL OR trim(s.doc, ' \t\r\n') = '') ",
            "AND (?1 IS NULL OR f.path >= ?1) AND (?2 IS NULL OR f.path < ?2) ",
            "ORDER BY f.path, s.start_line, s.id LIMIT ?3"
        ),
        params![lower, upper, limit_to_sql(limit)],
        symbol_from_row,
    )
}
