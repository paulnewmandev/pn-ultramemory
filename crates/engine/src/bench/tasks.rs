// SPDX-License-Identifier: Apache-2.0
//! Choosing the symbols the benchmark asks for, and turning each one into two queries.
//!
//! # Role in the architecture
//! Application layer, the sampling half of the offline benchmark. It reads the index, keeps the
//! symbols that carry enough documentation to build a question from, and draws a sample of them with
//! a generator seeded by the caller.
//!
//! # The two queries
//! * **description**: the first sentence of the symbol's own documentation, with every word that also
//!   appears in the symbol's name removed. Without that removal the query would simply contain the
//!   answer, and the measurement would be of string matching rather than of retrieval. A word counts
//!   as appearing in the name when it is one of its words, or when one of the two is a prefix of the
//!   other and the shorter has at least [`MIN_STEM_CHARS`] characters, so that `parses` does not
//!   survive the name `parse` and `configuration` does not survive `config`.
//! * **name**: the symbol's name split into lowercase words, which is what an agent types when it
//!   half-remembers an identifier.
//!
//! Either query can come out empty (a documentation sentence made only of words from the name, a
//! name with no letters), and an empty query is not run: the row then reports the number of tasks
//! that were.
//!
//! # Invariants
//! * **Deterministic.** The generator depends only on the seed, the sample is drawn in one pass over
//!   the index in path order, and the drawn tasks are then sorted by path, line and identity.
//! * **Bounded.** Memory is proportional to the sample, not to the repository: reservoir sampling
//!   keeps exactly as many candidates as were asked for.

use pn_ultramemory_core::{SymbolId, SymbolKind, first_sentence};

use crate::engine::Engine;
use crate::error::EngineError;
use crate::recall::ranking::split_words;

/// The kinds of symbol a benchmark task can be about.
const TASK_KINDS: [SymbolKind; 5] = [
    SymbolKind::Function,
    SymbolKind::Method,
    SymbolKind::Class,
    SymbolKind::Struct,
    SymbolKind::Interface,
];

/// How many words of documentation a symbol needs before a question can be built from it.
const MIN_DOC_WORDS: usize = 5;

/// The shortest shared prefix that makes two words count as the same word.
const MIN_STEM_CHARS: usize = 3;

/// A deterministic pseudo-random generator (xorshift64 star), so that a seed fixes the sample and
/// no dependency is needed.
#[derive(Debug, Clone)]
pub(crate) struct Rng(u64);

impl Rng {
    /// A generator for `seed`. Any seed works, including zero.
    pub(crate) const fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03 | 1)
    }

    /// The next 64-bit value.
    pub(crate) const fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A value below `bound`, or zero when `bound` is zero.
    pub(crate) fn below(&mut self, bound: usize) -> usize {
        let limit = u64::try_from(bound).unwrap_or(u64::MAX);
        if limit == 0 {
            return 0;
        }
        usize::try_from(self.next_u64() % limit).unwrap_or(0)
    }
}

/// One symbol the benchmark asks for, with both queries already built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Task {
    /// Identity of the symbol a capsule has to contain for the task to count as a hit.
    pub(crate) id: SymbolId,
    /// Path of the file that declares it, which the baseline has to choose for its own hit.
    pub(crate) path: String,
    /// The 1-based line the declaration starts on, used only to order the tasks.
    pub(crate) line: u32,
    /// The description query, or empty when nothing was left after removing the name's words.
    pub(crate) description: String,
    /// The name query, or empty when the name holds no letters or digits.
    pub(crate) name: String,
}

/// The lowercase words of an identifier, split at case, digit and underscore boundaries, with the
/// whole lowercase name added so that a one-word name is also matched.
fn name_words(name: &str) -> Vec<String> {
    let mut words = split_words(name);
    let whole = name.to_lowercase();
    if !whole.is_empty() && !words.contains(&whole) {
        words.push(whole);
    }
    words
}

/// The words of a text in order, each once.
fn distinct_words(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for word in split_words(text) {
        if !out.contains(&word) {
            out.push(word);
        }
    }
    out
}

