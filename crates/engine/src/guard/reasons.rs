// SPDX-License-Identifier: Apache-2.0
//! The reasons the guard can give, as a compact set.
//!
//! Every reason has a stable short name (it is part of the output of `remember` and of the MCP
//! tool), a severity (blocking or only suspicious) and a fixed position, so the list a caller sees
//! is always in the same order.

/// Why a text was flagged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Reason {
    /// An explicit request to discard earlier instructions.
    OverrideInstructions,
    /// A header announcing new instructions.
    NewInstructions,
    /// An attempt to reassign the reader's role.
    RoleHijack,
    /// A request to reveal the prompt or the instructions of the reader.
    RevealPrompt,
    /// An attempt to make the reader act without restrictions.
    UnrestrictedPersona,
    /// Unicode tag characters, which are invisible.
    TagCharacters,
    /// Bidirectional override or isolate characters.
    BidiControl,
    /// A run of three or more zero-width characters.
    ZeroWidthRun,
    /// Markup that imitates the messages of a tool or chat protocol.
    ProtocolMarkup,
    /// An instruction to run a command.
    RunCommand,
    /// A download piped into an interpreter.
    PipeToShell,
    /// A PowerShell command with an encoded payload.
    EncodedPowershell,
    /// An ANSI escape sequence.
    AnsiEscape,
    /// Second-person directives.
    DirectiveLanguage,
    /// A long base64-looking blob.
    Base64Blob,
    /// Four or more URLs.
    ManyUrls,
    /// Mostly upper-case text.
    Shouting,
    /// A very long text.
    TooLong,
}

impl Reason {
    /// Every reason, blocking ones first, in the order they are reported.
    pub(super) const ALL: [Self; 18] = [
        Self::OverrideInstructions,
        Self::NewInstructions,
        Self::RoleHijack,
        Self::RevealPrompt,
        Self::UnrestrictedPersona,
        Self::TagCharacters,
        Self::BidiControl,
        Self::ZeroWidthRun,
        Self::ProtocolMarkup,
        Self::RunCommand,
        Self::PipeToShell,
        Self::EncodedPowershell,
        Self::AnsiEscape,
        Self::DirectiveLanguage,
        Self::Base64Blob,
        Self::ManyUrls,
        Self::Shouting,
        Self::TooLong,
    ];

    /// The stable name of the reason.
    pub(super) const fn name(self) -> &'static str {
        match self {
            Self::OverrideInstructions => "override-instructions",
            Self::NewInstructions => "new-instructions",
            Self::RoleHijack => "role-hijack",
            Self::RevealPrompt => "reveal-prompt",
            Self::UnrestrictedPersona => "unrestricted-persona",
            Self::TagCharacters => "tag-characters",
            Self::BidiControl => "bidi-control",
            Self::ZeroWidthRun => "zero-width-run",
            Self::ProtocolMarkup => "protocol-markup",
            Self::RunCommand => "run-command",
            Self::PipeToShell => "pipe-to-shell",
            Self::EncodedPowershell => "encoded-powershell",
            Self::AnsiEscape => "ansi-escape",
            Self::DirectiveLanguage => "directive-language",
            Self::Base64Blob => "base64-blob",
            Self::ManyUrls => "many-urls",
            Self::Shouting => "shouting",
            Self::TooLong => "too-long",
        }
    }

    /// Whether the reason makes the text unacceptable, and not just worth a warning.
    pub(super) const fn is_blocking(self) -> bool {
        !matches!(
            self,
            Self::DirectiveLanguage
                | Self::Base64Blob
                | Self::ManyUrls
                | Self::Shouting
                | Self::TooLong
        )
    }

    /// The bit that stands for the reason in a [`Reasons`] set.
    const fn bit(self) -> u32 {
        1 << self as u32
    }
}

/// A set of reasons.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Reasons(u32);

impl Reasons {
    /// Adds a reason.
    pub(super) fn add(&mut self, reason: Reason) {
        self.0 |= reason.bit();
    }

    /// Whether the reason is in the set.
    pub(super) const fn has(self, reason: Reason) -> bool {
        self.0 & reason.bit() != 0
    }

    /// The names of the reasons that match `blocking`, in the fixed order.
    pub(super) fn names(self, blocking: bool) -> Vec<String> {
        Reason::ALL
            .into_iter()
            .filter(|reason| reason.is_blocking() == blocking && self.has(*reason))
            .map(|reason| reason.name().to_owned())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{Reason, Reasons};

    /// Names are unique and blocking reasons come before the suspicious ones.
    #[test]
    fn names_are_unique_and_ordered() {
        let mut names: Vec<_> = Reason::ALL.iter().map(|r| r.name()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), Reason::ALL.len());
        let first_soft = Reason::ALL
            .iter()
            .position(|r| !r.is_blocking())
            .unwrap_or(0);
        assert!(Reason::ALL[first_soft..].iter().all(|r| !r.is_blocking()));
        assert!(Reason::ALL[..first_soft].iter().all(|r| r.is_blocking()));
    }

    /// A set reports its members once, in the fixed order, split by severity.
    #[test]
    fn sets_report_in_order() {
        let mut set = Reasons::default();
        set.add(Reason::ProtocolMarkup);
        set.add(Reason::OverrideInstructions);
        set.add(Reason::ProtocolMarkup);
        set.add(Reason::TooLong);
        assert_eq!(
            set.names(true),
            ["override-instructions", "protocol-markup"]
        );
        assert_eq!(set.names(false), ["too-long"]);
        assert!(!set.has(Reason::Shouting));
    }
}
