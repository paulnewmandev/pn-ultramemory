// SPDX-License-Identifier: Apache-2.0
//! Noticing that two memories about the same code contradict each other.
//!
//! # Conservative on purpose
//! Telling a person that two notes contradict when they do not wastes their attention and teaches
//! them to ignore the warning. Missing a real contradiction only leaves things as they were. So
//! this aims for **high precision and accepts low recall**: it reports only opposition that is
//! visible in the structure of the two texts, never a guess at meaning.
//!
//! # What it detects
//! 1. **Negation of the same claim.** The two texts share most of their content words, and **one
//!    of them denies something the other states plainly**: "the parser rejects a large file"
//!    against "the parser does **not** reject a large file", or "calibrated, **not** guessed"
//!    against "**not** calibrated". The second shape is why this module exists at all: a denial
//!    changes almost none of a sentence's words, so a memory could otherwise be reinforced by its
//!    own opposite.
//! 2. **Two answers to one choice.** Both are decisions or conventions about the same code, both
//!    say to use something, and they name different things. "Use Postgres" against "Use SQLite".
//!
//! # Why the denials are compared, and not merely counted
//! An earlier version also reported a pair when exactly one text carried a negation marker at
//! all. That is too coarse, and it produced a real false report: "calibrated against a real
//! tokenizer, **not** guessed" against "calibrated against an actual tokenizer **rather than**
//! guessed". Those agree — both deny being guessed — but only the first says so with a word on
//! the list. A negation is evidence of opposition only once it is known *what* it denies, so that
//! is the only test left.
//!
//! # What it does not detect
//! A contradiction expressed in different words, one that needs knowledge of the domain, or one
//! spread across several memories. It also never resolves anything: it reports, and a person
//! decides.

use std::collections::BTreeSet;

use pn_ultramemory_core::MemoryKind;

use super::similarity::content_words;

/// Words that turn a claim into its opposite.
///
/// `instead` and `rather` are here because of what follows them: "instead of X" and "rather than
/// X" both deny X, and `of` and `than` are stop words, so the marker ends up next to the word it
/// denies exactly as `not` does.
const NEGATIONS: [&str; 11] = [
    "not", "never", "no", "none", "don't", "doesn't", "avoid", "stop", "without", "instead",
    "rather",
];

/// Words that introduce the thing a decision picked.
const CHOICE_MARKERS: [&str; 6] = ["use", "prefer", "choose", "adopt", "switch", "keep"];

/// How much of their vocabulary two texts must share before opposition is even considered.
const MIN_SHARED: f64 = 0.45;

/// Why two memories look like they contradict each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// One says a thing and the other denies it.
    Negated,
    /// Both pick something for the same slot, and they pick differently.
    DifferentChoice,
}

impl Reason {
    /// A short, stable name used in output.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Negated => "negated",
            Self::DifferentChoice => "different_choice",
        }
    }

    /// A sentence a person can read.
    #[must_use]
    pub const fn describe(self) -> &'static str {
        match self {
            Self::Negated => "one of these denies what the other states",
            Self::DifferentChoice => "these pick different things for the same decision",
        }
    }
}

/// The content words of a text that are not negations or choice markers.
fn subject_words(words: &[String]) -> Vec<&str> {
    words
        .iter()
        .map(String::as_str)
        .filter(|word| !NEGATIONS.contains(word) && !CHOICE_MARKERS.contains(word))
        .collect()
}

/// How much of their vocabulary two word lists share, ignoring order.
fn shared_fraction(left: &[&str], right: &[&str]) -> f64 {
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let left_set: BTreeSet<&&str> = left.iter().collect();
    let right_set: BTreeSet<&&str> = right.iter().collect();
    let shared = left_set.intersection(&right_set).count();
    let smaller = left_set.len().min(right_set.len());
    if smaller == 0 {
        return 0.0;
    }
    #[allow(clippy::cast_precision_loss, reason = "word counts are far below 2^53")]
    let value = shared as f64 / smaller as f64;
    value
}

/// The shortest a stripped word may be and still be treated as a root.
const MIN_ROOT_LEN: usize = 4;

/// The suffixes stripped to find a root, longest first so that `-es` is not read as `-s`.
const SUFFIXES: [&str; 4] = ["ing", "ed", "es", "s"];

/// A crude root of a word, used only to compare a denial against a plain statement.
///
/// English marks the same verb differently in the two places that matter here: a denial writes
/// "does not **reject**" while the statement it denies writes "the parser **rejects**". Comparing
/// the words as written would therefore miss the commonest shape of a contradiction entirely.
///
/// Only four suffixes are stripped, and only when what remains is still long enough to be a word.
/// An aggressive stem would make unrelated words equal, and every such collision here turns two
/// notes that agree into a false report, which is the one mistake this module must not make.
///
/// # Examples
/// ```text
/// assert_eq!(root("rejects"), "reject");
/// assert_eq!(root("reject"), "reject");
/// assert_eq!(root("used"), "used");  // stripping would leave "us", too short to be a root
/// ```
fn root(word: &str) -> &str {
    for suffix in SUFFIXES {
        if let Some(stem) = word.strip_suffix(suffix) {
            if stem.len() >= MIN_ROOT_LEN {
                return stem;
            }
        }
    }
    word
}

