// SPDX-License-Identifier: Apache-2.0
//! Full-text search over symbols: answering queries.
//!
//! # Index
//! `symbol_fts` is an FTS5 table with four columns: the name, the qualified name, the signature
//! and the documentation of a symbol, each already split into lowercase word parts by
//! [`crate::tokens`]. It is contentless (only the index is stored) and keyed by the symbol id as
//! its rowid; rows are written by the `upsert` module and removed by a database trigger when the
//! symbol is deleted.
//!
//! # Ranking
//! `bm25` with column weights 10 (name), 5 (qualified name), 2 (signature) and 1 (documentation),
//! so a match in a name outweighs one in a signature, which outweighs one in prose. FTS5 reports
//! bm25 as a negative number where smaller is better; the score returned to callers is its
//! negation, so higher is better. Ties break by path, start line and id, so the order is total.
//!
//! # Safety
//! The query text is rebuilt from word parts (see [`crate::tokens::match_expression`]); kind
//! filters and path bounds are bound parameters. Nothing a caller types reaches SQL or the FTS5
//! query language as syntax.

use pn_ultramemory_core::{SearchHit, SearchQuery, SymbolKind};
use rusqlite::types::Value;
use rusqlite::{Connection, params_from_iter};

use crate::convert::limit_to_sql;
use crate::error::Result;
use crate::query::query_all;
use crate::symbols::{path_bounds, symbol_columns, symbol_from_row};
use crate::tokens::{Join, match_expression};

/// Weight of a match in the name column.
const WEIGHT_NAME: f64 = 10.0;

/// Weight of a match in the qualified name column.
const WEIGHT_QUALIFIED: f64 = 5.0;

/// Weight of a match in the signature column.
const WEIGHT_SIGNATURE: f64 = 2.0;

/// Weight of a match in the documentation column.
const WEIGHT_DOC: f64 = 1.0;

/// Searches symbols. Every word of the query must match somewhere in the name, qualified name,
/// signature or documentation (or any word, with [`SearchQuery::any_word`]), and the last word
/// also matches by prefix.
///
/// A query without any word, and a limit of zero, give an empty result.
pub(crate) fn search_symbols(
    conn: &Connection,
    query: &SearchQuery,
    limit: usize,
) -> Result<Vec<SearchHit>> {
    let join = if query.any_word { Join::Any } else { Join::All };
    let Some(expression) = match_expression(&query.text, join) else {
        return Ok(Vec::new());
    };
    if limit == 0 {
        return Ok(Vec::new());
    }
    let mut kinds: Vec<SymbolKind> = Vec::new();
    for kind in &query.kinds {
        if !kinds.contains(kind) {
            kinds.push(*kind);
        }
    }
    let (lower, upper) = path_bounds(query.path_prefix.as_deref());

    let mut values: Vec<Value> = vec![
        Value::Text(expression),
        lower.map_or(Value::Null, Value::Text),
        upper.map_or(Value::Null, Value::Text),
        Value::Integer(limit_to_sql(limit)),
    ];
    let mut filter = String::new();
    if !kinds.is_empty() {
        let first = values.len() + 1;
        let list: Vec<String> = (first..first + kinds.len())
            .map(|n| format!("?{n}"))
            .collect();
        filter = format!(" AND s.kind IN ({})", list.join(","));
        values.extend(
            kinds
                .iter()
                .map(|kind| Value::Text(kind.as_str().to_owned())),
        );
    }
    let sql = format!(
        "WITH hits (id, score) AS MATERIALIZED (\
             SELECT rowid, bm25(symbol_fts, {WEIGHT_NAME:?}, {WEIGHT_QUALIFIED:?}, \
                                {WEIGHT_SIGNATURE:?}, {WEIGHT_DOC:?}) \
             FROM symbol_fts WHERE symbol_fts MATCH ?1) \
         {HITS_SELECT}\
         WHERE (?2 IS NULL OR f.path >= ?2) AND (?3 IS NULL OR f.path < ?3){filter} \
         ORDER BY h.score ASC, f.path ASC, s.start_line ASC, s.id ASC LIMIT ?4",
    );
    query_all(conn, &sql, params_from_iter(values.iter()), |row| {
        let symbol = symbol_from_row(row)?;
        // symbol_columns! now includes pagerank as column 18, so bm25 shifts to 19.
        let bm25: f64 = row.get(19)?;
        Ok(SearchHit {
            symbol,
            score: -bm25,
        })
    })
}

/// The `SELECT` and joins of a search, reading the CTE named `hits`.
const HITS_SELECT: &str = concat!(
    "SELECT ",
    symbol_columns!(),
    ", h.score FROM hits h JOIN symbols s ON s.id = h.id JOIN files f ON f.id = s.file_id "
);

#[cfg(test)]
mod tests {
    use super::{WEIGHT_DOC, WEIGHT_NAME, WEIGHT_QUALIFIED, WEIGHT_SIGNATURE};

    /// The weights keep the documented order: name > qualified name > signature > doc.
    #[test]
    fn weights_are_ordered() {
        const {
            assert!(WEIGHT_NAME > WEIGHT_QUALIFIED);
            assert!(WEIGHT_QUALIFIED > WEIGHT_SIGNATURE);
            assert!(WEIGHT_SIGNATURE > WEIGHT_DOC);
        }
    }
}
