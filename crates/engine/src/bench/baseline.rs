// SPDX-License-Identifier: Apache-2.0
//! The arm the engine is measured against: read whole files, chosen by counting query words.
//!
//! # Role in the architecture
//! Application layer, the control group of the offline benchmark. It is deliberately the crudest
//! thing an agent can do without any index: rank every file by how many distinct words of the query
//! it contains, read the best few in full, and hope the answer is in there. Its hit is the symbol's
//! own file being among the files read, and its cost is what those files would cost in tokens.
//!
//! # Invariants
//! * **Each file is read once.** The corpus keeps, per file, the hashes of its distinct lowercase
//!   words and its token cost, and the source text is dropped straight away. That is what makes
//!   hundreds of queries over a large repository affordable.
//! * **Bounded memory.** At most [`CACHE_BYTES`] of source are read. A larger repository is sampled
//!   with a fixed stride in path order, which is deterministic, and the files the tasks are about are
//!   always kept so that the baseline is never denied a hit it could have had.
//! * **Deterministic ranking.** Files are ordered by score and then by path, so ties never depend on
//!   iteration order.

use std::collections::BTreeSet;

use pn_ultramemory_codec::estimate_tokens;
use pn_ultramemory_core::hash64;

use crate::engine::Engine;
use crate::error::EngineError;
use crate::recall::ranking::split_words;

/// How many bytes of source the corpus reads before it starts sampling files.
const CACHE_BYTES: u64 = 256 * 1024 * 1024;

/// One indexed file as the baseline sees it.
#[derive(Debug, Clone)]
struct CorpusFile {
    /// Path relative to the repository root.
    path: String,
    /// What reading the whole file costs, by this project's estimator.
    tokens: u32,
    /// Hashes of the distinct lowercase words of the file, sorted so a lookup is a binary search.
    words: Vec<u64>,
}

/// Every file the baseline may read, with what it holds and what it costs.
#[derive(Debug, Clone, Default)]
pub(crate) struct Corpus {
    /// The files, in path order.
    files: Vec<CorpusFile>,
    /// Files that were left out to stay inside the read cache.
    pub(crate) left_out: usize,
}

/// What one run of the baseline produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BaselineRun {
    /// Whether the file that declares the wanted symbol was among the files read.
    pub(crate) hit: bool,
    /// What reading those files costs in tokens.
    pub(crate) tokens: u32,
}

/// The hashes of the distinct words of a text, sorted.
fn word_hashes(text: &str) -> Vec<u64> {
    let mut hashes: Vec<u64> = split_words(text)
        .iter()
        .map(|word| hash64(word.as_bytes()))
        .collect();
    hashes.sort_unstable();
    hashes.dedup();
    hashes
}

/// How many files to skip between the files that are kept, so that the corpus stays inside the
/// read cache. One means keeping every file.
fn stride_for(total_bytes: u64) -> usize {
    if total_bytes <= CACHE_BYTES {
        return 1;
    }
    let ratio = total_bytes / CACHE_BYTES;
    usize::try_from(ratio.saturating_add(1)).unwrap_or(usize::MAX)
}

impl Corpus {
    /// Reads the files of the repository once each, keeping only what ranking and pricing need.
    ///
    /// `required` names files that are kept whatever the sampling decides, because a task is about
    /// a symbol they declare.
    ///
    /// # Errors
    /// Returns a storage error when the files cannot be listed. A file that cannot be read is left
    /// out, exactly as an agent reading files would find it.
    pub(crate) fn build(engine: &Engine, required: &BTreeSet<String>) -> Result<Self, EngineError> {
        let listed = engine.storage().list_files()?;
        let total: u64 = listed
            .iter()
            .map(|file| file.size)
            .fold(0, u64::saturating_add);
        let stride = stride_for(total);
        let mut corpus = Self::default();
        for (index, file) in listed.iter().enumerate() {
            let kept = index % stride == 0 || required.contains(&file.path);
            if !kept {
                corpus.left_out += 1;
                continue;
            }
            let Ok(text) = engine.deps.tree.read(&file.path) else {
                corpus.left_out += 1;
                continue;
            };
            corpus.files.push(CorpusFile {
                path: file.path.clone(),
                tokens: estimate_tokens(&text),
                words: word_hashes(&text),
            });
        }
        Ok(corpus)
    }

