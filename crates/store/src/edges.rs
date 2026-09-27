// SPDX-License-Identifier: Apache-2.0
//! Turning stored references into edges, and following edges.
//!
//! # Resolution rules
//! A reference is a name written inside a symbol (its *owner*), before anyone knows which symbol
//! it means. Resolution links the owner to candidate symbols that have that name:
//!
//! Only candidates in files of the **same language family** (`Language::family`) count at every
//! tier: a TypeScript reference never links to a C# class of the same name, JavaScript, TypeScript
//! and TSX see each other, and so do C and C++. "One symbol in the whole index" means one symbol
//! in the reference's family.
//!
//! | Situation | Edges written | Confidence |
//! |---|---|---|
//! | A symbol of the **same file** has the name | to those symbols, at most 4, nearest line first | `Resolved` |
//! | Else exactly **one** symbol in the whole index has the name | to it | `Heuristic` |
//! | Else **several** do | to at most 4, whose path shares the longest prefix with the referencing file | `Guess` |
//! | Else none | none | none |
//!
//! Further rules:
//! * A reference kind maps to an edge kind: call to `calls`, type mention to `uses`, inheritance
//!   to `inherits`.
//! * A reference at file level (no owner) has nothing to attach an edge to and produces none.
//! * A symbol is never linked to itself, unless the reference is unqualified or written on the
//!   symbol itself (`self`, `this`, `Self`, `cls`, `$this`, `static`), which is recursion.
//! * Several references from one owner to one target collapse into one edge (the edge is unique
//!   per source, target and kind) that remembers the first line.
//!
//! # How it is computed
//! The work is set-based. The references in scope are copied into a scratch table, flagged in
//! bulk (which have a same-file match, how many candidates each name has) and turned into edges
//! by three `INSERT ... SELECT` statements, one per confidence tier. Only the "longest shared path
//! prefix" ranking of ambiguous names is done in Rust, once per distinct (name, referencing
//! file) pair, never once per reference.
//!
//! # Scope
//! `All` deletes every edge and resolves every reference. `Touching` resolves the references of
//! the given files, the references to the given names, and the references to every name recorded
//! as dirty (a symbol with that name was added or removed since the last resolve in that
//! language family, see `upsert` and `files`; a definition changing in one family does not touch
//! the references of another). That is enough: what a reference means depends only on the set of symbols that
//! carry its name, and on its own file. Edges into a symbol that survives an upsert are kept, so
//! nothing else needs redoing. Resolution is a pure function of the stored references and
//! symbols, so `Touching` after a change yields the same edges as `All`; the randomized
//! reference tests check exactly that, including edits that only modify symbols.

use pn_ultramemory_core::{
    Confidence, Direction, EdgeKind, EdgeRecord, Neighbor, RefKind, ResolveScope, ResolveStats,
    SymbolId,
};
use rusqlite::{Connection, TransactionBehavior, params};

use crate::convert::{confidence_from_sql, confidence_to_sql, edge_kind, limit_to_sql};
use crate::error::{DbResult, Result};
use crate::query::{drop_index, execute, query_all};
use crate::symbols::{symbol_columns, symbol_from_row};

/// The most edges one reference can produce.
const MAX_LINKS: usize = 4;

/// How many ranked candidates are kept per (name, file): one more than [`MAX_LINKS`], so that the
/// owner can be skipped when it is itself a candidate.
const PICK_DEPTH: usize = MAX_LINKS + 1;

/// Qualifiers that mean "this very symbol", which keep a self-link possible.
const SELF_QUALIFIERS: &[&str] = &["self", "this", "Self", "cls", "$this", "static"];

/// Whether a reference must not link its owner to itself: the owner carries the referenced name
/// (so name matching would find it) but the reference is qualified by something that is not
/// "this symbol", so it is a call to some other thing of that name.
pub(crate) fn must_not_link_to_self(
    owner_name: &str,
    reference_name: &str,
    qualifier: Option<&str>,
) -> bool {
    owner_name == reference_name
        && qualifier.is_some_and(|qualifier| !SELF_QUALIFIERS.contains(&qualifier))
}