/// The roots a text denies: those directly following a negation marker.
fn denied_roots(words: &[String]) -> BTreeSet<&str> {
    words
        .windows(2)
        .filter_map(|pair| match pair {
            [marker, denied] if NEGATIONS.contains(&marker.as_str()) => Some(root(denied)),
            _ => None,
        })
        .collect()
}

/// Whether `words` states any of `denied` plainly, that is without denying it itself.
fn states_any(words: &[String], own_denials: &BTreeSet<&str>, denied: &BTreeSet<&str>) -> bool {
    denied.iter().any(|target| {
        !own_denials.contains(target) && words.iter().any(|word| root(word) == *target)
    })
}

/// Whether one text denies something the other states plainly.
///
/// This is the whole negation test. It is deliberately symmetric and deliberately narrow: a
/// negation counts only when the thing it denies is present, undenied, in the other text. Two
/// texts that deny the same thing agree, however differently they word the denial, and a text that
/// denies something the other never mentions is not opposition at all.
fn denies_what_the_other_states(left: &[String], right: &[String]) -> bool {
    let left_denied = denied_roots(left);
    let right_denied = denied_roots(right);
    states_any(right, &right_denied, &left_denied) || states_any(left, &left_denied, &right_denied)
}

/// The word a choice marker introduces, if the text makes a choice at all.
fn chosen(words: &[String]) -> Option<&str> {
    words.windows(2).find_map(|pair| match pair {
        [marker, picked] if CHOICE_MARKERS.contains(&marker.as_str()) => Some(picked.as_str()),
        _ => None,
    })
}

