// SPDX-License-Identifier: Apache-2.0
//! Replacing everything stored for one file, or for a whole batch of files, in one transaction.
//!
//! # What an upsert does, in order
//! 1. Stores the file row (its id is stable, the row is updated in place).
//! 2. Loads the file's current symbols with one query and gives every new symbol its stable
//!    identity (see `crate::ids`): a symbol that keeps its qualified name, kind and ordinal keeps
//!    its id.
//! 3. Diffs old and new by id: **added** (new id), **removed** (old id gone), **modified** (same
//!    id, different signature hash or declaration hash).
//! 4. Drops the edges that start at one of the file's old symbols (they came from references that
//!    are being replaced). Edges that end at a symbol that no longer exists go away with it,
//!    through the delete trigger. Edges that end at a symbol that **survives** (same stable id)
//!    are kept: they come from other files' references, which this upsert did not touch, and they
//!    stay valid because nothing about the set of definitions of their name changed.
//! 5. Deletes the removed symbols (their memory anchors keep the recorded name, path and hashes
//!    and lose the symbol link), writes the new symbols and their full-text rows, updates the
//!    surviving ones, and replaces the file's references.
//! 6. Marks stale the memories that were fresh and whose anchor no longer matches, using
//!    `Anchor::is_stale_against`.
//! 7. Records as "dirty" the names whose *set of definitions* changed **in the file's language
//!    family** (a symbol added or removed, not one merely modified), so a later `Touching`
//!    resolve revisits every reference to them in that family even if the caller forgets to pass
//!    the names. Those are the only references that can change meaning. A file that changes
//!    family (same path, other language) counts as removing all its symbols from the old family
//!    and adding them to the new one, and loses the edges into it.
//!
//! # Batches
//! [`upsert_files`] runs any number of files in a single `IMMEDIATE` transaction with every
//! statement prepared once. It is exactly the per-file upsert applied to each file in turn (a
//! later file sees the earlier ones), and it is atomic: if one file fails, nothing of the batch
//! is stored. One commit instead of one per file is where most of the speed of a bulk index comes
//! from; the rest is not doing work twice (no per-symbol lookups, reused text buffers, and the
//! full-text row of an unchanged symbol left alone).
//!
//! # Identity collisions
//! A fresh id is *optimistically* inserted. If the store already has a row with that id (a
//! collision that happens about once in ten billion symbols) the insert fails on the primary key
//! and the next derivation attempt is used instead, so the common path performs no lookup.
//!
//! # Hostile input
//! An extractor can report nonsense. A `parent` that does not point to an *earlier* symbol is
//! stored as no parent; a reference `owner` outside the symbol list is stored as file level. Both
//! are dropped silently instead of failing the whole file.

use std::collections::{BTreeSet, HashMap, HashSet};

use pn_ultramemory_core::{
    Anchor, FileExtract, FileId, FileInput, StorageError, SymbolId, SymbolKind, UpsertOutcome,
};
use rusqlite::{CachedStatement, Connection, ToSql, TransactionBehavior, params};

use crate::convert::{encode_names, size_to_sql, u64_from_sql, u64_to_sql};
use crate::edges::{must_not_link_to_self, receiver_word};
use crate::error::{DbResult, Result, from_sqlite};
use crate::ids::{Allocator, IdKey, derive_id_with};
use crate::query::drop_index;
use crate::tokens::{MAX_INDEXED_BYTES, Scratch, index_text_into};

/// A symbol as it is stored before the upsert.
struct OldSymbol {
    /// Its identity.
    id: i64,
    /// Its simple name.
    name: String,
    /// Its qualified name.
    qualified_name: String,
    /// The stable name of its kind.
    kind: String,
    /// Its ordinal among symbols of the same qualified name and kind.
    ordinal: u32,
    /// Its signature hash, as stored.
    sig_hash: i64,
    /// Its declaration hash, as stored.
    body_hash: i64,
    /// Its signature, as stored.
    signature: String,
    /// Its documentation, as stored.
    doc: Option<String>,
}