/// Conflict clause shared by every edge insert: keep the strongest confidence and first line.
const ON_CONFLICT: &str = "ON CONFLICT (src, dst, kind) DO UPDATE SET \
     confidence = MAX(edges.confidence, excluded.confidence), \
     line = MIN(edges.line, excluded.line)";

/// Empties every scratch table used by resolution.
fn clear_scratch(conn: &Connection) -> Result<()> {
    for table in [
        "tmp_scope_files",
        "tmp_scope_names",
        "tmp_scope_dirty",
        "tmp_refs",
        "tmp_names",
        "tmp_pick",
    ] {
        execute(conn, &format!("DELETE FROM {table}"), [])?;
    }
    Ok(())
}

/// The SQL expression mapping a stored reference kind (column `r.kind`) to an edge kind name.
fn edge_kind_case() -> String {
    format!(
        "CASE r.kind WHEN '{}' THEN '{}' WHEN '{}' THEN '{}' WHEN '{}' THEN '{}' ELSE '{}' END",
        RefKind::Call.as_str(),
        EdgeKind::Calls.as_str(),
        RefKind::Type.as_str(),
        EdgeKind::Uses.as_str(),
        RefKind::Inherit.as_str(),
        EdgeKind::Inherits.as_str(),
        EdgeKind::Uses.as_str(),
    )
}

/// The statement that copies references into `tmp_refs`, ready for a `WHERE` clause, with
/// everything the tiers need worked out on the way.
///
/// References without an owner are left out. `excl` is the reference's `no_self` flag and
/// `family` the language family of its file. `nsame` counts the candidates in the reference's
/// file and `cnt` those in the whole family (from `tmp_names`); both leave out the owner when it
/// is excluded.
fn load_refs_sql(filter: &str) -> String {
    format!(
        "INSERT OR IGNORE INTO tmp_refs \
             (rid, file_id, owner, name, kind, line, excl, family, nsame, cnt) \
         SELECT j.rid, j.file_id, j.owner, j.name, j.kind, j.line, j.excl, j.family, \
                (SELECT COUNT(*) FROM symbols s \
                 WHERE s.name = j.name AND s.file_id = j.file_id) - j.excl, \
                COALESCE((SELECT n.cnt FROM tmp_names n \
                          WHERE n.name = j.name AND n.family = j.family), 0) - j.excl \
         FROM (SELECT r.id AS rid, r.file_id AS file_id, r.owner_id AS owner, r.name AS name, \
                      {kind} AS kind, r.line AS line, r.no_self AS excl, rf.family AS family \
               FROM refs r JOIN files rf ON rf.id = r.file_id \
               WHERE r.owner_id IS NOT NULL {filter}) j",
        kind = edge_kind_case(),
    )
}

/// Resolves references into edges for the given scope.
///
/// # Errors
/// [`pn_ultramemory_core::StorageError::Backend`] when SQLite fails; nothing is changed then.
pub(crate) fn resolve_edges(conn: &mut Connection, scope: &ResolveScope) -> Result<ResolveStats> {
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .db()?;
    let written = resolve_in_tx(&tx, scope)?;
    tx.commit().db()?;
    Ok(ResolveStats {
        edges_written: written,
    })
}

