// SPDX-License-Identifier: Apache-2.0
//! Connection setup and forward-only schema migrations.
//!
//! Every connection the adapter opens goes through [`configure`] (pragmas), [`migrate`] (bring the
//! file to the schema this build expects) and [`create_temp_tables`] (per-connection scratch
//! tables). Migrations are the numbered files in `crates/store/migrations/`, embedded with
//! `include_str!`, applied in order, each in its own `IMMEDIATE` transaction, and recorded in
//! `PRAGMA user_version`.
//!
//! # Invariants
//! * Migrations only move forward. A database whose `user_version` is **newer** than this build
//!   knows is refused with [`StorageError::Corrupt`], because writing to it could damage data
//!   this build does not understand.
//! * A file that is not ours is refused too: a non-empty database with `user_version` 0, or one
//!   whose `application_id` differs from [`APPLICATION_ID`].
//! * Two processes opening the same new file at once cannot apply a migration twice: the version
//!   is read again after the write lock is taken.

use std::time::{Duration, Instant};

use pn_ultramemory_core::StorageError;
use rusqlite::{Connection, ErrorCode, TransactionBehavior};

use crate::error::{DbResult, Result};

/// The identifier written to the database header (`PNUM`), so foreign files are recognized.
pub(crate) const APPLICATION_ID: i64 = 0x504E_554D;

/// How long a connection waits for another writer before failing, in milliseconds.
const BUSY_TIMEOUT_MS: u64 = 5_000;

/// Memory-mapped I/O window: 256 MiB.
const MMAP_SIZE_BYTES: u64 = 256 * 1024 * 1024;

/// Page cache size, as a negative number of KiB (SQLite convention): 64 MiB.
const CACHE_SIZE_KIB: i64 = -65_536;

/// The most the write-ahead log file may keep after a checkpoint: 64 MiB.
const JOURNAL_SIZE_LIMIT_BYTES: u64 = 64 * 1024 * 1024;

/// After how many pages the write-ahead log is folded back into the database file. The default
/// (1000 pages) checkpoints, and therefore syncs, after every few file updates during a bulk
/// index; 8192 pages (32 MiB) makes a full index several times faster and still keeps the log
/// small.
const WAL_AUTOCHECKPOINT_PAGES: u32 = 8_192;

/// How many prepared statements a connection keeps.
const STATEMENT_CACHE: usize = 128;

/// The migrations, in order: the version each one leads to, and its SQL.
const MIGRATIONS: &[(u32, &str)] = &[
    (1, include_str!("../migrations/0001_initial.sql")),
    (2, include_str!("../migrations/0002_report_totals.sql")),
    (3, include_str!("../migrations/0003_language_families.sql")),
];

/// The scratch tables of a connection.
const TEMP_TABLES: &str = include_str!("../sql/temp_tables.sql");

/// The schema version this build reads and writes: the version of the last migration.
pub(crate) const SCHEMA_VERSION: u32 = 3;

/// Runs a step that SQLite may refuse with "busy" without consulting the busy timeout (switching
/// a fresh file to WAL is one: several processes opening a new database together can hit it), and
/// retries it for as long as the busy timeout lasts.
fn retry_busy<T>(mut step: impl FnMut() -> rusqlite::Result<T>) -> rusqlite::Result<T> {
    let deadline = Instant::now() + Duration::from_millis(BUSY_TIMEOUT_MS);
    loop {
        match step() {
            Err(rusqlite::Error::SqliteFailure(failure, _))
                if matches!(
                    failure.code,
                    ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked
                ) && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(10));
            }
            other => return other,
        }
    }
}

