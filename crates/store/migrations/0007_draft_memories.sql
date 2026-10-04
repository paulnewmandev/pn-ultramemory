-- SPDX-License-Identifier: Apache-2.0
-- Purpose: schema version 7 of the pn-ultramemory store. Draft memories capture decisions the
--          system observed but has not yet confirmed, so an agent can review and accept them
--          without having to restate what it already did.
-- Migration: forward-only, applied to a version 6 database in one transaction. The table is
--          empty on creation; drafts are written by the engine when it detects an expand-after-
--          recall pattern and cleared when confirmed, discarded or expired.

CREATE TABLE draft_memories (
    id INTEGER PRIMARY KEY,
    kind TEXT NOT NULL,
    text TEXT NOT NULL,
    about_symbols TEXT NOT NULL DEFAULT '',
    suggested_at INTEGER NOT NULL,
    source_query TEXT NOT NULL DEFAULT ''
);