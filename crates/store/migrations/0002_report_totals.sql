-- SPDX-License-Identifier: Apache-2.0
-- Purpose: schema version 2 of the pn-ultramemory store. It prepares the store for reports and
--          for fast bulk work: per-file parse-error counts, the files at both ends of every edge,
--          edges that no longer depend on foreign keys, and a larger full-text write buffer.
-- Migration: forward-only, applied to a version 1 database in one transaction. Everything that
--          version 1 stored is kept; what could not be known then gets an honest default.

-- Files stored by version 1 did not record how many syntax errors the parser recovered from, so
-- they get 0 until they are indexed again.
ALTER TABLE files ADD COLUMN parse_errors INTEGER NOT NULL DEFAULT 0;

-- `no_self` is 1 when the owner of a reference has the referenced name but the reference is
-- qualified by something other than "this symbol" (`db.save()` inside `save`): it must then not
-- link the owner to itself. It used to be worked out while resolving; now it is stored. The
-- backfill applies the same rule as the code: the qualifiers that mean "this symbol" are self,
-- this, Self, cls, $this and static.
ALTER TABLE refs ADD COLUMN no_self INTEGER NOT NULL DEFAULT 0;

UPDATE refs
SET no_self = 1
WHERE owner_id IS NOT NULL
  AND qualifier IS NOT NULL
  AND qualifier NOT IN ('self', 'this', 'Self', 'cls', '$this', 'static')
  AND name = (SELECT name FROM symbols WHERE symbols.id = refs.owner_id);

-- Edges are rebuilt without foreign keys and with the files of both ends. A table without foreign
-- keys can be emptied in one step, which is what a full resolve does, its rows are written without
-- a parent lookup each, and module reports read the files straight from the edge instead of
-- looking each symbol up. Only the resolver writes edges, and it only writes ids it just read
-- from `symbols`; the delete trigger below removes the edges of a deleted symbol.
CREATE TABLE edges_v2 (
    src        INTEGER NOT NULL,
    dst        INTEGER NOT NULL,
    kind       TEXT    NOT NULL,
    confidence INTEGER NOT NULL,
    line       INTEGER NOT NULL,
    src_file   INTEGER NOT NULL,
    dst_file   INTEGER NOT NULL,
    PRIMARY KEY (src, dst, kind)
) WITHOUT ROWID;

INSERT INTO edges_v2 (src, dst, kind, confidence, line, src_file, dst_file)
SELECT e.src, e.dst, e.kind, e.confidence, e.line, s.file_id, d.file_id
FROM edges e
JOIN symbols s ON s.id = e.src
JOIN symbols d ON d.id = e.dst;

DROP TABLE edges;
ALTER TABLE edges_v2 RENAME TO edges;
CREATE INDEX idx_edges_dst ON edges (dst, confidence);

DROP TRIGGER symbols_fts_delete;

-- Keep the full-text index and the edges in step with deletions of symbols, including the
-- cascaded ones (a deleted file deletes its symbols).
CREATE TRIGGER symbols_fts_delete AFTER DELETE ON symbols
BEGIN
    DELETE FROM symbol_fts WHERE rowid = old.id;
    DELETE FROM edges WHERE src = old.id;
    DELETE FROM edges WHERE dst = old.id;
END;

-- Rows of a bulk write are buffered in memory (up to 16 MiB) before an index segment is written,
-- instead of the default 1 MiB, so a large index is built from a few big segments.
INSERT INTO symbol_fts (symbol_fts, rank) VALUES ('hashsize', 16777216);