/// What the diff of old and new symbols found.
struct Plan<'a> {
    /// What identifies every new symbol, in the extractor's order.
    keys: Vec<IdKey<'a>>,
    /// The ids that are spoken for, for the rare retry after a primary key conflict.
    allocator: Allocator,
    /// The identity of every new symbol; final only once the symbol has been inserted.
    ids: Vec<i64>,
    /// The derivation attempt that produced each id.
    attempts: Vec<u32>,
    /// Whether each new symbol keeps a stored identity.
    survivor: Vec<bool>,
    /// Whether the full-text row of each surviving symbol is still right.
    text_unchanged: Vec<bool>,
    /// Identities that were stored and are not any more.
    removed: Vec<i64>,
    /// Number of identities that are new.
    added: u32,
    /// Number of identities that kept their id but changed.
    modified: u32,
    /// Identities that kept their id but whose simple name is different now.
    renamed: Vec<i64>,
    /// Simple names of every added, removed or modified symbol.
    changed_names: BTreeSet<String>,
    /// Simple names whose set of definitions changed: symbols added or removed.
    definition_names: BTreeSet<String>,
}

/// The full-text row of a symbol, waiting to be written.
struct FtsRow {
    /// The symbol id, which is the rowid of the row.
    id: i64,
    /// Whether an older row of this symbol must be removed first.
    replace: bool,
    /// The four columns: name, qualified name, signature and documentation, already split.
    columns: [String; 4],
}

/// How many full-text rows are held back before they are written.
const FTS_BATCH_ROWS: usize = 16_384;

/// Statements prepared once and used for every file of a batch.
struct Writer<'c> {
    /// Inserts or updates the row of a file, returning its id.
    file_row: CachedStatement<'c>,
    /// Lists the symbols currently stored for a file.
    old_symbols: CachedStatement<'c>,
    /// Inserts a symbol with a new identity.
    insert_symbol: CachedStatement<'c>,
    /// Updates a symbol that keeps its identity.
    update_symbol: CachedStatement<'c>,
    /// Deletes a symbol (its edges and full-text row follow).
    delete_symbol: CachedStatement<'c>,
    /// Adds the full-text row of a symbol.
    fts_insert: CachedStatement<'c>,
    /// Removes the full-text row of a symbol.
    fts_delete: CachedStatement<'c>,
    /// Deletes the references of a file.
    delete_refs: CachedStatement<'c>,
    /// Inserts one reference.
    insert_ref: CachedStatement<'c>,
    /// Deletes the edges that start at a symbol of a file.
    drop_out_edges: CachedStatement<'c>,
    /// Deletes the edges that end at one symbol.
    drop_in_edges: CachedStatement<'c>,
    /// Records a dirty name, if a stored reference mentions it.
    dirty_name: CachedStatement<'c>,
    /// Fresh memory anchors recorded for the path of a file.
    anchors: CachedStatement<'c>,
    /// Marks a memory stale.
    mark_stale: CachedStatement<'c>,
    /// Names whose set of definitions changed in this batch, with the language family they
    /// changed in; see [`Writer::record_dirty_names`].
    dirty_candidates: BTreeSet<(String, String)>,
    /// Reads the family a file was stored with.
    family_of: CachedStatement<'c>,
    /// Scanner state for splitting identifiers.
    scratch: Scratch,
    /// Full-text rows waiting to be written in id order; see [`Writer::flush_fts`].
    pending_fts: Vec<FtsRow>,
}