/// Does the work of [`resolve_edges`] inside an open transaction.
fn resolve_in_tx(tx: &Connection, scope: &ResolveScope) -> Result<u64> {
    clear_scratch(tx)?;
    match scope {
        ResolveScope::All => {
            execute(tx, "DELETE FROM edges", [])?;
            execute(
                tx,
                "INSERT INTO tmp_names (name, family, cnt) \
                 SELECT s.name, f.family, COUNT(*) \
                 FROM symbols s JOIN files f ON f.id = s.file_id \
                 GROUP BY s.name, f.family",
                [],
            )?;
            execute(tx, &load_refs_sql(""), [])?;
        }
        ResolveScope::Touching { file_ids, names } => {
            fill_scope(tx, file_ids, names)?;
            execute(
                tx,
                "DELETE FROM edges WHERE src IN ( \
                     SELECT id FROM symbols WHERE file_id IN (SELECT file_id FROM tmp_scope_files))",
                [],
            )?;
            // Edges into a symbol whose name was asked for (any family), or whose name changed
            // definition in the symbol's own family.
            execute(
                tx,
                "DELETE FROM edges WHERE dst IN ( \
                     SELECT id FROM symbols WHERE name IN (SELECT name FROM tmp_scope_names))",
                [],
            )?;
            execute(
                tx,
                "DELETE FROM edges WHERE dst IN ( \
                     SELECT s.id FROM symbols s JOIN files f ON f.id = s.file_id \
                     JOIN tmp_scope_dirty d ON d.name = s.name AND d.family = f.family)",
                [],
            )?;
            // How many symbols of each family carry each name that a reference in scope mentions.
            execute(
                tx,
                "INSERT INTO tmp_names (name, family, cnt) \
                 SELECT s.name, f.family, COUNT(*) \
                 FROM symbols s JOIN files f ON f.id = s.file_id \
                 WHERE s.name IN ( \
                     SELECT name FROM refs WHERE file_id IN (SELECT file_id FROM tmp_scope_files) \
                     UNION SELECT name FROM tmp_scope_names \
                     UNION SELECT name FROM tmp_scope_dirty) \
                 GROUP BY s.name, f.family",
                [],
            )?;
            execute(
                tx,
                &load_refs_sql("AND r.file_id IN (SELECT file_id FROM tmp_scope_files)"),
                [],
            )?;
            execute(
                tx,
                &load_refs_sql("AND r.name IN (SELECT name FROM tmp_scope_names)"),
                [],
            )?;
            execute(
                tx,
                &load_refs_sql(
                    "AND r.name IN (SELECT name FROM tmp_scope_dirty) \
                     AND EXISTS (SELECT 1 FROM tmp_scope_dirty d \
                                 WHERE d.name = r.name AND d.family = rf.family)",
                ),
                [],
            )?;
        }
    }
    // A big resolve writes edges into an empty table; its by-destination index is built at the
    // end, from sorted data, instead of being updated row by row.
    let rebuild = if matches!(scope, ResolveScope::All) {
        drop_index(tx, "idx_edges_dst")?
    } else {
        None
    };
    let mut written = 0_u64;
    written += link_same_file(tx)?;
    written += link_unique(tx)?;
    written += link_ambiguous(tx)?;
    if let Some(sql) = rebuild {
        tx.execute_batch(&sql).db()?;
    }
    execute(tx, "DELETE FROM dirty_names", [])?;
    clear_scratch(tx)?;
    Ok(written)
}

/// Fills the scope tables of a `Touching` resolve: the files, the names that were given (every
/// family), and the names recorded as dirty (each in its own family).
fn fill_scope(
    tx: &Connection,
    file_ids: &[pn_ultramemory_core::FileId],
    names: &[String],
) -> Result<()> {
    for file in file_ids {
        execute(
            tx,
            "INSERT OR IGNORE INTO tmp_scope_files (file_id) VALUES (?1)",
            params![file.0],
        )?;
    }
    for name in names {
        execute(
            tx,
            "INSERT OR IGNORE INTO tmp_scope_names (name) VALUES (?1)",
            params![name],
        )?;
    }
    execute(
        tx,
        "INSERT OR IGNORE INTO tmp_scope_dirty (name, family) SELECT name, family FROM dirty_names",
        [],
    )?;
    Ok(())
}

