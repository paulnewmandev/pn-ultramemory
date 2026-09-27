// SPDX-License-Identifier: Apache-2.0
//! Learning from what the agent does with what it was shown.
//!
//! Learning here means updating small local statistics; nothing is trained and nothing leaves the
//! machine. This module defines the session record and the three hooks the retrieval operations
//! call: [`Engine::note_shown`] after a capsule is built, [`Engine::note_expanded`] when the agent
//! asks for a symbol, and [`Engine::multipliers`] to bias ranking with what was learned. It also
//! offers the explicit operations: [`Engine::feedback`], [`Engine::utility_of`],
//! [`Engine::learning_status`] and [`Engine::reset_learning`].
//!
//! # The rules
//! 1. **Ignored.** When a new capsule is shown, every symbol of the *previous* capsule that was
//!    shown at level L1 or richer, was not expanded and was shown less than 30 minutes ago gets a
//!    weak [`SignalKind::Ignored`] (at most 20 per settlement). A capsule that is followed by
//!    silence for longer than that says nothing about relevance, so it teaches nothing.
//! 2. **Used.** Expanding a symbol records [`SignalKind::Used`] for it and strengthens its
//!    co-access with each of the last eight other symbols expanded in the session.
//! 3. **Bounded.** A signal updates a decayed Beta estimate, and the estimate becomes a ranking
//!    multiplier in `[0.5, 1.5]`. Learning re-ranks; it never creates facts.
//!
//! # Failure
//! The hooks run inside retrieval and must never make it fail: storage errors inside them are
//! swallowed and counted (see the unit tests). The explicit operations report their errors.
//!
//! # Concurrency
//! The session lock is held for the whole of a hook, including its storage writes, so concurrent
//! hooks and feedback calls cannot lose each other's updates.

use std::collections::{HashMap, VecDeque};

use pn_ultramemory_core::{
    Detail, LearningStatus, MemoryId, SignalKind, StorageError, SymbolId, Target, UtilityState,
};
use serde_json::{Value, json};

use crate::engine::Engine;
use crate::error::EngineError;

/// A capsule counts as ignored only if the next one comes within this many seconds.
const IGNORE_WINDOW_SECS: i64 = 30 * 60;

/// The most symbols one settlement marks as ignored.
const MAX_IGNORED_PER_SETTLEMENT: usize = 20;

/// How many other symbols an expansion is paired with.
#[allow(
    dead_code,
    reason = "read by `note_expanded`, whose caller lands with the expand module"
)]
const COACCESS_WINDOW: usize = 8;

/// Evidence below this after decay counts as no evidence: the target is not re-ranked.
const MIN_EVIDENCE: f64 = 0.05;

/// What was shown to the agent recently and what it did next.
#[derive(Debug, Default)]
pub(crate) struct Session {
    /// The symbols of the most recent capsule and the level each was shown at.
    pub(crate) shown: Vec<(SymbolId, Detail)>,
    /// The symbols the agent expanded since that capsule.
    pub(crate) expanded: Vec<SymbolId>,
    /// When that capsule was shown, in seconds since the Unix epoch.
    pub(crate) shown_at: Option<i64>,
    /// The symbols expanded most recently, oldest first, each once. It spans capsules, and holds
    /// one more than the pairing window so that the window is full of *other* symbols.
    #[allow(
        dead_code,
        reason = "read by `note_expanded`, whose caller lands with the expand module"
    )]
    pub(crate) recent: VecDeque<SymbolId>,
    /// How many storage errors the hooks swallowed.
    pub(crate) errors: u64,
}

/// The symbols of the previous capsule that the agent ignored.
fn ignored_symbols(session: &Session, now: i64) -> Vec<SymbolId> {
    let Some(shown_at) = session.shown_at else {
        return Vec::new();
    };
    if now.saturating_sub(shown_at) >= IGNORE_WINDOW_SECS {
        return Vec::new();
    }
    let mut seen: Vec<SymbolId> = Vec::new();
    for (id, detail) in &session.shown {
        if *detail >= Detail::Signature && !session.expanded.contains(id) && !seen.contains(id) {
            seen.push(*id);
            if seen.len() == MAX_IGNORED_PER_SETTLEMENT {
                break;
            }
        }
    }
    seen
}

