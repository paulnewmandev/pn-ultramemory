// SPDX-License-Identifier: Apache-2.0
//! What was learned: utility estimates, the signal history and co-access strength.
//!
//! The rules of learning (how a signal changes an estimate, how evidence fades) live in the
//! domain crate. This module only stores and retrieves the state, and applies the one rule the
//! storage port assigns to it: co-access strength decays with `decay_factor` and
//! `DEFAULT_HALF_LIFE_SECS` whenever it is read or bumped.
//!
//! # Tables
//! * `utility`: one row per target, `(alpha, beta, updated_at)`, replaced as a whole.
//! * `signals`: append-only history, only counted and inspected.
//! * `coaccess`: an unordered pair stored once with `a < b`, a weight and the time it was last
//!   updated.
//!
//! Invariants: non-finite numbers are refused instead of stored; a pair of a symbol with itself
//! is ignored; strength never goes below zero.

use pn_ultramemory_core::{
    DEFAULT_HALF_LIFE_SECS, LearningStatus, SignalKind, StorageError, SymbolId, Target,
    UtilityState, decay_factor,
};
use rusqlite::{Connection, TransactionBehavior, params};

use crate::error::{DbResult, Result};
use crate::query::{count, execute, query_all, query_opt};

/// Refuses a number that cannot be stored or compared.
fn finite(value: f64, what: &str) -> Result<f64> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(StorageError::Backend(format!(
            "{what} must be a finite number"
        )))
    }
}

/// The stored utility state of each target that has one, in the order the targets were given
/// (each target once).
pub(crate) fn utility_states(
    conn: &Connection,
    targets: &[Target],
) -> Result<Vec<(Target, UtilityState)>> {
    let mut seen: std::collections::HashSet<(&'static str, i64)> = std::collections::HashSet::new();
    let mut out = Vec::new();
    for target in targets {
        if !seen.insert((target.kind_str(), target.raw_id())) {
            continue;
        }
        let state = query_opt(
            conn,
            "SELECT alpha, beta, updated_at FROM utility WHERE target_kind = ?1 AND target_id = ?2",
            params![target.kind_str(), target.raw_id()],
            |row| {
                Ok(UtilityState {
                    alpha: row.get(0)?,
                    beta: row.get(1)?,
                    updated_at: row.get(2)?,
                })
            },
        )?;
        if let Some(state) = state {
            out.push((*target, state));
        }
    }
    Ok(out)
}

/// Stores the utility state of a target, replacing any previous one.
///
/// # Errors
/// [`StorageError::Backend`] when `alpha` or `beta` is not finite, or when SQLite fails.
pub(crate) fn put_utility_state(
    conn: &Connection,
    target: Target,
    state: UtilityState,
) -> Result<()> {
    let alpha = finite(state.alpha, "alpha")?;
    let beta = finite(state.beta, "beta")?;
    execute(
        conn,
        "INSERT INTO utility (target_kind, target_id, alpha, beta, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5) \
         ON CONFLICT (target_kind, target_id) DO UPDATE SET \
             alpha = excluded.alpha, beta = excluded.beta, updated_at = excluded.updated_at",
        params![
            target.kind_str(),
            target.raw_id(),
            alpha,
            beta,
            state.updated_at
        ],
    )?;
    Ok(())
}

/// Appends one signal to the history.
pub(crate) fn log_signal(
    conn: &Connection,
    target: Target,
    kind: SignalKind,
    now: i64,
) -> Result<()> {
    execute(
        conn,
        "INSERT INTO signals (target_kind, target_id, kind, at) VALUES (?1, ?2, ?3, ?4)",
        params![target.kind_str(), target.raw_id(), kind.as_str(), now],
    )?;
    Ok(())
}

/// Adds `weight` to the strength of a pair, after decaying what was stored up to `now`.
///
/// # Errors
/// [`StorageError::Backend`] when `weight` is not finite, or when SQLite fails.
pub(crate) fn bump_coaccess(
    conn: &mut Connection,
    a: SymbolId,
    b: SymbolId,
    weight: f64,
    now: i64,
) -> Result<()> {
    let weight = finite(weight, "weight")?;
    if a == b {
        return Ok(());
    }
    let (low, high) = if a.0 < b.0 { (a.0, b.0) } else { (b.0, a.0) };
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .db()?;
    let previous = query_opt(
        &tx,
        "SELECT weight, updated_at FROM coaccess WHERE a = ?1 AND b = ?2",
        params![low, high],
        |row| Ok((row.get::<_, f64>(0)?, row.get::<_, i64>(1)?)),
    )?;
    let (decayed, stamp) = match previous {
        Some((stored, updated_at)) => (
            stored * decay_factor(now.saturating_sub(updated_at), DEFAULT_HALF_LIFE_SECS),
            now.max(updated_at),
        ),
        None => (0.0, now),
    };
    let strength = (decayed + weight).max(0.0);
    execute(
        &tx,
        "INSERT INTO coaccess (a, b, weight, updated_at) VALUES (?1, ?2, ?3, ?4) \
         ON CONFLICT (a, b) DO UPDATE SET weight = excluded.weight, updated_at = excluded.updated_at",
        params![low, high, strength, stamp],
    )?;
    tx.commit().db()?;
    Ok(())
}

/// The symbols most strongly co-accessed with `id`, strength decayed to `now`, strongest first
/// (ties by symbol id). Pairs whose strength has decayed to nothing are left out.
pub(crate) fn coaccess_neighbors(
    conn: &Connection,
    id: SymbolId,
    limit: usize,
    now: i64,
) -> Result<Vec<(SymbolId, f64)>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let mut pairs = query_all(
        conn,
        "SELECT b, weight, updated_at FROM coaccess WHERE a = ?1 \
         UNION ALL \
         SELECT a, weight, updated_at FROM coaccess WHERE b = ?1",
        params![id.0],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, f64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        },
    )?
    .into_iter()
    .map(|(other, weight, updated_at)| {
        let strength =
            weight * decay_factor(now.saturating_sub(updated_at), DEFAULT_HALF_LIFE_SECS);
        (SymbolId(other), strength)
    })
    .filter(|(_, strength)| *strength > 0.0)
    .collect::<Vec<_>>();
    pairs.sort_by(|left, right| right.1.total_cmp(&left.1).then(left.0.cmp(&right.0)));
    pairs.truncate(limit);
    Ok(pairs)
}

/// Counts what has been learned.
pub(crate) fn learning_status(conn: &Connection) -> Result<LearningStatus> {
    Ok(LearningStatus {
        signals: count(conn, "SELECT COUNT(*) FROM signals")?,
        tracked_targets: count(conn, "SELECT COUNT(*) FROM utility")?,
        coaccess_pairs: count(conn, "SELECT COUNT(*) FROM coaccess")?,
    })
}

/// Forgets utility states, signals and co-access; symbols and memories are untouched.
pub(crate) fn reset_learning(conn: &mut Connection) -> Result<()> {
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .db()?;
    for table in ["utility", "signals", "coaccess"] {
        execute(&tx, &format!("DELETE FROM {table}"), [])?;
    }
    tx.commit().db()?;
    Ok(())
}
