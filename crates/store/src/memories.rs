// SPDX-License-Identifier: Apache-2.0
//! Memories: storing them anchored to symbols, finding them, keeping them honest.
//!
//! A memory is text written by an agent, a tool or a person, with a kind and a provenance. It is
//! anchored to symbols: for each one the store records the qualified name, the path and the two
//! hashes the symbol had *when the memory was written*. That record is what makes staleness
//! decidable later, without trusting the symbol to still exist: when a file is re-indexed the
//! `upsert` module compares the recorded hashes with the current ones (through
//! `Anchor::is_stale_against`) and marks the memory stale.
//!
//! # Anchors
//! `symbol_id` links an anchor to a live symbol. When the symbol disappears the link becomes
//! empty but the recorded name, path and hashes stay, so the memory can still say what it was
//! about. Finding memories for a symbol also follows an empty link by path and qualified name, so
//! a memory about a symbol that was removed and later came back is found again (still stale until
//! someone re-anchors it).
//!
//! # Search
//! `memory_fts` holds the memory text split like identifiers (`parseConfig` and "parse config"
//! meet). A search first requires every word; if that finds nothing, it accepts any word and
//! ranks by bm25, which suits questions written as sentences.
//!
//! Position in the architecture: the memory side of the storage adapter.

use pn_ultramemory_core::{
    Anchor, MemoryFilter, MemoryId, MemoryKind, MemoryRecord, NewMemory, StaleReason, StorageError,
    SymbolId,
};
use rusqlite::{Connection, TransactionBehavior, params};

use crate::convert::{limit_to_sql, memory_kind, provenance, u64_from_sql};
use crate::error::{DbResult, Result};
use crate::query::{execute, query_all, query_opt};
use crate::tokens::{Join, index_text, match_expression};

/// How many memories a listing returns when the filter does not say.
const DEFAULT_LIST_LIMIT: usize = 100;

/// The most bytes of memory text that are indexed for search.
const MAX_INDEXED_TEXT_BYTES: usize = 16 * 1024;

/// What a symbol looked like at the moment an anchor is written.
struct SymbolSnapshot {
    /// The symbol's identity.
    id: i64,
    /// Its qualified name.
    qualified_name: String,
    /// The path of its file.
    path: String,
    /// Its signature hash, as stored.
    sig_hash: i64,
    /// Its declaration hash, as stored.
    body_hash: i64,
}

/// Reads the snapshot of a symbol by id.
fn snapshot_by_id(conn: &Connection, id: i64) -> Result<Option<SymbolSnapshot>> {
    query_opt(
        conn,
        "SELECT s.id, s.qualified_name, f.path, s.sig_hash, s.body_hash \
         FROM symbols s JOIN files f ON f.id = s.file_id WHERE s.id = ?1",
        params![id],
        snapshot_from_row,
    )
}

/// Reads the snapshot of the first symbol with a qualified name in a file.
fn snapshot_by_name(
    conn: &Connection,
    path: &str,
    qualified_name: &str,
) -> Result<Option<SymbolSnapshot>> {
    query_opt(
        conn,
        "SELECT s.id, s.qualified_name, f.path, s.sig_hash, s.body_hash \
         FROM symbols s JOIN files f ON f.id = s.file_id \
         WHERE f.path = ?1 AND s.qualified_name = ?2 ORDER BY s.seq, s.id LIMIT 1",
        params![path, qualified_name],
        snapshot_from_row,
    )
}

/// Builds a snapshot from a row of `id, qualified_name, path, sig_hash, body_hash`.
fn snapshot_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SymbolSnapshot> {
    Ok(SymbolSnapshot {
        id: row.get(0)?,
        qualified_name: row.get(1)?,
        path: row.get(2)?,
        sig_hash: row.get(3)?,
        body_hash: row.get(4)?,
    })
}

