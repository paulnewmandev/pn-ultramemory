// SPDX-License-Identifier: Apache-2.0
//! Multi-resolution knapsack packing under a token budget.
//!
//! # The problem
//! A capsule for an agent contains many symbols. Each can be shown at several
//! levels of [`Detail`] (name, signature, summary, outline, source), and each
//! level costs a different number of tokens and is worth a different amount.
//! Given a budget, which level should each symbol get? This is the
//! *multiple-choice knapsack problem* (MCKP).
//!
//! # The algorithm
//! 1. For each candidate compute the *value* of every option as
//!    `relevance * utility`. Options that are worthless or not finite are ignored.
//! 2. Reduce each candidate's options to the vertices of their **concave
//!    envelope** in the (tokens, value) plane, starting from the empty choice
//!    `(0, 0)`. Dominated options (more tokens, no more value) and options that a
//!    cheaper jump beats never survive.
//! 3. Each consecutive pair of vertices is an *upgrade step* with a marginal
//!    efficiency `value / tokens`. Along one candidate the efficiency strictly
//!    falls.
//! 4. Repeatedly take the most efficient available step that still fits in the
//!    remaining budget (a max-heap over each candidate's next step). A step that
//!    does not fit blocks that candidate's later steps, and packing continues
//!    with the others.
//! 5. **Leftover fill.** The envelope discards options that are feasible but
//!    unattractive on average (a cheap option can sit below the line joining
//!    the origin to an expensive one). When a large step did not fit, budget
//!    would be wasted, so a final pass spends what is left on the best
//!    per-token upgrade among *all* original options.
//!
//! # Guarantees
//! * **Deterministic**: ties break by candidate order; output follows input order.
//! * **Cost**: `O(N * L * log N)` for `N` candidates with `L` options each, plus
//!   `O(k * N * L)` for the leftover fill, where `k` is the number of extra
//!   upgrades it makes (each consumes at least one leftover token).
//! * **Near-optimal**: the greedy prefix that stops at the first step that does
//!   not fit is optimal for the LP relaxation, which upper-bounds the true
//!   optimum. [`Packing::upper_bound`] reports that bound, so callers can see how
//!   far from provably optimal a result is via [`Packing::optimality_gap`]. The
//!   greedy prefix loses at most the value of one upgrade step against the true
//!   optimum, and the leftover fill only ever adds value on top of it.
//! * **Total**: never panics. Non-finite or non-positive numbers are ignored.

use core::cmp::Ordering;
use std::collections::BinaryHeap;

use pn_ultramemory_core::Detail;

/// One selectable representation of a candidate: a level of detail, what it
/// costs in tokens and how useful it is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LevelOption {
    /// The level of detail this option renders.
    pub detail: Detail,
    /// Cost of rendering this option, in tokens of whatever tokenizer the caller uses.
    pub tokens: u32,
    /// Intrinsic usefulness of this option; must be finite and positive to count.
    /// It normally grows with `detail`, but that is not required.
    pub utility: f64,
}

/// An item that may enter the capsule, with every way of rendering it.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate<K> {
    /// Caller-defined identity, returned untouched in the [`Selection`].
    pub key: K,
    /// How relevant the item is to the query; multiplies every option's utility.
    pub relevance: f64,
    /// The available levels of detail.
    pub options: Vec<LevelOption>,
}

/// The level chosen for one candidate.
#[derive(Debug, Clone, PartialEq)]
pub struct Selection<K> {
    /// The candidate's key.
    pub key: K,
    /// The level of detail chosen.
    pub detail: Detail,
    /// Tokens the chosen level costs.
    pub tokens: u32,
    /// `relevance * utility` of the chosen level.
    pub value: f64,
}

/// The result of [`pack`].
#[derive(Debug, Clone, PartialEq)]
pub struct Packing<K> {
    /// One entry per candidate that received any detail, in input order.
    pub selections: Vec<Selection<K>>,
    /// Total tokens used; never more than the budget.
    pub used_tokens: u32,
    /// Sum of the selected values.
    pub total_value: f64,
    /// LP-relaxation upper bound on the value any packing could reach.
    pub upper_bound: f64,
}

