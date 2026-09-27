// SPDX-License-Identifier: Apache-2.0
//! Deciding whether one error message names a way forward, and reading the by-design markers.
//!
//! # Role in the harness
//! This is the pure half of the refusal ratchet: text in, verdict out. It is separated from the
//! syntax walking so that the rules can be tested exhaustively without a Rust file in sight, and so
//! that a contributor arguing with a finding has one short place to read the rules.
//!
//! # Invariants
//! * All distances are measured in characters, not bytes, so an accented word does not move the
//!   40-character window.
//! * The by-design vocabulary is closed. An unrecognised shape is a hard failure, never a silent
//!   pass, because a typo in a suppression would otherwise suppress silently.

/// The three shapes a deliberate refusal may take. The vocabulary is closed on purpose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shape {
    /// Only the operator knows what to do next, so the tool cannot name it.
    OperatorKnowledge,
    /// The fix is outside this tool: a file, a permission, a machine, another program.
    WorldAction,
    /// A person must decide; naming a command would push them past the decision.
    HumanAuthority,
}

impl Shape {
    /// Parses one shape name, rejecting anything outside the closed vocabulary.
    ///
    /// # Errors
    /// Returns the list of accepted names when `name` is not one of them.
    fn parse(name: &str) -> Result<Self, String> {
        match name {
            "operator-knowledge" => Ok(Self::OperatorKnowledge),
            "world-action" => Ok(Self::WorldAction),
            "human-authority" => Ok(Self::HumanAuthority),
            _ => Err(format!(
                "unknown refusal shape `{name}`; use one of `operator-knowledge`, \
                 `world-action`, `human-authority`"
            )),
        }
    }

    /// Returns the name used in source comments and in the summary.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::OperatorKnowledge => "operator-knowledge",
            Self::WorldAction => "world-action",
            Self::HumanAuthority => "human-authority",
        }
    }
}

/// The comment marker that excuses a message from the continuation rule.
const MARKER: &str = "refusal:by-design";

/// How many lines above a message site a marker may sit and still excuse it.
const MARKER_REACH: usize = 3;

/// How many characters a backticked span may sit after a verb and still count as its object.
const VERB_REACH: usize = 40;

/// Command names that make a backticked span a command rather than a bare identifier.
const COMMAND_HEADS: &[&str] = &[
    "bash",
    "cargo",
    "git",
    "npm",
    "npx",
    "pip",
    "pn-ultramemory",
    "python",
    "python3",
    "rustup",
    "sh",
];

/// Verbs that turn a following backticked span into an explicit instruction.
const INSTRUCTION_VERBS: &[&str] = &[
    "add", "call", "disable", "enable", "install", "pass", "provide", "rerun", "set", "specify",
    "supply", "try", "use",
];

/// A by-design marker read out of the source text.
#[derive(Debug)]
pub(crate) struct Marker {
    /// 1-based line the marker sits on.
    pub(crate) line: usize,
    /// The declared shape.
    pub(crate) shape: Shape,
    /// The reason the author gave, kept for the summary.
    pub(crate) reason: String,
    /// Set once the marker has excused at least one finding.
    pub(crate) used: bool,
}

impl Marker {
    /// Returns true when this marker is close enough to excuse a site starting on `line`.
    pub(crate) fn covers(&self, line: usize) -> bool {
        line >= self.line && line - self.line <= MARKER_REACH
    }
}

/// Reads every by-design marker out of a file's text.
///
/// # Errors
/// Returns a message for a marker with an unknown shape or an empty reason. Both are failures rather
/// than warnings: a suppression nobody can read is worse than no suppression.
pub(crate) fn markers(text: &str) -> Result<Vec<Marker>, Vec<String>> {
    let mut found = Vec::new();
    let mut problems = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let number = index + 1;
        let Some(at) = line.find(MARKER) else {
            continue;
        };
        let Some(comment) = line.find("//") else {
            continue;
        };
        if comment > at {
            continue;
        }
        let rest = line.get(at + MARKER.len()..).unwrap_or_default().trim();
        let Some((shape, reason)) = rest.split_once(':') else {
            problems.push(format!(
                "line {number}: `{MARKER}` must read `// {MARKER} <shape>: <reason>`, \
                 with a colon before the reason"
            ));
            continue;
        };
        let shape = match Shape::parse(shape.trim()) {
            Ok(shape) => shape,
            Err(message) => {
                problems.push(format!("line {number}: {message}"));
                continue;
            }
        };
        if reason.trim().is_empty() {
            problems.push(format!(
                "line {number}: `{MARKER} {}` has no reason after the colon",
                shape.name()
            ));
            continue;
        }
        found.push(Marker {
            line: number,
            shape,
            reason: reason.trim().to_owned(),
            used: false,
        });
    }
    if problems.is_empty() {
        Ok(found)
    } else {
        Err(problems)
    }
}

