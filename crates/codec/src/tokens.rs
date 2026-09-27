// SPDX-License-Identifier: Apache-2.0
//! A fast, dependency-free estimate of how many tokens a text costs.
//!
//! The packer needs a cost for every option it can choose, and the real tokenizer differs by
//! model and is far too heavy to bundle. This module instead counts a handful of cheap features in
//! one pass and combines them linearly:
//!
//! ```text
//! tokens ~= 0.4878 * digits + 0.2783 * words + 0.8258 * pieces
//!         + 0.4020 * punctuation + 1.7999 * newlines + 0.8308 * non_ascii
//! ```
//!
//! * *words* are runs of ASCII letters, digits and underscores;
//! * *pieces* are the parts a word splits into at `camelCase`, digit and underscore boundaries
//!   (`parseHTTPConfig2` has four: `parse`, `HTTP`, `Config`, `2`);
//! * *punctuation* is every other ASCII character that is not whitespace;
//! * spaces and tabs are free, because a tokenizer merges them into the next token.
//!
//! # Accuracy
//! The weights were fitted by weighted least squares against the `cl100k_base` tokenizer on 565
//! real files (TypeScript, JavaScript, Python, C, Rust, Markdown, JSON, YAML, TOML, shell, HTML and
//! CSS). On files held out from the fit the mean absolute error was 4.5 %, the median 3.6 % and
//! the worst file 17 %; the naive `characters / 4` rule scored 8.1 % on the same files. It is an
//! **estimate for budgeting**, not a tokenizer: another model's tokenizer will differ, and
//! benchmarks that report billed tokens must use the provider's own counts.

/// Weight of one digit.
const W_DIGIT: f64 = 0.4878;
/// Weight of one word (a run of letters, digits and underscores).
const W_WORD: f64 = 0.2783;
/// Weight of one word piece.
const W_PIECE: f64 = 0.8258;
/// Weight of one punctuation character.
const W_PUNCT: f64 = 0.402;
/// Weight of one newline.
const W_NEWLINE: f64 = 1.7999;
/// Weight of one non-ASCII character.
const W_NON_ASCII: f64 = 0.8308;

/// The counts the estimate is built from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Features {
    /// ASCII digits inside words.
    digits: u32,
    /// Runs of ASCII letters, digits and underscores.
    words: u32,
    /// Word pieces after splitting at case, digit and underscore boundaries.
    pieces: u32,
    /// Non-whitespace ASCII characters that are not part of a word.
    puncts: u32,
    /// Line feeds.
    newlines: u32,
    /// Characters outside ASCII.
    non_ascii: u32,
}

/// Returns `true` for the bytes that make up a word.
const fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// Counts the pieces of one ASCII word and how many of its bytes are digits.
///
/// A piece is a run of capitals not followed by a lowercase letter (`HTTP`), a capital followed
/// by lowercase letters (`Config`), a run of lowercase letters, or a run of digits. Underscores
/// separate pieces and are not pieces themselves.
fn count_pieces(word: &[u8]) -> (u32, u32) {
    let mut pieces = 0_u32;
    let mut digits = 0_u32;
    let mut i = 0;
    while i < word.len() {
        let byte = word[i];
        if byte.is_ascii_digit() {
            let start = i;
            while i < word.len() && word[i].is_ascii_digit() {
                i += 1;
            }
            pieces += 1;
            digits += u32::try_from(i - start).unwrap_or(u32::MAX);
        } else if byte.is_ascii_uppercase() {
            let start = i;
            while i < word.len() && word[i].is_ascii_uppercase() {
                i += 1;
            }
            let followed_by_lower = i < word.len() && word[i].is_ascii_lowercase();
            if followed_by_lower {
                if i - start >= 2 {
                    // `HTTPServer`: the last capital starts the next piece.
                    pieces += 1;
                    i -= 1;
                } else {
                    while i < word.len() && word[i].is_ascii_lowercase() {
                        i += 1;
                    }
                    pieces += 1;
                }
            } else {
                pieces += 1;
            }
        } else if byte.is_ascii_lowercase() {
            while i < word.len() && word[i].is_ascii_lowercase() {
                i += 1;
            }
            pieces += 1;
        } else {
            i += 1;
        }
    }
    (pieces, digits)
}

impl Features {
    /// Counts the features of a text in one pass.
    fn of(text: &str) -> Self {
        let bytes = text.as_bytes();
        let mut features = Self::default();
        let mut i = 0;
        while i < bytes.len() {
            let byte = bytes[i];
            if is_word_byte(byte) {
                let start = i;
                while i < bytes.len() && is_word_byte(bytes[i]) {
                    i += 1;
                }
                let (pieces, digits) = count_pieces(&bytes[start..i]);
                features.words += 1;
                features.pieces += pieces;
                features.digits += digits;
            } else if byte == b'\n' {
                features.newlines += 1;
                i += 1;
            } else if byte.is_ascii_whitespace() {
                i += 1;
            } else if byte.is_ascii() {
                features.puncts += 1;
                i += 1;
            } else {
                // Skip the whole UTF-8 sequence and count one character.
                features.non_ascii += 1;
                i += 1;
                while i < bytes.len() && (bytes[i] & 0b1100_0000) == 0b1000_0000 {
                    i += 1;
                }
            }
        }
        features
    }

