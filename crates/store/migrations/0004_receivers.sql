-- SPDX-License-Identifier: Apache-2.0
-- Purpose: schema version 4 of the pn-ultramemory store. A reference remembers the last word of
--          its receiver (`couponservice` for `$this->couponService->validate()`), so resolution can
--          tell a call on one of the repository's own classes from a call on a library object that
--          only shares the method's name, such as Laravel's `$request->validate()`.
-- Migration: forward-only, applied to a version 3 database in one transaction. References stored
--          by earlier versions did not record their receiver, and the word cannot be worked out
--          in SQL from every language's syntax, so they keep no word, which resolves them exactly
--          as before; every file is marked as changed, so the next `index` reads it again and
--          records the words.

ALTER TABLE refs ADD COLUMN recv TEXT;

UPDATE files SET hash = '';