impl<'c> Writer<'c> {
    /// Prepares every statement.
    fn new(conn: &'c Connection) -> Result<Self> {
        let prepare = |sql: &str| conn.prepare_cached(sql).db();
        Ok(Self {
            family_of: prepare("SELECT family FROM files WHERE path = ?1")?,
            file_row: prepare(
                "INSERT INTO files (path, language, hash, size, mtime, lines, symbol_count, \
                     indexed_at, parse_errors, family) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10) \
                 ON CONFLICT (path) DO UPDATE SET language = excluded.language, \
                     hash = excluded.hash, size = excluded.size, mtime = excluded.mtime, \
                     lines = excluded.lines, symbol_count = excluded.symbol_count, \
                     indexed_at = excluded.indexed_at, parse_errors = excluded.parse_errors, \
                     family = excluded.family \
                 RETURNING id",
            )?,
            old_symbols: prepare(
                "SELECT id, name, qualified_name, kind, ordinal, sig_hash, body_hash, signature, \
                     doc FROM symbols WHERE file_id = ?1 ORDER BY seq, id",
            )?,
            insert_symbol: prepare(
                "INSERT INTO symbols (id, file_id, seq, ordinal, name, qualified_name, kind, \
                     signature, doc, visibility, start_line, end_line, start_byte, end_byte, \
                     parent_id, outline, sig_hash, body_hash) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, \
                     ?17, ?18)",
            )?,
            update_symbol: prepare(
                "UPDATE symbols SET file_id = ?2, seq = ?3, ordinal = ?4, name = ?5, \
                     qualified_name = ?6, kind = ?7, signature = ?8, doc = ?9, \
                     visibility = ?10, start_line = ?11, end_line = ?12, start_byte = ?13, \
                     end_byte = ?14, parent_id = ?15, outline = ?16, sig_hash = ?17, \
                     body_hash = ?18 \
                 WHERE id = ?1",
            )?,
            delete_symbol: prepare("DELETE FROM symbols WHERE id = ?1")?,
            fts_insert: prepare(
                "INSERT INTO symbol_fts (rowid, name, qname, sig, doc) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?,
            fts_delete: prepare("DELETE FROM symbol_fts WHERE rowid = ?1")?,
            delete_refs: prepare("DELETE FROM refs WHERE file_id = ?1")?,
            insert_ref: prepare(
                "INSERT INTO refs (file_id, owner_id, name, kind, line, qualifier, no_self, recv) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )?,
            drop_out_edges: prepare(
                "DELETE FROM edges WHERE src IN (SELECT id FROM symbols WHERE file_id = ?1)",
            )?,
            drop_in_edges: prepare("DELETE FROM edges WHERE dst = ?1")?,
            dirty_name: prepare(
                "INSERT OR IGNORE INTO dirty_names (name, family) \
                 SELECT ?1, ?2 WHERE EXISTS ( \
                     SELECT 1 FROM refs r JOIN files f ON f.id = r.file_id \
                     WHERE r.name = ?1 AND f.family = ?2)",
            )?,
            anchors: prepare(
                "SELECT a.memory_id, a.symbol_id, a.qualified_name, a.sig_hash, a.body_hash \
                 FROM memory_anchors a JOIN memories m ON m.id = a.memory_id \
                 WHERE a.path = ?1 AND m.stale_since IS NULL",
            )?,
            mark_stale: prepare(
                "UPDATE memories SET stale_since = ?1 WHERE id = ?2 AND stale_since IS NULL",
            )?,
            dirty_candidates: BTreeSet::new(),
            scratch: Scratch::default(),
            pending_fts: Vec::new(),
        })
    }

    /// Stores one file, in the transaction the statements belong to.
    fn write_file(
        &mut self,
        file: &FileInput,
        extract: &FileExtract,
        now: i64,
    ) -> Result<UpsertOutcome> {
        let family = file.language.family();
        let previous_family = self.stored_family(&file.path)?;
        let file_id = self.store_file_row(file, extract, now)?;
        let old = self.load_old_symbols(file_id)?;
        let mut plan = Self::plan_symbols(&file.path, extract, &old)?;

        if !old.is_empty() {
            // This file already has rows: bring the held-back full-text rows up to date first,
            // so that removing and replacing symbols below sees them.
            self.flush_fts()?;
            self.drop_out_edges.execute(params![file_id]).db()?;
            for id in &plan.renamed {
                self.drop_in_edges.execute(params![id]).db()?;
            }
        }
        if let Some(previous) = previous_family.filter(|previous| previous != family) {
            // The file moved to another language family: what references mean changes in both.
            for symbol in &old {
                self.drop_in_edges.execute(params![symbol.id]).db()?;
                self.dirty_candidates
                    .insert((symbol.name.clone(), previous.clone()));
            }
            for draft in &extract.symbols {
                self.dirty_candidates
                    .insert((draft.name.clone(), family.to_owned()));
            }
        }
        for id in &plan.removed {
            self.delete_symbol.execute(params![id]).db()?;
        }
        self.write_symbols(file_id, extract, &mut plan)?;
        self.write_references(file_id, extract, &plan)?;
        let memories_marked_stale = self.mark_stale_memories(&file.path, extract, &plan, now)?;
        self.dirty_candidates.extend(
            plan.definition_names
                .iter()
                .map(|name| (name.clone(), family.to_owned())),
        );
        Ok(UpsertOutcome {
            file_id: Some(FileId(file_id)),
            symbols_added: plan.added,
            symbols_removed: u32::try_from(plan.removed.len()).unwrap_or(u32::MAX),
            symbols_modified: plan.modified,
            memories_marked_stale,
            changed_names: plan.changed_names.into_iter().collect(),
        })
    }

    /// The language family the file was stored with, if it is stored.
    fn stored_family(&mut self, path: &str) -> Result<Option<String>> {
        let mut rows = self.family_of.query(params![path]).db()?;
        match rows.next().db()? {
            Some(row) => Ok(Some(row.get::<_, String>(0).db()?)),
            None => Ok(None),
        }
    }

    /// Inserts or updates the row of the file and returns its id.
    fn store_file_row(&mut self, file: &FileInput, extract: &FileExtract, now: i64) -> Result<i64> {
        let symbol_count = i64::try_from(extract.symbols.len()).unwrap_or(i64::MAX);
        let mut rows = self
            .file_row
            .query(params![
                file.path,
                file.language.name(),
                file.hash,
                size_to_sql(file.size),
                file.mtime_secs,
                extract.line_count,
                symbol_count,
                now,
                extract.parse_errors,
                file.language.family(),
            ])
            .db()?;
        let row = rows
            .next()
            .db()?
            .ok_or_else(|| StorageError::Backend("the file row was not stored".to_owned()))?;
        row.get::<_, i64>(0).db()
    }

    /// Loads the symbols currently stored for a file.
    fn load_old_symbols(&mut self, file_id: i64) -> Result<Vec<OldSymbol>> {
        let rows = self
            .old_symbols
            .query_map(params![file_id], |row| {
                Ok(OldSymbol {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    qualified_name: row.get(2)?,
                    kind: row.get(3)?,
                    ordinal: row.get(4)?,
                    sig_hash: row.get(5)?,
                    body_hash: row.get(6)?,
                    signature: row.get(7)?,
                    doc: row.get(8)?,
                })
            })
            .db()?;
        rows.collect::<rusqlite::Result<Vec<_>>>().db()
    }

    /// Gives every new symbol its identity and works out what was added, removed and modified.
    fn plan_symbols<'a>(
        path: &'a str,
        extract: &'a FileExtract,
        old: &[OldSymbol],
    ) -> Result<Plan<'a>> {
        let count = extract.symbols.len();
        let mut counters: HashMap<(&str, SymbolKind), u32> = HashMap::new();
        let mut keys: Vec<IdKey<'a>> = Vec::with_capacity(count);
        for draft in &extract.symbols {
            let counter = counters
                .entry((draft.qualified_name.as_str(), draft.kind))
                .or_insert(0);
            keys.push(IdKey {
                path,
                qualified_name: &draft.qualified_name,
                kind: draft.kind.as_str(),
                ordinal: *counter,
            });
            *counter += 1;
        }

        let by_key: HashMap<(&str, &str, u32), &OldSymbol> = old
            .iter()
            .map(|o| ((o.qualified_name.as_str(), o.kind.as_str(), o.ordinal), o))
            .collect();
        let mut allocator = Allocator::new(old.iter().map(|o| o.id));
        let mut ids = Vec::with_capacity(count);
        let mut attempts = Vec::with_capacity(count);
        let mut survivor = Vec::with_capacity(count);
        let mut text_unchanged = Vec::with_capacity(count);
        let mut kept: HashSet<i64> = HashSet::new();
        let mut added = 0_u32;
        let mut modified = 0_u32;
        let mut renamed = Vec::new();
        let mut changed_names = BTreeSet::new();
        let mut definition_names = BTreeSet::new();
        for (draft, key) in extract.symbols.iter().zip(&keys) {
            let previous = if by_key.is_empty() {
                None
            } else {
                by_key
                    .get(&(key.qualified_name, key.kind, key.ordinal))
                    .copied()
            };
            if let Some(previous) = previous {
                ids.push(previous.id);
                attempts.push(0);
                survivor.push(true);
                kept.insert(previous.id);
                text_unchanged.push(
                    previous.name == draft.name
                        && previous.signature == draft.signature
                        && previous.doc == draft.doc,
                );
                let unchanged = previous.sig_hash == u64_to_sql(draft.sig_hash)
                    && previous.body_hash == u64_to_sql(draft.body_hash);
                if !unchanged {
                    modified += 1;
                    changed_names.insert(draft.name.clone());
                }
                if previous.name != draft.name {
                    // The identity is kept but the name is not: for references, the old
                    // definition is gone and a new one appeared.
                    renamed.push(previous.id);
                    changed_names.insert(previous.name.clone());
                    changed_names.insert(draft.name.clone());
                    definition_names.insert(previous.name.clone());
                    definition_names.insert(draft.name.clone());
                }
            } else {
                let (id, attempt) = allocator.claim(key, 0, derive_id_with)?;
                ids.push(id);
                attempts.push(attempt);
                survivor.push(false);
                text_unchanged.push(false);
                added += 1;
                changed_names.insert(draft.name.clone());
                definition_names.insert(draft.name.clone());
            }
        }
        let mut removed = Vec::new();
        for previous in old {
            if !kept.contains(&previous.id) {
                removed.push(previous.id);
                changed_names.insert(previous.name.clone());
                definition_names.insert(previous.name.clone());
            }
        }
        Ok(Plan {
            keys,
            allocator,
            ids,
            attempts,
            survivor,
            text_unchanged,
            removed,
            added,
            modified,
            renamed,
            changed_names,
            definition_names,
        })
    }

