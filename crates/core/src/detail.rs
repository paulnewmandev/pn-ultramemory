// SPDX-License-Identifier: Apache-2.0
//! Levels of detail in which a symbol can be rendered into an agent context.
//!
//! The same symbol can cost a handful of tokens (just its name) or thousands
//! (its whole body). Modelling that as an explicit, ordered ladder lets the
//! capsule packer decide, under a token budget, how much of each symbol to show.

use core::fmt;

/// A rendering level, ordered from cheapest (`Name`, L0) to richest (`Source`, L4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Detail {
    /// L0: kind and qualified name only.
    Name,
    /// L1: the declaration signature.
    Signature,
    /// L2: signature plus a one-line summary (first doc sentence).
    Summary,
    /// L3: signature plus a control-flow outline (called identifiers, branches).
    Outline,
    /// L4: the full source text.
    Source,
}

impl Detail {
    /// Every level, from cheapest to richest.
    pub const ALL: [Self; 5] = [
        Self::Name,
        Self::Signature,
        Self::Summary,
        Self::Outline,
        Self::Source,
    ];

    /// Numeric level from 0 (`Name`) to 4 (`Source`).
    ///
    /// # Examples
    /// ```
    /// use pn_ultramemory_core::Detail;
    ///
    /// assert_eq!(Detail::Name.level(), 0);
    /// assert_eq!(Detail::Source.level(), 4);
    /// ```
    #[must_use]
    pub const fn level(self) -> u8 {
        match self {
            Self::Name => 0,
            Self::Signature => 1,
            Self::Summary => 2,
            Self::Outline => 3,
            Self::Source => 4,
        }
    }

    /// Short label such as `"L2"`, used in compact tool output.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Name => "L0",
            Self::Signature => "L1",
            Self::Summary => "L2",
            Self::Outline => "L3",
            Self::Source => "L4",
        }
    }

    /// The next richer level, or `None` at `Source`.
    ///
    /// # Examples
    /// ```
    /// use pn_ultramemory_core::Detail;
    ///
    /// assert_eq!(Detail::Signature.next(), Some(Detail::Summary));
    /// assert_eq!(Detail::Source.next(), None);
    /// ```
    #[must_use]
    pub const fn next(self) -> Option<Self> {
        match self {
            Self::Name => Some(Self::Signature),
            Self::Signature => Some(Self::Summary),
            Self::Summary => Some(Self::Outline),
            Self::Outline => Some(Self::Source),
            Self::Source => None,
        }
    }
}

impl fmt::Display for Detail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

#[cfg(test)]
mod tests {
    use super::Detail;

    /// `ALL` lists levels in ascending order and `level()` matches the position.
    #[test]
    fn all_is_ordered_and_levels_match_position() {
        for (index, detail) in Detail::ALL.iter().enumerate() {
            assert_eq!(usize::from(detail.level()), index);
        }
        assert!(Detail::ALL.windows(2).all(|pair| pair[0] < pair[1]));
    }

    /// Following `next()` from the cheapest level visits every level once.
    #[test]
    fn next_walks_the_whole_ladder() {
        let mut walked = vec![Detail::Name];
        while let Some(next) = walked.last().and_then(|d| d.next()) {
            walked.push(next);
        }
        assert_eq!(walked, Detail::ALL);
    }

    /// Labels are `L` followed by the level number.
    #[test]
    fn labels_follow_levels() {
        for detail in Detail::ALL {
            assert_eq!(detail.to_string(), format!("L{}", detail.level()));
        }
    }
}
