// SPDX-License-Identifier: Apache-2.0
//! Deciding whether two memories say the same thing.
//!
//! # Why this is deliberately conservative
//! Merging two memories that mean different things destroys information a person wrote down.
//! Failing to merge two identical ones costs a little noise. Those costs are not symmetric, so
//! every threshold here is set to make the first mistake much rarer than the second, and the
//! measured counts are published in `docs/memory.md`.
//!
//! # The measure
//! Two texts are compared in three steps, because no single measure is safe on its own:
//!
//! 1. **Normalize.** Lowercase, drop punctuation that carries no meaning, split into words, and
//!    remove a short list of English stop words. What remains are the *content words*.
//! 2. **Pre-filter.** A 64-bit [`simhash`] of the content words. Two texts whose hashes differ in
//!    more than [`crate::similarity::MAX_HAMMING`] bits are never compared in full. This is what keeps a write cheap
//!    when thousands of memories already exist.
//! 3. **Score.** The mean of two measures: the Jaccard overlap of the word *sets*, which ignores
//!    order, and the length of their longest common subsequence of words, which does not. Using
//!    both stops two texts that share a vocabulary but say opposite things from scoring as
//!    similar, while still merging two ways of writing the same note. A trigram overlap was tried
//!    first and rejected: inserting one word breaks three trigrams, so it refused to merge plain
//!    rewordings.
//!
//! # The short-text rule
//! Below [`crate::similarity::MIN_CONTENT_WORDS`] content words there is not enough signal, and short notes are
//! exactly where two different decisions look alike ("use a file, not env vars" against "use a
//! mutex, not a channel"). Such a pair is judged similar only when its content words are
//! identical.
//!
//! # What this does not do
//! It does not understand meaning. Its stop words are English. It will not recognise a paraphrase
//! that shares no words, and it cannot tell a true statement from a false one.

use std::collections::BTreeSet;

use super::tongue::{Tongue, detect_tongue, split_words};

/// Fewer content words than this and only identical sets count as similar.
pub const MIN_CONTENT_WORDS: usize = 4;

/// How many bits two hashes may differ in before a full comparison is skipped.
///
/// It is set generously: the pre-filter must never discard a pair the full score would have
/// accepted, which a test checks over a generated corpus.
pub const MAX_HAMMING: u32 = 24;

/// At or above this score two memories are the same memory: nothing new is stored.
pub const IDENTICAL: f64 = 0.94;

/// At or above this score two memories say the same thing: the existing one is reinforced.
pub const SIMILAR: f64 = 0.80;

/// Splits a text into its content words: lowercase, punctuation removed, stop words dropped.
///
/// The language is decided from the text itself, so a Spanish memory loses Spanish function words
/// and an English one loses English ones. See [`crate::Tongue`].
///
/// # Examples
/// ```
/// use pn_ultramemory_engine::content_words;
///
/// assert_eq!(content_words("Use a file, not env vars!"), ["use", "file", "not", "env", "vars"]);
/// assert_eq!(content_words("Usar un fichero, no variables de entorno"),
///            ["usar", "un", "fichero", "no", "variables", "entorno"]);
/// assert!(content_words("   ").is_empty());
/// ```
#[must_use]
pub fn content_words(text: &str) -> Vec<String> {
    words_in(text, detect_tongue(text))
}

/// The content words of a text read in a given language.
pub(super) fn words_in(text: &str, tongue: Tongue) -> Vec<String> {
    split_words(text)
        .filter(|word| !tongue.is_stop(word))
        .collect()
}

/// A 64-bit hash of a set of words, built so that similar sets give similar hashes.
///
/// Each word contributes its own hash to a running vector of 64 counters, one per bit; the result
/// takes the sign of each counter. Two sets that share most of their words therefore agree on most
/// bits, which is what makes this usable as a cheap pre-filter.
#[must_use]
pub fn simhash(words: &[String]) -> u64 {
    let unique: BTreeSet<&String> = words.iter().collect();
    let mut counters = [0_i32; 64];
    for word in unique {
        let hash = pn_ultramemory_core::hash64(word.as_bytes());
        for (bit, counter) in counters.iter_mut().enumerate() {
            if hash >> bit & 1 == 1 {
                *counter += 1;
            } else {
                *counter -= 1;
            }
        }
    }
    let mut result = 0_u64;
    for (bit, counter) in counters.iter().enumerate() {
        if *counter > 0 {
            result |= 1 << bit;
        }
    }
    result
}