    /// Combines the counts into an estimated number of tokens.
    fn estimate(self) -> f64 {
        W_DIGIT * f64::from(self.digits)
            + W_WORD * f64::from(self.words)
            + W_PIECE * f64::from(self.pieces)
            + W_PUNCT * f64::from(self.puncts)
            + W_NEWLINE * f64::from(self.newlines)
            + W_NON_ASCII * f64::from(self.non_ascii)
    }
}

/// Estimates how many tokens `text` costs. Empty text costs zero and any other text costs at
/// least one. See the [module documentation](self) for how it was fitted and how accurate it is.
///
/// # Examples
/// ```
/// use pn_ultramemory_codec::estimate_tokens;
///
/// assert_eq!(estimate_tokens(""), 0);
/// assert!(estimate_tokens("fn parse_config(path: &str) -> Result<Config, Error> {") > 10);
/// // Longer text costs more.
/// assert!(estimate_tokens("hello world, hello world") > estimate_tokens("hello world"));
/// ```
#[must_use]
pub fn estimate_tokens(text: &str) -> u32 {
    if text.is_empty() {
        return 0;
    }
    let raw = Features::of(text).estimate().round();
    // The estimate is non-negative and far below `u32::MAX` for any text that fits in memory.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let tokens = raw.min(f64::from(u32::MAX)) as u32;
    tokens.max(1)
}

#[cfg(test)]
mod tests {
    use super::{Features, count_pieces, estimate_tokens};

    /// Splits a word the way the regular expression used for the fit does, as a reference.
    fn pieces_of(word: &str) -> u32 {
        count_pieces(word.as_bytes()).0
    }

    /// Piece counting follows camelCase, acronym, digit and underscore boundaries.
    #[test]
    fn pieces_follow_case_digit_and_underscore_boundaries() {
        assert_eq!(pieces_of("parse"), 1);
        assert_eq!(pieces_of("parseConfig"), 2);
        assert_eq!(pieces_of("parse_config"), 2);
        assert_eq!(pieces_of("HTTPServer"), 2);
        assert_eq!(pieces_of("parseHTTPConfig2"), 4);
        assert_eq!(pieces_of("ABC"), 1);
        assert_eq!(pieces_of("Ab"), 1);
        assert_eq!(pieces_of("__init__"), 1);
        assert_eq!(pieces_of("a1b2"), 4);
        assert_eq!(pieces_of("_"), 0);
    }

    /// Feature counts for a small snippet match values computed by the reference implementation
    /// used for the fit (a Python script with the same definitions).
    #[test]
    fn features_of_a_snippet_match_the_reference() {
        let text = "fn parse_config(path: &str) -> Result<Config, Error> {\n    x2\n}\n";
        let f = Features::of(text);
        assert_eq!(f.words, 8);
        assert_eq!(f.pieces, 10);
        assert_eq!(f.digits, 1);
        assert_eq!(f.puncts, 11);
        assert_eq!(f.newlines, 3);
        assert_eq!(f.non_ascii, 0);
    }

    /// Non-ASCII characters count once per character, not per byte.
    #[test]
    fn non_ascii_counts_characters() {
        let f = Features::of("año 日本語 🎉");
        assert_eq!(f.non_ascii, 5);
        assert_eq!(f.words, 2);
    }

    /// Whitespace other than newlines is free, and empty text costs nothing.
    #[test]
    fn whitespace_is_free() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("a"), estimate_tokens("   a    "));
        assert!(estimate_tokens("\n") >= 1);
    }

    /// The estimate grows monotonically as text is appended.
    #[test]
    fn estimate_is_monotonic_when_appending() {
        let mut text = String::new();
        let mut last = 0;
        for word in [
            "let", " total", " =", " count", "(", "items", ")", ";", "\n",
        ] {
            text.push_str(word);
            let now = estimate_tokens(&text);
            assert!(now >= last, "{text:?}");
            last = now;
        }
    }

    /// It stays close to the plain `characters / 4` rule on ordinary prose and code.
    #[test]
    fn stays_in_a_sane_range_of_characters_over_four() {
        let text = "The quick brown fox jumps over the lazy dog while the packer chooses \
                    how much detail every symbol deserves under a fixed token budget.";
        let naive = f64::from(u32::try_from(text.len()).unwrap_or(0)) / 4.0;
        let estimate = f64::from(estimate_tokens(text));
        assert!(
            (0.6..1.6).contains(&(estimate / naive)),
            "{estimate} vs {naive}"
        );
    }

    /// Hostile input neither panics nor overflows.
    #[test]
    fn hostile_input_is_handled() {
        let big = "a\n".repeat(200_000);
        assert!(estimate_tokens(&big) > 200_000);
        let invalid_looking = "\u{FFFD}\u{0}\u{7F}\t\r";
        assert!(estimate_tokens(invalid_looking) >= 1);
    }
}
