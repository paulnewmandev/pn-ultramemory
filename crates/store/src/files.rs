// SPDX-License-Identifier: Apache-2.0
//! Files: looking them up, listing them, counting the index and removing the ones that are gone.
//!
//! Writing a file together with its symbols and references is the job of the `upsert` module;
//! this module holds the rest of the file-level operations of the storage port.
//!
//! # Removal
//! [`remove_files_not_in`] deletes every file whose path is not in the list of present paths.
//! The database cascades the deletion to the file's symbols and references, and a trigger removes
//! the edges and the full-text rows of every deleted symbol. Before that, memories anchored in a removed file are marked
//! stale (a removed symbol is stale by definition, see `Anchor::is_stale_against`), and the names
//! of the removed symbols are recorded as dirty so the next `resolve_edges` revisits every
//! reference to them.

use pn_ultramemory_core::{FileId, FileRecord, IndexStats};
use rusqlite::{Connection, TransactionBehavior, params};

use crate::convert::language;
use crate::error::{DbResult, Result};
use crate::query::{count, execute, query_all, query_opt};

/// Returns the stored content hash of a file.
pub(crate) fn file_hash(conn: &Connection, path: &str) -> Result<Option<String>> {
    query_opt(
        conn,
        "SELECT hash FROM files WHERE path = ?1",
        params![path],
        |row| row.get(0),
    )
}

/// Lists the indexed files ordered by path.
pub(crate) fn list_files(conn: &Connection) -> Result<Vec<FileRecord>> {
    query_all(
        conn,
        "SELECT id, path, language, size, lines, symbol_count FROM files ORDER BY path",
        [],
        |row| {
            let language_name: String = row.get(2)?;
            let size: i64 = row.get(3)?;
            Ok(FileRecord {
                id: FileId(row.get(0)?),
                path: row.get(1)?,
                language: language(&language_name, 2)?,
                size: u64::try_from(size).unwrap_or(0),
                lines: row.get(4)?,
                symbol_count: row.get(5)?,
            })
        },
    )
}

/// Counts what the index holds.
pub(crate) fn stats(conn: &Connection) -> Result<IndexStats> {
    let languages = query_all(
        conn,
        "SELECT language, COUNT(*) FROM files GROUP BY language ORDER BY COUNT(*) DESC, language",
        [],
        |row| {
            let count: i64 = row.get(1)?;
            Ok((row.get::<_, String>(0)?, u64::try_from(count).unwrap_or(0)))
        },
    )?;
    Ok(IndexStats {
        files: count(conn, "SELECT COUNT(*) FROM files")?,
        symbols: count(conn, "SELECT COUNT(*) FROM symbols")?,
        edges: count(conn, "SELECT COUNT(*) FROM edges")?,
        memories: count(conn, "SELECT COUNT(*) FROM memories")?,
        stale_memories: count(
            conn,
            "SELECT COUNT(*) FROM memories WHERE stale_since IS NOT NULL",
        )?,
        languages,
    })
}

/// Removes every file whose path is not in `present`, and returns how many were removed.
///
/// An empty `present` removes every file.
pub(crate) fn remove_files_not_in(
    conn: &mut Connection,
    present: &[String],
    now: i64,
) -> Result<u32> {
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .db()?;
    execute(&tx, "DELETE FROM tmp_paths", [])?;
    execute(&tx, "DELETE FROM tmp_removed", [])?;
    for path in present {
        execute(
            &tx,
            "INSERT OR IGNORE INTO tmp_paths (path) VALUES (?1)",
            params![path],
        )?;
    }
    let removed = execute(
        &tx,
        "INSERT INTO tmp_removed (file_id) \
         SELECT id FROM files WHERE path NOT IN (SELECT path FROM tmp_paths)",
        [],
    )?;
    if removed > 0 {
        execute(
            &tx,
            "INSERT OR IGNORE INTO dirty_names (name, family) \
             SELECT DISTINCT s.name, f.family \
             FROM symbols s JOIN files f ON f.id = s.file_id \
             WHERE f.id IN (SELECT file_id FROM tmp_removed)",
            [],
        )?;
        execute(
            &tx,
            "UPDATE memories SET stale_since = ?1, stale_reason = 'file_removed' \
             WHERE stale_since IS NULL AND id IN ( \
                 SELECT a.memory_id FROM memory_anchors a \
                 WHERE a.path IN (SELECT path FROM files \
                                  WHERE id IN (SELECT file_id FROM tmp_removed)))",
            params![now],
        )?;
        execute(
            &tx,
            "DELETE FROM files WHERE id IN (SELECT file_id FROM tmp_removed)",
            [],
        )?;
    }
    execute(&tx, "DELETE FROM tmp_paths", [])?;
    execute(&tx, "DELETE FROM tmp_removed", [])?;
    tx.commit().db()?;
    Ok(u32::try_from(removed).unwrap_or(u32::MAX))
}
