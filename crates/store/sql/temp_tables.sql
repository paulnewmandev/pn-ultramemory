-- SPDX-License-Identifier: Apache-2.0
-- Purpose: scratch tables of the pn-ultramemory store. They live in the per-connection TEMP schema
--          (kept in memory), are created every time a connection is opened, and are emptied at the
--          end of the operation that fills them. They let large sets (the present paths, the
--          references being resolved, a list of symbol ids) take part in set-based SQL without
--          binding thousands of parameters.

-- Files whose references must be resolved again.
CREATE TEMP TABLE IF NOT EXISTS tmp_scope_files (
    file_id INTEGER PRIMARY KEY
);

-- Names whose references must be resolved again, in every language family.
CREATE TEMP TABLE IF NOT EXISTS tmp_scope_names (
    name TEXT PRIMARY KEY
) WITHOUT ROWID;

-- Names whose references must be resolved again, in one language family only.
CREATE TEMP TABLE IF NOT EXISTS tmp_scope_dirty (
    name   TEXT NOT NULL,
    family TEXT NOT NULL,
    PRIMARY KEY (name, family)
) WITHOUT ROWID;

-- The references being resolved. `kind` is already an edge kind name, `excl` is 1 when the
-- owner itself must not be a candidate, `family` is the language family of the reference's file,
-- `nsame` is the number of candidates in the same file and `cnt` the number of candidates in the
-- family (the owner not counted when excluded). `recv` is the last word of the receiver (NULL for
-- none, empty for an expression) and `hint` the one candidate that word points at, if exactly one.
CREATE TEMP TABLE IF NOT EXISTS tmp_refs (
    rid     INTEGER PRIMARY KEY,
    file_id INTEGER NOT NULL,
    owner   INTEGER NOT NULL,
    name    TEXT    NOT NULL,
    kind    TEXT    NOT NULL,
    line    INTEGER NOT NULL,
    excl    INTEGER NOT NULL,
    family  TEXT    NOT NULL,
    nsame   INTEGER NOT NULL,
    cnt     INTEGER NOT NULL,
    recv    TEXT,
    hint    INTEGER
);

-- The candidate a receiver word points at, worked out once per name, family and word: a
-- repository writes `Vec::new()` thousands of times and the answer is the same every time.
CREATE TEMP TABLE IF NOT EXISTS tmp_hints (
    name   TEXT NOT NULL,
    family TEXT NOT NULL,
    recv   TEXT NOT NULL,
    hint   INTEGER,
    PRIMARY KEY (name, family, recv)
) WITHOUT ROWID;

-- How many symbols carry each name, per language family.
CREATE TEMP TABLE IF NOT EXISTS tmp_names (
    name   TEXT NOT NULL,
    family TEXT NOT NULL,
    cnt    INTEGER NOT NULL,
    PRIMARY KEY (name, family)
) WITHOUT ROWID;

-- The best few candidates of an ambiguous name, per referencing file, best first; `sfile` is the
-- file of the candidate.
CREATE TEMP TABLE IF NOT EXISTS tmp_pick (
    name    TEXT    NOT NULL,
    file_id INTEGER NOT NULL,
    rank    INTEGER NOT NULL,
    sid     INTEGER NOT NULL,
    sfile   INTEGER NOT NULL,
    PRIMARY KEY (name, file_id, rank)
) WITHOUT ROWID;

-- A set of symbol ids given by the caller.
CREATE TEMP TABLE IF NOT EXISTS tmp_ids (
    id INTEGER PRIMARY KEY
);

-- The paths that are still present in the source tree.
CREATE TEMP TABLE IF NOT EXISTS tmp_paths (
    path TEXT PRIMARY KEY
) WITHOUT ROWID;

-- The module (directory prefix) of every file, for the report aggregates, and its number.
CREATE TEMP TABLE IF NOT EXISTS tmp_modules (
    file_id INTEGER PRIMARY KEY,
    module  TEXT NOT NULL,
    mid     INTEGER
);

-- The distinct module names, numbered in name order.
CREATE TEMP TABLE IF NOT EXISTS tmp_module_names (
    mid  INTEGER PRIMARY KEY,
    name TEXT NOT NULL
);

-- Edges between different modules, summed per ordered pair of module numbers.
CREATE TEMP TABLE IF NOT EXISTS tmp_pairs (
    from_mid INTEGER NOT NULL,
    to_mid   INTEGER NOT NULL,
    weight   INTEGER NOT NULL,
    PRIMARY KEY (from_mid, to_mid)
) WITHOUT ROWID;

-- The files being removed.
CREATE TEMP TABLE IF NOT EXISTS tmp_removed (
    file_id INTEGER PRIMARY KEY
);