    /// Writes every new symbol and its full-text row, and updates the surviving ones.
    fn write_symbols(
        &mut self,
        file_id: i64,
        extract: &FileExtract,
        plan: &mut Plan<'_>,
    ) -> Result<()> {
        for (index, draft) in extract.symbols.iter().enumerate() {
            let parent = draft
                .parent
                .filter(|&parent| parent < index)
                .map(|parent| plan.ids[parent]);
            let outline = encode_names(&draft.outline);
            let seq = i64::try_from(index).unwrap_or(i64::MAX);
            let ordinal = plan.keys[index].ordinal;
            let sig_hash = u64_to_sql(draft.sig_hash);
            let body_hash = u64_to_sql(draft.body_hash);
            let kind = draft.kind.as_str();
            let visibility = draft.visibility.as_str();
            loop {
                let id = plan.ids[index];
                let values: [&dyn ToSql; 18] = [
                    &id,
                    &file_id,
                    &seq,
                    &ordinal,
                    &draft.name,
                    &draft.qualified_name,
                    &kind,
                    &draft.signature,
                    &draft.doc,
                    &visibility,
                    &draft.span.start_line,
                    &draft.span.end_line,
                    &draft.span.start_byte,
                    &draft.span.end_byte,
                    &parent,
                    &outline,
                    &sig_hash,
                    &body_hash,
                ];
                if plan.survivor[index] {
                    self.update_symbol.execute(&values[..]).db()?;
                    break;
                }
                match self.insert_symbol.execute(&values[..]) {
                    Ok(_) => break,
                    Err(error) if is_primary_key_conflict(&error) => {
                        // The store already has a symbol with this id: take the next attempt.
                        let (next, attempt) = plan.allocator.claim(
                            &plan.keys[index],
                            plan.attempts[index] + 1,
                            derive_id_with,
                        )?;
                        plan.ids[index] = next;
                        plan.attempts[index] = attempt;
                    }
                    Err(error) => return Err(from_sqlite(&error)),
                }
            }
            let id = plan.ids[index];
            if plan.survivor[index] && plan.text_unchanged[index] {
                continue;
            }
            let mut columns: [String; 4] = Default::default();
            let [name, qualified, signature, doc] = &mut columns;
            let scratch = &mut self.scratch;
            index_text_into(name, &draft.name, MAX_INDEXED_BYTES, scratch);
            index_text_into(qualified, &draft.qualified_name, MAX_INDEXED_BYTES, scratch);
            index_text_into(signature, &draft.signature, MAX_INDEXED_BYTES, scratch);
            index_text_into(
                doc,
                draft.doc.as_deref().unwrap_or(""),
                MAX_INDEXED_BYTES,
                scratch,
            );
            self.pending_fts.push(FtsRow {
                id,
                replace: plan.survivor[index],
                columns,
            });
        }
        if self.pending_fts.len() >= FTS_BATCH_ROWS {
            self.flush_fts()?;
        }
        Ok(())
    }

