// SPDX-License-Identifier: Apache-2.0
//! Storing, finding and confirming memories, without letting them turn into noise or into lies.
//!
//! # Role in the architecture
//! Application layer. It sits between the agent-facing surfaces and
//! [`pn_ultramemory_core::Storage`], and it is where a memory is sanitized, screened, compared
//! against what is already known, and anchored to the code it describes.
//!
//! # The two failure modes this module exists to prevent
//! **Redundancy.** Fifty near-identical notes about one symbol turn retrieval into noise, spend the
//! token budget this whole product exists to protect, and bury the one note that mattered.
//!
//! **Truth by repetition.** If saying something often enough made the system believe it, an agent
//! repeating its own mistake in a loop would manufacture a fact, and that fact would be served with
//! confidence to every later session. This is the worst thing the product could do.
//!
//! # The rule everything follows
//! **Corroboration raises how likely a memory is to be RETRIEVED; it never raises how likely it is
//! to be TRUE.** Retrieval priority and truth are different quantities and are never mixed. A
//! corroboration is recorded through the learning channel, whose influence on ranking is bounded to
//! `[0.5, 1.5]` and which can never create a fact.
//!
//! # Independence
//! A repetition only counts when it is an independent observation. Two corroborations of the same
//! memory closer together than [`INDEPENDENCE_WINDOW_SECS`] count as **one**, whoever sent them, so
//! an agent repeating itself inside a session leaves the same trace as a single call. That test is
//! deliberately the strict direction: it can miss a genuinely independent second observation inside
//! the window, and it can never let a loop inflate anything. Text captured from a tool
//! ([`Provenance::Tool`]) is untrusted and never corroborates at all, and no memory may ever gather
//! more than [`MAX_CORROBORATIONS`].
//!
//! # What this does not do
//! It does not understand meaning; [`similarity`] and [`conflict`] state the honest limits of both
//! comparisons. It cannot tell a true statement from a false one. It never edits or deletes what
//! someone stored, and it never resolves a contradiction on its own.

/// Noticing that two memories about the same code contradict each other.
pub mod conflict;
/// Deciding whether two memories say the same thing.
pub mod similarity;
/// Which language a memory is written in, and the function words of each.
pub mod tongue;

use std::collections::BTreeSet;

use pn_ultramemory_core::{
    MemoryFilter, MemoryId, MemoryKind, MemoryRecord, NewMemory, Provenance, SignalKind, SymbolId,
    Target, UtilityState,
};
use serde_json::{Value, json};

pub use crate::guard::{Verdict, redact, scan};
pub use crate::learn::{FeedbackTarget, UtilityReport};
pub use conflict::{Reason as ConflictReason, contradicts};
pub use similarity::{content_words, hamming, simhash, similarity};

use crate::engine::Engine;
use crate::error::EngineError;
use crate::metrics::Event;

/// Two corroborations of one memory closer together than this are one observation.
pub const INDEPENDENCE_WINDOW_SECS: i64 = 15 * 60;

/// The most corroborations one memory may ever gather, so no amount of repetition dominates
/// ranking.
pub const MAX_CORROBORATIONS: u32 = 8;

/// The longest memory text accepted.
const MAX_TEXT: usize = 4_000;

/// The most symbols one memory may be anchored to.
const MAX_ANCHORS: usize = 10;

/// How many existing memories a write is compared against.
const COMPARE_LIMIT: usize = 200;

/// How many near misses are reported back to the caller.
const MAX_SIMILAR_REPORTED: usize = 5;

/// What happened to a memory that was offered for storing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// It was new, and it was stored.
    Stored,
    /// An existing memory already said this, so nothing was stored and its retrieval priority
    /// rose.
    Reinforced {
        /// The memory that was reinforced.
        existing: MemoryId,
        /// How many independent observations it now stands on, at most [`MAX_CORROBORATIONS`].
        corroborations: u32,
    },
    /// An existing memory said exactly this about exactly the same code: nothing changed.
    Duplicate {
        /// The memory that already said it.
        existing: MemoryId,
    },
    /// It was stored, and it appears to contradict what is already known.
    Conflicts {
        /// The memories it appears to contradict.
        existing: Vec<MemoryId>,
    },
}

impl Outcome {
    /// A short, stable name used in output.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Stored => "stored",
            Self::Reinforced { .. } => "reinforced",
            Self::Duplicate { .. } => "duplicate",
            Self::Conflicts { .. } => "conflicts",
        }
    }
}

