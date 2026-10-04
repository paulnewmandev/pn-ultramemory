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
//! | Else exactly **one** symbol in the whole index has the name | to it | `Heuristic`, or `Guess` when the call is made on a receiver that does not point at it |
//! | Else **several** do, and the receiver points at exactly one | to that one | `Heuristic` |
//! | Else **several** do | to at most 4, whose path shares the longest prefix with the referencing file | `Guess` |
//! | Else none | none | none |
//!
//! # Receivers
//! A name alone is weak evidence for a method: `$request->validate()` in a Laravel controller and
//! `items.is_empty()` in Rust share their method's name with one symbol of the repository, and are
//! calls on a framework object and on a standard vector. So a reference keeps the last word of
//! its receiver (`couponservice` for `$this->couponService`, empty for an expression such as
//! `items()`), and that word *points at* a candidate when it names the candidate's type (its
//! enclosing symbol, or the part of its qualified name before its own name; equal, or the end of a
//! longer name of at least four letters, ignoring case), the candidate's file (`utils` for
//! `utils.py`) or one of its directories (`store` for `crates/store/src/db.rs`). Among several
//! candidates, a match by type is preferred to a match by place. A reference with no receiver of its own (unqualified, `self`,
//! `this`, `super` and the like) resolves by name alone, as before.
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
    Confidence, Direction, EXPRESSION_QUALIFIER, EdgeKind, EdgeRecord, Neighbor, RefKind,
    ResolveScope, ResolveStats, SymbolId,
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

/// Qualifiers that name no receiver at all: the enclosing module or crate.
const MODULE_QUALIFIERS: &[&str] = &["super", "crate", "parent"];

/// The last word of a reference's receiver, lowercased: `couponservice` for
/// `$this->couponService`, `storage` for `self.storage`, `fs` for `std::fs`.
///
/// `None` means the reference has no receiver of its own: it is unqualified, or qualified by the
/// symbol itself or by the enclosing module, and resolves by name as it always did. An empty
/// word means a receiver that is an expression, which says only that the call is made on some
/// other object.
///
/// # Examples
/// ```text
/// receiver_word(None)                          == None
/// receiver_word(Some("$this"))                 == None
/// receiver_word(Some("$this->couponService"))  == Some("couponservice")
/// receiver_word(Some("(expr)"))                == Some("")
/// ```
pub(crate) fn receiver_word(qualifier: Option<&str>) -> Option<String> {
    let qualifier = qualifier?.trim();
    if qualifier.is_empty()
        || SELF_QUALIFIERS.contains(&qualifier)
        || MODULE_QUALIFIERS.contains(&qualifier)
    {
        return None;
    }
    if qualifier == EXPRESSION_QUALIFIER {
        return Some(String::new());
    }
    let last = qualifier
        .rsplit(['.', ':', '>', '\\', '/', '#', '@'])
        .find(|part| !part.is_empty())
        .unwrap_or_default()
        .trim_start_matches('$')
        .trim_end_matches('-');
    if SELF_QUALIFIERS.contains(&last) {
        return None;
    }
    Some(last.to_lowercase())
}

/// The SQL condition, 0 or 1, that the receiver word in column `recv` names the type the
/// candidate `symbol` belongs to: its enclosing symbol `parent` (left-joined, so it may be NULL),
/// or, when the type is declared elsewhere (a Rust `impl` in another file), the segment of its
/// qualified name just before its own name (`Engine` in `Engine::recall`). The word must equal
/// that name or, with at least four letters, end it, ignoring case (and underscores, for a parent).
fn names_type(recv: &str, symbol: &str, parent: &str) -> String {
    let segment = |sep: &str| {
        format!(
            "{symbol}.qualified_name LIKE {recv} || '{sep}' || {symbol}.name \
             OR {symbol}.qualified_name LIKE '%{sep}' || {recv} || '{sep}' || {symbol}.name \
             OR (length({recv}) >= 4 \
                 AND {symbol}.qualified_name LIKE '%' || {recv} || '{sep}' || {symbol}.name)"
        )
    };
    format!(
        "COALESCE({recv} <> '' AND ( \
             replace(lower({parent}.name), '_', '') = replace({recv}, '_', '') \
             OR (length({recv}) >= 4 \
                 AND replace(lower({parent}.name), '_', '') LIKE '%' || replace({recv}, '_', '')) \
             OR {} OR {}), 0)",
        segment("::"),
        segment("."),
    )
}

