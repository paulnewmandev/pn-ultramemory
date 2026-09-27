// SPDX-License-Identifier: Apache-2.0
//! Confidence levels attached to every edge of the code graph.
//!
//! An index built without a language server can rarely be certain about a call
//! or import edge. Instead of hiding that uncertainty, every edge records how it
//! was obtained so consumers (impact analysis, capsule packing) can filter or
//! explain results. Edges below [`Confidence::Resolved`] are never used as
//! structural facts.

use core::fmt;

/// How certain the indexer is that an edge exists.
///
/// Variants are ordered from least to most certain, so
/// `Confidence::Exact > Confidence::Guess` and values can be used as thresholds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Confidence {
    /// Matched by name only, with no supporting structural evidence.
    Guess,
    /// Inferred from structural hints, for example an unqualified call to a name
    /// that is unique in the repository.
    Heuristic,
    /// Resolved through scopes, imports or types to a single target.
    Resolved,
    /// Stated directly by the syntax, for example an `import` statement or a
    /// fully qualified call.
    Exact,
}

impl Confidence {
    /// Every variant, from least to most certain.
    pub const ALL: [Self; 4] = [Self::Guess, Self::Heuristic, Self::Resolved, Self::Exact];

    /// Returns `true` when the edge is reliable enough to feed impact analysis
    /// and community detection.
    ///
    /// # Examples
    /// ```
    /// use pn_ultramemory_core::Confidence;
    ///
    /// assert!(Confidence::Resolved.is_structural());
    /// assert!(!Confidence::Heuristic.is_structural());
    /// ```
    #[must_use]
    pub const fn is_structural(self) -> bool {
        matches!(self, Self::Resolved | Self::Exact)
    }

    /// Stable lowercase name used in storage and in tool output.
    ///
    /// # Examples
    /// ```
    /// use pn_ultramemory_core::Confidence;
    ///
    /// assert_eq!(Confidence::Exact.as_str(), "exact");
    /// ```
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Guess => "guess",
            Self::Heuristic => "heuristic",
            Self::Resolved => "resolved",
            Self::Exact => "exact",
        }
    }
}

impl fmt::Display for Confidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::Confidence;

    /// Variants must be listed from least to most certain and compare accordingly.
    #[test]
    fn all_is_sorted_ascending() {
        assert!(Confidence::ALL.windows(2).all(|pair| pair[0] < pair[1]));
    }

    /// Only `Resolved` and `Exact` count as structural facts.
    #[test]
    fn only_resolved_and_exact_are_structural() {
        let structural: Vec<_> = Confidence::ALL
            .into_iter()
            .filter(|c| c.is_structural())
            .collect();
        assert_eq!(structural, [Confidence::Resolved, Confidence::Exact]);
    }

    /// Storage names are unique and match the `Display` output.
    #[test]
    fn names_are_unique_and_displayed() {
        let mut names: Vec<_> = Confidence::ALL.iter().map(|c| c.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), Confidence::ALL.len());
        assert_eq!(Confidence::Heuristic.to_string(), "heuristic");
    }
}