impl<K> Packing<K> {
    /// Relative distance between the achieved value and the upper bound, in
    /// `[0, 1]`. A gap of `0.0` means the packing is provably optimal.
    ///
    /// # Examples
    /// ```
    /// use pn_ultramemory_codec::{Candidate, LevelOption, pack};
    /// use pn_ultramemory_core::Detail;
    ///
    /// let candidate = Candidate {
    ///     key: "only",
    ///     relevance: 1.0,
    ///     options: vec![LevelOption { detail: Detail::Name, tokens: 4, utility: 1.0 }],
    /// };
    /// assert!(pack(vec![candidate], 100).optimality_gap() < 1e-12);
    /// ```
    #[must_use]
    pub fn optimality_gap(&self) -> f64 {
        if self.upper_bound <= 0.0 {
            0.0
        } else {
            ((self.upper_bound - self.total_value) / self.upper_bound).clamp(0.0, 1.0)
        }
    }
}

/// A vertex of a candidate's concave envelope: one option worth choosing.
#[derive(Debug, Clone, Copy)]
struct Vertex {
    /// The level of detail this vertex renders.
    detail: Detail,
    /// Cumulative token cost of choosing this vertex.
    tokens: u32,
    /// Cumulative value of choosing this vertex.
    value: f64,
}

/// The option currently chosen for one candidate while packing.
#[derive(Debug, Clone, Copy)]
struct Choice {
    /// The level of detail chosen.
    detail: Detail,
    /// Tokens it costs.
    tokens: u32,
    /// `relevance * utility` it is worth.
    value: f64,
}

/// A frontier entry of the greedy pass: the next upgrade step of one candidate.
#[derive(Debug, Clone, Copy)]
struct Step {
    /// Marginal value per token of this step (`f64::INFINITY` for free steps).
    efficiency: f64,
    /// Index of the candidate in the input.
    candidate: usize,
    /// Index of the vertex this step upgrades to.
    vertex: usize,
    /// Extra tokens this step costs.
    tokens: u32,
    /// Extra value this step adds.
    value: f64,
}

