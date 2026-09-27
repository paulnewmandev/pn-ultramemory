// SPDX-License-Identifier: Apache-2.0
//! Aggregates for reports: sizes and connections of modules, documentation coverage, and file
//! totals.
//!
//! Everything here is computed by the database in a handful of set-based statements, never by
//! walking symbols or edges one at a time from Rust.
//!
//! # Modules
//! A *module* is the directory prefix of a file's path, `depth` directories deep: at depth 2
//! `crates/core/src/lib.rs` belongs to `crates/core`. A file at the root of the repository belongs
//! to the module `.`, a path with fewer directories than `depth` uses all of them, and depth 0
//! puts every file in `.`. Module names are worked out for all files by one recursive query and
//! kept in a scratch table for the length of the call.
//!
//! Edges carry the files at both their ends, so they are summed per ordered pair of modules in a
//! second scratch table with one pass over the edges; an edge that stays
//! inside one module is not counted anywhere. Both module reports read from that pair table.
//!
//! Position in the architecture: the reporting side of the storage adapter.

use pn_ultramemory_core::{Confidence, DocCoverageRow, FileTotals, ModuleEdge, ModuleStats};
use rusqlite::{Connection, TransactionBehavior, params};

use crate::convert::{confidence_to_sql, limit_to_sql};
use crate::error::{DbResult, Result};
use crate::query::{count, execute, query_all};

/// The most directories a module can be deep; more than any path has, and it keeps the depth a
/// caller passes inside an `i64`.
const MAX_DEPTH: usize = 1 << 20;

/// Works out the module of every file for the given depth, and numbers the distinct modules in
/// name order.
///
/// The recursive query walks each path from one `/` to the next, at most `depth` times, and keeps
/// the last position reached; the module is the path up to just before it.
fn fill_modules(tx: &Connection, depth: usize) -> Result<()> {
    clear(tx)?;
    let depth = i64::try_from(depth.min(MAX_DEPTH)).unwrap_or(0);
    execute(
        tx,
        "INSERT INTO tmp_modules (file_id, module) \
         WITH RECURSIVE cut (id, path, n, pos) AS ( \
             SELECT id, path, 0, 0 FROM files \
             UNION ALL \
             SELECT id, path, n + 1, pos + instr(substr(path, pos + 1), '/') FROM cut \
             WHERE n < ?1 AND instr(substr(path, pos + 1), '/') > 0) \
         SELECT id, COALESCE(NULLIF(CASE WHEN pos > 0 THEN substr(path, 1, pos - 1) \
                                         ELSE '' END, ''), '.') \
         FROM (SELECT id, path, MAX(n), pos FROM cut GROUP BY id)",
        params![depth],
    )?;
    execute(
        tx,
        "INSERT INTO tmp_module_names (name) SELECT DISTINCT module FROM tmp_modules ORDER BY module",
        [],
    )?;
    execute(
        tx,
        "UPDATE tmp_modules SET mid = (SELECT mid FROM tmp_module_names n \
                                       WHERE n.name = tmp_modules.module)",
        [],
    )?;
    Ok(())
}

/// Sums the edges that go from one module to another, per ordered pair, counting only edges at
/// least as confident as `min_confidence`. The files of both ends are stored in the edge, so
/// this is one pass over the edges and two lookups in a small table per edge.
fn fill_pairs(tx: &Connection, min_confidence: Confidence) -> Result<()> {
    execute(
        tx,
        "INSERT INTO tmp_pairs (from_mid, to_mid, weight) \
         SELECT a.mid, b.mid, COUNT(*) \
         FROM edges e \
         JOIN tmp_modules a ON a.file_id = e.src_file \
         JOIN tmp_modules b ON b.file_id = e.dst_file \
         WHERE e.confidence >= ?1 AND a.mid <> b.mid \
         GROUP BY a.mid, b.mid",
        params![confidence_to_sql(min_confidence)],
    )?;
    Ok(())
}