/// What the explicit operations can give feedback about.
///
/// # Examples
/// ```
/// use pn_ultramemory_core::MemoryId;
/// use pn_ultramemory_engine::FeedbackTarget;
///
/// let symbol = FeedbackTarget::Symbol("Config::validate".into());
/// let memory = FeedbackTarget::Memory(MemoryId(3));
/// assert_ne!(symbol, memory);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeedbackTarget {
    /// A symbol, named the way `resolve_symbol` accepts: an id, a name or `path:name`.
    Symbol(String),
    /// A stored memory.
    Memory(MemoryId),
}

/// An explanation of why something ranks as it does: the evidence behind it.
#[derive(Debug, Clone, PartialEq)]
pub struct UtilityReport {
    /// What the report is about, such as `symbol 12 src/config.rs Config::validate`.
    pub target: String,
    /// Positive evidence, decayed to now.
    pub alpha: f64,
    /// Negative evidence, decayed to now.
    pub beta: f64,
    /// Posterior mean of the usefulness, `0.5` with no evidence.
    pub mean: f64,
    /// The factor recall applies to the relevance of the target, in `[0.5, 1.5]`. It is `1.0`
    /// while the evidence is negligible.
    pub multiplier: f64,
    /// Total evidence, positive and negative.
    pub evidence: f64,
}

/// Rounds a number to four decimals for output.
fn round4(value: f64) -> f64 {
    (value * 10_000.0).round() / 10_000.0
}

impl UtilityReport {
    /// The report as a structured value.
    ///
    /// # Examples
    /// ```
    /// use pn_ultramemory_engine::UtilityReport;
    ///
    /// let report = UtilityReport {
    ///     target: "symbol 1 a.rs f".into(),
    ///     alpha: 2.0,
    ///     beta: 0.5,
    ///     mean: 0.6,
    ///     multiplier: 1.1,
    ///     evidence: 2.5,
    /// };
    /// assert_eq!(report.to_value()["multiplier"], 1.1);
    /// ```
    #[must_use]
    pub fn to_value(&self) -> Value {
        json!({
            "target": self.target,
            "alpha": round4(self.alpha),
            "beta": round4(self.beta),
            "mean": round4(self.mean),
            "multiplier": round4(self.multiplier),
            "evidence": round4(self.evidence),
        })
    }
}

impl Engine {
    /// Records the symbols of a capsule that was just handed to the agent.
    ///
    /// The previous capsule is settled first (see the module documentation), then this one becomes
    /// the current capsule.
    pub(crate) fn note_shown(&self, shown: &[(SymbolId, Detail)]) {
        let now = self.now();
        let mut session = self.session();
        for id in ignored_symbols(&session, now) {
            self.signal_best_effort(&mut session, Target::Symbol(id), SignalKind::Ignored, now);
        }
        session.shown = shown.to_vec();
        session.expanded.clear();
        session.shown_at = Some(now);
    }