    /// Writes the held-back full-text rows.
    ///
    /// The rows are written in ascending id order. FTS5 buffers new rows in memory only while
    /// their rowids increase; a smaller rowid forces the buffer out into a new index segment.
    /// Symbol ids are hashes, so in file order nearly every row would do that, and the index
    /// would be rebuilt from thousands of tiny segments. Sorted, a whole run of rows becomes one
    /// segment. The sort is stable, so rows of one id keep the order they were queued in.
    fn flush_fts(&mut self) -> Result<()> {
        let mut rows = std::mem::take(&mut self.pending_fts);
        rows.sort_by_key(|row| row.id);
        for row in &rows {
            if row.replace {
                self.fts_delete.execute(params![row.id]).db()?;
            }
            let [name, qualified, signature, doc] = &row.columns;
            self.fts_insert
                .execute(params![row.id, name, qualified, signature, doc])
                .db()?;
        }
        rows.clear();
        self.pending_fts = rows;
        Ok(())
    }

    /// Replaces the references of the file.
    fn write_references(
        &mut self,
        file_id: i64,
        extract: &FileExtract,
        plan: &Plan<'_>,
    ) -> Result<()> {
        self.delete_refs.execute(params![file_id]).db()?;
        for reference in &extract.references {
            let owner_index = reference.owner.filter(|&owner| owner < plan.ids.len());
            let owner = owner_index.map(|owner| plan.ids[owner]);
            let qualifier = reference.qualifier.as_deref().filter(|q| !q.is_empty());
            let no_self = owner_index.is_some_and(|owner| {
                must_not_link_to_self(&extract.symbols[owner].name, &reference.name, qualifier)
            });
            self.insert_ref
                .execute(params![
                    file_id,
                    owner,
                    reference.name,
                    reference.kind.as_str(),
                    reference.line,
                    qualifier,
                    i64::from(no_self),
                    receiver_word(qualifier),
                ])
                .db()?;
        }
        Ok(())
    }

