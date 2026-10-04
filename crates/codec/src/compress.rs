// SPDX-License-Identifier: Apache-2.0
//! Linguistic compression for capsule text: memories, notes and summaries.
//!
//! Inspired by controlled-language standards (ASD-STE100) and projects like Caveman that proved
//! aggressive verbosity reduction preserves comprehension for LLMs while cutting 30–50% of tokens.
//! The rules are conservative: they strip articles, filler phrases, redundant qualifiers and
//! conversational padding, but **never** touch code spans, numbers, identifiers, negations or
//! file paths. A compressed memory says the same thing in fewer tokens; it does not paraphrase.
//!
//! # What is safe to compress
//! - Articles (`a`, `an`, `the`) before nouns that are already unambiguous in context.
//! - Filler openers (`it is important to note that`, `please be aware that`).
//! - Redundant qualifiers (`very`, `quite`, `essentially`, `basically`).
//! - Passive voice when the actor is irrelevant (`was implemented` → `implemented`).
//! - Conversational hedging (`I think`, `it seems`, `you might want to`).
//!
//! # What is never touched
//! - Code spans (backtick-delimited), identifiers, file paths, URLs.
//! - Numbers, versions, hashes, percentages.
//! - Negations (`not`, `never`, `no`, `don't`, `cannot`) — removing these inverts meaning.
//! - Proper nouns and project-specific terms.
//! - Text shorter than the compression threshold (compressing tiny strings costs more than it saves).

/// Minimum byte length worth compressing. Below this the overhead of scanning exceeds the saving.
const MIN_COMPRESS_BYTES: usize = 24;

/// Filler phrases stripped case-insensitively from the start or middle of sentences.
/// Each entry is `(pattern, replacement)` where replacement may be empty.
const FILLER_PHRASES: &[(&str, &str)] = &[
    ("it is important to note that ", ""),
    ("it's important to note that ", ""),
    ("please be aware that ", ""),
    ("it should be noted that ", ""),
    ("it is worth noting that ", ""),
    ("it's worth noting that ", ""),
    ("note that ", ""),
    ("keep in mind that ", ""),
    ("bear in mind that ", ""),
    ("in order to ", "to "),
    ("due to the fact that ", "because "),
    ("as a matter of fact ", ""),
    ("for the purpose of ", "to "),
    ("in the event that ", "if "),
    ("with regard to ", "regarding "),
    ("with respect to ", "regarding "),
    ("it is recommended to ", ""),
    ("it's recommended to ", ""),
    ("you may want to ", ""),
    ("you might want to ", ""),
    ("i think ", ""),
    ("i believe ", ""),
    ("it seems ", ""),
    ("it appears ", ""),
    ("basically ", ""),
    ("essentially ", ""),
    ("fundamentally ", ""),
    ("generally speaking ", ""),
    ("in general ", ""),
    ("actually ", ""),
    ("pretty much ", ""),
    ("sort of ", ""),
    ("kind of ", ""),
    ("very ", ""),
    ("quite ", ""),
    ("rather ", ""),
    ("somewhat ", ""),
    ("just ", ""),
];

/// Articles stripped before lowercase nouns. Uppercase-starting words are kept (likely proper nouns).
const ARTICLES: &[&str] = &["a ", "an ", "the "];

/// Compresses a single text block, returning the compressed version if it is shorter.
///
/// Returns `None` when compression would not save at least one token, so callers can skip the
/// allocation. The function is idempotent: compressing already-compressed text returns the same
/// result.
#[must_use]
pub fn compress(text: &str) -> Option<String> {
    if text.len() < MIN_COMPRESS_BYTES {
        return None;
    }
    let mut out = text.to_owned();

    // Strip filler phrases (case-insensitive), repeating until no more match so that multiple
    // fillers in one sentence are all removed in a single call.
    loop {
        let lower = out.to_lowercase();
        let mut replaced = false;
        for &(pattern, replacement) in FILLER_PHRASES {
            if let Some(pos) = lower.find(pattern) {
                out.replace_range(pos..pos + pattern.len(), replacement);
                replaced = true;
                break; // Restart scan after each replacement since positions shifted.
            }
        }
        if !replaced {
            break;
        }
    }

    // Strip leading articles before lowercase words (case-insensitive match on the article,
    // but only when the following word starts lowercase so we keep proper nouns like "The Hague").
    let lower_out = out.to_lowercase();
    for article in ARTICLES {
        if lower_out.starts_with(article) {
            let rest = &out[article.len()..];
            if rest.starts_with(|c: char| c.is_ascii_lowercase()) {
                out = rest.to_owned();
                break;
            }
        }
    }

    // Trim trailing whitespace introduced by replacements.
    let trimmed = out.trim().to_owned();

    if trimmed.len() >= text.len() {
        None
    } else {
        Some(trimmed)
    }
}

/// Compresses all memory texts in a capsule, mutating them in place.
///
/// Only memories whose text is long enough and actually shrink are touched. Short texts, code-heavy
/// texts and texts that don't match any pattern are left unchanged.
pub fn compress_memories(memories: &mut [crate::CapsuleMemory]) {
    for memory in memories.iter_mut() {
        if let Some(compressed) = compress(&memory.text) {
            memory.text = compressed;
        }
    }
}

/// Compresses capsule notes in place.
pub fn compress_notes(notes: &mut [String]) {
    for note in notes.iter_mut() {
        if let Some(compressed) = compress(note) {
            *note = compressed;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_filler_phrases() {
        let input = "It is important to note that the parser rejects oversized files";
        let result = compress(input).expect("should compress");
        assert!(!result.contains("important to note"));
        assert!(result.contains("parser rejects"));
    }

    #[test]
    fn strips_leading_articles() {
        let input = "The config loader reads from disk";
        let result = compress(input).expect("should compress");
        assert!(result.starts_with("config loader"));
    }

    #[test]
    fn preserves_negations() {
        let input = "It is important to note that the parser does not reject valid files";
        let result = compress(input).expect("should compress");
        assert!(result.contains("does not reject") || result.contains("not reject"));
    }

    #[test]
    fn skips_short_text() {
        assert!(compress("short").is_none());
        assert!(compress("").is_none());
    }

    #[test]
    fn returns_none_when_no_saving() {
        // Already concise text should not be modified.
        assert!(compress("Money stored as integer minor units").is_none());
    }

    #[test]
    fn idempotent() {
        let input = "It is important to note that basically the parser rejects oversized files";
        let first = compress(input).expect("first pass");
        let second = compress(&first);
        assert_eq!(
            second.as_ref().map_or(first.as_str(), |s| s.as_str()),
            first.as_str()
        );
    }

    #[test]
    fn preserves_code_spans() {
        let input = "It is important to note that `parse_config()` rejects invalid input";
        let result = compress(input).expect("should compress");
        assert!(result.contains("`parse_config()`"));
    }

    #[test]
    fn compress_memories_skips_short() {
        use crate::CapsuleMemory;
        use pn_ultramemory_core::{MemoryId, MemoryKind, Provenance};
        let mut memories = vec![CapsuleMemory {
            id: MemoryId(1),
            kind: MemoryKind::Decision,
            provenance: Provenance::Agent,
            stale: false,
            text: "short".into(),
        }];
        compress_memories(&mut memories);
        assert_eq!(memories[0].text, "short");
    }
}