    /// Records that the agent asked for the full source of a symbol.
    ///
    /// Called by the expand operation, which is being written; until then nothing invokes it.
    #[allow(
        dead_code,
        reason = "called by the expand operation, which is being written"
    )]
    pub(crate) fn note_expanded(&self, id: SymbolId) {
        let now = self.now();
        let mut session = self.session();
        self.signal_best_effort(&mut session, Target::Symbol(id), SignalKind::Used, now);
        let others: Vec<SymbolId> = session
            .recent
            .iter()
            .rev()
            .filter(|other| **other != id)
            .take(COACCESS_WINDOW)
            .copied()
            .collect();
        for other in others {
            if self.storage().bump_coaccess(id, other, 1.0, now).is_err() {
                session.errors += 1;
            }
        }
        session.recent.retain(|other| *other != id);
        session.recent.push_back(id);
        while session.recent.len() > COACCESS_WINDOW + 1 {
            session.recent.pop_front();
        }
        if !session.expanded.contains(&id) {
            session.expanded.push(id);
        }
    }

    /// The factor by which learned evidence scales the relevance of each target, in `[0.5, 1.5]`.
    /// Targets with no evidence are absent from the map, which callers treat as `1.0`.
    pub(crate) fn multipliers(&self, targets: &[Target]) -> HashMap<Target, f64> {
        let mut map = HashMap::new();
        if targets.is_empty() {
            return map;
        }
        match self.storage().utility_states(targets) {
            Ok(states) => {
                let now = self.now();
                for (target, state) in states {
                    let state = state.decayed_to(now, self.config.half_life_secs);
                    if state.evidence() >= MIN_EVIDENCE {
                        map.insert(target, state.rank_multiplier());
                    }
                }
            }
            Err(_) => self.session().errors += 1,
        }
        map
    }

    /// Applies one signal to the utility of a target and logs it.
    fn apply_signal(&self, target: Target, kind: SignalKind, now: i64) -> Result<(), StorageError> {
        let storage = self.storage();
        let current = storage
            .utility_states(&[target])?
            .into_iter()
            .next()
            .map_or_else(|| UtilityState::empty(now), |(_, state)| state);
        let next = current.apply(kind, now, self.config.half_life_secs);
        storage.put_utility_state(target, next)?;
        storage.log_signal(target, kind, now)
    }

    /// Applies a signal inside a hook: a storage error is counted, never reported.
    fn signal_best_effort(
        &self,
        session: &mut Session,
        target: Target,
        kind: SignalKind,
        now: i64,
    ) {
        if self.apply_signal(target, kind, now).is_err() {
            session.errors += 1;
        }
    }

    #[allow(
        dead_code,
        reason = "the counter the module documentation promises, read by tests"
    )]
    /// The number of storage errors the learning hooks have swallowed.
    #[cfg(test)]
    pub(crate) fn learning_errors(&self) -> u64 {
        self.session().errors
    }

    /// Finds the target of a feedback call and describes it.
    fn feedback_target(&self, target: &FeedbackTarget) -> Result<(Target, String), EngineError> {
        match target {
            FeedbackTarget::Symbol(reference) => {
                let symbol = self.resolve_symbol(reference)?;
                let label = format!(
                    "symbol {} {} {}",
                    symbol.id, symbol.path, symbol.qualified_name
                );
                Ok((Target::Symbol(symbol.id), label))
            }
            FeedbackTarget::Memory(id) => {
                if self.find_memory(*id)?.is_none() {
                    return Err(EngineError::NotFound(format!("no memory with id {id}")));
                }
                Ok((Target::Memory(*id), format!("memory {id}")))
            }
        }
    }

    /// Records an explicit signal about a symbol or a memory: the agent or a person says that it
    /// was useful, was a dead end, or was wrong.
    ///
    /// # Errors
    /// Returns [`EngineError::NotFound`] or [`EngineError::Ambiguous`] when the symbol does not
    /// name exactly one symbol, [`EngineError::NotFound`] when the memory does not exist, and a
    /// storage error when the signal cannot be stored. Unlike the automatic hooks, this call
    /// reports every failure.
    pub fn feedback(&self, target: &FeedbackTarget, kind: SignalKind) -> Result<(), EngineError> {
        let (resolved, _) = self.feedback_target(target)?;
        let now = self.now();
        let _serialized = self.session();
        self.apply_signal(resolved, kind, now)?;
        Ok(())
    }

    /// Explains why a symbol or a memory ranks as it does: the evidence collected about it and
    /// the multiplier recall applies.
    ///
    /// # Errors
    /// Returns the same errors as [`Engine::feedback`] for a target that does not exist, and a
    /// storage error when the evidence cannot be read.
    pub fn utility_of(&self, target: &FeedbackTarget) -> Result<UtilityReport, EngineError> {
        let (resolved, label) = self.feedback_target(target)?;
        let now = self.now();
        let stored = self
            .storage()
            .utility_states(&[resolved])?
            .into_iter()
            .next()
            .map_or_else(|| UtilityState::empty(now), |(_, state)| state);
        let state = stored.decayed_to(now, self.config.half_life_secs);
        let multiplier = if state.evidence() >= MIN_EVIDENCE {
            state.rank_multiplier()
        } else {
            1.0
        };
        Ok(UtilityReport {
            target: label,
            alpha: state.alpha,
            beta: state.beta,
            mean: state.mean(),
            multiplier,
            evidence: state.evidence(),
        })
    }

    /// Counts describing what has been learned: signals, tracked targets and co-access pairs.
    ///
    /// # Errors
    /// Returns a storage error when the counts cannot be read.
    pub fn learning_status(&self) -> Result<LearningStatus, EngineError> {
        Ok(self.storage().learning_status()?)
    }

    /// Forgets everything that was learned, in storage and in the current session. Symbols and
    /// memories are untouched.
    ///
    /// # Errors
    /// Returns a storage error when the reset fails; the session is then left as it was.
    pub fn reset_learning(&self) -> Result<(), EngineError> {
        let mut session = self.session();
        self.storage().reset_learning()?;
        let errors = session.errors;
        *session = Session::default();
        session.errors = errors;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
