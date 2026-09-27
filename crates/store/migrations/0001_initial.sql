-- SPDX-License-Identifier: Apache-2.0
-- Purpose: schema version 1 of the pn-ultramemory store. It creates every persistent table, index,
--          full-text index and trigger the SQLite adapter needs. Migrations are forward-only and
--          applied in file order; the applied version is recorded in `PRAGMA user_version`.
-- Layout:  files -> symbols -> edges (the code graph), refs (unresolved references), memories and
--          memory_anchors (what was learned about the code), utility / signals / coaccess (local
--          learning), meta (key/value), dirty_names (definitions changed since the last resolve).
-- Notes:   `refs` is the table of references; `references` is an SQL keyword, so it is not used.
--          Full-text tables are contentless: they hold only the pre-split tokens computed in Rust,
--          keyed by the rowid of the row they describe, and are kept in sync by the adapter and by
--          the delete triggers below.

-- Marks the file as a pn-ultramemory database ("PNUM"), so foreign databases are refused.
PRAGMA application_id = 1347310925;

-- One row per indexed source file. `id` is stable across re-indexing of the same path.
CREATE TABLE files (
    id           INTEGER PRIMARY KEY,
    path         TEXT    NOT NULL UNIQUE,
    language     TEXT    NOT NULL,
    hash         TEXT    NOT NULL,
    size         INTEGER NOT NULL,
    mtime        INTEGER NOT NULL,
    lines        INTEGER NOT NULL,
    symbol_count INTEGER NOT NULL DEFAULT 0,
    indexed_at   INTEGER NOT NULL
);

-- One row per declared symbol. `id` is derived from (path, qualified_name, kind, ordinal) with
-- core::hash64 and masked to 63 bits, so it survives re-indexing. `seq` is the position in the
-- extractor's source order. `parent_id` is a soft link to a symbol of the same file (rewritten
-- with the rest of the file on every upsert). Hashes are u64 values stored bit for bit.
CREATE TABLE symbols (
    id             INTEGER PRIMARY KEY,
    file_id        INTEGER NOT NULL REFERENCES files (id) ON DELETE CASCADE,
    seq            INTEGER NOT NULL,
    ordinal        INTEGER NOT NULL,
    name           TEXT    NOT NULL,
    qualified_name TEXT    NOT NULL,
    kind           TEXT    NOT NULL,
    signature      TEXT    NOT NULL,
    doc            TEXT,
    visibility     TEXT    NOT NULL,
    start_line     INTEGER NOT NULL,
    end_line       INTEGER NOT NULL,
    start_byte     INTEGER NOT NULL,
    end_byte       INTEGER NOT NULL,
    parent_id      INTEGER,
    outline        TEXT    NOT NULL,
    sig_hash       INTEGER NOT NULL,
    body_hash      INTEGER NOT NULL
);

-- Source order within a file, and file cascades.
CREATE INDEX idx_symbols_file_seq ON symbols (file_id, seq);
-- Name lookups of the resolver: all symbols of a name, or the ones of a name in one file.
CREATE INDEX idx_symbols_name ON symbols (name, file_id);
-- Case-insensitive exact lookups (`find_symbols`).
CREATE INDEX idx_symbols_name_nocase ON symbols (name COLLATE NOCASE);
CREATE INDEX idx_symbols_qname_nocase ON symbols (qualified_name COLLATE NOCASE);

-- Unresolved references found inside a file. `owner_id` is the symbol that contains the
-- reference (NULL at file level, which never produces an edge); `kind` is a core::RefKind name.
CREATE TABLE refs (
    id        INTEGER PRIMARY KEY,
    file_id   INTEGER NOT NULL REFERENCES files (id) ON DELETE CASCADE,
    owner_id  INTEGER,
    name      TEXT    NOT NULL,
    kind      TEXT    NOT NULL,
    line      INTEGER NOT NULL,
    qualifier TEXT
);

CREATE INDEX idx_refs_name ON refs (name);
CREATE INDEX idx_refs_file ON refs (file_id);