/// Joins words with single spaces.
fn joined(words: &[String]) -> String {
    let mut out = String::new();
    for word in words {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    out
}

/// Returns `true` when a word of a sentence repeats a word of the name.
///
/// Equality is the obvious case. The prefix rule catches the inflections a sentence uses for the
/// same word, which would otherwise leave the answer inside the question.
fn repeats(word: &str, forbidden: &[String]) -> bool {
    forbidden.iter().any(|candidate| {
        let (short, long) = if word.len() <= candidate.len() {
            (word, candidate.as_str())
        } else {
            (candidate.as_str(), word)
        };
        short == long || (short.len() >= MIN_STEM_CHARS && long.starts_with(short))
    })
}

/// The description query: the first documentation sentence without the words of the name.
pub(crate) fn description_query(name: &str, doc: &str) -> String {
    let forbidden = name_words(name);
    let sentence = first_sentence(doc);
    let kept: Vec<String> = distinct_words(&sentence)
        .into_iter()
        .filter(|word| !repeats(word, &forbidden))
        .collect();
    joined(&kept)
}

/// The name query: the name split into lowercase words.
pub(crate) fn name_query(name: &str) -> String {
    joined(&distinct_words(name))
}

/// Returns `true` when a symbol can carry a task: the right kind, and enough documentation.
fn is_candidate(kind: SymbolKind, doc: Option<&str>) -> bool {
    TASK_KINDS.contains(&kind)
        && doc.is_some_and(|text| text.split_whitespace().count() >= MIN_DOC_WORDS)
}

/// Draws up to `count` tasks from the index with a generator seeded by `seed`.
///
/// Every eligible symbol has the same chance of being drawn (reservoir sampling), and the result is
/// sorted by path, line and identity so that the order does not depend on which symbols were drawn
/// first.
///
/// # Errors
/// Returns a storage error when the files or their symbols cannot be read.
pub(crate) fn sample_tasks(
    engine: &Engine,
    count: usize,
    seed: u64,
) -> Result<Vec<Task>, EngineError> {
    let mut rng = Rng::new(seed);
    let mut reservoir: Vec<Task> = Vec::with_capacity(count.min(1024));
    let mut examined = 0_usize;
    if count == 0 {
        return Ok(reservoir);
    }
    for file in engine.storage().list_files()? {
        for symbol in engine.storage().symbols_in_file(&file.path)? {
            let doc = symbol.doc.as_deref();
            if !is_candidate(symbol.kind, doc) {
                continue;
            }
            let task = Task {
                id: symbol.id,
                path: symbol.path,
                line: symbol.span.start_line,
                description: description_query(&symbol.name, doc.unwrap_or_default()),
                name: name_query(&symbol.name),
            };
            if reservoir.len() < count {
                reservoir.push(task);
            } else {
                let slot = rng.below(examined + 1);
                if let Some(entry) = reservoir.get_mut(slot) {
                    *entry = task;
                }
            }
            examined += 1;
        }
    }
    reservoir.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then(left.line.cmp(&right.line))
            .then(left.id.cmp(&right.id))
    });
    Ok(reservoir)
}

#[cfg(test)]
mod tests {
    use super::{
        MIN_DOC_WORDS, Rng, description_query, distinct_words, is_candidate, name_query,
        name_words, repeats,
    };
    use pn_ultramemory_core::SymbolKind;

    /// The generator repeats for a seed and stays below the bound, including a bound of zero.
    #[test]
    fn generator_is_deterministic_and_bounded() {
        let mut left = Rng::new(1);
        let mut right = Rng::new(1);
        for _ in 0..500 {
            let value = left.below(17);
            assert_eq!(value, right.below(17));
            assert!(value < 17);
        }
        assert_eq!(Rng::new(0).below(0), 0);
        assert_ne!(Rng::new(1).next_u64(), Rng::new(2).next_u64());
    }

    /// A name contributes its pieces and its whole lowercase form.
    #[test]
    fn names_split_into_words() {
        assert_eq!(
            name_words("parse_config"),
            ["parse", "config", "parse_config"]
        );
        assert_eq!(name_words("fetchUser"), ["fetch", "user", "fetchuser"]);
        assert_eq!(name_words("Config"), ["config"]);
    }

    /// Words are kept once each, in order of first appearance.
    #[test]
    fn words_are_distinct_and_ordered() {
        assert_eq!(distinct_words("the cat the hat"), ["the", "cat", "hat"]);
        assert_eq!(distinct_words("  "), Vec::<String>::new());
    }

    /// A word repeats the name when it matches it or shares a long enough prefix with it.
    #[test]
    fn inflections_count_as_the_name() {
        let forbidden = name_words("parse_config");
        assert!(repeats("parse", &forbidden));
        assert!(repeats("parses", &forbidden));
        assert!(repeats("configuration", &forbidden));
        assert!(!repeats("value", &forbidden));
        let short = name_words("id");
        assert!(repeats("id", &short));
        assert!(!repeats("identifier", &short), "two letters is not a stem");
    }

    /// The description query drops every word the name already holds.
    #[test]
    fn description_removes_the_name() {
        let query = description_query("parse_config", "Parses configuration text into a value.");
        assert_eq!(query, "text into a value");
        let only_name = description_query("load", "Loads. Something else entirely.");
        assert_eq!(only_name, "");
        let camel = description_query("fetchUser", "Fetches a user by id.");
        assert_eq!(camel, "a by id");
    }

    /// The description query stops at the first sentence.
    #[test]
    fn description_uses_one_sentence() {
        let query = description_query("f", "Returns the port. Never fails, ever.");
        assert_eq!(query, "returns the port");
    }

    /// The name query is the lowercase pieces of the name.
    #[test]
    fn name_query_splits_the_identifier() {
        assert_eq!(name_query("parseHTTPConfig"), "parse http config");
        assert_eq!(name_query("__init__"), "init");
        assert_eq!(name_query("___"), "");
    }

    /// Only the five kinds with enough documentation are candidates.
    #[test]
    fn candidate_rules() {
        let doc = "One two three four five.";
        assert!(is_candidate(SymbolKind::Function, Some(doc)));
        assert!(is_candidate(SymbolKind::Interface, Some(doc)));
        assert!(!is_candidate(SymbolKind::Constant, Some(doc)));
        assert!(!is_candidate(SymbolKind::Function, None));
        let short = "One two three four";
        assert_eq!(short.split_whitespace().count(), MIN_DOC_WORDS - 1);
        assert!(!is_candidate(SymbolKind::Function, Some(short)));
    }
}