    /// How many files the baseline can choose from.
    pub(crate) fn len(&self) -> usize {
        self.files.len()
    }

    /// Runs the baseline for one query: rank the files, read the best `files` of them, and report
    /// whether `wanted` was among them and what they cost.
    pub(crate) fn run(&self, query: &str, wanted: &str, files: usize) -> BaselineRun {
        let needles = word_hashes(query);
        let mut ranked: Vec<(usize, &CorpusFile)> = self
            .files
            .iter()
            .map(|file| {
                let score = needles
                    .iter()
                    .filter(|needle| file.words.binary_search(needle).is_ok())
                    .count();
                (score, file)
            })
            .collect();
        ranked.sort_by(|left, right| right.0.cmp(&left.0).then(left.1.path.cmp(&right.1.path)));
        let mut run = BaselineRun {
            hit: false,
            tokens: 0,
        };
        for (_, file) in ranked.iter().take(files) {
            run.tokens = run.tokens.saturating_add(file.tokens);
            if file.path == wanted {
                run.hit = true;
            }
        }
        run
    }
}

#[cfg(test)]
mod tests {
    use super::{CACHE_BYTES, Corpus, CorpusFile, stride_for, word_hashes};

    /// Builds a corpus by hand, without a repository.
    fn corpus(files: &[(&str, &str, u32)]) -> Corpus {
        Corpus {
            files: files
                .iter()
                .map(|(path, text, tokens)| CorpusFile {
                    path: (*path).to_owned(),
                    tokens: *tokens,
                    words: word_hashes(text),
                })
                .collect(),
            left_out: 0,
        }
    }

    /// Word hashes are sorted, deduplicated and case-insensitive.
    #[test]
    fn hashes_are_sorted_and_distinct() {
        let hashes = word_hashes("Parse parse PARSE config");
        assert_eq!(hashes.len(), 2);
        assert!(hashes.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(word_hashes(""), Vec::<u64>::new());
    }

    /// A repository inside the cache is read whole; a larger one is sampled.
    #[test]
    fn stride_grows_with_the_repository() {
        assert_eq!(stride_for(0), 1);
        assert_eq!(stride_for(CACHE_BYTES), 1);
        assert_eq!(stride_for(CACHE_BYTES * 2), 3);
        assert_eq!(
            stride_for(u64::MAX),
            usize::try_from(u64::MAX / CACHE_BYTES + 1).unwrap()
        );
    }

    /// The best files by distinct query words are read, ties go to the first path, and the cost is
    /// their total.
    #[test]
    fn ranking_prefers_more_distinct_words() {
        let corpus = corpus(&[
            ("a.rs", "parse config value", 10),
            ("b.rs", "parse parse parse", 20),
            ("c.rs", "config", 30),
            ("d.rs", "nothing here", 40),
        ]);
        let run = corpus.run("parse config", "c.rs", 2);
        assert_eq!(run.tokens, 30, "a.rs and one of b.rs or c.rs");
        assert!(!run.hit, "c.rs loses the tie to b.rs on path order");
        let found = corpus.run("parse config", "a.rs", 1);
        assert!(found.hit);
        assert_eq!(found.tokens, 10);
    }

    /// With no matching word the baseline still reads its quota, by path order.
    #[test]
    fn a_hopeless_query_still_costs() {
        let corpus = corpus(&[("a.rs", "one", 5), ("b.rs", "two", 7)]);
        let run = corpus.run("absent words", "b.rs", 2);
        assert_eq!(run.tokens, 12);
        assert!(run.hit);
        assert_eq!(corpus.len(), 2);
    }

    /// Asking for more files than exist reads all of them and nothing more.
    #[test]
    fn quota_is_clamped_by_the_corpus() {
        let corpus = corpus(&[("a.rs", "one", 5)]);
        assert_eq!(corpus.run("one", "a.rs", 9).tokens, 5);
        assert_eq!(Corpus::default().run("one", "a.rs", 3).tokens, 0);
    }
}