impl PartialEq for Step {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Step {}

impl PartialOrd for Step {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Step {
    /// Higher efficiency first; ties go to the lower candidate index, then the
    /// lower vertex, so that the max-heap pops in a fully deterministic order.
    fn cmp(&self, other: &Self) -> Ordering {
        self.efficiency
            .total_cmp(&other.efficiency)
            .then_with(|| other.candidate.cmp(&self.candidate))
            .then_with(|| other.vertex.cmp(&self.vertex))
    }
}

/// Cross product `(b - a) x (p - a)` in the (tokens, value) plane.
///
/// Non-negative means `b` lies on or below the segment from `a` to `p`, so `b`
/// is not on the upper envelope.
fn cross(a: Vertex, b: Vertex, p: Vertex) -> f64 {
    (f64::from(b.tokens) - f64::from(a.tokens)) * (p.value - a.value)
        - (b.value - a.value) * (f64::from(p.tokens) - f64::from(a.tokens))
}

/// Builds the concave envelope of a candidate's options, excluding the implicit
/// origin `(0, 0)`. Vertices are ordered by increasing tokens and value.
fn envelope<K>(candidate: &Candidate<K>) -> Vec<Vertex> {
    let mut points: Vec<Vertex> = candidate
        .options
        .iter()
        .filter_map(|option| {
            let value = candidate.relevance * option.utility;
            (value.is_finite() && value > 0.0).then_some(Vertex {
                detail: option.detail,
                tokens: option.tokens,
                value,
            })
        })
        .collect();

    // Cheapest first; among equal costs the most valuable first, then drop the rest.
    points.sort_by(|a, b| {
        a.tokens
            .cmp(&b.tokens)
            .then_with(|| b.value.total_cmp(&a.value))
    });
    points.dedup_by_key(|point| point.tokens);

    // Keep only options that strictly improve on every cheaper one.
    let mut best = 0.0_f64;
    points.retain(|point| {
        let improves = point.value > best;
        if improves {
            best = point.value;
        }
        improves
    });

    let origin = Vertex {
        detail: Detail::Name,
        tokens: 0,
        value: 0.0,
    };
    let mut hull = vec![origin];
    for point in points {
        while hull.len() >= 2 && cross(hull[hull.len() - 2], hull[hull.len() - 1], point) >= 0.0 {
            hull.pop();
        }
        hull.push(point);
    }
    hull.remove(0);
    hull
}

/// Builds the upgrade step that leads to `vertices[index]` of `candidate_index`.
fn step_to(candidate_index: usize, vertices: &[Vertex], index: usize) -> Step {
    let (prev_tokens, prev_value) = index
        .checked_sub(1)
        .map_or((0, 0.0), |i| (vertices[i].tokens, vertices[i].value));
    let tokens = vertices[index].tokens - prev_tokens;
    let value = vertices[index].value - prev_value;
    let efficiency = if tokens == 0 {
        f64::INFINITY
    } else {
        value / f64::from(tokens)
    };
    Step {
        efficiency,
        candidate: candidate_index,
        vertex: index,
        tokens,
        value,
    }
}

/// Spends the budget left after the greedy pass on the best per-token upgrade
/// among all original options, repeating until nothing more fits.
///
/// Ties go to the lowest candidate index and then the lowest option index, so the
/// result is deterministic.
fn fill_leftover<K>(
    candidates: &[Candidate<K>],
    chosen: &mut [Option<Choice>],
    remaining: &mut u32,
) {
    while *remaining > 0 {
        let mut best: Option<(f64, usize, Choice)> = None;
        for (index, candidate) in candidates.iter().enumerate() {
            let (held_tokens, held_value) = chosen[index].map_or((0, 0.0), |c| (c.tokens, c.value));
            for option in &candidate.options {
                let value = candidate.relevance * option.utility;
                if !(value.is_finite() && value > held_value) {
                    continue;
                }
                let extra = option.tokens.saturating_sub(held_tokens);
                if extra > *remaining {
                    continue;
                }
                let score = if extra == 0 {
                    f64::INFINITY
                } else {
                    (value - held_value) / f64::from(extra)
                };
                if best.is_none_or(|(top, _, _)| score > top) {
                    let choice = Choice {
                        detail: option.detail,
                        tokens: option.tokens,
                        value,
                    };
                    best = Some((score, index, choice));
                }
            }
        }
        let Some((_, index, choice)) = best else {
            break;
        };
        let held_tokens = chosen[index].map_or(0, |c| c.tokens);
        let extra = choice.tokens.saturating_sub(held_tokens);
        let refund = held_tokens.saturating_sub(choice.tokens);
        *remaining = (*remaining - extra).saturating_add(refund);
        chosen[index] = Some(choice);
    }
}

/// Chooses a level of detail for each candidate so that the total value is as
/// large as possible within `budget` tokens.
///
/// Candidates that receive no detail are omitted from the result. See the
/// [module documentation](self) for the algorithm and its guarantees.
///
/// # Examples
/// ```
/// use pn_ultramemory_codec::{Candidate, LevelOption, pack};
/// use pn_ultramemory_core::Detail;
///
/// let ladder = |scale: f64| {
///     vec![
///         LevelOption { detail: Detail::Name, tokens: 4, utility: 1.0 * scale },
///         LevelOption { detail: Detail::Signature, tokens: 12, utility: 3.0 * scale },
///         LevelOption { detail: Detail::Source, tokens: 120, utility: 6.0 * scale },
///     ]
/// };
/// let candidates = vec![
///     Candidate { key: "hot", relevance: 1.0, options: ladder(1.0) },
///     Candidate { key: "cold", relevance: 0.1, options: ladder(1.0) },
/// ];
///
/// let packing = pack(candidates, 30);
/// assert!(packing.used_tokens <= 30);
/// // The relevant symbol gets at least its signature.
/// assert!(packing.selections[0].detail >= Detail::Signature);
/// ```
#[must_use]
pub fn pack<K>(candidates: Vec<Candidate<K>>, budget: u32) -> Packing<K> {
    let envelopes: Vec<Vec<Vertex>> = candidates.iter().map(envelope).collect();

    let mut frontier: BinaryHeap<Step> = envelopes
        .iter()
        .enumerate()
        .filter(|(_, vertices)| !vertices.is_empty())
        .map(|(candidate, vertices)| step_to(candidate, vertices, 0))
        .collect();

    let mut remaining = budget;
    let mut taken = vec![0_usize; candidates.len()];
    // The LP bound follows the greedy prefix up to the first step that does not fit.
    let mut prefix_value = 0.0_f64;
    let mut prefix_open = true;
    let mut upper_bound = 0.0_f64;

    while let Some(step) = frontier.pop() {
        if step.tokens <= remaining {
            remaining -= step.tokens;
            taken[step.candidate] = step.vertex + 1;
            if prefix_open {
                prefix_value += step.value;
            }
            let vertices = &envelopes[step.candidate];
            if step.vertex + 1 < vertices.len() {
                frontier.push(step_to(step.candidate, vertices, step.vertex + 1));
            }
        } else if prefix_open {
            // First misfit: fill the leftover budget fractionally to obtain the LP bound.
            prefix_open = false;
            upper_bound = prefix_value + step.value * f64::from(remaining) / f64::from(step.tokens);
        }
    }
    if prefix_open {
        upper_bound = prefix_value;
    }

    let mut chosen: Vec<Option<Choice>> = taken
        .iter()
        .enumerate()
        .map(|(index, &count)| {
            (count > 0).then(|| {
                let vertex = envelopes[index][count - 1];
                Choice {
                    detail: vertex.detail,
                    tokens: vertex.tokens,
                    value: vertex.value,
                }
            })
        })
        .collect();
    fill_leftover(&candidates, &mut chosen, &mut remaining);

    let mut selections = Vec::new();
    let mut total_value = 0.0_f64;
    for (candidate, choice) in candidates.into_iter().zip(chosen) {
        if let Some(choice) = choice {
            total_value += choice.value;
            selections.push(Selection {
                key: candidate.key,
                detail: choice.detail,
                tokens: choice.tokens,
                value: choice.value,
            });
        }
    }

    Packing {
        selections,
        used_tokens: budget - remaining,
        total_value,
        upper_bound,
    }
}

#[cfg(test)]
mod tests {
    use super::{Candidate, LevelOption, Packing, pack};
    use pn_ultramemory_core::Detail;

