// SPDX-License-Identifier: Apache-2.0
//! The rules of local learning: how signals about what proved useful update a bounded estimate.
//!
//! Each symbol or memory that has been recalled carries a small state, a Beta distribution over
//! "how useful is this when recalled", with exponential decay so that old evidence fades. The
//! state is turned into a ranking multiplier that can only nudge results: it is bounded to
//! `[0.5, 1.5]`, and it is neutral (`1.0`) until there is evidence. Learning can therefore
//! re-rank recall but never creates facts.

use crate::{MemoryId, SymbolId};

/// Default half-life of evidence: after this long, a signal counts for half.
pub const DEFAULT_HALF_LIFE_SECS: i64 = 30 * 24 * 60 * 60;

/// Something worth remembering about how a recalled item fared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SignalKind {
    /// The agent used the item: it expanded it, edited it or cited it.
    Used,
    /// A human or agent explicitly marked the item as useful.
    Useful,
    /// The item was shown and then ignored.
    Ignored,
    /// The item led nowhere: an approach that was tried and abandoned.
    DeadEnd,
    /// The item was wrong and had to be corrected.
    Corrected,
}

impl SignalKind {
    /// Every kind.
    pub const ALL: [Self; 5] = [
        Self::Used,
        Self::Useful,
        Self::Ignored,
        Self::DeadEnd,
        Self::Corrected,
    ];

    /// How much this signal adds to the positive and negative evidence, in that order.
    ///
    /// # Examples
    /// ```
    /// use pn_ultramemory_core::SignalKind;
    ///
    /// assert_eq!(SignalKind::Used.deltas(), (1.0, 0.0));
    /// assert_eq!(SignalKind::Corrected.deltas(), (0.0, 2.0));
    /// ```
    #[must_use]
    pub const fn deltas(self) -> (f64, f64) {
        match self {
            Self::Used => (1.0, 0.0),
            Self::Useful => (2.0, 0.0),
            Self::Ignored => (0.0, 0.25),
            Self::DeadEnd => (0.0, 1.5),
            Self::Corrected => (0.0, 2.0),
        }
    }

    /// Stable lowercase name used in storage and in output.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Used => "used",
            Self::Useful => "useful",
            Self::Ignored => "ignored",
            Self::DeadEnd => "dead_end",
            Self::Corrected => "corrected",
        }
    }

    /// Looks a kind up by the name returned from [`SignalKind::as_str`].
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == name)
    }
}

/// The thing a signal is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Target {
    /// A symbol.
    Symbol(SymbolId),
    /// A memory.
    Memory(MemoryId),
}

impl Target {
    /// The name of the kind of target, for storage.
    #[must_use]
    pub const fn kind_str(self) -> &'static str {
        match self {
            Self::Symbol(_) => "symbol",
            Self::Memory(_) => "memory",
        }
    }

    /// The raw numeric identity, for storage.
    #[must_use]
    pub const fn raw_id(self) -> i64 {
        match self {
            Self::Symbol(id) => id.0,
            Self::Memory(id) => id.0,
        }
    }

    /// Rebuilds a target from the two parts returned by [`Target::kind_str`] and
    /// [`Target::raw_id`].
    #[must_use]
    pub fn from_parts(kind: &str, id: i64) -> Option<Self> {
        match kind {
            "symbol" => Some(Self::Symbol(SymbolId(id))),
            "memory" => Some(Self::Memory(MemoryId(id))),
            _ => None,
        }
    }
}

/// Decayed evidence about how useful a target is when recalled.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UtilityState {
    /// Decayed positive evidence.
    pub alpha: f64,
    /// Decayed negative evidence.
    pub beta: f64,
    /// When the evidence was last updated, in seconds since the Unix epoch.
    pub updated_at: i64,
}

/// The fraction of evidence that remains after `elapsed_secs`, given the half-life.
///
/// Negative elapsed time counts as none, and a non-positive half-life disables decay.
///
/// # Examples
/// ```
/// use pn_ultramemory_core::decay_factor;
///
/// assert!((decay_factor(100, 100) - 0.5).abs() < 1e-12);
/// assert!((decay_factor(0, 100) - 1.0).abs() < 1e-12);
/// ```
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn decay_factor(elapsed_secs: i64, half_life_secs: i64) -> f64 {
    if half_life_secs <= 0 || elapsed_secs <= 0 {
        return 1.0;
    }
    0.5_f64.powf(elapsed_secs as f64 / half_life_secs as f64)
}

impl UtilityState {
    /// A state with no evidence, as of `now`.
    #[must_use]
    pub const fn empty(now: i64) -> Self {
        Self {
            alpha: 0.0,
            beta: 0.0,
            updated_at: now,
        }
    }

    /// The same evidence, decayed forward to `now`.
    #[must_use]
    pub fn decayed_to(self, now: i64, half_life_secs: i64) -> Self {
        let factor = decay_factor(now - self.updated_at, half_life_secs);
        Self {
            alpha: self.alpha * factor,
            beta: self.beta * factor,
            updated_at: now.max(self.updated_at),
        }
    }