/// Returns true when `message` carries text a reader could act on, rather than only placeholders.
///
/// A format string such as `"{}"` or `"{}: {}"` forwards somebody else's message and says nothing
/// itself, so it is not treated as a message site at all.
pub(crate) fn carries_prose(message: &str) -> bool {
    let mut depth = 0_usize;
    for ch in message.chars() {
        match ch {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            _ if depth == 0 && ch.is_alphabetic() => return true,
            _ => {}
        }
    }
    false
}

/// Returns true when `message` names a continuation: something the reader can do next.
///
/// The rules, in order, are:
/// 1. it names a subcommand of the tool, as in `pn-ultramemory index`;
/// 2. it contains a backticked command or flag, as in `` `cargo test` `` or `` `--force` ``;
/// 3. the word *run* is followed within 40 characters by a backticked span;
/// 4. an instruction verb such as *pass* or *use* is followed within 40 characters by a backticked
///    span.
pub(crate) fn names_continuation(message: &str) -> bool {
    let chars: Vec<char> = message.chars().collect();
    let spans = backtick_spans(&chars);
    names_subcommand(&chars)
        || spans.iter().any(|span| is_command_like(&span.content))
        || verb_then_span(&chars, &spans, &["run"])
        || verb_then_span(&chars, &spans, INSTRUCTION_VERBS)
}

/// A backticked span: where it opens, and what is inside it.
struct Span {
    /// Character index of the opening backtick.
    start: usize,
    /// The text between the backticks.
    content: String,
}

/// Finds every pair of backticks in `chars`. An unpaired trailing backtick is ignored.
fn backtick_spans(chars: &[char]) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut opening: Option<usize> = None;
    for (index, ch) in chars.iter().enumerate() {
        if *ch != '`' {
            continue;
        }
        match opening {
            None => opening = Some(index),
            Some(start) => {
                spans.push(Span {
                    start,
                    content: chars
                        .get(start + 1..index)
                        .unwrap_or_default()
                        .iter()
                        .collect(),
                });
                opening = None;
            }
        }
    }
    spans
}

/// Returns true when the span looks like something to type: a known command, or a flag.
fn is_command_like(content: &str) -> bool {
    let trimmed = content.trim();
    if trimmed.starts_with('-') && trimmed.len() > 1 {
        return true;
    }
    trimmed
        .split_whitespace()
        .next()
        .is_some_and(|head| COMMAND_HEADS.contains(&head))
}

/// Returns true when `chars` holds `pn-ultramemory` followed by a lowercase subcommand word.
fn names_subcommand(chars: &[char]) -> bool {
    let needle: Vec<char> = "pn-ultramemory ".chars().collect();
    for start in 0..chars.len() {
        if chars
            .get(start..start + needle.len())
            .is_none_or(|slice| slice != needle.as_slice())
        {
            continue;
        }
        let mut cursor = start + needle.len();
        let first = chars.get(cursor).copied();
        if !first.is_some_and(|ch| ch.is_ascii_lowercase()) {
            continue;
        }
        cursor += 1;
        while chars
            .get(cursor)
            .is_some_and(|ch| ch.is_ascii_lowercase() || *ch == '-')
        {
            cursor += 1;
        }
        return true;
    }
    false
}

/// Returns true when any of `verbs` appears as a whole word with a backticked span close behind it.
fn verb_then_span(chars: &[char], spans: &[Span], verbs: &[&str]) -> bool {
    for verb in verbs {
        for end in whole_word_ends(chars, verb) {
            if spans
                .iter()
                .any(|span| span.start >= end && span.start - end <= VERB_REACH)
            {
                return true;
            }
        }
    }
    false
}

/// Returns the character index just past each whole-word, case-insensitive occurrence of `word`.
fn whole_word_ends(chars: &[char], word: &str) -> Vec<usize> {
    let needle: Vec<char> = word.chars().collect();
    let mut ends = Vec::new();
    for start in 0..chars.len() {
        let Some(slice) = chars.get(start..start + needle.len()) else {
            break;
        };
        let matches = slice
            .iter()
            .zip(&needle)
            .all(|(left, right)| left.to_ascii_lowercase() == *right);
        if !matches {
            continue;
        }
        let before = start
            .checked_sub(1)
            .and_then(|index| chars.get(index).copied());
        let after = chars.get(start + needle.len()).copied();
        if before.is_some_and(is_word_char) || after.is_some_and(is_word_char) {
            continue;
        }
        ends.push(start + needle.len());
    }
    ends
}

