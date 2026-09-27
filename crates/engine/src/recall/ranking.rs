// SPDX-License-Identifier: Apache-2.0
//! The scoring math of recall and the text helpers it shares with the benchmark.
//!
//! Everything here is pure: numbers and strings in, numbers and strings out.

use pn_ultramemory_core::Confidence;

use super::tuning;

/// The relevance of a text match, from its score and the best score of the same result list.
///
/// The best hit gets `1.0`, the weakest possible one [`tuning::TEXT_FLOOR`], and scores that carry
/// no information (a best score that is zero, negative or not a number) give
/// [`tuning::TEXT_UNSCORED`].
///
/// # Examples
/// ```text
/// text_relevance(8.0, 8.0) = 1.0
/// text_relevance(0.0, 8.0) = 0.25
/// text_relevance(1.0, 0.0) = 0.5
/// ```
pub(crate) fn text_relevance(score: f64, best: f64) -> f64 {
    if !best.is_finite() || best <= 0.0 || score.is_nan() {
        return tuning::TEXT_UNSCORED;
    }
    let ratio = (score / best).clamp(0.0, 1.0);
    tuning::TEXT_FLOOR + tuning::TEXT_SPAN * ratio
}

/// The relevance of a symbol that matched some, but not all, words of the query.
///
/// `matched` is the sum, over the words the symbol matched, of that word's normalized score
/// (each in `0..=1`), and `words` the number of words searched for.
pub(crate) fn partial_relevance(matched: f64, words: usize) -> f64 {
    if words == 0 || !matched.is_finite() || matched <= 0.0 {
        return tuning::PARTIAL_FLOOR;
    }
    // A query never holds more words than a `u32` counts, so the conversion is exact; a longer
    // one would only saturate the coverage, which is clamped below anyway.
    let total = f64::from(u32::try_from(words).unwrap_or(u32::MAX));
    let coverage = (matched / total).clamp(0.0, 1.0);
    tuning::PARTIAL_FLOOR + tuning::PARTIAL_SPAN * coverage
}

/// How much an edge of a given confidence weighs when it carries relevance from a seed.
pub(crate) fn edge_weight(confidence: Confidence) -> f64 {
    match confidence {
        Confidence::Exact | Confidence::Resolved => 1.0,
        Confidence::Heuristic => tuning::HEURISTIC_WEIGHT,
        Confidence::Guess => tuning::HEURISTIC_WEIGHT / 2.0,
    }
}

/// The relevance a neighbor inherits from a seed through an edge.
pub(crate) fn neighbor_relevance(seed: f64, confidence: Confidence) -> f64 {
    seed * tuning::NEIGHBOR_FACTOR * edge_weight(confidence)
}

/// The relevance a symbol inherits from a seed it was used together with, given the strength of
/// that pairing. Zero, negative and non-finite strengths inherit nothing.
pub(crate) fn coaccess_relevance(seed: f64, weight: f64) -> f64 {
    if !weight.is_finite() || weight <= 0.0 {
        return 0.0;
    }
    seed * tuning::COACCESS_FACTOR * (weight / tuning::COACCESS_FULL_WEIGHT).min(1.0)
}

/// Applies a learned multiplier to a relevance, never going above [`tuning::RELEVANCE_CAP`].
/// A multiplier that is not a finite positive number counts as `1.0`.
pub(crate) fn boosted(relevance: f64, multiplier: f64) -> f64 {
    let multiplier = if multiplier.is_finite() && multiplier > 0.0 {
        multiplier
    } else {
        1.0
    };
    (relevance * multiplier).min(tuning::RELEVANCE_CAP)
}

/// The words that carry no meaning of their own in a question, dropped by the relaxed search.
const STOP_WORDS: &[&str] = &[
    "a", "about", "after", "all", "also", "an", "and", "any", "are", "as", "at", "be", "been",
    "before", "but", "by", "can", "could", "did", "do", "does", "for", "from", "had", "has",
    "have", "how", "i", "if", "in", "into", "is", "it", "its", "me", "my", "no", "not", "of", "on",
    "or", "our", "please", "should", "so", "that", "the", "their", "then", "there", "these",
    "this", "those", "to", "up", "was", "we", "were", "what", "when", "where", "which", "who",
    "why", "will", "with", "would", "you", "your",
];