-- Resolved relationships between symbols. Unique per (src, dst, kind); `confidence` is
-- 0 guess, 1 heuristic, 2 resolved, 3 exact. Clustered by `src`, and indexed by `dst`.
CREATE TABLE edges (
    src        INTEGER NOT NULL REFERENCES symbols (id) ON DELETE CASCADE,
    dst        INTEGER NOT NULL REFERENCES symbols (id) ON DELETE CASCADE,
    kind       TEXT    NOT NULL,
    confidence INTEGER NOT NULL,
    line       INTEGER NOT NULL,
    PRIMARY KEY (src, dst, kind)
) WITHOUT ROWID;

CREATE INDEX idx_edges_dst ON edges (dst, confidence);

-- Names whose definitions changed since the last time references were resolved.
CREATE TABLE dirty_names (
    name TEXT PRIMARY KEY
) WITHOUT ROWID;

-- Memories. AUTOINCREMENT so an identity is never reused for a different memory.
CREATE TABLE memories (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    kind        TEXT    NOT NULL,
    text        TEXT    NOT NULL,
    provenance  TEXT    NOT NULL,
    created_at  INTEGER NOT NULL,
    stale_since INTEGER
);

CREATE INDEX idx_memories_created ON memories (created_at DESC, id DESC);

-- The symbols a memory is about, with what they looked like when the memory was written.
-- `symbol_id` becomes NULL when the symbol disappears; the recorded values are kept.
CREATE TABLE memory_anchors (
    memory_id      INTEGER NOT NULL REFERENCES memories (id) ON DELETE CASCADE,
    ordinal        INTEGER NOT NULL,
    symbol_id      INTEGER REFERENCES symbols (id) ON DELETE SET NULL,
    qualified_name TEXT    NOT NULL,
    path           TEXT    NOT NULL,
    sig_hash       INTEGER NOT NULL,
    body_hash      INTEGER NOT NULL,
    PRIMARY KEY (memory_id, ordinal)
) WITHOUT ROWID;

CREATE INDEX idx_anchors_symbol ON memory_anchors (symbol_id);
CREATE INDEX idx_anchors_path ON memory_anchors (path, qualified_name);

-- Full-text index of symbols. Columns hold identifier tokens already split at camelCase,
-- snake_case and digit boundaries and lowercased. Contentless: only the index is stored.
CREATE VIRTUAL TABLE symbol_fts USING fts5 (
    name,
    qname,
    sig,
    doc,
    content = '',
    contentless_delete = 1,
    tokenize = 'unicode61 remove_diacritics 2',
    prefix = '2 3'
);

-- Full-text index of memory text, built the same way.
CREATE VIRTUAL TABLE memory_fts USING fts5 (
    text,
    content = '',
    contentless_delete = 1,
    tokenize = 'unicode61 remove_diacritics 2',
    prefix = '2 3'
);

-- Keep the full-text indexes in step with deletions, including cascaded ones.
CREATE TRIGGER symbols_fts_delete AFTER DELETE ON symbols
BEGIN
    DELETE FROM symbol_fts WHERE rowid = old.id;
END;

CREATE TRIGGER memories_fts_delete AFTER DELETE ON memories
BEGIN
    DELETE FROM memory_fts WHERE rowid = old.id;
END;

-- Decayed evidence about how useful a symbol or memory is when recalled.
CREATE TABLE utility (
    target_kind TEXT    NOT NULL,
    target_id   INTEGER NOT NULL,
    alpha       REAL    NOT NULL,
    beta        REAL    NOT NULL,
    updated_at  INTEGER NOT NULL,
    PRIMARY KEY (target_kind, target_id)
) WITHOUT ROWID;

-- Append-only history of signals, used for counting and inspection.
CREATE TABLE signals (
    id          INTEGER PRIMARY KEY,
    target_kind TEXT    NOT NULL,
    target_id   INTEGER NOT NULL,
    kind        TEXT    NOT NULL,
    at          INTEGER NOT NULL
);

CREATE INDEX idx_signals_target ON signals (target_kind, target_id);

-- Strength of "used together" between two symbols. The pair is stored once with a < b.
CREATE TABLE coaccess (
    a          INTEGER NOT NULL,
    b          INTEGER NOT NULL,
    weight     REAL    NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (a, b),
    CHECK (a < b)
) WITHOUT ROWID;

CREATE INDEX idx_coaccess_b ON coaccess (b);

-- Free-form key/value metadata, such as the repository root.
CREATE TABLE meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
) WITHOUT ROWID;