    /// Builds a candidate from `(detail, tokens, utility)` triples.
    fn candidate(key: u32, relevance: f64, options: &[(Detail, u32, f64)]) -> Candidate<u32> {
        Candidate {
            key,
            relevance,
            options: options
                .iter()
                .map(|&(detail, tokens, utility)| LevelOption {
                    detail,
                    tokens,
                    utility,
                })
                .collect(),
        }
    }

    /// A typical ladder: cheap names, mid-priced signatures, expensive source.
    fn ladder(key: u32, relevance: f64) -> Candidate<u32> {
        candidate(
            key,
            relevance,
            &[
                (Detail::Name, 4, 1.0),
                (Detail::Signature, 12, 3.0),
                (Detail::Summary, 24, 4.0),
                (Detail::Source, 200, 6.0),
            ],
        )
    }

    /// Returns the detail chosen for `key`, if any.
    fn chosen(packing: &Packing<u32>, key: u32) -> Option<Detail> {
        packing
            .selections
            .iter()
            .find(|s| s.key == key)
            .map(|s| s.detail)
    }

    /// Nothing in, nothing out.
    #[test]
    fn empty_input_yields_empty_packing() {
        let packing = pack::<u32>(Vec::new(), 100);
        assert!(packing.selections.is_empty());
        assert_eq!(packing.used_tokens, 0);
        assert!(packing.optimality_gap() < 1e-12);
    }