/// Whether a lowercase word is a stop word.
pub(crate) fn is_stop_word(word: &str) -> bool {
    STOP_WORDS.binary_search(&word).is_ok()
}

/// The kind of character that decides where a word ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    /// A lowercase letter.
    Lower,
    /// An uppercase letter.
    Upper,
    /// A digit or another numeric character.
    Digit,
    /// A letter without case.
    Other,
    /// Anything that is not a letter or a digit.
    Separator,
}

/// Classifies one character.
fn classify(ch: char) -> Class {
    if ch.is_lowercase() {
        Class::Lower
    } else if ch.is_uppercase() {
        Class::Upper
    } else if ch.is_numeric() {
        Class::Digit
    } else if ch.is_alphabetic() {
        Class::Other
    } else {
        Class::Separator
    }
}

/// Whether a word ends before `current`, given the class before it and after it.
fn breaks_before(previous: Class, current: Class, next: Option<Class>) -> bool {
    match (previous, current) {
        (Class::Separator, _) => false,
        (Class::Lower, Class::Upper) => true,
        (Class::Upper, Class::Upper) => next == Some(Class::Lower),
        (a, b) => a != b && !(a == Class::Upper && b == Class::Lower),
    }
}

/// Splits text into lowercase words the way the full-text index does: at separators, at
/// `camelCase` and `snake_case` boundaries and between letters and digits.
///
/// # Examples
/// ```text
/// parseConfig   -> parse, config
/// HTTPServer2Go -> http, server, 2, go
/// ```
pub(crate) fn split_words(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut previous = Class::Separator;
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        let class = classify(ch);
        let next = chars.peek().map(|c| classify(*c));
        let ends = class == Class::Separator || breaks_before(previous, class, next);
        if ends && !word.is_empty() {
            words.push(std::mem::take(&mut word));
        }
        if class != Class::Separator {
            word.extend(ch.to_lowercase());
        }
        previous = class;
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
}

/// The distinct words of a query that mean something: stop words and one-letter words are dropped,
/// the order of first appearance is kept and at most `limit` are returned.
pub(crate) fn content_terms(text: &str, limit: usize) -> Vec<String> {
    let mut terms: Vec<String> = Vec::new();
    for word in split_words(text) {
        let meaningful = word.chars().count() >= 2 && !is_stop_word(&word);
        if meaningful && !terms.contains(&word) {
            terms.push(word);
            if terms.len() == limit {
                break;
            }
        }
    }
    terms
}

/// The first `max` characters of `text`, with an ellipsis when it was cut.
pub(crate) fn clip_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('\u{2026}');
    out
}

#[cfg(test)]
mod tests {
    use pn_ultramemory_core::Confidence;

    use super::{
        STOP_WORDS, boosted, clip_chars, coaccess_relevance, content_terms, edge_weight,
        is_stop_word, neighbor_relevance, partial_relevance, split_words, text_relevance,
    };