/// The SQL condition, 0 or 1, that the receiver word in column `recv` names the place the
/// candidate is declared in: its file (`utils` for `utils.py`) or one of its directories (`store`
/// for `crates/store/src/db.rs`), where `file` is the row alias of the candidate's file.
fn names_place(recv: &str, file: &str) -> String {
    format!(
        "COALESCE({recv} <> '' AND ( \
             {file}.path LIKE '%/' || {recv} || '.%' OR {file}.path LIKE {recv} || '.%' \
             OR {file}.path LIKE '%/' || {recv} || '/%' OR {file}.path LIKE {recv} || '/%'), 0)"
    )
}

/// The SQL condition that the receiver word points at a candidate at all, by its type or by its
/// place. An empty word, the receiver of an expression, points at nothing.
fn points_at(recv: &str, symbol: &str, parent: &str, file: &str) -> String {
    format!(
        "({} OR {})",
        names_type(recv, symbol, parent),
        names_place(recv, file)
    )
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
        "tmp_hints",
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
/// The count for a reference without a receiver reads only the name index, which is most of them;
/// the receiver of an expression points at nothing, so its count is zero; a named receiver needs
/// each candidate's enclosing symbol, and only those references pay for it.
///
/// References without an owner are left out. `excl` is the reference's `no_self` flag and
/// `family` the language family of its file. `nsame` counts the candidates in the reference's
/// file that its receiver allows (all of them without a receiver, those it points at with one:
/// `engine.recall()` is not a call to the `recall` method of another type in the same file), and
/// `cnt` those in the whole family (from `tmp_names`); both leave out the owner when it is
/// excluded.
fn load_refs_sql(filter: &str) -> String {
    format!(
        "INSERT OR IGNORE INTO tmp_refs \
             (rid, file_id, owner, name, kind, line, excl, family, nsame, cnt, recv) \
         SELECT j.rid, j.file_id, j.owner, j.name, j.kind, j.line, j.excl, j.family, \
                CASE WHEN j.recv IS NULL THEN \
                    (SELECT COUNT(*) FROM symbols s \
                     WHERE s.name = j.name AND s.file_id = j.file_id) - j.excl \
                WHEN j.recv = '' THEN 0 \
                ELSE \
                    (SELECT COUNT(*) FROM symbols s LEFT JOIN symbols p ON p.id = s.parent_id \
                     WHERE s.name = j.name AND s.file_id = j.file_id \
                       AND (j.excl = 0 OR s.id <> j.owner) AND {allowed}) \
                END, \
                COALESCE((SELECT n.cnt FROM tmp_names n \
                          WHERE n.name = j.name AND n.family = j.family), 0) - j.excl, \
                j.recv \
         FROM (SELECT r.id AS rid, r.file_id AS file_id, r.owner_id AS owner, r.name AS name, \
                      {kind} AS kind, r.line AS line, r.no_self AS excl, rf.family AS family, \
                      r.recv AS recv, rf.path AS path \
               FROM refs r JOIN files rf ON rf.id = r.file_id \
               WHERE r.owner_id IS NOT NULL {filter}) j",
        kind = edge_kind_case(),
        allowed = points_at("j.recv", "s", "p", "j"),
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
    point_receivers(tx)?;
    let mut written = 0_u64;
    written += link_same_file(tx)?;
    written += link_unique(tx)?;
    written += link_pointed(tx)?;
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
    let allowed = points_at("r.recv", "s", "p", "f");
    // Without a receiver every candidate of the file counts, and nothing but the name is read.
    let plain = format!(
        "INSERT INTO edges (src, dst, kind, confidence, line, src_file, dst_file) \
         SELECT r.owner, s.id, r.kind, ?1, MIN(r.line), r.file_id, r.file_id \
         FROM tmp_refs r JOIN symbols s ON s.name = r.name AND s.file_id = r.file_id \
         WHERE r.nsame BETWEEN 1 AND {MAX_LINKS} AND r.recv IS NULL \
           AND (r.excl = 0 OR s.id <> r.owner) \
         GROUP BY r.owner, s.id, r.kind {ON_CONFLICT}"
    );
    // With one, only the candidates it points at.
    let received = format!(
        "INSERT INTO edges (src, dst, kind, confidence, line, src_file, dst_file) \
         SELECT r.owner, s.id, r.kind, ?1, MIN(r.line), r.file_id, r.file_id \
         FROM tmp_refs r JOIN symbols s ON s.name = r.name AND s.file_id = r.file_id \
         JOIN files f ON f.id = r.file_id LEFT JOIN symbols p ON p.id = s.parent_id \
         WHERE r.nsame BETWEEN 1 AND {MAX_LINKS} AND r.recv IS NOT NULL AND r.recv <> '' \
           AND (r.excl = 0 OR s.id <> r.owner) AND {allowed} \
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
             JOIN files f ON f.id = r.file_id LEFT JOIN symbols p ON p.id = s.parent_id \
             WHERE r.nsame > {MAX_LINKS} AND (r.excl = 0 OR s.id <> r.owner) \
               AND (r.recv IS NULL OR {allowed})) \
         WHERE rn <= {MAX_LINKS} GROUP BY owner, sid, kind {ON_CONFLICT}"
    );
    let resolved = params![confidence_to_sql(Confidence::Resolved)];
    let mut changed = execute(tx, &plain, resolved)?;
    changed += execute(
        tx,
        &received,
        params![confidence_to_sql(Confidence::Resolved)],
    )?;
    changed += execute(
        tx,
        &ranked,
        params![confidence_to_sql(Confidence::Resolved)],
    )?;
    Ok(u64::try_from(changed).unwrap_or(0))
}

/// Records, for every reference with a named receiver and no candidate in its own file, the one
/// candidate its receiver points at (see the module documentation). A receiver that names the
/// type of exactly one candidate points at it; only when it names no candidate's type does a
/// single match by file or directory count, so `engine.recall()` is `Engine::recall` and not the
/// `mod recall` that happens to live under `crates/engine/`.
fn point_receivers(tx: &Connection) -> Result<()> {
    // The choice, for a receiver word in column `recv` of the row alias `of`: a match by type
    // when exactly one candidate has one, else a match by place when exactly one has that.
    let choice = |of: &str| {
        let typed = names_type(&format!("{of}.recv"), "s", "p");
        let placed = names_place(&format!("{of}.recv"), "sf");
        format!(
            "CASE \
                 WHEN SUM({typed}) = 1 THEN MAX(CASE WHEN {typed} THEN s.id END) \
                 WHEN SUM({typed}) = 0 AND SUM({placed}) = 1 \
                     THEN MAX(CASE WHEN {placed} THEN s.id END) \
             END"
        )
    };
    let scope = "nsame = 0 AND recv IS NOT NULL AND recv <> ''";
    // Most references may link to their owner, so their answer depends only on the name, the
    // family and the word, and is worked out once for each distinct triple.
    execute(
        tx,
        &format!(
            "INSERT OR IGNORE INTO tmp_hints (name, family, recv) \
             SELECT DISTINCT name, family, recv FROM tmp_refs WHERE {scope} AND excl = 0"
        ),
        [],
    )?;
    execute(
        tx,
        &format!(
            "UPDATE tmp_hints SET hint = ( \
                 SELECT {} FROM symbols s \
                 JOIN files sf ON sf.id = s.file_id AND sf.family = tmp_hints.family \
                 LEFT JOIN symbols p ON p.id = s.parent_id \
                 WHERE s.name = tmp_hints.name)",
            choice("tmp_hints")
        ),
        [],
    )?;
    execute(
        tx,
        &format!(
            "UPDATE tmp_refs SET hint = ( \
                 SELECT h.hint FROM tmp_hints h \
                 WHERE h.name = tmp_refs.name AND h.family = tmp_refs.family \
                   AND h.recv = tmp_refs.recv) \
             WHERE {scope} AND excl = 0"
        ),
        [],
    )?;
    // The few that must not link to their owner leave it out of the candidates, one by one.
    execute(
        tx,
        &format!(
            "UPDATE tmp_refs SET hint = ( \
                 SELECT {} FROM symbols s \
                 JOIN files sf ON sf.id = s.file_id AND sf.family = tmp_refs.family \
                 LEFT JOIN symbols p ON p.id = s.parent_id \
                 WHERE s.name = tmp_refs.name AND s.id <> tmp_refs.owner) \
             WHERE {scope} AND excl = 1",
            choice("tmp_refs")
        ),
        [],
    )?;
    Ok(())
}

/// Links references to the only symbol of that name in the index: at `Heuristic` when nothing
/// argues against it, at `Guess` when the reference is made on a receiver that does not point at
/// that symbol.
fn link_unique(tx: &Connection) -> Result<u64> {
    let sql = format!(
        "INSERT INTO edges (src, dst, kind, confidence, line, src_file, dst_file) \
         SELECT r.owner, s.id, r.kind, \
                MAX(CASE WHEN r.recv IS NULL OR r.hint = s.id THEN ?1 ELSE ?2 END), \
                MIN(r.line), r.file_id, s.file_id \
         FROM tmp_refs r JOIN symbols s ON s.name = r.name \
         JOIN files sf ON sf.id = s.file_id AND sf.family = r.family \
         WHERE r.nsame = 0 AND r.cnt = 1 AND (r.excl = 0 OR s.id <> r.owner) \
         GROUP BY r.owner, s.id, r.kind {ON_CONFLICT}"
    );
    let changed = execute(
        tx,
        &sql,
        params![
            confidence_to_sql(Confidence::Heuristic),
            confidence_to_sql(Confidence::Guess)
        ],
    )?;
    Ok(u64::try_from(changed).unwrap_or(0))
}

/// Links references to an ambiguous name whose receiver points at exactly one of the candidates,
/// to that one, at `Heuristic`: `engine.recall()` is `Engine::recall` and not the other `recall`
/// functions of the repository.
fn link_pointed(tx: &Connection) -> Result<u64> {
    let sql = format!(
        "INSERT INTO edges (src, dst, kind, confidence, line, src_file, dst_file) \
         SELECT r.owner, r.hint, r.kind, ?1, MIN(r.line), r.file_id, s.file_id \
         FROM tmp_refs r JOIN symbols s ON s.id = r.hint \
         WHERE r.nsame = 0 AND r.cnt >= 2 \
         GROUP BY r.owner, r.hint, r.kind {ON_CONFLICT}"
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

/// Links references to an ambiguous name (several candidates, none in the same file, no receiver
/// pointing at one of them) at `Guess`, to the best four by shared path prefix.
fn link_ambiguous(tx: &Connection) -> Result<u64> {
    let groups: Vec<(String, String, i64, String)> = query_all(
        tx,
        "SELECT DISTINCT r.name, r.family, r.file_id, f.path \
         FROM tmp_refs r JOIN files f ON f.id = r.file_id \
         WHERE r.nsame = 0 AND r.cnt >= 2 AND r.hint IS NULL \
         ORDER BY r.name, r.family, r.file_id",
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
         WHERE r.nsame = 0 AND r.cnt >= 2 AND r.hint IS NULL AND r.excl = 0 \
           AND p.rank < {MAX_LINKS} \
         GROUP BY r.owner, p.sid, r.kind {ON_CONFLICT}"
    );
    let ranked = format!(
        "INSERT INTO edges (src, dst, kind, confidence, line, src_file, dst_file) \
         SELECT owner, sid, kind, ?1, MIN(line), file, sfile FROM ( \
             SELECT r.owner AS owner, p.sid AS sid, r.kind AS kind, r.line AS line, \
                    r.file_id AS file, p.sfile AS sfile, \
                    ROW_NUMBER() OVER (PARTITION BY r.rid ORDER BY p.rank) AS rn \
             FROM tmp_refs r JOIN tmp_pick p ON p.name = r.name AND p.file_id = r.file_id \
             WHERE r.nsame = 0 AND r.cnt >= 2 AND r.hint IS NULL AND r.excl = 1 \
               AND p.sid <> r.owner) \
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
            // symbol_columns! now includes pagerank as column 18, so edge columns shift by one.
            let kind_name: String = row.get(21)?;
            let confidence: i64 = row.get(22)?;
            Ok(Neighbor {
                symbol,
                edge: EdgeRecord {
                    src: SymbolId(row.get(19)?),
                    dst: SymbolId(row.get(20)?),
                    kind: edge_kind(&kind_name, 21)?,
                    confidence: confidence_from_sql(confidence, 22)?,
                    line: row.get(23)?,
                },
            })
        },
    )
}

/// Returns every resolved edge as `(src, dst)` pairs for structural scoring.
pub(crate) fn all_edges(
    conn: &Connection,
) -> Result<Vec<(pn_ultramemory_core::SymbolId, pn_ultramemory_core::SymbolId)>> {
    query_all(conn, "SELECT src, dst FROM edges", [], |row| {
        let src: i64 = row.get(0)?;
        let dst: i64 = row.get(1)?;
        Ok((
            pn_ultramemory_core::SymbolId(src),
            pn_ultramemory_core::SymbolId(dst),
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::{Candidate, best_candidates, common_prefix, receiver_word};

    /// A receiver word is the last word of the receiver, lowercased; the symbol itself and the
    /// enclosing module are no receiver at all, and an expression is a receiver with no word.
    #[test]
    fn receiver_words() {
        let word = |q: Option<&str>| receiver_word(q);
        assert_eq!(word(None), None);
        for own in [
            "self", "this", "$this", "Self", "cls", "static", "super", "crate", "parent",
        ] {
            assert_eq!(word(Some(own)), None, "{own}");
        }
        assert_eq!(
            word(Some("$this->couponService")).as_deref(),
            Some("couponservice")
        );
        assert_eq!(word(Some("$request")).as_deref(), Some("request"));
        assert_eq!(word(Some("self.storage")).as_deref(), Some("storage"));
        assert_eq!(word(Some("std::fs")).as_deref(), Some("fs"));
        assert_eq!(
            word(Some("App\\Support\\Settings")).as_deref(),
            Some("settings")
        );
        assert_eq!(
            word(Some("CouponService")).as_deref(),
            Some("couponservice")
        );
        assert_eq!(word(Some("(expr)")).as_deref(), Some(""));
    }

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