    /// Marks stale the fresh memories anchored in this file whose anchor no longer matches, and
    /// returns how many there were.
    ///
    /// The current state of an anchor is the symbol it points to, or, when the symbol vanished and
    /// left the link empty, the symbol that now has its recorded qualified name in the file, or
    /// nothing. `Anchor::is_stale_against` decides.
    fn mark_stale_memories(
        &mut self,
        path: &str,
        extract: &FileExtract,
        plan: &Plan<'_>,
        now: i64,
    ) -> Result<u32> {
        let rows = self
            .anchors
            .query_map(params![path], |row| {
                let symbol: Option<i64> = row.get(1)?;
                let sig_hash: i64 = row.get(3)?;
                let body_hash: i64 = row.get(4)?;
                Ok((
                    row.get::<_, i64>(0)?,
                    Anchor {
                        symbol: symbol.map(SymbolId),
                        qualified_name: row.get(2)?,
                        path: path.to_owned(),
                        sig_hash: u64_from_sql(sig_hash),
                        body_hash: u64_from_sql(body_hash),
                    },
                ))
            })
            .db()?;
        let anchors = rows.collect::<rusqlite::Result<Vec<_>>>().db()?;
        if anchors.is_empty() {
            return Ok(0);
        }
        let mut by_id: HashMap<i64, (u64, u64)> = HashMap::new();
        let mut by_name: HashMap<&str, (u64, u64)> = HashMap::new();
        for (draft, id) in extract.symbols.iter().zip(&plan.ids) {
            let hashes = (draft.sig_hash, draft.body_hash);
            by_id.insert(*id, hashes);
            by_name
                .entry(draft.qualified_name.as_str())
                .or_insert(hashes);
        }
        let mut stale: BTreeSet<i64> = BTreeSet::new();
        for (memory, anchor) in &anchors {
            let current = match anchor.symbol {
                Some(symbol) => by_id.get(&symbol.0).copied(),
                None => by_name.get(anchor.qualified_name.as_str()).copied(),
            };
            if anchor.is_stale_against(current) {
                stale.insert(*memory);
            }
        }
        let mut marked = 0_u32;
        for memory in stale {
            let changed = self.mark_stale.execute(params![now, memory]).db()?;
            marked += u32::from(changed > 0);
        }
        Ok(marked)
    }