    /// The best hit is one, the worst possible is the floor, and useless scores are neutral.
    #[test]
    fn text_relevance_spans_the_range() {
        assert!((text_relevance(8.0, 8.0) - 1.0).abs() < 1e-12);
        assert!((text_relevance(0.0, 8.0) - 0.25).abs() < 1e-12);
        assert!((text_relevance(4.0, 8.0) - 0.625).abs() < 1e-12);
        assert!((text_relevance(-3.0, 8.0) - 0.25).abs() < 1e-12);
        assert!((text_relevance(9.0, 8.0) - 1.0).abs() < 1e-12);
        for best in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!((text_relevance(1.0, best) - 0.5).abs() < 1e-12);
        }
        assert!((text_relevance(f64::NAN, 2.0) - 0.5).abs() < 1e-12);
    }

    /// Partial matches grow with the number of matched words and stay below an exact one.
    #[test]
    fn partial_relevance_grows_with_coverage() {
        assert!(partial_relevance(1.0, 4) < partial_relevance(2.0, 4));
        assert!(partial_relevance(4.0, 4) <= 0.6 + 1e-12);
        assert!(partial_relevance(0.0, 4) >= 0.15);
        assert!(partial_relevance(1.0, 0) >= 0.15);
        assert!(partial_relevance(f64::NAN, 3) >= 0.15);
    }

    /// Heuristic edges carry less than resolved ones, and both less than the seed.
    #[test]
    fn neighbor_relevance_uses_the_confidence() {
        let resolved = neighbor_relevance(1.0, Confidence::Resolved);
        let exact = neighbor_relevance(1.0, Confidence::Exact);
        let heuristic = neighbor_relevance(1.0, Confidence::Heuristic);
        assert!((resolved - 0.45).abs() < 1e-12);
        assert!((exact - 0.45).abs() < 1e-12);
        assert!((heuristic - 0.315).abs() < 1e-12);
        assert!(edge_weight(Confidence::Guess) < edge_weight(Confidence::Heuristic));
    }

    /// Learned neighbors saturate at a strength of three, and weak or invalid strengths fade out.
    #[test]
    fn coaccess_relevance_saturates() {
        assert!((coaccess_relevance(1.0, 3.0) - 0.35).abs() < 1e-12);
        assert!((coaccess_relevance(1.0, 30.0) - 0.35).abs() < 1e-12);
        assert!((coaccess_relevance(1.0, 1.5) - 0.175).abs() < 1e-12);
        assert!(coaccess_relevance(1.0, 0.0).abs() < 1e-12);
        assert!(coaccess_relevance(1.0, -2.0).abs() < 1e-12);
        assert!(coaccess_relevance(1.0, f64::NAN).abs() < 1e-12);
    }

    /// The learned multiplier scales relevance up to a cap and ignores invalid values.
    #[test]
    fn boost_is_capped() {
        assert!((boosted(0.5, 1.2) - 0.6).abs() < 1e-12);
        assert!((boosted(1.0, 1.5) - 1.5).abs() < 1e-12);
        assert!((boosted(1.4, 1.5) - 1.5).abs() < 1e-12);
        assert!((boosted(0.8, 0.5) - 0.4).abs() < 1e-12);
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!((boosted(0.8, bad) - 0.8).abs() < 1e-12);
        }
    }

    /// Words split like the full-text index splits them.
    #[test]
    fn words_split_on_case_snake_digits_and_separators() {
        assert_eq!(split_words("parseConfig"), ["parse", "config"]);
        assert_eq!(split_words("parse_config"), ["parse", "config"]);
        assert_eq!(split_words("HTTPServer2Go"), ["http", "server", "2", "go"]);
        assert_eq!(split_words("Config::validate()"), ["config", "validate"]);
        assert_eq!(
            split_words("caf\u{e9} au lait"),
            ["caf\u{e9}", "au", "lait"]
        );
        assert!(split_words("  !!! ").is_empty());
        assert!(split_words("").is_empty());
    }

    /// The stop word list is sorted, so that it can be searched by bisection.
    #[test]
    fn stop_words_are_sorted() {
        assert!(STOP_WORDS.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(is_stop_word("the"));
        assert!(!is_stop_word("config"));
    }

    /// Content terms drop stop words, single letters and repeats, and respect the limit.
    #[test]
    fn content_terms_keep_the_meaning() {
        assert_eq!(
            content_terms("How do I load the configuration from a file?", 8),
            ["load", "configuration", "file"]
        );
        assert_eq!(
            content_terms("parse parse PARSE config", 8),
            ["parse", "config"]
        );
        assert_eq!(
            content_terms("alpha beta gamma delta", 2),
            ["alpha", "beta"]
        );
        assert!(content_terms("the of a", 8).is_empty());
    }

    /// Clipping counts characters, not bytes, and marks the cut.
    #[test]
    fn clipping_is_by_character() {
        assert_eq!(clip_chars("short", 10), "short");
        assert_eq!(clip_chars("abcdef", 4), "abc\u{2026}");
        assert_eq!(
            clip_chars("\u{e9}\u{e9}\u{e9}\u{e9}", 3),
            "\u{e9}\u{e9}\u{2026}"
        );
        assert_eq!(
            clip_chars("abc", 0),
            "abc".chars().take(0).collect::<String>() + "\u{2026}"
        );
    }
}