/// How many bits two hashes differ in.
#[must_use]
pub fn hamming(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

/// The Jaccard overlap of two word sets: how much of their combined vocabulary they share.
fn jaccard(a: &BTreeSet<&str>, b: &BTreeSet<&str>) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let shared = a.intersection(b).count();
    let combined = a.union(b).count();
    if combined == 0 {
        return 0.0;
    }
    #[allow(clippy::cast_precision_loss, reason = "word counts are far below 2^53")]
    let value = shared as f64 / combined as f64;
    value
}

/// The most words compared by the sequence measure, which costs the product of the two lengths.
/// Longer texts are compared by their first words, which is where a note states its point.
const MAX_SEQUENCE_WORDS: usize = 400;

/// The length of the longest common subsequence of two word sequences.
///
/// It is the usual dynamic program, kept to two rows so the memory it uses is proportional to the
/// shorter sequence rather than to their product.
fn longest_common_subsequence(left: &[String], right: &[String]) -> usize {
    let left = &left[..left.len().min(MAX_SEQUENCE_WORDS)];
    let right = &right[..right.len().min(MAX_SEQUENCE_WORDS)];
    if left.is_empty() || right.is_empty() {
        return 0;
    }
    let mut previous = vec![0_usize; right.len() + 1];
    let mut current = vec![0_usize; right.len() + 1];
    for word in left {
        for (index, other) in right.iter().enumerate() {
            current[index + 1] = if word == other {
                previous[index] + 1
            } else {
                current[index].max(previous[index + 1])
            };
        }
        std::mem::swap(&mut previous, &mut current);
        current.fill(0);
    }
    previous[right.len()]
}

/// How much of two sequences is a shared subsequence, from `0.0` to `1.0`.
fn sequence_overlap(left: &[String], right: &[String]) -> f64 {
    let total = left.len() + right.len();
    if total == 0 {
        return 1.0;
    }
    let shared = longest_common_subsequence(left, right);
    #[allow(clippy::cast_precision_loss, reason = "word counts are far below 2^53")]
    let value = (2 * shared) as f64 / total as f64;
    value
}

/// How much two texts say the same thing, from `0.0` to `1.0`.
///
/// See the [module documentation](self) for the measure and for what it deliberately does not do.
///
/// # Examples
/// ```
/// use pn_ultramemory_engine::similarity;
///
/// let a = "Invoices are immutable once issued; corrections are credit notes.";
/// assert!((similarity(a, a) - 1.0).abs() < 1e-9);
///
/// // Two short decisions that share their shape but not their meaning.
/// let left = "Use a file, not env vars";
/// let right = "Use a mutex, not a channel";
/// assert!(similarity(left, right) < 0.5, "{}", similarity(left, right));
/// ```
#[must_use]
pub fn similarity(a: &str, b: &str) -> f64 {
    let left = content_words(a);
    let right = content_words(b);
    similarity_of(&left, &right)
}

/// The similarity of two texts whose content words were already computed.
pub(super) fn similarity_of(left: &[String], right: &[String]) -> f64 {
    if left.is_empty() && right.is_empty() {
        return 1.0;
    }
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let left_set: BTreeSet<&str> = left.iter().map(String::as_str).collect();
    let right_set: BTreeSet<&str> = right.iter().map(String::as_str).collect();

    // Too few words to judge: only an identical set counts, because short notes that differ in one
    // word routinely mean opposite things.
    if left.len() < MIN_CONTENT_WORDS || right.len() < MIN_CONTENT_WORDS {
        return if left_set == right_set { 1.0 } else { 0.0 };
    }

    let by_set = jaccard(&left_set, &right_set);
    let by_order = sequence_overlap(left, right);
    by_set.mul_add(0.5, by_order * 0.5)
}

