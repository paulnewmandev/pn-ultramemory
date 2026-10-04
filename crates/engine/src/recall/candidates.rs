// SPDX-License-Identifier: Apache-2.0
//! The set of symbols that may enter a capsule, with why each one is there.
//!
//! Symbols come from several places (exact names, text search, graph neighbors, learned
//! neighbors). [`CandidateSet`] merges them: a symbol found twice keeps the highest relevance it
//! was offered and every reason. All containers are ordered, so the merge never depends on the
//! order in which hash maps happen to iterate.

use std::collections::BTreeMap;

use pn_ultramemory_core::{Confidence, EdgeKind, SymbolId, SymbolRecord};

use super::ranking::boosted;
use super::tuning;

/// A symbol that may enter the capsule.
#[derive(Debug, Clone)]
pub(super) struct Cand {
    /// The symbol.
    pub(super) record: SymbolRecord,
    /// How relevant it is to the query, after every adjustment.
    pub(super) relevance: f64,
    /// Why it is a candidate, best reason first. Each reason keeps the relevance it gave.
    reasons: Vec<(f64, String)>,
    /// The note added to the explanation for the learned multiplier, if it matters.
    usage_note: Option<String>,
}

impl Cand {
    /// The explanation of why the symbol is in the capsule: the reasons, best first, separated by
    /// semicolons, and the learned multiplier when it mattered.
    pub(super) fn why(&self) -> String {
        let mut parts: Vec<&str> = self
            .reasons
            .iter()
            .take(tuning::MAX_REASONS)
            .map(|(_, reason)| reason.as_str())
            .collect();
        if let Some(note) = &self.usage_note {
            parts.push(note);
        }
        parts.join("; ")
    }

    /// Applies the learned multiplier `multiplier` to the relevance, and remembers a note about it
    /// when it changes the relevance by at least [`tuning::MULTIPLIER_NOTE_DELTA`].
    pub(super) fn apply_multiplier(&mut self, multiplier: f64) {
        self.relevance = boosted(self.relevance, multiplier);
        if multiplier.is_finite() && (multiplier - 1.0).abs() >= tuning::MULTIPLIER_NOTE_DELTA {
            self.usage_note = Some(format!("usage x{multiplier:.1}"));
        }
    }
}

/// The candidates of one recall, keyed by symbol.
#[derive(Debug, Default)]
pub(super) struct CandidateSet {
    /// The candidates in symbol order.
    map: BTreeMap<SymbolId, Cand>,
}

impl CandidateSet {
    /// Offers a symbol with a relevance and a reason.
    ///
    /// A symbol offered again keeps the highest relevance; each distinct reason is remembered, and
    /// the reason that gave the highest relevance comes first. A relevance that is not a positive
    /// finite number is ignored.
    pub(super) fn offer(&mut self, record: &SymbolRecord, relevance: f64, reason: &str) {
        if !relevance.is_finite() || relevance <= 0.0 {
            return;
        }
        let entry = self.map.entry(record.id).or_insert_with(|| Cand {
            record: record.clone(),
            relevance: 0.0,
            reasons: Vec::new(),
            usage_note: None,
        });
        entry.relevance = entry.relevance.max(relevance);
        match entry.reasons.iter_mut().find(|(_, known)| known == reason) {
            Some((known_relevance, _)) => *known_relevance = known_relevance.max(relevance),
            None => entry.reasons.push((relevance, reason.to_owned())),
        }
        entry.reasons.sort_by(|a, b| b.0.total_cmp(&a.0));
    }

    /// The number of candidates.
    #[allow(
        dead_code,
        reason = "the companion of `is_empty`, used by the operations being written"
    )]
    pub(super) fn len(&self) -> usize {
        self.map.len()
    }

    /// Whether there is no candidate.
    pub(super) fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// The best `count` candidates as `(id, relevance)`, most relevant first, ties by id.
    pub(super) fn top(&self, count: usize) -> Vec<(SymbolId, f64)> {
        let mut all: Vec<(SymbolId, f64)> = self
            .map
            .iter()
            .map(|(id, cand)| (*id, cand.relevance))
            .collect();
        all.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        all.truncate(count);
        all
    }

    /// The candidate for a symbol, if there is one.
    pub(super) fn get(&self, id: SymbolId) -> Option<&Cand> {
        self.map.get(&id)
    }

    /// The identities of every candidate, in symbol order.
    pub(super) fn ids(&self) -> Vec<SymbolId> {
        self.map.keys().copied().collect()
    }

    /// Consumes the set into candidates ordered by descending relevance, then by identity, and
    /// keeps at most `limit` of them.
    pub(super) fn into_ranked(self, limit: usize) -> Vec<Cand> {
        let mut all: Vec<Cand> = self.map.into_values().collect();
        all.sort_by(|a, b| {
            b.relevance
                .total_cmp(&a.relevance)
                .then(a.record.id.cmp(&b.record.id))
        });
        all.truncate(limit);
        all
    }

    /// Adds the structural (`PageRank`) signal to every candidate's relevance.
    ///
    /// The score is additive and bounded by [`super::ranking::structural_relevance`], so it lifts
    /// hubs without drowning text or neighbor signals. Symbols with no precomputed score get the
    /// floor value, which keeps them visible but never dominant.
    pub(super) fn apply_structural(&mut self) {
        let max = self
            .map
            .values()
            .map(|c| c.record.pagerank)
            .fold(0.0_f64, f64::max);
        for cand in self.map.values_mut() {
            let boost = super::ranking::structural_relevance(cand.record.pagerank, max);
            cand.relevance += boost;
        }
    }

    /// Applies each learned multiplier to its candidate. Symbols missing from `multipliers` keep
    /// their relevance.
    pub(super) fn apply_multipliers(&mut self, multipliers: &dyn Fn(SymbolId) -> Option<f64>) {
        for (id, cand) in &mut self.map {
            if let Some(multiplier) = multipliers(*id) {
                cand.apply_multiplier(multiplier);
            }
        }
    }
}