/// Stores a memory and anchors it to the given symbols.
///
/// # Errors
/// [`StorageError::NotFound`] when a symbol in `about` does not exist (nothing is stored), and
/// [`StorageError::Backend`] when SQLite fails.
pub(crate) fn add_memory(conn: &mut Connection, new: &NewMemory, now: i64) -> Result<MemoryRecord> {
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .db()?;
    let mut snapshots: Vec<SymbolSnapshot> = Vec::new();
    for symbol in &new.about {
        if snapshots.iter().any(|s| s.id == symbol.0) {
            continue;
        }
        let snapshot = snapshot_by_id(&tx, symbol.0)?
            .ok_or_else(|| StorageError::NotFound(format!("symbol {symbol}")))?;
        snapshots.push(snapshot);
    }
    execute(
        &tx,
        "INSERT INTO memories (kind, text, provenance, created_at, stale_since) \
         VALUES (?1, ?2, ?3, ?4, NULL)",
        params![new.kind.as_str(), new.text, new.provenance.as_str(), now],
    )?;
    let id = tx.last_insert_rowid();
    let mut anchors = Vec::with_capacity(snapshots.len());
    for (ordinal, snapshot) in snapshots.iter().enumerate() {
        execute(
            &tx,
            "INSERT INTO memory_anchors \
                 (memory_id, ordinal, symbol_id, qualified_name, path, sig_hash, body_hash) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                id,
                i64::try_from(ordinal).unwrap_or(i64::MAX),
                snapshot.id,
                snapshot.qualified_name,
                snapshot.path,
                snapshot.sig_hash,
                snapshot.body_hash,
            ],
        )?;
        anchors.push(Anchor {
            symbol: Some(SymbolId(snapshot.id)),
            qualified_name: snapshot.qualified_name.clone(),
            path: snapshot.path.clone(),
            sig_hash: u64_from_sql(snapshot.sig_hash),
            body_hash: u64_from_sql(snapshot.body_hash),
        });
    }
    execute(
        &tx,
        "INSERT INTO memory_fts (rowid, text) VALUES (?1, ?2)",
        params![id, index_text(&new.text, MAX_INDEXED_TEXT_BYTES)],
    )?;
    tx.commit().db()?;
    Ok(MemoryRecord {
        id: MemoryId(id),
        kind: new.kind,
        text: new.text.clone(),
        provenance: new.provenance,
        created_at: now,
        stale_since: None,
        stale_reason: None,
        anchors,
    })
}

/// Loads one memory with its anchors.
fn load_memory(conn: &Connection, id: i64) -> Result<Option<MemoryRecord>> {
    let head = query_opt(
        conn,
        "SELECT kind, text, provenance, created_at, stale_since, stale_reason \
         FROM memories WHERE id = ?1",
        params![id],
        |row| {
            let kind: String = row.get(0)?;
            let source: String = row.get(2)?;
            let reason: Option<String> = row.get(5)?;
            Ok((
                memory_kind(&kind, 0)?,
                row.get::<_, String>(1)?,
                provenance(&source, 2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Option<i64>>(4)?,
                reason.and_then(|r| StaleReason::from_name(&r)),
            ))
        },
    )?;
    let Some((kind, text, provenance, created_at, stale_since, stale_reason)) = head else {
        return Ok(None);
    };
    let anchors = query_all(
        conn,
        "SELECT symbol_id, qualified_name, path, sig_hash, body_hash \
         FROM memory_anchors WHERE memory_id = ?1 ORDER BY ordinal",
        params![id],
        |row| {
            let symbol: Option<i64> = row.get(0)?;
            Ok(Anchor {
                symbol: symbol.map(SymbolId),
                qualified_name: row.get(1)?,
                path: row.get(2)?,
                sig_hash: u64_from_sql(row.get(3)?),
                body_hash: u64_from_sql(row.get(4)?),
            })
        },
    )?;
    Ok(Some(MemoryRecord {
        id: MemoryId(id),
        kind,
        text,
        provenance,
        created_at,
        stale_since,
        stale_reason,
        anchors,
    }))
}

/// Loads memories by id, keeping the order of `ids`.
fn load_memories(conn: &Connection, ids: &[i64]) -> Result<Vec<MemoryRecord>> {
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(memory) = load_memory(conn, *id)? {
            out.push(memory);
        }
    }
    Ok(out)
}