/// A memory offered for storing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RememberInput {
    /// What kind of memory it is.
    pub kind: MemoryKind,
    /// The memory itself, as a statement. It is cleaned, screened and redacted before anything is
    /// stored, so what ends up in the store is not always byte for byte what was passed here.
    pub text: String,
    /// The symbols it is about, named the way [`Engine::resolve_symbol`] accepts.
    pub about: Vec<String>,
    /// Who produced it.
    pub provenance: Provenance,
    /// The session it came from, when the caller knows one. It is recorded for the caller's own
    /// bookkeeping; independence is decided by time, which no caller can misreport.
    pub session: Option<String>,
}

/// What happened when a memory was offered.
#[derive(Debug, Clone, PartialEq)]
pub struct RememberOutcome {
    /// The memory, stored or already existing.
    pub memory: Option<MemoryRecord>,
    /// What was decided.
    pub outcome: Outcome,
    /// The `about` entries that named no symbol, or named several.
    pub unresolved: Vec<String>,
    /// How many secrets were replaced before storing.
    pub redactions: u32,
    /// Anything the caller should know: a redaction, a suspicious pattern, a contradiction.
    pub warnings: Vec<String>,
    /// The existing memories that came closest, with their scores, so the decision can be checked.
    pub similar: Vec<(MemoryId, f64)>,
}

/// Rounds a score to three decimals for output.
fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

impl RememberOutcome {
    /// The outcome as a structured value.
    #[must_use]
    pub fn to_value(&self) -> Value {
        // Built as a map rather than by assigning into a `Value`: indexing a `Value` by name is
        // only defined when it already holds an object, so it states an assumption the type does
        // not carry, while an insert into a map cannot mean anything else.
        let mut map = serde_json::Map::new();
        let mut put = |key: &str, value: Value| {
            map.insert(key.to_owned(), value);
        };
        put("outcome", Value::from(self.outcome.as_str()));
        put("redactions", Value::from(self.redactions));
        if let Some(memory) = &self.memory {
            put("id", Value::from(memory.id.0));
            put("kind", Value::from(memory.kind.as_str()));
            put("stale", Value::from(memory.stale_since.is_some()));
        }
        if let Outcome::Reinforced { corroborations, .. } = self.outcome {
            put("corroborations", Value::from(corroborations));
        }
        if let Outcome::Conflicts { existing } = &self.outcome {
            put(
                "conflicts",
                existing.iter().map(|id| Value::from(id.0)).collect(),
            );
        }
        if !self.unresolved.is_empty() {
            put(
                "unresolved",
                self.unresolved
                    .iter()
                    .map(|s| Value::from(s.as_str()))
                    .collect(),
            );
        }
        if !self.warnings.is_empty() {
            put(
                "warnings",
                self.warnings
                    .iter()
                    .map(|s| Value::from(s.as_str()))
                    .collect(),
            );
        }
        if !self.similar.is_empty() {
            put(
                "similar",
                self.similar
                    .iter()
                    .map(|(id, score)| json!({ "id": id.0, "score": round3(*score) }))
                    .collect(),
            );
        }
        Value::Object(map)
    }
}

/// One memory as a structured value.
#[must_use]
pub fn memory_to_value(memory: &MemoryRecord) -> Value {
    let about: Vec<&str> = memory
        .anchors
        .iter()
        .map(|anchor| anchor.qualified_name.as_str())
        .collect();
    let mut entry = serde_json::Map::new();
    entry.insert("id".into(), Value::from(memory.id.0));
    entry.insert("kind".into(), Value::from(memory.kind.as_str()));
    entry.insert("by".into(), Value::from(memory.provenance.as_str()));
    entry.insert(
        "stale".into(),
        Value::from(if memory.stale_since.is_some() {
            "yes"
        } else {
            "no"
        }),
    );
    if let Some(reason) = memory.stale_reason {
        entry.insert("stale_reason".into(), Value::from(reason.as_str()));
    }
    entry.insert("about".into(), Value::from(about.join(" ")));
    entry.insert("text".into(), Value::from(memory.text.clone()));
    Value::Object(entry)
}

/// Several memories as a uniform table, which prints compactly.
#[must_use]
pub fn memories_to_value(memories: &[MemoryRecord]) -> Value {
    json!({ "memories": memories.iter().map(memory_to_value).collect::<Vec<Value>>() })
}