    /// Records the names whose set of definitions changed anywhere in the batch, unless no stored
    /// reference mentions them. Done once at the end, when every reference of the batch is in, so
    /// a name is looked up once however many symbols carry it. (Per file it would look at fewer
    /// references; more names than that are recorded, never fewer, and recording extra names only
    /// makes a later partial resolve do a little more work.)
    fn record_dirty_names(&mut self) -> Result<()> {
        for (name, family) in std::mem::take(&mut self.dirty_candidates) {
            self.dirty_name.execute(params![name, family]).db()?;
        }
        Ok(())
    }
}

/// Whether an error is a primary key conflict, the signal that a fresh symbol id is taken.
fn is_primary_key_conflict(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(failure, _)
            if failure.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_PRIMARYKEY
    )
}

/// The secondary indexes that are left out while a bulk load fills an empty index, and built
/// afterwards from the sorted data, which is much cheaper than maintaining them row by row.
/// (`idx_refs_file` and `idx_symbols_file_seq` stay: the loader itself uses them, and they only
/// ever grow at the end.)
const DEFERRED_INDEXES: [&str; 4] = [
    "idx_symbols_name",
    "idx_symbols_name_nocase",
    "idx_symbols_qname_nocase",
    "idx_refs_name",
];

/// A batch is treated as a bulk load when it brings at least this many symbols and references.
const BULK_LOAD_ROWS: usize = 20_000;

/// If the batch is a bulk load into an empty index, drops the deferred indexes and returns the
/// statements that create them again, read from the schema so they can never drift from the
/// migration. Otherwise returns nothing and drops nothing.
fn defer_indexes(tx: &Connection, files: &[(&FileInput, &FileExtract)]) -> Result<Vec<String>> {
    let rows: usize = files
        .iter()
        .map(|(_, extract)| extract.symbols.len() + extract.references.len())
        .sum();
    if rows < BULK_LOAD_ROWS {
        return Ok(Vec::new());
    }
    let occupied: bool = tx
        .query_row(
            "SELECT EXISTS (SELECT 1 FROM symbols) OR EXISTS (SELECT 1 FROM refs)",
            [],
            |row| row.get(0),
        )
        .db()?;
    if occupied {
        return Ok(Vec::new());
    }
    let mut create = Vec::new();
    for name in DEFERRED_INDEXES {
        if let Some(sql) = drop_index(tx, name)? {
            create.push(sql);
        }
    }
    Ok(create)
}

/// Replaces everything stored for the given files, atomically, and returns one outcome per file
/// in the same order.
///
/// A batch that fills an empty index with many rows leaves the deferred secondary indexes out
/// while it writes and builds them at the end, inside the same transaction.
///
/// # Errors
/// [`StorageError::Backend`] when SQLite fails (nothing of the batch is written), and
/// [`StorageError::Corrupt`] when stored rows cannot be decoded.
pub(crate) fn upsert_files(
    conn: &mut Connection,
    files: &[(&FileInput, &FileExtract)],
    now: i64,
) -> Result<Vec<UpsertOutcome>> {
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .db()?;
    let rebuild = defer_indexes(&tx, files)?;
    let outcomes = {
        let mut writer = Writer::new(&tx)?;
        let outcomes = files
            .iter()
            .map(|(file, extract)| writer.write_file(file, extract, now))
            .collect::<Result<Vec<_>>>()?;
        writer.flush_fts()?;
        for sql in &rebuild {
            tx.execute_batch(sql).db()?;
        }
        writer.record_dirty_names()?;
        outcomes
    };
    tx.commit().db()?;
    Ok(outcomes)
}