/// Empties the scratch tables.
fn clear(tx: &Connection) -> Result<()> {
    execute(tx, "DELETE FROM tmp_modules", [])?;
    execute(tx, "DELETE FROM tmp_module_names", [])?;
    execute(tx, "DELETE FROM tmp_pairs", [])?;
    Ok(())
}

/// Reads a non-negative count from a row.
fn unsigned(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

/// Size and boundary-crossing edges of every module, most symbols first.
pub(crate) fn module_stats(conn: &mut Connection, depth: usize) -> Result<Vec<ModuleStats>> {
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Deferred)
        .db()?;
    fill_modules(&tx, depth)?;
    fill_pairs(&tx, Confidence::Heuristic)?;
    let stats = query_all(
        &tx,
        "SELECT n.name, COUNT(*) AS files, SUM(f.symbol_count) AS symbols, \
                COALESCE(i.weight, 0), COALESCE(o.weight, 0) \
         FROM tmp_modules m \
         JOIN tmp_module_names n ON n.mid = m.mid \
         JOIN files f ON f.id = m.file_id \
         LEFT JOIN (SELECT to_mid AS mid, SUM(weight) AS weight FROM tmp_pairs \
                    GROUP BY to_mid) i ON i.mid = m.mid \
         LEFT JOIN (SELECT from_mid AS mid, SUM(weight) AS weight FROM tmp_pairs \
                    GROUP BY from_mid) o ON o.mid = m.mid \
         GROUP BY m.mid \
         ORDER BY symbols DESC, n.name",
        [],
        |row| {
            Ok(ModuleStats {
                name: row.get(0)?,
                files: unsigned(row.get(1)?),
                symbols: unsigned(row.get(2)?),
                incoming: unsigned(row.get(3)?),
                outgoing: unsigned(row.get(4)?),
            })
        },
    )?;
    clear(&tx)?;
    tx.commit().db()?;
    Ok(stats)
}

/// The heaviest edges between different modules.
pub(crate) fn module_edges(
    conn: &mut Connection,
    depth: usize,
    min_confidence: Confidence,
    limit: usize,
) -> Result<Vec<ModuleEdge>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Deferred)
        .db()?;
    fill_modules(&tx, depth)?;
    fill_pairs(&tx, min_confidence)?;
    let edges = query_all(
        &tx,
        "SELECT f.name, t.name, p.weight FROM tmp_pairs p \
         JOIN tmp_module_names f ON f.mid = p.from_mid \
         JOIN tmp_module_names t ON t.mid = p.to_mid \
         ORDER BY p.weight DESC, f.name, t.name LIMIT ?1",
        params![limit_to_sql(limit)],
        |row| {
            Ok(ModuleEdge {
                from: row.get(0)?,
                to: row.get(1)?,
                weight: unsigned(row.get(2)?),
            })
        },
    )?;
    clear(&tx)?;
    tx.commit().db()?;
    Ok(edges)
}

/// Public symbols per language and how many of them are documented. Modules are not counted,
/// and documentation that is only whitespace counts as missing, as in `undocumented_public`.
pub(crate) fn doc_coverage(conn: &Connection) -> Result<Vec<DocCoverageRow>> {
    query_all(
        conn,
        "SELECT f.language, COUNT(*), \
                COALESCE(SUM(s.doc IS NOT NULL AND trim(s.doc, ' \t\r\n') <> ''), 0) \
         FROM symbols s JOIN files f ON f.id = s.file_id \
         WHERE s.visibility = 'public' AND s.kind <> 'module' \
         GROUP BY f.language \
         ORDER BY COUNT(*) DESC, f.language",
        [],
        |row| {
            Ok(DocCoverageRow {
                language: row.get(0)?,
                public_symbols: unsigned(row.get(1)?),
                documented: unsigned(row.get(2)?),
            })
        },
    )
}

/// Total lines, and how many files had syntax errors when they were parsed.
pub(crate) fn file_totals(conn: &Connection) -> Result<FileTotals> {
    Ok(FileTotals {
        lines: count(conn, "SELECT COALESCE(SUM(lines), 0) FROM files")?,
        parse_error_files: count(conn, "SELECT COUNT(*) FROM files WHERE parse_errors > 0")?,
    })
}
