// SPDX-License-Identifier: Apache-2.0
//! Every tunable number of recall, in one place.
//!
//! Relevances are dimensionless and live roughly in `0..=1` (a learned multiplier may lift one to
//! [`RELEVANCE_CAP`]). Utilities say how much more a richer level of detail is worth than a poorer
//! one; the packer multiplies the two. Nothing here is fitted to a repository: the numbers are
//! plain, documented choices, and the benchmark (`Engine::bench`) exists to check them.

// ---- seeds ------------------------------------------------------------------------------------

/// The most exact-name matches taken as seeds.
pub(super) const EXACT_LIMIT: usize = 10;

/// The relevance of an exact-name match.
pub(super) const EXACT_RELEVANCE: f64 = 1.0;

/// The most text matches taken as seeds.
pub(super) const TEXT_LIMIT: usize = 40;

/// The relevance of the weakest text match; the best one gets [`TEXT_FLOOR`] + [`TEXT_SPAN`].
pub(super) const TEXT_FLOOR: f64 = 0.25;

/// How much of the relevance range the text score decides.
pub(super) const TEXT_SPAN: f64 = 0.75;

/// The relevance of a text match when the scores carry no information (the best score is zero).
pub(super) const TEXT_UNSCORED: f64 = 0.5;

/// A text search that finds fewer symbols than this is relaxed (stop words dropped, then one search
/// for any of the words), because the store requires every word of a query to match.
pub(super) const RELAX_BELOW: usize = 8;

/// The most words a relaxed search looks at.
pub(super) const RELAX_TERMS: usize = 6;

/// The relevance of the weakest partial match; a symbol that matches every word gets at most
/// [`PARTIAL_FLOOR`] + [`PARTIAL_SPAN`].
pub(super) const PARTIAL_FLOOR: f64 = 0.15;

/// How much of the relevance range the fraction of matched words decides.
pub(super) const PARTIAL_SPAN: f64 = 0.45;

/// The factor applied to matches of the query without its stop words, which is a weaker claim than
/// matching the query as typed.
pub(super) const RELAXED_FACTOR: f64 = 0.9;

// ---- graph expansion --------------------------------------------------------------------------

/// How many of the best seeds are expanded through the code graph.
pub(super) const EXPAND_SEEDS: usize = 6;

/// The most neighbors followed in each direction from a seed.
pub(super) const NEIGHBOR_LIMIT: usize = 6;

/// The share of a seed's relevance that a neighbor inherits.
pub(super) const NEIGHBOR_FACTOR: f64 = 0.45;

/// The weight of a heuristic edge; resolved and exact edges weigh `1.0`.
pub(super) const HEURISTIC_WEIGHT: f64 = 0.7;

// ---- learned neighbors ------------------------------------------------------------------------

/// How many of the best seeds ask for symbols that were used together with them.
pub(super) const COACCESS_SEEDS: usize = 4;

/// The most learned neighbors per seed.
pub(super) const COACCESS_LIMIT: usize = 4;

/// The share of a seed's relevance that a learned neighbor inherits at full strength.
pub(super) const COACCESS_FACTOR: f64 = 0.35;

/// The co-access strength at which a learned neighbor counts at full strength.
pub(super) const COACCESS_FULL_WEIGHT: f64 = 3.0;

// ---- ranking ----------------------------------------------------------------------------------

/// The highest relevance a symbol can have after the learned multiplier.
pub(super) const RELEVANCE_CAP: f64 = 1.5;

/// A learned multiplier that differs from one by at least this much is mentioned in explanations.
pub(super) const MULTIPLIER_NOTE_DELTA: f64 = 0.1;

/// The most candidates that go to the packer.
pub(super) const MAX_CANDIDATES: usize = 60;

/// The most reasons printed for one symbol when the caller asks for explanations.
pub(super) const MAX_REASONS: usize = 3;

// ---- levels of detail -------------------------------------------------------------------------

/// The utility of showing a symbol by name only (L0).
pub(super) const UTILITY_NAME: f64 = 1.0;

/// The utility of the signature (L1).
pub(super) const UTILITY_SIGNATURE: f64 = 2.6;

/// The utility of the signature and the first documentation sentence (L2).
pub(super) const UTILITY_SUMMARY: f64 = 3.3;

/// The utility of the signature, the summary and the called names (L3).
pub(super) const UTILITY_OUTLINE: f64 = 4.0;

/// The utility of the full source (L4).
pub(super) const UTILITY_SOURCE: f64 = 5.0;

/// A symbol whose span is longer than this many bytes per allowed source token is never read: its
/// source could not be offered anyway.
pub(super) const SOURCE_BYTES_PER_TOKEN: u32 = 40;

// ---- memories ---------------------------------------------------------------------------------

/// The most memories asked for by the symbols of the candidates.
pub(super) const MEMORY_SYMBOL_LIMIT: usize = 20;

/// The most memories asked for by the text of the query.
pub(super) const MEMORY_TEXT_LIMIT: usize = 5;

/// The relevance of a memory that matched the query text but is not anchored to a candidate.
pub(super) const MEMORY_TEXT_RELEVANCE: f64 = 0.5;

/// The factor applied to a memory whose code changed since it was written.
pub(super) const STALE_MEMORY_FACTOR: f64 = 0.6;

/// Memories may use at most `1 / MEMORY_BUDGET_DIVISOR` of the budget.
pub(super) const MEMORY_BUDGET_DIVISOR: u32 = 4;

/// The tokens a memory costs beyond its text: its identity, kind and separators.
pub(super) const MEMORY_ROW_TOKENS: u32 = 8;

/// The tokens the header of the memory table costs when there is any memory.
pub(super) const MEMORY_HEADER_TOKENS: u32 = 15;

// ---- packing ----------------------------------------------------------------------------------

/// The tokens reserved for the header of the symbol table, on top of the measured frame.
pub(super) const TABLE_HEADER_TOKENS: u32 = 30;

/// The tokens one row of the file table costs besides its path: the index, the separator and the
/// line break.
pub(super) const FILE_ROW_TOKENS: u32 = 3;

/// The tokens the header of the file table costs when any row points at a file.
pub(super) const FILE_HEADER_TOKENS: u32 = 8;

/// How many of the most relevant candidates keep at least their signature while anything else can
/// still be lowered to make a capsule fit.
pub(super) const KEEP_SIGNATURE_TOP: usize = 3;

/// How many times the reserve is corrected for the files and relations of the actual selection.
pub(super) const RESERVE_ROUNDS: usize = 4;

/// The most relations printed.
pub(super) const MAX_RELATIONS: usize = 12;

/// The most stale-source notes printed one by one; the rest are counted in one note.
pub(super) const MAX_STALE_NOTES: usize = 3;

/// The most characters of the query printed back in the capsule, before the budget lowers it.
pub(super) const QUERY_ECHO_CHARS: usize = 160;

/// The fewest characters of the query printed back, whatever the budget.
pub(super) const QUERY_ECHO_MIN_CHARS: usize = 24;
