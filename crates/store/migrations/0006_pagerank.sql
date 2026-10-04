-- SPDX-License-Identifier: Apache-2.0
-- Purpose: schema version 6 of the pn-ultramemory store. Symbols carry a precomputed PageRank
--          score so recall can rank structural hubs even when no keyword seed points at them.
-- Migration: forward-only, applied to a version 5 database in one transaction. Existing symbols
--          start at 0.0; the next full index populates the column via update_pageranks.

ALTER TABLE symbols ADD COLUMN pagerank REAL NOT NULL DEFAULT 0.0;