    /// With ample budget the richest level is chosen.
    #[test]
    fn picks_richest_level_when_budget_is_ample() {
        let packing = pack(vec![ladder(1, 1.0)], 10_000);
        assert_eq!(chosen(&packing, 1), Some(Detail::Source));
    }

    /// A zero budget selects nothing.
    #[test]
    fn zero_budget_selects_nothing() {
        let packing = pack(vec![ladder(1, 1.0)], 0);
        assert!(packing.selections.is_empty());
    }

    /// An option with more tokens and no more value is never chosen.
    #[test]
    fn dominated_option_is_never_selected() {
        let options = [(Detail::Signature, 10, 5.0), (Detail::Summary, 20, 4.0)];
        let packing = pack(vec![candidate(1, 1.0, &options)], 1_000);
        assert_eq!(chosen(&packing, 1), Some(Detail::Signature));
    }

    /// Budget the greedy pass cannot use (the big step does not fit) is spent on
    /// options that are off the concave envelope.
    #[test]
    fn leftover_budget_is_filled_with_options_off_the_envelope() {
        // 10 tokens -> 1.0, 20 tokens -> 1.2, 30 tokens -> 9.0. Only the jump to 30 is
        // on the envelope; with a budget of 20 the fill pass must still reach 1.2.
        let options = [
            (Detail::Name, 10, 1.0),
            (Detail::Signature, 20, 1.2),
            (Detail::Source, 30, 9.0),
        ];
        let tight = pack(vec![candidate(1, 1.0, &options)], 20);
        assert_eq!(chosen(&tight, 1), Some(Detail::Signature));
        assert_eq!(tight.used_tokens, 20);
        let roomy = pack(vec![candidate(1, 1.0, &options)], 30);
        assert_eq!(chosen(&roomy, 1), Some(Detail::Source));
    }

    /// More relevant candidates receive richer detail from the same ladder.
    #[test]
    fn higher_relevance_gets_richer_detail() {
        let packing = pack(vec![ladder(1, 1.0), ladder(2, 0.05)], 40);
        assert!(chosen(&packing, 1) > chosen(&packing, 2));
    }

    /// A step that does not fit must not stop smaller candidates from being packed.
    #[test]
    fn misfit_does_not_block_other_candidates() {
        let big = candidate(1, 10.0, &[(Detail::Source, 500, 10.0)]);
        let small = candidate(2, 1.0, &[(Detail::Name, 5, 1.0)]);
        let packing = pack(vec![big, small], 50);
        assert_eq!(chosen(&packing, 1), None);
        assert_eq!(chosen(&packing, 2), Some(Detail::Name));
    }

    /// Non-finite and non-positive numbers are ignored instead of panicking.
    #[test]
    fn invalid_numbers_are_ignored() {
        let bad = candidate(
            1,
            f64::NAN,
            &[
                (Detail::Name, 4, 1.0),
                (Detail::Signature, 8, f64::INFINITY),
            ],
        );
        let negative = candidate(2, -3.0, &[(Detail::Name, 4, 1.0)]);
        let packing = pack(vec![bad, negative], 100);
        assert!(packing.selections.is_empty());
    }

    /// An option that costs nothing but has value is always taken.
    #[test]
    fn zero_token_option_is_free() {
        let packing = pack(vec![candidate(1, 1.0, &[(Detail::Name, 0, 2.0)])], 0);
        assert_eq!(chosen(&packing, 1), Some(Detail::Name));
        assert_eq!(packing.used_tokens, 0);
    }

