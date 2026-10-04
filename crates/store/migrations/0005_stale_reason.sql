-- SPDX-License-Identifier: Apache-2.0
-- Purpose: schema version 5 of the pn-ultramemory store. Memories record why they went stale so a
--          brief can tell an agent whether the symbol vanished, its signature changed, or only its
--          body changed, instead of showing a bare counter.
-- Migration: forward-only, applied to a version 4 database in one transaction. Existing stale
--          memories keep NULL as their reason; new ones are written by mark_stale_memories going
--          forward. Reanchoring clears both stale_since and stale_reason together.

ALTER TABLE memories ADD COLUMN stale_reason TEXT;