/// Whether two memories of the same kind, about the same code, contradict each other.
///
/// `kind` is the kind both memories share; a pair of different kinds is never reported, because a
/// lesson and a decision about the same code are usually complementary rather than opposed.
///
/// # Examples
/// ```
/// use pn_ultramemory_core::MemoryKind;
/// use pn_ultramemory_engine::contradicts;
///
/// let stated = "The parser rejects a file larger than the configured limit";
/// let denied = "The parser does not reject a file larger than the configured limit";
/// assert!(contradicts(stated, denied, MemoryKind::Fact).is_some());
///
/// let unrelated = "Dashboard charts read from a refreshed view";
/// assert!(contradicts(stated, unrelated, MemoryKind::Fact).is_none());
/// ```
#[must_use]
pub fn contradicts(a: &str, b: &str, kind: MemoryKind) -> Option<Reason> {
    let left = content_words(a);
    let right = content_words(b);
    if left.is_empty() || right.is_empty() {
        return None;
    }

    let left_subject = subject_words(&left);
    let right_subject = subject_words(&right);
    if shared_fraction(&left_subject, &right_subject) < MIN_SHARED {
        return None;
    }

    // One denies something the other states plainly. Merely carrying a negation the other lacks
    // proves nothing: it may deny something the other never mentions, or deny the same thing in
    // words this module does not know.
    if denies_what_the_other_states(&left, &right) {
        return Some(Reason::Negated);
    }

    // Both pick something, for the same decision, and pick differently.
    if matches!(kind, MemoryKind::Decision | MemoryKind::Convention) {
        if let (Some(first), Some(second)) = (chosen(&left), chosen(&right)) {
            if first != second {
                return Some(Reason::DifferentChoice);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{Reason, contradicts};
    use pn_ultramemory_core::MemoryKind;

    /// A denial of the same claim is reported.
    #[test]
    fn a_denial_is_reported() {
        let pairs = [
            (
                "The parser rejects a file larger than the configured limit",
                "The parser does not reject a file larger than the configured limit",
            ),
            (
                "Handlers unwrap inside a request path when the value is known",
                "Handlers never unwrap inside a request path when the value is known",
            ),
            (
                "Money is stored as a floating point number in the ledger table",
                "Money is not stored as a floating point number in the ledger table",
            ),
        ];
        for (left, right) in pairs {
            assert_eq!(
                contradicts(left, right, MemoryKind::Fact),
                Some(Reason::Negated),
                "`{left}` vs `{right}`"
            );
        }
    }

    /// Two texts that both deny, but deny different things, are reported. This is the case a
    /// plain "one denies, the other does not" test misses, and the one that would otherwise let a
    /// memory be reinforced by its own opposite.
    #[test]
    fn denying_different_things_is_reported() {
        let pairs = [
            (
                "The token estimator is calibrated against a real tokenizer, not guessed",
                "The token estimator is not calibrated against a real tokenizer",
            ),
            (
                "Handlers return an error and never unwrap inside a request path",
                "Handlers never return an error inside a request path, they unwrap",
            ),
        ];
        for (left, right) in pairs {
            assert_eq!(
                contradicts(left, right, MemoryKind::Decision),
                Some(Reason::Negated),
                "`{left}` vs `{right}`"
            );
        }
    }

    /// Two texts that deny the same thing agree, and are not reported.
    #[test]
    fn denying_the_same_thing_is_agreement() {
        let left = "Handlers never unwrap inside a request path, they return an error";
        let right = "Handlers never unwrap in a request path; they return an error instead";
        assert_eq!(contradicts(left, right, MemoryKind::Convention), None);
    }

    /// The same denial worded without a word from the list is still the same denial.
    ///
    /// This pair came out of a live run and was reported as a contradiction by an earlier version
    /// that asked only whether one text carried a negation and the other did not. Both deny being
    /// guessed; only the first says so with `not`. It is kept because it is the exact shape of the
    /// mistake this module must never make.
    #[test]
    fn a_denial_worded_differently_is_not_opposition() {
        let left = "The token estimator is calibrated against a real tokenizer, not guessed";
        let right =
            "Token estimation is calibrated against an actual tokenizer rather than guessed";
        for kind in MemoryKind::ALL {
            assert_eq!(contradicts(left, right, kind), None, "as {kind}");
        }
    }

    /// A denial and the statement it denies are recognised across the endings English puts on the
    /// same verb, which is the commonest shape a contradiction takes.
    #[test]
    fn a_denial_is_matched_across_word_endings() {
        let pairs = [
            (
                "The importer skips a row whose identifier is already present",
                "The importer does not skip a row whose identifier is already present",
            ),
            (
                "The exporter writes a header before the first record",
                "The exporter is not writing a header before the first record",
            ),
        ];
        for (left, right) in pairs {
            assert_eq!(
                contradicts(left, right, MemoryKind::Fact),
                Some(Reason::Negated),
                "`{left}` vs `{right}`"
            );
        }
    }

    /// Denying something the other text never mentions is not opposition, however much of their
    /// vocabulary the two share.
    #[test]
    fn denying_something_unmentioned_is_not_opposition() {
        let left = "The importer reads the ledger file in one pass and holds no row in memory";
        let right = "The importer reads the ledger file in one pass without a temporary table";
        assert_eq!(contradicts(left, right, MemoryKind::Fact), None);
    }

    /// Two decisions that pick differently for the same slot are reported.
    #[test]
    fn two_different_choices_are_reported() {
        let left = "Use Postgres for the ledger store and the reporting replica";
        let right = "Use SQLite for the ledger store and the reporting replica";
        assert_eq!(
            contradicts(left, right, MemoryKind::Decision),
            Some(Reason::DifferentChoice)
        );
        assert_eq!(
            contradicts(left, right, MemoryKind::Convention),
            Some(Reason::DifferentChoice)
        );
        // The same pair as plain facts is not a decision, so it is not reported as one.
        assert_eq!(contradicts(left, right, MemoryKind::Fact), None);
    }

    /// Notes about different subjects are never reported, however alike their grammar.
    #[test]
    fn unrelated_notes_are_not_reported() {
        let pairs = [
            (
                "The parser rejects a file larger than the configured limit",
                "Dashboard charts read from a view refreshed every five minutes",
            ),
            (
                "Payments retry with exponential backoff at most five attempts",
                "Invoices are immutable once issued and corrections are credit notes",
            ),
            (
                "Use Postgres for the ledger",
                "Never log a token in the request path",
            ),
            ("", "The parser rejects a large file"),
            ("The parser rejects a large file", ""),
        ];
        for (left, right) in pairs {
            for kind in [
                MemoryKind::Fact,
                MemoryKind::Decision,
                MemoryKind::Convention,
            ] {
                assert_eq!(
                    contradicts(left, right, kind),
                    None,
                    "`{left}` vs `{right}` as {kind}"
                );
            }
        }
    }

    /// Two notes that agree are not a contradiction, including a reworded one.
    #[test]
    fn agreement_is_not_a_contradiction() {
        let pairs = [
            (
                "The parser rejects a file larger than the configured limit",
                "The parser rejects any file larger than the configured limit",
            ),
            (
                "Handlers never unwrap inside a request path, they return an error",
                "Handlers never unwrap in a request path; they return an error instead",
            ),
            (
                "Use Postgres for the ledger store",
                "Use Postgres for the ledger store",
            ),
        ];
        for (left, right) in pairs {
            for kind in [MemoryKind::Fact, MemoryKind::Decision] {
                assert_eq!(
                    contradicts(left, right, kind),
                    None,
                    "`{left}` vs `{right}`"
                );
            }
        }
    }

    /// A text is never reported as contradicting itself.
    #[test]
    fn nothing_contradicts_itself() {
        for text in [
            "The parser rejects a large file",
            "Never unwrap inside a request path",
            "Use Postgres for the ledger",
        ] {
            for kind in MemoryKind::ALL {
                assert_eq!(contradicts(text, text, kind), None, "{text} as {kind}");
            }
        }
    }

    /// The reasons carry a stable name and a readable sentence.
    #[test]
    fn reasons_describe_themselves() {
        assert_eq!(Reason::Negated.as_str(), "negated");
        assert_eq!(Reason::DifferentChoice.as_str(), "different_choice");
        assert!(Reason::Negated.describe().contains("denies"));
        assert!(Reason::DifferentChoice.describe().contains("different"));
    }
}