/// Links references to symbols of their own file, at `Resolved`.
///
/// The usual case, up to four symbols of the file with that name, is a plain join. Only a name
/// defined more than four times in one file (the four nearest lines are linked) needs ranking.
fn link_same_file(tx: &Connection) -> Result<u64> {
    let plain = format!(
        "INSERT INTO edges (src, dst, kind, confidence, line, src_file, dst_file) \
         SELECT r.owner, s.id, r.kind, ?1, MIN(r.line), r.file_id, r.file_id \
         FROM tmp_refs r JOIN symbols s ON s.name = r.name AND s.file_id = r.file_id \
         WHERE r.nsame BETWEEN 1 AND {MAX_LINKS} AND (r.excl = 0 OR s.id <> r.owner) \
         GROUP BY r.owner, s.id, r.kind {ON_CONFLICT}"
    );
    let ranked = format!(
        "INSERT INTO edges (src, dst, kind, confidence, line, src_file, dst_file) \
         SELECT owner, sid, kind, ?1, MIN(line), file, file FROM ( \
             SELECT r.owner AS owner, s.id AS sid, r.kind AS kind, r.line AS line, \
                    r.file_id AS file, \
                    ROW_NUMBER() OVER (PARTITION BY r.rid \
                                       ORDER BY ABS(s.start_line - r.line), s.id) AS rn \
             FROM tmp_refs r JOIN symbols s ON s.name = r.name AND s.file_id = r.file_id \
             WHERE r.nsame > {MAX_LINKS} AND (r.excl = 0 OR s.id <> r.owner)) \
         WHERE rn <= {MAX_LINKS} GROUP BY owner, sid, kind {ON_CONFLICT}"
    );
    let resolved = params![confidence_to_sql(Confidence::Resolved)];
    let mut changed = execute(tx, &plain, resolved)?;
    changed += execute(
        tx,
        &ranked,
        params![confidence_to_sql(Confidence::Resolved)],
    )?;
    Ok(u64::try_from(changed).unwrap_or(0))
}

/// Links references to the only symbol of that name in the index, at `Heuristic`.
fn link_unique(tx: &Connection) -> Result<u64> {
    let sql = format!(
        "INSERT INTO edges (src, dst, kind, confidence, line, src_file, dst_file) \
         SELECT r.owner, s.id, r.kind, ?1, MIN(r.line), r.file_id, s.file_id \
         FROM tmp_refs r JOIN symbols s ON s.name = r.name \
         JOIN files sf ON sf.id = s.file_id AND sf.family = r.family \
         WHERE r.nsame = 0 AND r.cnt = 1 AND (r.excl = 0 OR s.id <> r.owner) \
         GROUP BY r.owner, s.id, r.kind {ON_CONFLICT}"
    );
    let changed = execute(tx, &sql, params![confidence_to_sql(Confidence::Heuristic)])?;
    Ok(u64::try_from(changed).unwrap_or(0))
}

/// Length of the common prefix of two paths, in bytes.
fn common_prefix(a: &str, b: &str) -> usize {
    a.bytes().zip(b.bytes()).take_while(|(x, y)| x == y).count()
}

/// One candidate for an ambiguous name.
struct Candidate {
    /// The candidate's symbol id.
    id: i64,
    /// The file that declares it.
    file_id: i64,
    /// The path of its file.
    path: String,
}

/// Keeps the `PICK_DEPTH` best candidates for a referencing path: the longest shared path prefix
/// first, then the order the candidates came in (path, then id). Nothing that an edit of a file
/// can change (line numbers, source order) takes part in the ranking, which is what lets edges
/// into an untouched symbol stay valid.
fn best_candidates<'a>(candidates: &'a [Candidate], from_path: &str) -> Vec<&'a Candidate> {
    let mut best: Vec<(usize, usize)> = Vec::with_capacity(PICK_DEPTH + 1);
    for (position, candidate) in candidates.iter().enumerate() {
        let shared = common_prefix(&candidate.path, from_path);
        let slot = best
            .iter()
            .position(|&(other, _)| shared > other)
            .unwrap_or(best.len());
        if slot < PICK_DEPTH {
            best.insert(slot, (shared, position));
            best.truncate(PICK_DEPTH);
        }
    }
    best.into_iter()
        .map(|(_, position)| &candidates[position])
        .collect()
}