/// Returns true when `ch` would continue a word, so a match beside it is not a whole word.
fn is_word_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_' || ch == '-'
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A message naming a subcommand of the tool names a continuation.
    #[test]
    fn a_subcommand_is_a_continuation() {
        assert!(names_continuation(
            "no index yet; run pn-ultramemory index first"
        ));
        assert!(names_continuation("try pn-ultramemory re-index"));
        assert!(!names_continuation("pn-ultramemory Index"));
        assert!(!names_continuation("pn-ultramemory"));
    }

    /// A backticked command or flag is a continuation; a backticked identifier on its own is not.
    #[test]
    fn a_backticked_command_is_a_continuation() {
        assert!(names_continuation(
            "the workspace is dirty; `cargo fmt --all` fixes it"
        ));
        assert!(names_continuation("`--force` overrides this"));
        assert!(!names_continuation("the field `line` is out of range"));
    }

    /// The word run followed by a backticked span is a continuation, but only within reach.
    #[test]
    fn run_then_a_backticked_token_is_a_continuation() {
        assert!(names_continuation("run `repair` to rebuild the database"));
        let far = format!("run {} `repair`", "x".repeat(60));
        assert!(!names_continuation(&far));
        assert!(!names_continuation("the rerunner `x` failed"));
    }

    /// An explicit instruction such as pass or use is a continuation.
    #[test]
    fn an_explicit_instruction_is_a_continuation() {
        assert!(names_continuation("pass `--budget` to raise the limit"));
        assert!(names_continuation("use `Detail::Signature` instead"));
        assert!(!names_continuation(
            "the value `Detail::Signature` is unsupported"
        ));
    }

    /// A message with no way forward is not a continuation, whatever it explains.
    #[test]
    fn a_bare_explanation_is_not_a_continuation() {
        assert!(!names_continuation("line 7: duplicate key"));
        assert!(!names_continuation("the capsule exceeds the token budget"));
        assert!(!names_continuation(""));
    }

    /// Distances are counted in characters, so accented text does not shrink the window: a message
    /// answers the same way whether its filler is ASCII or not.
    #[test]
    fn distances_are_counted_in_characters() {
        for filler in [10_usize, 30, 60] {
            let ascii = format!("run {} `x`", "x".repeat(filler));
            let accented = format!("run {} `x`", "\u{e9}".repeat(filler));
            assert_eq!(
                names_continuation(&ascii),
                names_continuation(&accented),
                "filler of {filler}"
            );
        }
        assert!(names_continuation(&format!(
            "run {} `x`",
            "\u{e9}".repeat(30)
        )));
        assert!(!names_continuation(&format!(
            "run {} `x`",
            "\u{e9}".repeat(60)
        )));
    }

    /// A format string that is only placeholders carries no prose and is not a message site.
    #[test]
    fn placeholder_only_strings_carry_no_prose() {
        assert!(!carries_prose("{}"));
        assert!(!carries_prose("{}: {}"));
        assert!(!carries_prose("{count} {total}"));
        assert!(carries_prose("line {}: bad escape"));
        assert!(carries_prose("caf\u{e9}"));
    }

    /// A well-formed marker parses into its shape and reason.
    #[test]
    fn reads_a_well_formed_marker() {
        let found =
            markers("// refusal:by-design world-action: the disk is full\n").expect("reads");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].line, 1);
        assert_eq!(found[0].shape, Shape::WorldAction);
        assert_eq!(found[0].reason, "the disk is full");
    }

    /// An unknown shape is a hard failure, so a typo cannot suppress a finding silently.
    #[test]
    fn an_unknown_shape_fails() {
        let problems =
            markers("// refusal:by-design operator-knowledg: typo\n").expect_err("fails");
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("unknown refusal shape"));
    }

    /// A marker without a reason, or without the colon, is a hard failure too.
    #[test]
    fn a_marker_without_a_reason_fails() {
        let problems = markers("// refusal:by-design human-authority:   \n").expect_err("fails");
        assert!(problems[0].contains("no reason"));
        let problems = markers("// refusal:by-design human-authority\n").expect_err("fails");
        assert!(problems[0].contains("must read"));
    }

    /// The marker is only honoured inside a comment, so a string mentioning it does not suppress.
    #[test]
    fn the_marker_must_be_in_a_comment() {
        let found = markers("let s = \"refusal:by-design world-action: no\";\n").expect("reads");
        assert!(found.is_empty());
    }

    /// A marker reaches its own line and the three lines below it, and no further.
    #[test]
    fn a_marker_reaches_three_lines() {
        let marker = Marker {
            line: 10,
            shape: Shape::WorldAction,
            reason: "x".to_owned(),
            used: false,
        };
        assert!(marker.covers(10));
        assert!(marker.covers(13));
        assert!(!marker.covers(14));
        assert!(!marker.covers(9));
    }

    /// Every shape name round-trips through parsing, so the vocabulary has exactly three members.
    #[test]
    fn the_vocabulary_has_three_members() {
        for name in ["operator-knowledge", "world-action", "human-authority"] {
            assert_eq!(Shape::parse(name).expect("parses").name(), name);
        }
        assert!(Shape::parse("").is_err());
    }
}
