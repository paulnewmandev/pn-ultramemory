// SPDX-License-Identifier: Apache-2.0
//! What a memory is and who produced it.
//!
//! Memories are **data, never instructions**: whatever their [`Provenance`],
//! consumers must present them to the model as context and must not execute or
//! obey them. Provenance only records how much a human vouched for the content,
//! which the learning loop uses to decide what may be promoted.

use core::fmt;

/// Who or what produced a memory or a summary, ordered from least to most trusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Provenance {
    /// Captured from the output of a tool, a web page or a file. Untrusted.
    Tool,
    /// Written by a coding agent. Useful, but may be wrong or manipulated.
    Agent,
    /// Written or explicitly confirmed by a human.
    User,
}

impl Provenance {
    /// Returns `true` only when a human vouched for the content.
    ///
    /// # Examples
    /// ```
    /// use pn_ultramemory_core::Provenance;
    ///
    /// assert!(Provenance::User.is_human_verified());
    /// assert!(!Provenance::Agent.is_human_verified());
    /// ```
    #[must_use]
    pub const fn is_human_verified(self) -> bool {
        matches!(self, Self::User)
    }

    /// Stable lowercase name used in storage and in tool output.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Tool => "tool",
            Self::Agent => "agent",
            Self::User => "user",
        }
    }

    /// Looks a provenance up by the name returned from [`Provenance::as_str`].
    ///
    /// # Examples
    /// ```
    /// use pn_ultramemory_core::Provenance;
    ///
    /// assert_eq!(Provenance::from_name("agent"), Some(Provenance::Agent));
    /// assert_eq!(Provenance::from_name("robot"), None);
    /// ```
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        [Self::Tool, Self::Agent, Self::User]
            .into_iter()
            .find(|p| p.as_str() == name)
    }
}

impl fmt::Display for Provenance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The kind of thing a memory records.
///
/// The set will grow, so the enum is `#[non_exhaustive]`: downstream `match`
/// expressions need a wildcard arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum MemoryKind {
    /// A choice that was made, with its reasons.
    Decision,
    /// A stable statement about the code or the project.
    Fact,
    /// Something learned from experience.
    Lesson,
    /// An approach that was tried and abandoned.
    DeadEnd,
    /// A pairing of a recurring error with the fix that resolved it.
    ErrorFix,
    /// A rule the codebase follows, promoted from repeated lessons.
    Convention,
    /// Something the system must do, anchored to the symbols that implement it.
    ///
    /// A requirement is a contract about behavior, written declaratively ("the parser SHALL
    /// reject a file larger than the configured limit"), not a plan naming functions. Anchoring it
    /// to the code that implements it is what makes it more than a document: when that code
    /// changes, the requirement is reported as stale, so a specification cannot silently drift
    /// away from the system it describes.
    Requirement,
    /// A unit of work, open or finished.
    Task,
    /// A summary of one working session.
    Session,
}

impl MemoryKind {
    /// Every kind currently defined.
    pub const ALL: [Self; 9] = [
        Self::Decision,
        Self::Fact,
        Self::Lesson,
        Self::DeadEnd,
        Self::ErrorFix,
        Self::Convention,
        Self::Requirement,
        Self::Task,
        Self::Session,
    ];

    /// Stable lowercase name used in storage and in tool output.
    ///
    /// # Examples
    /// ```
    /// use pn_ultramemory_core::MemoryKind;
    ///
    /// assert_eq!(MemoryKind::DeadEnd.as_str(), "dead_end");
    /// ```
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Decision => "decision",
            Self::Fact => "fact",
            Self::Lesson => "lesson",
            Self::DeadEnd => "dead_end",
            Self::ErrorFix => "error_fix",
            Self::Convention => "convention",
            Self::Requirement => "requirement",
            Self::Task => "task",
            Self::Session => "session",
        }
    }

    /// Looks a kind up by the name returned from [`MemoryKind::as_str`].
    ///
    /// # Examples
    /// ```
    /// use pn_ultramemory_core::MemoryKind;
    ///
    /// assert_eq!(MemoryKind::from_name("dead_end"), Some(MemoryKind::DeadEnd));
    /// assert_eq!(MemoryKind::from_name("nope"), None);
    /// ```
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == name)
    }
}

impl fmt::Display for MemoryKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::{MemoryKind, Provenance};

    /// Trust grows from tool output to agent to human.
    #[test]
    fn provenance_is_ordered_by_trust() {
        assert!(Provenance::Tool < Provenance::Agent);
        assert!(Provenance::Agent < Provenance::User);
        assert!(!Provenance::Tool.is_human_verified());
    }

    /// Storage names of memory kinds are unique.
    #[test]
    fn memory_kind_names_are_unique() {
        let mut names: Vec<_> = MemoryKind::ALL.iter().map(|k| k.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), MemoryKind::ALL.len());
    }
}