    /// Decays to `now`, then adds the evidence carried by one signal.
    ///
    /// # Examples
    /// ```
    /// use pn_ultramemory_core::{SignalKind, UtilityState, DEFAULT_HALF_LIFE_SECS};
    ///
    /// let state = UtilityState::empty(0).apply(SignalKind::Useful, 0, DEFAULT_HALF_LIFE_SECS);
    /// assert!(state.rank_multiplier() > 1.0);
    /// ```
    #[must_use]
    pub fn apply(self, kind: SignalKind, now: i64, half_life_secs: i64) -> Self {
        let mut next = self.decayed_to(now, half_life_secs);
        let (positive, negative) = kind.deltas();
        next.alpha += positive;
        next.beta += negative;
        next
    }

    /// Posterior mean of the usefulness, under a uniform prior. `0.5` with no evidence.
    #[must_use]
    pub fn mean(&self) -> f64 {
        (1.0 + self.alpha) / (2.0 + self.alpha + self.beta)
    }

    /// Total evidence, positive and negative.
    #[must_use]
    pub fn evidence(&self) -> f64 {
        self.alpha + self.beta
    }

    /// The factor recall multiplies a relevance by. It is `1.0` with no evidence, grows toward
    /// `1.5` as positive evidence accumulates, shrinks toward `0.5` as negative evidence does,
    /// and can never leave `[0.5, 1.5]`.
    ///
    /// # Examples
    /// ```
    /// use pn_ultramemory_core::UtilityState;
    ///
    /// assert!((UtilityState::empty(0).rank_multiplier() - 1.0).abs() < 1e-12);
    /// let loved = UtilityState { alpha: 1000.0, beta: 0.0, updated_at: 0 };
    /// assert!(loved.rank_multiplier() <= 1.5);
    /// ```
    #[must_use]
    pub fn rank_multiplier(&self) -> f64 {
        let evidence = self.evidence();
        let confidence = evidence / (evidence + 4.0);
        (1.0 + 0.5 * (2.0 * self.mean() - 1.0) * confidence).clamp(0.5, 1.5)
    }
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_HALF_LIFE_SECS, SignalKind, Target, UtilityState, decay_factor};
    use crate::{MemoryId, SymbolId};

    /// Decay halves the evidence per half-life and never amplifies it.
    #[test]
    fn decay_halves_per_half_life() {
        let h = 1_000;
        assert!((decay_factor(h, h) - 0.5).abs() < 1e-12);
        assert!((decay_factor(2 * h, h) - 0.25).abs() < 1e-12);
        assert!((decay_factor(-5, h) - 1.0).abs() < 1e-12);
        assert!((decay_factor(50, 0) - 1.0).abs() < 1e-12);
    }

    /// With no evidence the multiplier is neutral.
    #[test]
    fn empty_state_is_neutral() {
        let state = UtilityState::empty(10);
        assert!((state.mean() - 0.5).abs() < 1e-12);
        assert!((state.rank_multiplier() - 1.0).abs() < 1e-12);
    }

    /// Positive evidence raises the multiplier, negative evidence lowers it, both stay bounded.
    #[test]
    fn multiplier_is_monotonic_and_bounded() {
        let mut good = UtilityState::empty(0);
        let mut bad = UtilityState::empty(0);
        let mut last_good = good.rank_multiplier();
        let mut last_bad = bad.rank_multiplier();
        for _ in 0..200 {
            good = good.apply(SignalKind::Useful, 0, DEFAULT_HALF_LIFE_SECS);
            bad = bad.apply(SignalKind::Corrected, 0, DEFAULT_HALF_LIFE_SECS);
            assert!(good.rank_multiplier() >= last_good);
            assert!(bad.rank_multiplier() <= last_bad);
            last_good = good.rank_multiplier();
            last_bad = bad.rank_multiplier();
            assert!((0.5..=1.5).contains(&last_good));
            assert!((0.5..=1.5).contains(&last_bad));
        }
        assert!(last_good > 1.4);
        assert!(last_bad < 0.6);
    }

    /// Old evidence fades: a signal one half-life ago counts half as much as a fresh one.
    #[test]
    fn evidence_fades_with_time() {
        let h = DEFAULT_HALF_LIFE_SECS;
        let state = UtilityState::empty(0).apply(SignalKind::Used, 0, h);
        let later = state.decayed_to(h, h);
        assert!((later.alpha - 0.5).abs() < 1e-12);
        assert_eq!(later.updated_at, h);
    }

    /// Signal and target names round-trip.
    #[test]
    fn names_round_trip() {
        for kind in SignalKind::ALL {
            assert_eq!(SignalKind::from_name(kind.as_str()), Some(kind));
        }
        for target in [Target::Symbol(SymbolId(5)), Target::Memory(MemoryId(9))] {
            assert_eq!(
                Target::from_parts(target.kind_str(), target.raw_id()),
                Some(target)
            );
        }
        assert_eq!(Target::from_parts("file", 1), None);
    }
}