#[cfg(test)]
mod tests {
    use super::{
        IDENTICAL, MAX_HAMMING, MIN_CONTENT_WORDS, SIMILAR, content_words, hamming, simhash,
        similarity,
    };

    /// Normalization lowercases, drops punctuation and removes stop words.
    #[test]
    fn normalization_keeps_only_content_words() {
        assert_eq!(
            content_words("The parser SHALL reject it!"),
            ["parser", "shall", "reject"]
        );
        assert_eq!(content_words("a an and the"), Vec::<String>::new());
        assert_eq!(
            content_words("snake_case and camelCase"),
            ["snake_case", "camelcase"]
        );
    }

    /// A text is identical to itself, and to itself with different punctuation and case.
    #[test]
    fn a_text_matches_itself() {
        let text = "Handlers return a result type and never unwrap in request paths";
        assert!((similarity(text, text) - 1.0).abs() < 1e-9);
        let noisy = "HANDLERS return a result type, and never unwrap in request paths!!";
        assert!(
            similarity(text, noisy) >= IDENTICAL,
            "{}",
            similarity(text, noisy)
        );
    }

    /// Short texts that share their shape but not their meaning are not similar. This is the
    /// case the thresholds exist to get right.
    #[test]
    fn short_texts_that_differ_in_meaning_are_not_similar() {
        let pairs = [
            ("Use a file, not env vars", "Use a mutex, not a channel"),
            ("Retry on timeout", "Retry on conflict"),
            ("Cache the result", "Drop the result"),
            ("Prefer Postgres", "Prefer SQLite"),
            ("Never log tokens", "Never log paths"),
        ];
        for (left, right) in pairs {
            let score = similarity(left, right);
            assert!(score < SIMILAR, "`{left}` vs `{right}` scored {score}");
        }
    }

    /// Longer texts that say the same thing in different words are similar enough to merge.
    #[test]
    fn rewordings_of_the_same_note_are_similar() {
        let pairs = [
            (
                "Deadlock in the repository save path was fixed by taking row locks in identifier order",
                "Deadlock in the repository save path fixed by taking row locks in identifier order",
            ),
            (
                "Money is stored as integer minor units and never as a floating point number",
                "Money is stored as integer minor units, never as a floating point number",
            ),
        ];
        for (left, right) in pairs {
            let score = similarity(left, right);
            assert!(score >= SIMILAR, "`{left}` vs `{right}` scored {score}");
        }
    }

    /// Two notes about different subjects are never similar, however alike their grammar.
    #[test]
    fn different_subjects_are_not_similar() {
        let left = "Payments retry with exponential backoff, at most five attempts";
        let right = "Dashboard charts read from a view refreshed every five minutes";
        let score = similarity(left, right);
        assert!(score < 0.3, "scored {score}");
    }