/// Applies the pragmas every connection needs.
///
/// `persistent` is `false` for an in-memory database, which has no journal to switch to WAL.
///
/// Pragmas: `journal_mode=WAL` (readers do not block the writer), `synchronous=NORMAL` (safe with
/// WAL, far fewer flushes), `foreign_keys=ON` (file cascades and `SET NULL` on anchors depend on it),
/// `busy_timeout=5000`, `temp_store=MEMORY`, a 256 MiB `mmap_size` and a 64 MiB page cache.
pub(crate) fn configure(conn: &Connection, persistent: bool) -> Result<()> {
    conn.busy_timeout(Duration::from_millis(BUSY_TIMEOUT_MS))
        .db()?;
    if persistent {
        // The answer is "wal", or the old mode on a file system that cannot do WAL; either way
        // the database works, so the answer is only read to drain the row.
        retry_busy(|| {
            conn.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get::<_, String>(0))
        })
        .db()?;
        conn.execute_batch(&format!(
            "PRAGMA synchronous = NORMAL; \
             PRAGMA wal_autocheckpoint = {WAL_AUTOCHECKPOINT_PAGES}; \
             PRAGMA journal_size_limit = {JOURNAL_SIZE_LIMIT_BYTES};"
        ))
        .db()?;
    }
    conn.execute_batch(&format!(
        "PRAGMA foreign_keys = ON;
         PRAGMA temp_store = MEMORY;
         PRAGMA mmap_size = {MMAP_SIZE_BYTES};
         PRAGMA cache_size = {CACHE_SIZE_KIB};"
    ))
    .db()?;
    conn.set_prepared_statement_cache_capacity(STATEMENT_CACHE);
    Ok(())
}

/// Reads `PRAGMA user_version`.
fn user_version(conn: &Connection) -> Result<u32> {
    let raw: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .db()?;
    u32::try_from(raw)
        .map_err(|_| StorageError::Corrupt(format!("the schema version {raw} is not valid")))
}

/// Refuses a database that is not one of ours.
///
/// A database with a version has our application id (both are written by the first migration, in
/// one transaction). An unversioned one must be empty: anything already in it belongs to someone
/// else. Callers that may race with another process opening the same new file must call this
/// with the write lock held, so that the file cannot change between the check and the migration.
fn check_ownership(conn: &Connection, version: u32) -> Result<()> {
    let foreign = || {
        StorageError::Corrupt(
            "the file is a database that pn-ultramemory did not create".to_owned(),
        )
    };
    if version == 0 {
        let objects: i64 = conn
            .query_row("SELECT count(*) FROM sqlite_schema", [], |row| row.get(0))
            .db()?;
        return if objects > 0 { Err(foreign()) } else { Ok(()) };
    }
    let application_id: i64 = conn
        .pragma_query_value(None, "application_id", |row| row.get(0))
        .db()?;
    if application_id == APPLICATION_ID {
        Ok(())
    } else {
        Err(foreign())
    }
}

/// Brings the database to the schema of this build.
///
/// # Errors
/// [`StorageError::Corrupt`] when the database is newer than this build, is not ours, or is not a
/// database at all; [`StorageError::Backend`] when applying a migration fails (nothing of that
/// migration is kept).
pub(crate) fn migrate(conn: &mut Connection) -> Result<()> {
    let latest = SCHEMA_VERSION;
    let version = user_version(conn)?;
    if version > latest {
        return Err(StorageError::Corrupt(format!(
            "the database has schema version {version}, but this build only understands up to \
             version {latest}; upgrade pn-ultramemory instead of opening it with an older build"
        )));
    }
    if version > 0 {
        check_ownership(conn, version)?;
    }
    if version == latest {
        return Ok(());
    }
    for (target, sql) in MIGRATIONS.iter().filter(|(target, _)| *target > version) {
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .db()?;
        // Another process may have applied this migration while this one waited for the lock, so
        // the version is read again, and an unversioned file is judged, with the lock held.
        let current = user_version(&tx)?;
        if current >= *target {
            tx.commit().db()?;
            continue;
        }
        check_ownership(&tx, current)?;
        tx.execute_batch(sql).db()?;
        tx.execute_batch(&format!("PRAGMA user_version = {target};"))
            .db()?;
        tx.commit().db()?;
    }
    Ok(())
}

/// Creates the per-connection scratch tables.
pub(crate) fn create_temp_tables(conn: &Connection) -> Result<()> {
    conn.execute_batch(TEMP_TABLES).db()
}