/// Cleans a memory's text: one kind of line ending, no control characters, no long blank runs.
fn sanitize(text: &str) -> String {
    let unified = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut out = String::with_capacity(unified.len());
    let mut blank_run = 0_u32;
    for line in unified.split('\n') {
        let cleaned: String = line
            .chars()
            .filter(|c| *c == '\t' || !c.is_control())
            .collect();
        let cleaned = cleaned.trim_end();
        if cleaned.is_empty() {
            blank_run += 1;
            if blank_run > 2 {
                continue;
            }
        } else {
            blank_run = 0;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(cleaned);
    }
    out.trim().to_owned()
}

/// A memory already stored, prepared for comparison.
struct Candidate {
    /// The stored memory.
    record: MemoryRecord,
    /// Its content words.
    words: Vec<String>,
    /// The hash of those words, for the cheap pre-filter.
    hash: u64,
}

/// The symbols a memory is anchored to.
fn anchor_set(memory: &MemoryRecord) -> BTreeSet<SymbolId> {
    memory
        .anchors
        .iter()
        .filter_map(|anchor| anchor.symbol)
        .collect()
}

/// Whether two memories are about at least one symbol in common.
fn share_anchor(a: &MemoryRecord, b: &MemoryRecord) -> bool {
    anchor_set(a).intersection(&anchor_set(b)).next().is_some()
}

/// Cleans the text of a memory and checks its size.
///
/// # Errors
/// Returns an invalid-request error, naming the command to run instead, when the text is empty or
/// longer than [`MAX_TEXT`] characters.
fn prepare_text(raw: &str) -> Result<String, EngineError> {
    let text = sanitize(raw);
    if text.is_empty() {
        return Err(EngineError::Invalid(
            "the memory is empty; give the text to remember, as in \
             `pn-ultramemory remember decision \"...\"`"
                .into(),
        ));
    }
    if text.chars().count() > MAX_TEXT {
        return Err(EngineError::Invalid(format!(
            "the memory is longer than {MAX_TEXT} characters; shorten it, or split it across \
             several `pn-ultramemory remember` calls"
        )));
    }
    Ok(text)
}

/// Refuses text that tries to direct whoever reads it, and notes anything suspicious.
///
/// A memory is shown to an agent as context. Text that reads as an instruction would therefore
/// arrive as one, which is why this refuses rather than warns: a warning is attached to the stored
/// memory, and by then the text is already in the store waiting to be served.
///
/// # Errors
/// Returns a rejection naming what was found when the text reads as an instruction.
fn screen(text: &str, warnings: &mut Vec<String>) -> Result<(), EngineError> {
    match scan(text) {
        Verdict::Blocked(reasons) => Err(EngineError::Rejected(format!(
            "this text tries to direct whoever reads it ({}); a memory is shown to an agent as \
             context and must never instruct it. Rewrite it as a statement and run \
             `pn-ultramemory remember` again",
            reasons.join(", ")
        ))),
        Verdict::Suspicious(reasons) => {
            warnings.push(format!("stored, but worth a look: {}", reasons.join(", ")));
            Ok(())
        }
        Verdict::Clean => Ok(()),
    }
}

impl Engine {
    /// Fetches one memory by its identity, or `None` when no memory has it.
    ///
    /// # Errors
    /// Returns a storage error when the lookup fails.
    pub fn find_memory(&self, id: MemoryId) -> Result<Option<MemoryRecord>, EngineError> {
        Ok(self.storage().memory(id)?)
    }

    /// Lists memories, newest first.
    ///
    /// # Errors
    /// Returns a storage error when the list cannot be read.
    pub fn memories(&self, filter: &MemoryFilter) -> Result<Vec<MemoryRecord>, EngineError> {
        Ok(self.storage().list_memories(filter)?)
    }

    /// Deletes a memory. Returns `false` when no memory had that identity.
    ///
    /// # Errors
    /// Returns a storage error when the memory cannot be deleted.
    pub fn forget(&self, id: MemoryId) -> Result<bool, EngineError> {
        Ok(self.storage().forget_memory(id)?)
    }

    /// Confirms that a stale memory is still true, re-anchoring it to the code as it is now.
    /// Returns `false` when no memory had that identity.
    ///
    /// # Errors
    /// Returns a storage error when the memory cannot be re-anchored.
    pub fn reanchor(&self, id: MemoryId) -> Result<bool, EngineError> {
        Ok(self.storage().reanchor_memory(id)?)
    }

    /// The pairs among these memories that appear to contradict each other.
    ///
    /// # Errors
    /// Returns a storage error when a memory cannot be read.
    pub fn conflicts_for(
        &self,
        ids: &[MemoryId],
    ) -> Result<Vec<(MemoryId, MemoryId)>, EngineError> {
        let mut records = Vec::new();
        for id in ids {
            if let Some(record) = self.find_memory(*id)? {
                records.push(record);
            }
        }
        let mut pairs = Vec::new();
        for (index, left) in records.iter().enumerate() {
            for right in records.iter().skip(index + 1) {
                if left.kind == right.kind
                    && share_anchor(left, right)
                    && contradicts(&left.text, &right.text, left.kind).is_some()
                {
                    pairs.push((left.id, right.id));
                }
            }
        }
        Ok(pairs)
    }

    /// Stores a memory, unless something already says it.
    ///
    /// The whole pipeline, and the rule that governs corroboration, are described in
    /// `docs/memory.md`.
    ///
    /// # Errors
    /// Returns [`EngineError::Invalid`] for text that is empty or too long,
    /// [`EngineError::Rejected`] for text that tries to direct whoever reads it, and a storage
    /// error when the memory cannot be written.
    pub fn remember(&self, input: &RememberInput) -> Result<RememberOutcome, EngineError> {
        let mut warnings = Vec::new();
        let text = prepare_text(&input.text)?;
        let (redactions, text) = {
            let (redacted, count) = redact(&text);
            (count, redacted)
        };
        if redactions > 0 {
            warnings.push(format!(
                "{redactions} secret(s) were replaced before storing"
            ));
        }
        screen(&text, &mut warnings)?;

        let (anchors, unresolved) = self.resolve_anchors(&input.about)?;
        let candidates = self.comparison_set(&anchors, &text)?;
        let words = content_words(&text);
        let hash = simhash(&words);

        let mut similar: Vec<(MemoryId, f64)> = Vec::new();
        let mut best: Option<(usize, f64)> = None;
        let mut conflicts: Vec<MemoryId> = Vec::new();
        for (index, candidate) in candidates.iter().enumerate() {
            if candidate.record.kind != input.kind {
                continue;
            }
            // A contradiction is checked FIRST and disqualifies the candidate from ever counting
            // as the same memory. Negating a sentence changes almost none of its words, so a
            // denial scores as highly similar; treating that as corroboration would let a memory
            // be reinforced by its own opposite, which is the failure this module exists to
            // prevent.
            let opposed = contradicts(&text, &candidate.record.text, input.kind).is_some();
            if opposed {
                conflicts.push(candidate.record.id);
                continue;
            }
            if hamming(hash, candidate.hash) <= similarity::MAX_HAMMING {
                let score = similarity::similarity_of(&words, &candidate.words);
                if score >= similarity::SIMILAR {
                    similar.push((candidate.record.id, score));
                    if best.is_none_or(|(_, previous)| score > previous) {
                        best = Some((index, score));
                    }
                }
            }
        }
        similar.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        similar.truncate(MAX_SIMILAR_REPORTED);
        conflicts.sort_unstable();
        conflicts.dedup();

        // `best` holds a position into `candidates`, recorded while scoring it, so the lookup
        // always succeeds; asking for it keeps that local instead of trusting an index.
        if let Some((existing, score)) =
            best.and_then(|(index, score)| Some((&candidates.get(index)?.record, score)))
        {
            let outcome = if score >= similarity::IDENTICAL && anchor_set(existing) == anchors {
                Outcome::Duplicate {
                    existing: existing.id,
                }
            } else {
                let corroborations = self.corroborate(existing.id, input.provenance)?;
                Outcome::Reinforced {
                    existing: existing.id,
                    corroborations,
                }
            };
            return Ok(RememberOutcome {
                memory: Some(existing.clone()),
                outcome,
                unresolved,
                redactions,
                warnings,
                similar,
            });
        }

        let new = NewMemory {
            kind: input.kind,
            text,
            provenance: input.provenance,
            about: anchors.iter().copied().collect(),
        };
        let stored = self.storage().add_memory(&new, self.now())?;
        let outcome = if conflicts.is_empty() {
            Outcome::Stored
        } else {
            warnings.push(format!(
                "this appears to contradict {} stored memor{}; both are kept, run \
                 `pn-ultramemory memories` to compare them",
                conflicts.len(),
                if conflicts.len() == 1 { "y" } else { "ies" }
            ));
            Outcome::Conflicts {
                existing: conflicts,
            }
        };
        self.record(Event::Remember {
            redactions,
            warnings: u32::try_from(warnings.len()).unwrap_or(u32::MAX),
        });
        Ok(RememberOutcome {
            memory: Some(stored),
            outcome,
            unresolved,
            redactions,
            warnings,
            similar,
        })
    }

    /// Resolves the `about` references into symbols, collecting the ones that named nothing.
    fn resolve_anchors(
        &self,
        about: &[String],
    ) -> Result<(BTreeSet<SymbolId>, Vec<String>), EngineError> {
        let mut anchors = BTreeSet::new();
        let mut unresolved = Vec::new();
        for reference in about.iter().take(MAX_ANCHORS) {
            match self.resolve_symbol(reference) {
                Ok(symbol) => {
                    anchors.insert(symbol.id);
                }
                Err(EngineError::Storage(error)) => return Err(EngineError::Storage(error)),
                Err(_) => unresolved.push(reference.clone()),
            }
        }
        Ok((anchors, unresolved))
    }

    /// The memories a new one is compared against: those sharing an anchor, or what a text search
    /// finds when there is no anchor to go by.
    fn comparison_set(
        &self,
        anchors: &BTreeSet<SymbolId>,
        text: &str,
    ) -> Result<Vec<Candidate>, EngineError> {
        let ids: Vec<SymbolId> = anchors.iter().copied().collect();
        let mut records = if ids.is_empty() {
            self.storage().search_memories(text, COMPARE_LIMIT)?
        } else {
            self.storage().memories_for_symbols(&ids, COMPARE_LIMIT)?
        };
        records.sort_by_key(|record| record.id);
        records.dedup_by_key(|record| record.id);
        Ok(records
            .into_iter()
            .map(|record| {
                let words = content_words(&record.text);
                let hash = simhash(&words);
                Candidate {
                    record,
                    words,
                    hash,
                }
            })
            .collect())
    }

    /// Records an independent corroboration of an existing memory and returns how many it now
    /// stands on.
    ///
    /// This raises the memory's retrieval priority through the bounded learning channel. It never
    /// marks the memory as more likely to be true, and it never edits it.
    fn corroborate(&self, id: MemoryId, provenance: Provenance) -> Result<u32, EngineError> {
        let target = Target::Memory(id);
        let now = self.now();
        let state = self
            .storage()
            .utility_states(&[target])?
            .into_iter()
            .next()
            .map_or_else(|| UtilityState::empty(now), |(_, state)| state);
        let count = corroborations_of(&state);

        // Untrusted text never corroborates, a memory never gathers more than the cap, and two
        // observations inside the window are one observation.
        if provenance == Provenance::Tool
            || count >= MAX_CORROBORATIONS
            || (count > 0 && now.saturating_sub(state.updated_at) < INDEPENDENCE_WINDOW_SECS)
        {
            return Ok(count);
        }

        let updated = state.apply(SignalKind::Useful, now, self.config.half_life_secs);
        self.storage().put_utility_state(target, updated)?;
        self.storage().log_signal(target, SignalKind::Useful, now)?;
        Ok(count + 1)
    }
}

/// How many corroborations a memory stands on, read from its evidence.
///
/// [`SignalKind::Useful`] contributes two units of positive evidence, so the count is half the
/// decayed positive evidence, rounded and capped.
fn corroborations_of(state: &UtilityState) -> u32 {
    let (positive, _) = SignalKind::Useful.deltas();
    if positive <= 0.0 {
        return 0;
    }
    let raw = (state.alpha / positive).max(0.0).round();
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let count = raw.min(f64::from(MAX_CORROBORATIONS)) as u32;
    count
}

/// Collapses near-duplicates for display, returning each kept memory with how many similar ones it
/// stands for, so a capsule shows one line and a count instead of four near-identical lines.
///
/// The memory kept is the one with the most anchors, then the longest text, then the lowest
/// identity, so the result never depends on the order they arrived in.
///
/// # Examples
/// ```
/// use pn_ultramemory_engine::dedupe_for_capsule;
///
/// assert!(dedupe_for_capsule(Vec::new()).is_empty());
/// ```
#[must_use]
pub fn dedupe_for_capsule(memories: Vec<MemoryRecord>) -> Vec<(MemoryRecord, u32)> {
    let mut ordered = memories;
    ordered.sort_by(|a, b| {
        b.anchors
            .len()
            .cmp(&a.anchors.len())
            .then_with(|| b.text.len().cmp(&a.text.len()))
            .then_with(|| a.id.cmp(&b.id))
    });
    let mut kept: Vec<(MemoryRecord, u32, Vec<String>)> = Vec::new();
    for memory in ordered {
        let words = content_words(&memory.text);
        let mut merged = false;
        for entry in &mut kept {
            if entry.0.kind == memory.kind
                && similarity::similarity_of(&words, &entry.2) >= similarity::SIMILAR
            {
                entry.1 += 1;
                merged = true;
                break;
            }
        }
        if !merged {
            kept.push((memory, 0, words));
        }
    }
    kept.into_iter()
        .map(|(memory, extra, _)| (memory, extra))
        .collect()
}

#[cfg(test)]
mod tests;