/// An edge seen while expanding the seeds, kept to describe how the symbols of the capsule relate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SeenEdge {
    /// The symbol the edge starts at.
    pub(super) src: SymbolId,
    /// The symbol the edge ends at.
    pub(super) dst: SymbolId,
    /// What the edge means.
    pub(super) kind: EdgeKind,
    /// How sure the indexer is about it.
    pub(super) confidence: Confidence,
}

/// The edges seen so far, without repeats and in the order they were first seen.
#[derive(Debug, Default)]
pub(super) struct EdgeBook {
    /// The edges.
    edges: Vec<SeenEdge>,
}

impl EdgeBook {
    /// Remembers an edge unless the same one was seen already.
    pub(super) fn add(&mut self, edge: SeenEdge) {
        let known = self
            .edges
            .iter()
            .any(|e| e.src == edge.src && e.dst == edge.dst && e.kind == edge.kind);
        if !known {
            self.edges.push(edge);
        }
    }

    /// Hands over the edges.
    pub(super) fn into_vec(self) -> Vec<SeenEdge> {
        self.edges
    }
}

#[cfg(test)]
mod tests {
    use pn_ultramemory_core::{
        Confidence, EdgeKind, FileId, Language, Span, SymbolId, SymbolKind, SymbolRecord,
        Visibility,
    };

    use super::{CandidateSet, EdgeBook, SeenEdge};

    /// A symbol record with the given identity and name.
    fn record(id: i64, name: &str) -> SymbolRecord {
        SymbolRecord {
            id: SymbolId(id),
            file_id: FileId(1),
            path: "a.rs".into(),
            language: Language::Rust,
            name: name.into(),
            qualified_name: name.into(),
            kind: SymbolKind::Function,
            signature: format!("fn {name}()"),
            doc: None,
            visibility: Visibility::Public,
            span: Span {
                start_line: 1,
                end_line: 2,
                start_byte: 0,
                end_byte: 10,
            },
            parent: None,
            outline: Vec::new(),
            sig_hash: 0,
            body_hash: 0,
            pagerank: 0.0,
        }
    }

    /// A symbol offered twice keeps the highest relevance and both reasons, best first.
    #[test]
    fn offers_merge_by_maximum() {
        let mut set = CandidateSet::default();
        set.offer(&record(1, "a"), 0.4, "text match");
        set.offer(&record(1, "a"), 1.0, "exact name match");
        set.offer(&record(1, "a"), 0.2, "text match");
        let cand = set.get(SymbolId(1)).expect("candidate");
        assert!((cand.relevance - 1.0).abs() < 1e-12);
        assert_eq!(cand.why(), "exact name match; text match");
    }

    /// Useless relevances are ignored and never create a candidate.
    #[test]
    fn invalid_relevances_are_ignored() {
        let mut set = CandidateSet::default();
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            set.offer(&record(1, "a"), bad, "x");
        }
        assert!(set.is_empty());
    }

    /// Ranking is by relevance, then identity, and honours the limit.
    #[test]
    fn ranking_is_total_and_bounded() {
        let mut set = CandidateSet::default();
        set.offer(&record(5, "e"), 0.5, "r");
        set.offer(&record(2, "b"), 0.5, "r");
        set.offer(&record(9, "i"), 0.9, "r");
        set.offer(&record(1, "a"), 0.1, "r");
        assert_eq!(set.len(), 4);
        let top: Vec<i64> = set.top(3).iter().map(|(id, _)| id.0).collect();
        assert_eq!(top, [9, 2, 5]);
        let ranked: Vec<i64> = set.into_ranked(3).iter().map(|c| c.record.id.0).collect();
        assert_eq!(ranked, [9, 2, 5]);
    }

    /// Multipliers scale relevance up to the cap and are explained only when they matter.
    #[test]
    fn multipliers_scale_and_explain() {
        let mut set = CandidateSet::default();
        set.offer(&record(1, "a"), 0.5, "text match");
        set.offer(&record(2, "b"), 0.5, "text match");
        set.offer(&record(3, "c"), 1.4, "text match");
        set.apply_multipliers(&|id| match id.0 {
            1 => Some(1.2),
            2 => Some(1.05),
            3 => Some(1.5),
            _ => None,
        });
        let one = set.get(SymbolId(1)).expect("one");
        assert!((one.relevance - 0.6).abs() < 1e-12);
        assert_eq!(one.why(), "text match; usage x1.2");
        assert_eq!(set.get(SymbolId(2)).expect("two").why(), "text match");
        assert!((set.get(SymbolId(3)).expect("three").relevance - 1.5).abs() < 1e-12);
    }

    /// The same edge is remembered once.
    #[test]
    fn edges_are_deduplicated() {
        let mut book = EdgeBook::default();
        let edge = SeenEdge {
            src: SymbolId(1),
            dst: SymbolId(2),
            kind: EdgeKind::Calls,
            confidence: Confidence::Resolved,
        };
        book.add(edge);
        book.add(edge);
        book.add(SeenEdge {
            kind: EdgeKind::Uses,
            ..edge
        });
        assert_eq!(book.into_vec().len(), 2);
    }
}