    /// A table of realistic developer notes: report the counts, and require that no pair that
    /// means different things is judged similar.
    #[test]
    fn the_threshold_has_no_false_positives_on_a_realistic_table() {
        // Pairs that MUST merge.
        let same = [
            (
                "The parser rejects a file larger than the configured limit",
                "The parser rejects any file larger than the configured limit",
            ),
            (
                "Tests for the export need a fixed clock to stay stable",
                "Tests for the export need a fixed clock so they stay stable",
            ),
            (
                "Handlers never unwrap inside a request path, they return an error",
                "Handlers never unwrap inside a request path; they return an error",
            ),
            (
                "Caching the tax rules per request was slower than reading them once at start",
                "Caching tax rules per request was slower than reading them once at start",
            ),
        ];
        // Pairs that MUST NOT merge.
        let different = [
            ("Use a file, not env vars", "Use a mutex, not a channel"),
            (
                "The parser rejects a file larger than the configured limit",
                "The parser accepts a file larger than the configured limit",
            ),
            (
                "Payments retry with exponential backoff at most five attempts",
                "Payments fail immediately and are never retried",
            ),
            (
                "Money is stored as integer minor units, never floating point",
                "Dates are stored as integer epoch seconds, never strings",
            ),
            (
                "Invoices are immutable once issued and corrections are credit notes",
                "Invoices may be edited until they are issued to the customer",
            ),
            (
                "Prefer the pooled connection",
                "Prefer the direct connection",
            ),
        ];

        let mut false_negatives = 0;
        for (left, right) in same {
            if similarity(left, right) < SIMILAR {
                false_negatives += 1;
            }
        }
        let mut false_positives = 0;
        for (left, right) in different {
            let score = similarity(left, right);
            if score >= SIMILAR {
                false_positives += 1;
                println!("false positive: `{left}` vs `{right}` scored {score}");
            }
        }
        println!("false positives {false_positives}, false negatives {false_negatives}");
        assert_eq!(
            false_positives, 0,
            "merging two different memories is the costly mistake"
        );
        assert!(
            false_negatives <= 1,
            "{false_negatives} rewordings failed to merge"
        );
    }

    /// The pre-filter never discards a pair the full score would have accepted.
    #[test]
    fn the_prefilter_never_hides_a_similar_pair() {
        let base = [
            "the parser rejects a file larger than the configured limit and reports the size",
            "handlers return an error type and never unwrap inside a request path at all",
            "money is stored as integer minor units and never as a floating point number",
            "caching the tax rules per request was slower than reading them once at start",
        ];
        let mut checked = 0;
        for text in base {
            let words: Vec<String> = text.split(' ').map(str::to_owned).collect();
            // Every prefix long enough to be judged, against the whole text.
            for cut in MIN_CONTENT_WORDS..words.len() {
                let part = words[..cut].join(" ");
                let score = similarity(text, &part);
                if score >= SIMILAR {
                    let distance = hamming(
                        simhash(&super::content_words(text)),
                        simhash(&super::content_words(&part)),
                    );
                    assert!(
                        distance <= MAX_HAMMING,
                        "score {score} but hashes differ in {distance} bits: `{part}`"
                    );
                    checked += 1;
                }
            }
        }
        assert!(checked > 0, "the test must actually exercise similar pairs");
    }

    /// The hash of a set does not depend on the order the words arrive in.
    #[test]
    fn the_hash_ignores_word_order() {
        let forward = content_words("alpha beta gamma delta");
        let mut backward = forward.clone();
        backward.reverse();
        assert_eq!(simhash(&forward), simhash(&backward));
        assert_eq!(hamming(simhash(&forward), simhash(&forward)), 0);
    }

    /// Empty and whitespace-only texts behave, and never panic.
    #[test]
    fn empty_texts_are_handled() {
        assert!((similarity("", "") - 1.0).abs() < 1e-9);
        assert!(similarity("", "something at all here").abs() < 1e-9);
        assert!((similarity("   \n\t ", "") - 1.0).abs() < 1e-9);
        assert_eq!(simhash(&[]), 0);
    }

    /// Unicode, very long text and odd characters never panic.
    #[test]
    fn hostile_input_is_handled() {
        let long = "palabra ".repeat(20_000);
        assert!((0.0..=1.0).contains(&similarity(&long, &long)));
        let odd = "日本語 emoji 🎉 acentuación ñ";
        assert!((similarity(odd, odd) - 1.0).abs() < 1e-9);
        assert!((0.0..=1.0).contains(&similarity(odd, &long)));
    }

    /// The thresholds keep the order the design depends on.
    ///
    /// Checked at compile time, so changing a threshold to something the rest of the module cannot
    /// hold up stops the build instead of a test run.
    #[test]
    fn the_thresholds_are_ordered() {
        const {
            assert!(SIMILAR < IDENTICAL);
            assert!(IDENTICAL < 1.0);
            assert!(
                SIMILAR > 0.5,
                "a threshold below one half would merge unrelated notes"
            );
        }
    }
}