/// Memories anchored to any of the symbols, newest first.
pub(crate) fn memories_for_symbols(
    conn: &mut Connection,
    ids: &[SymbolId],
    limit: usize,
) -> Result<Vec<MemoryRecord>> {
    if limit == 0 || ids.is_empty() {
        return Ok(Vec::new());
    }
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Deferred)
        .db()?;
    execute(&tx, "DELETE FROM tmp_ids", [])?;
    for id in ids {
        execute(
            &tx,
            "INSERT OR IGNORE INTO tmp_ids (id) VALUES (?1)",
            params![id.0],
        )?;
    }
    let found = query_all(
        &tx,
        "SELECT m.id FROM memories m WHERE m.id IN ( \
             SELECT a.memory_id FROM memory_anchors a \
             WHERE a.symbol_id IN (SELECT id FROM tmp_ids) \
             UNION \
             SELECT a.memory_id FROM memory_anchors a \
             JOIN files f ON f.path = a.path \
             JOIN symbols s ON s.file_id = f.id AND s.qualified_name = a.qualified_name \
             WHERE a.symbol_id IS NULL AND s.id IN (SELECT id FROM tmp_ids)) \
         ORDER BY m.created_at DESC, m.id DESC LIMIT ?1",
        params![limit_to_sql(limit)],
        |row| row.get::<_, i64>(0),
    )?;
    let memories = load_memories(&tx, &found)?;
    execute(&tx, "DELETE FROM tmp_ids", [])?;
    tx.commit().db()?;
    Ok(memories)
}

/// Full-text search over memory text, best matches first.
pub(crate) fn search_memories(
    conn: &Connection,
    text: &str,
    limit: usize,
) -> Result<Vec<MemoryRecord>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let Some(all_words) = match_expression(text, Join::All) else {
        return Ok(Vec::new());
    };
    let any_word = match_expression(text, Join::Any).unwrap_or_else(|| all_words.clone());
    for expression in [all_words, any_word] {
        let ids = query_all(
            conn,
            "WITH hits (id, score) AS MATERIALIZED ( \
                 SELECT rowid, bm25(memory_fts) FROM memory_fts WHERE memory_fts MATCH ?1) \
             SELECT m.id FROM hits h JOIN memories m ON m.id = h.id \
             ORDER BY h.score ASC, m.created_at DESC, m.id DESC LIMIT ?2",
            params![expression, limit_to_sql(limit)],
            |row| row.get::<_, i64>(0),
        )?;
        if !ids.is_empty() {
            return load_memories(conn, &ids);
        }
    }
    Ok(Vec::new())
}

/// Lists memories, newest first.
pub(crate) fn list_memories(conn: &Connection, filter: &MemoryFilter) -> Result<Vec<MemoryRecord>> {
    let limit = if filter.limit == 0 {
        DEFAULT_LIST_LIMIT
    } else {
        filter.limit
    };
    let ids = query_all(
        conn,
        "SELECT id FROM memories \
         WHERE (?1 IS NULL OR kind = ?1) AND (?2 = 0 OR stale_since IS NOT NULL) \
         ORDER BY created_at DESC, id DESC LIMIT ?3",
        params![
            filter.kind.map(MemoryKind::as_str),
            i64::from(filter.only_stale),
            limit_to_sql(limit)
        ],
        |row| row.get::<_, i64>(0),
    )?;
    load_memories(conn, &ids)
}

/// Deletes a memory and what was learned about it. Returns `false` when it did not exist.
pub(crate) fn forget_memory(conn: &mut Connection, id: MemoryId) -> Result<bool> {
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .db()?;
    let removed = execute(&tx, "DELETE FROM memories WHERE id = ?1", params![id.0])?;
    if removed > 0 {
        execute(
            &tx,
            "DELETE FROM utility WHERE target_kind = 'memory' AND target_id = ?1",
            params![id.0],
        )?;
    }
    tx.commit().db()?;
    Ok(removed > 0)
}