#[cfg(test)]
mod tests {
    use super::{APPLICATION_ID, MIGRATIONS, SCHEMA_VERSION, configure, create_temp_tables};
    use super::{migrate, user_version};
    use pn_ultramemory_core::StorageError;
    use rusqlite::Connection;

    /// A configured, migrated in-memory connection.
    fn ready() -> Connection {
        let mut conn = Connection::open_in_memory().expect("in-memory database");
        configure(&conn, false).expect("pragmas");
        migrate(&mut conn).expect("migrations");
        create_temp_tables(&conn).expect("temp tables");
        conn
    }

    /// Migration numbers start at one and increase by one, so `user_version` is meaningful.
    #[test]
    fn migrations_are_numbered_consecutively() {
        for (index, (version, sql)) in MIGRATIONS.iter().enumerate() {
            assert_eq!(*version as usize, index + 1);
            assert!(sql.starts_with("-- SPDX-License-Identifier: Apache-2.0"));
        }
        assert_eq!(SCHEMA_VERSION as usize, MIGRATIONS.len());
        assert_eq!(
            MIGRATIONS.last().map(|(version, _)| *version),
            Some(SCHEMA_VERSION)
        );
    }

    /// A fresh database ends at the latest version and carries our application id.
    #[test]
    fn migrates_a_fresh_database() {
        let conn = ready();
        assert_eq!(user_version(&conn).expect("version"), SCHEMA_VERSION);
        let id: i64 = conn
            .pragma_query_value(None, "application_id", |r| r.get(0))
            .expect("application id");
        assert_eq!(id, APPLICATION_ID);
    }

    /// Migrating again changes nothing and does not fail.
    #[test]
    fn migrating_twice_is_a_no_op() {
        let mut conn = ready();
        migrate(&mut conn).expect("second run");
        assert_eq!(user_version(&conn).expect("version"), SCHEMA_VERSION);
    }

    /// The pragmas take effect on the connection.
    #[test]
    fn pragmas_are_applied() {
        let conn = ready();
        let get = |name: &str| -> i64 {
            conn.pragma_query_value(None, name, |r| r.get(0))
                .expect("pragma")
        };
        assert_eq!(get("foreign_keys"), 1);
        assert_eq!(get("temp_store"), 2, "2 means MEMORY");
        assert_eq!(get("busy_timeout"), 5_000);
        assert_eq!(get("cache_size"), -65_536);
    }

    /// A database from a newer build is refused as corrupt, and left untouched.
    #[test]
    fn refuses_a_newer_database() {
        let mut conn = ready();
        conn.execute_batch("PRAGMA user_version = 99;")
            .expect("set");
        let error = migrate(&mut conn).expect_err("newer must be refused");
        assert!(matches!(error, StorageError::Corrupt(_)), "{error:?}");
        assert_eq!(user_version(&conn).expect("version"), 99);
    }

    /// A database that already holds foreign tables is refused.
    #[test]
    fn refuses_a_foreign_database() {
        let mut conn = Connection::open_in_memory().expect("open");
        conn.execute_batch("CREATE TABLE notes (x TEXT);")
            .expect("create");
        let error = migrate(&mut conn).expect_err("foreign must be refused");
        assert!(matches!(error, StorageError::Corrupt(_)), "{error:?}");
    }

    /// A versioned database with another application id is refused.
    #[test]
    fn refuses_a_foreign_application_id() {
        let mut conn = Connection::open_in_memory().expect("open");
        conn.execute_batch("PRAGMA user_version = 1; PRAGMA application_id = 5;")
            .expect("set");
        let error = migrate(&mut conn).expect_err("foreign must be refused");
        assert!(matches!(error, StorageError::Corrupt(_)), "{error:?}");
    }

    /// The scratch tables can be created repeatedly on one connection.
    #[test]
    fn temp_tables_are_idempotent() {
        let conn = ready();
        create_temp_tables(&conn).expect("again");
        let count: i64 = conn
            .query_row("SELECT count(*) FROM tmp_refs", [], |r| r.get(0))
            .expect("count");
        assert_eq!(count, 0);
    }
}