    /// Selections come back in the order the candidates were given.
    #[test]
    fn output_follows_input_order() {
        let packing = pack(vec![ladder(9, 0.1), ladder(3, 1.0), ladder(5, 0.5)], 10_000);
        let keys: Vec<u32> = packing.selections.iter().map(|s| s.key).collect();
        assert_eq!(keys, [9, 3, 5]);
    }

    /// Packing the same input twice yields identical results.
    #[test]
    fn is_deterministic() {
        let build = || vec![ladder(1, 1.0), ladder(2, 1.0), ladder(3, 0.5)];
        assert_eq!(pack(build(), 60), pack(build(), 60));
    }

    /// Small deterministic pseudo-random generator (xorshift64*), so the property
    /// test needs no external crate.
    struct Rng(u64);

    impl Rng {
        /// Next raw 64-bit value.
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }

        /// Uniform integer in `0..n`.
        fn below(&mut self, n: u32) -> u32 {
            u32::try_from(self.next() % u64::from(n)).unwrap()
        }

        /// Uniform float in `[0, 1)`.
        #[allow(clippy::cast_precision_loss)]
        fn unit(&mut self) -> f64 {
            (self.next() >> 11) as f64 / (1_u64 << 53) as f64
        }
    }

    /// Exact optimum by dynamic programming over the budget (test oracle).
    fn optimum(candidates: &[Candidate<u32>], budget: u32) -> f64 {
        let budget = usize::try_from(budget).unwrap();
        let mut best = vec![0.0_f64; budget + 1];
        for candidate in candidates {
            let mut next = best.clone();
            for option in &candidate.options {
                let value = candidate.relevance * option.utility;
                let cost = usize::try_from(option.tokens).unwrap();
                if !(value.is_finite() && value > 0.0) || cost > budget {
                    continue;
                }
                for spent in cost..=budget {
                    next[spent] = next[spent].max(best[spent - cost] + value);
                }
            }
            best = next;
        }
        best[budget]
    }

    /// Against an exact solver on thousands of random instances the packing is
    /// feasible, never beats the optimum, sits under the LP bound, and loses at most
    /// one option's value.
    #[test]
    fn matches_exact_solver_within_the_theoretical_gap() {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        let (mut ratio_sum, mut ratio_count) = (0.0_f64, 0.0_f64);
        for _ in 0..3_000 {
            let count = 1 + rng.below(6);
            let candidates: Vec<Candidate<u32>> = (0..count)
                .map(|key| {
                    let options = (0..=rng.below(5))
                        .map(|_| LevelOption {
                            detail: Detail::ALL[usize::try_from(rng.below(5)).unwrap()],
                            tokens: rng.below(31),
                            utility: rng.unit() * 10.0,
                        })
                        .collect();
                    Candidate {
                        key,
                        relevance: 0.1 + rng.unit(),
                        options,
                    }
                })
                .collect();
            let budget = rng.below(120);
            let max_option_value = candidates
                .iter()
                .flat_map(|c| c.options.iter().map(|o| c.relevance * o.utility))
                .fold(0.0_f64, f64::max);

            let exact = optimum(&candidates, budget);
            let packing = pack(candidates, budget);

            let summed_tokens: u32 = packing.selections.iter().map(|s| s.tokens).sum();
            assert_eq!(summed_tokens, packing.used_tokens);
            assert!(packing.used_tokens <= budget);
            assert!(packing.total_value <= exact + 1e-9, "beat the optimum");
            assert!(
                packing.upper_bound >= exact - 1e-9,
                "bound below the optimum"
            );
            assert!(
                packing.total_value + max_option_value >= exact - 1e-9,
                "gap too large"
            );
            if exact > 0.0 {
                ratio_sum += packing.total_value / exact;
                ratio_count += 1.0;
            }
        }
        // Empirical regression guard on this fixed corpus: quality should stay high on average.
        let mean = ratio_sum / ratio_count;
        assert!(
            mean >= 0.95,
            "mean value ratio versus exact optimum fell to {mean}"
        );
    }
}