/// Re-anchors a memory to the current hashes of its symbols and clears its stale mark.
///
/// An anchor whose symbol link is empty is looked up again by path and qualified name; an anchor
/// whose symbol is gone for good keeps its recorded values. Returns `false` when the memory does
/// not exist.
pub(crate) fn reanchor_memory(conn: &mut Connection, id: MemoryId) -> Result<bool> {
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .db()?;
    let exists = query_opt(
        &tx,
        "SELECT 1 FROM memories WHERE id = ?1",
        params![id.0],
        |row| row.get::<_, i64>(0),
    )?
    .is_some();
    if !exists {
        return Ok(false);
    }
    let anchors = query_all(
        &tx,
        "SELECT ordinal, symbol_id, qualified_name, path FROM memory_anchors \
         WHERE memory_id = ?1 ORDER BY ordinal",
        params![id.0],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        },
    )?;
    for (ordinal, symbol, qualified_name, path) in anchors {
        let mut current = match symbol {
            Some(symbol) => snapshot_by_id(&tx, symbol)?,
            None => None,
        };
        if current.is_none() {
            current = snapshot_by_name(&tx, &path, &qualified_name)?;
        }
        if let Some(now) = current {
            execute(
                &tx,
                "UPDATE memory_anchors SET symbol_id = ?1, qualified_name = ?2, path = ?3, \
                     sig_hash = ?4, body_hash = ?5 WHERE memory_id = ?6 AND ordinal = ?7",
                params![
                    now.id,
                    now.qualified_name,
                    now.path,
                    now.sig_hash,
                    now.body_hash,
                    id.0,
                    ordinal
                ],
            )?;
        }
    }
    execute(
        &tx,
        "UPDATE memories SET stale_since = NULL, stale_reason = NULL WHERE id = ?1",
        params![id.0],
    )?;
    tx.commit().db()?;
    Ok(true)
}

/// Persists a new draft memory suggested from observed agent behavior.
pub(crate) fn save_draft(
    conn: &mut Connection,
    draft: &pn_ultramemory_core::DraftMemory,
) -> Result<i64> {
    let about = draft.about_symbols.join(",");
    execute(
        conn,
        "INSERT INTO draft_memories (kind, text, about_symbols, suggested_at, source_query) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            draft.kind.as_str(),
            draft.text,
            about,
            draft.suggested_at,
            draft.source_query
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Lists pending drafts, newest first.
pub(crate) fn list_drafts(
    conn: &Connection,
    limit: usize,
) -> Result<Vec<pn_ultramemory_core::DraftMemory>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    query_all(
        conn,
        "SELECT id, kind, text, about_symbols, suggested_at, source_query \
         FROM draft_memories ORDER BY suggested_at DESC, id DESC LIMIT ?1",
        params![limit_to_sql(limit)],
        |row| {
            let id: i64 = row.get(0)?;
            let kind_str: String = row.get(1)?;
            let text: String = row.get(2)?;
            let about_raw: String = row.get(3)?;
            let suggested_at: i64 = row.get(4)?;
            let source_query: String = row.get(5)?;
            let kind = memory_kind(&kind_str, 1)?;
            let about_symbols: Vec<String> = if about_raw.is_empty() {
                Vec::new()
            } else {
                about_raw.split(',').map(|s| s.to_owned()).collect()
            };
            Ok(pn_ultramemory_core::DraftMemory {
                id,
                kind,
                text,
                about_symbols,
                suggested_at,
                source_query,
            })
        },
    )
}

/// Removes a draft after confirmation or discard. Returns `false` when it did not exist.
pub(crate) fn discard_draft(conn: &mut Connection, id: i64) -> Result<bool> {
    let removed = execute(conn, "DELETE FROM draft_memories WHERE id = ?1", params![id])?;
    Ok(removed > 0)
}
