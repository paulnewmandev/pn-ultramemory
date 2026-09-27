// SPDX-License-Identifier: Apache-2.0
//! Fixture for the documentation ratchet: docstrings that add something, on items of every shape.
//!
//! None of these must be flagged. The point of the fixture is that a short sentence is fine as long
//! as it says something the name does not, and that non-public items are not examined at all.

/// How much of a symbol to show: the tighter the level, the fewer tokens it costs.
pub enum Detail {
    /// Only the qualified name, which is enough to decide whether to expand further.
    Name,
    /// The signature and the documentation comment, without any body.
    Signature,
}

/// Something that can hand back source text for a symbol without knowing where it came from.
pub trait SymbolSource {
    /// What the source hands back, so a caller can stream it or own it as it prefers.
    type Output;

    /// Looks up one symbol by its qualified name, returning nothing when the name is unknown.
    fn fetch(&self, name: &str) -> Option<Self::Output>;
}

/// An upper bound on the tokens a reply may cost, chosen by the caller rather than by us.
pub struct Budget {
    /// The bound itself, counted in tokens of the model the caller named.
    pub ceiling: usize,
}

impl Budget {
    /// The bound used when a caller names none, chosen to fit a small context window.
    pub const DEFAULT: usize = 4096;

    /// Subtracts what a section already cost, saturating at zero rather than wrapping.
    pub fn spend(&self, cost: usize) -> usize {
        self.ceiling.saturating_sub(cost)
    }
}

/// The identifier a caller uses to refer to a stored capsule between requests.
pub type CapsuleId = u64;

/// Counts how many symbols fit, stopping as soon as the next one would cross the bound.
pub fn fit(budget: &Budget, costs: &[usize]) -> usize {
    let mut spent = 0;
    let mut taken = 0;
    for cost in costs {
        spent += cost;
        if spent > budget.ceiling {
            break;
        }
        taken += 1;
    }
    taken
}

/// Not examined: private items are the compiler's business, not this guard's.
fn helper() -> usize {
    0
}