/// Links references to an ambiguous name (several candidates, none in the same file) at `Guess`,
/// to the best four by shared path prefix.
fn link_ambiguous(tx: &Connection) -> Result<u64> {
    let groups: Vec<(String, String, i64, String)> = query_all(
        tx,
        "SELECT DISTINCT r.name, r.family, r.file_id, f.path \
         FROM tmp_refs r JOIN files f ON f.id = r.file_id \
         WHERE r.nsame = 0 AND r.cnt >= 2 ORDER BY r.name, r.family, r.file_id",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    if groups.is_empty() {
        return Ok(0);
    }
    let mut candidates: Vec<Candidate> = Vec::new();
    let mut loaded_for: Option<(&str, &str)> = None;
    {
        let mut insert = tx
            .prepare_cached(
                "INSERT INTO tmp_pick (name, file_id, rank, sid, sfile) VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .db()?;
        for (name, family, file_id, path) in &groups {
            if loaded_for != Some((name.as_str(), family.as_str())) {
                candidates = query_all(
                    tx,
                    "SELECT s.id, s.file_id, f.path FROM symbols s JOIN files f ON f.id = s.file_id \
                     WHERE s.name = ?1 AND f.family = ?2 ORDER BY f.path, s.id",
                    params![name, family],
                    |row| {
                        Ok(Candidate {
                            id: row.get(0)?,
                            file_id: row.get(1)?,
                            path: row.get(2)?,
                        })
                    },
                )?;
                loaded_for = Some((name.as_str(), family.as_str()));
            }
            for (rank, pick) in best_candidates(&candidates, path).into_iter().enumerate() {
                insert
                    .execute(params![
                        name,
                        file_id,
                        i64::try_from(rank).unwrap_or(0),
                        pick.id,
                        pick.file_id
                    ])
                    .db()?;
            }
        }
    }
    // A reference that may link to its owner (`excl = 0`) takes the first four candidates; the
    // few that must skip the owner take the first four that are not the owner, hence the ranking.
    let plain = format!(
        "INSERT INTO edges (src, dst, kind, confidence, line, src_file, dst_file) \
         SELECT r.owner, p.sid, r.kind, ?1, MIN(r.line), r.file_id, p.sfile \
         FROM tmp_refs r JOIN tmp_pick p ON p.name = r.name AND p.file_id = r.file_id \
         WHERE r.nsame = 0 AND r.cnt >= 2 AND r.excl = 0 AND p.rank < {MAX_LINKS} \
         GROUP BY r.owner, p.sid, r.kind {ON_CONFLICT}"
    );
    let ranked = format!(
        "INSERT INTO edges (src, dst, kind, confidence, line, src_file, dst_file) \
         SELECT owner, sid, kind, ?1, MIN(line), file, sfile FROM ( \
             SELECT r.owner AS owner, p.sid AS sid, r.kind AS kind, r.line AS line, \
                    r.file_id AS file, p.sfile AS sfile, \
                    ROW_NUMBER() OVER (PARTITION BY r.rid ORDER BY p.rank) AS rn \
             FROM tmp_refs r JOIN tmp_pick p ON p.name = r.name AND p.file_id = r.file_id \
             WHERE r.nsame = 0 AND r.cnt >= 2 AND r.excl = 1 AND p.sid <> r.owner) \
         WHERE rn <= {MAX_LINKS} GROUP BY owner, sid, kind {ON_CONFLICT}"
    );
    let mut changed = execute(tx, &plain, params![confidence_to_sql(Confidence::Guess)])?;
    changed += execute(tx, &ranked, params![confidence_to_sql(Confidence::Guess)])?;
    Ok(u64::try_from(changed).unwrap_or(0))
}

/// The query that follows edges out of a symbol, reading the symbol at the far end.
const NEIGHBORS_OUT: &str = concat!(
    "SELECT ",
    symbol_columns!(),
    ", e.src, e.dst, e.kind, e.confidence, e.line \
     FROM edges e JOIN symbols s ON s.id = e.dst JOIN files f ON f.id = s.file_id \
     WHERE e.src = ?1 AND e.confidence >= ?2 \
     ORDER BY e.confidence DESC, f.path, e.line, s.start_line, s.id, e.kind LIMIT ?3"
);

/// The query that follows edges into a symbol, reading the symbol at the far end.
const NEIGHBORS_IN: &str = concat!(
    "SELECT ",
    symbol_columns!(),
    ", e.src, e.dst, e.kind, e.confidence, e.line \
     FROM edges e JOIN symbols s ON s.id = e.src JOIN files f ON f.id = s.file_id \
     WHERE e.dst = ?1 AND e.confidence >= ?2 \
     ORDER BY e.confidence DESC, f.path, e.line, s.start_line, s.id, e.kind LIMIT ?3"
);

/// Follows edges from a symbol, keeping those at least as confident as `min_confidence`, by
/// descending confidence, then path and line.
pub(crate) fn neighbors(
    conn: &Connection,
    id: SymbolId,
    direction: Direction,
    min_confidence: Confidence,
    limit: usize,
) -> Result<Vec<Neighbor>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let sql = match direction {
        Direction::Out => NEIGHBORS_OUT,
        Direction::In => NEIGHBORS_IN,
    };
    query_all(
        conn,
        sql,
        params![id.0, confidence_to_sql(min_confidence), limit_to_sql(limit)],
        |row| {
            let symbol = symbol_from_row(row)?;
            let kind_name: String = row.get(20)?;
            let confidence: i64 = row.get(21)?;
            Ok(Neighbor {
                symbol,
                edge: EdgeRecord {
                    src: SymbolId(row.get(18)?),
                    dst: SymbolId(row.get(19)?),
                    kind: edge_kind(&kind_name, 20)?,
                    confidence: confidence_from_sql(confidence, 21)?,
                    line: row.get(22)?,
                },
            })
        },
    )
}

#[cfg(test)]
mod tests {
    use super::{Candidate, best_candidates, common_prefix};

    /// A candidate at a path.
    fn candidate(id: i64, path: &str) -> Candidate {
        Candidate {
            id,
            file_id: id * 10,
            path: path.to_owned(),
        }
    }

    /// The shared prefix counts bytes and stops at the first difference.
    #[test]
    fn common_prefix_lengths() {
        assert_eq!(common_prefix("src/a/x.rs", "src/a/y.rs"), 6);
        assert_eq!(common_prefix("src/a", "lib/a"), 0);
        assert_eq!(common_prefix("", "abc"), 0);
        assert_eq!(common_prefix("same", "same"), 4);
    }

    /// Candidates are ranked by shared prefix, ties keep their input order, and only the best
    /// few are kept.
    #[test]
    fn best_candidates_prefers_nearby_paths() {
        let all = [
            candidate(1, "docs/a.rs"),
            candidate(2, "src/net/a.rs"),
            candidate(3, "src/a.rs"),
            candidate(4, "src/net/b.rs"),
            candidate(5, "lib/a.rs"),
            candidate(6, "zzz/a.rs"),
            candidate(7, "src/net/c.rs"),
        ];
        let picked: Vec<i64> = best_candidates(&all, "src/net/main.rs")
            .iter()
            .map(|c| c.id)
            .collect();
        assert_eq!(picked, vec![2, 4, 7, 3, 1]);
        let none = best_candidates(&[], "src/x.rs");
        assert!(none.is_empty());
    }
}
