-- SPDX-License-Identifier: Apache-2.0
-- Purpose: schema version 3 of the pn-ultramemory store. A reference only resolves to symbols in
--          files of the same language family (core::Language::family), so the store keeps the
--          family of every file, and remembers per family which names changed definition.
-- Migration: forward-only, applied to a version 2 database in one transaction. Edges that link
--          across families (the false links this version stops making) are removed, and every
--          name is marked as changed, so the next partial resolve redoes the confidence tiers,
--          which now count candidates within the family only.

-- The family of the file's language. The backfill applies the same mapping as the code
-- (JavaScript, TypeScript and TSX are one family, C and C++ another, every other language is its
-- own); a test compares the two for every language.
ALTER TABLE files ADD COLUMN family TEXT NOT NULL DEFAULT '';

UPDATE files
SET family = CASE language
    WHEN 'javascript' THEN 'ecmascript'
    WHEN 'typescript' THEN 'ecmascript'
    WHEN 'tsx' THEN 'ecmascript'
    WHEN 'c' THEN 'c-family'
    WHEN 'cpp' THEN 'c-family'
    ELSE language
END;

-- Names whose set of definitions changed, now per family: a name defined in one family must not
-- make references of another family be resolved again.
DROP TABLE dirty_names;

CREATE TABLE dirty_names (
    name   TEXT NOT NULL,
    family TEXT NOT NULL,
    PRIMARY KEY (name, family)
) WITHOUT ROWID;

-- Edges between files of different families are not valid any more.
DELETE FROM edges
WHERE (SELECT family FROM files WHERE files.id = edges.src_file)
   <> (SELECT family FROM files WHERE files.id = edges.dst_file);

-- Every name of every family is treated as changed, so the next partial resolve revisits all the
-- references that could resolve differently now.
INSERT INTO dirty_names (name, family)
SELECT DISTINCT s.name, f.family
FROM symbols s
JOIN files f ON f.id = s.file_id;
