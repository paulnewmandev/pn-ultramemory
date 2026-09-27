// SPDX-License-Identifier: Apache-2.0
//! Identifier tokenization for the full-text indexes, and safe construction of full-text queries.
//!
//! Source identifiers are written as `parseConfig`, `parse_config`, `HTTPServer` or `sha256`.
//! A person (or an agent) searching for "parse config" expects to find all of them, so the
//! adapter splits text into lowercase word parts at `camelCase`, `snake_case` and digit
//! boundaries **in Rust**, before the text reaches SQLite, and stores the parts as the indexed
//! text. Queries are split by the very same function, which is what makes them line up.
//!
//! # Query safety
//! FTS5 has its own query language (`AND`, `NOT`, `NEAR`, column filters, parentheses, quotes).
//! Text from a user must never reach it as syntax. [`match_expression`] therefore never forwards
//! the caller's characters: it rebuilds the query from the alphanumeric word parts only, each
//! one wrapped in double quotes, so an operator word is just a word and no input can produce a
//! syntax error. The number and size of terms is capped, so a hostile query cannot make the
//! search expensive.
//!
//! Position in the architecture: an internal helper of the storage adapter, used by symbol and
//! memory indexing and by both searches.

use crate::convert::truncate_utf8;

/// The most bytes of one column that are indexed. Longer text is cut, at a character boundary.
pub(crate) const MAX_INDEXED_BYTES: usize = 8 * 1024;

/// The most bytes of a query that are looked at.
const MAX_QUERY_BYTES: usize = 2 * 1024;

/// The most distinct terms a query can have.
const MAX_QUERY_TERMS: usize = 32;

/// The kind of character that decides where a word part ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    /// A lowercase letter.
    Lower,
    /// An uppercase letter.
    Upper,
    /// A digit or another numeric character.
    Digit,
    /// A letter without case, such as a CJK ideograph.
    Other,
    /// Anything that is not a letter or a digit: punctuation, spaces, control characters.
    Separator,
}

/// Classifies one character.
fn classify(ch: char) -> Class {
    if ch.is_lowercase() {
        Class::Lower
    } else if ch.is_uppercase() {
        Class::Upper
    } else if ch.is_numeric() {
        Class::Digit
    } else if ch.is_alphabetic() {
        Class::Other
    } else {
        Class::Separator
    }
}

/// Decides whether a word part ends before `current`, given the previous and the next
/// character classes.
///
/// `parseConfig` breaks between `e` and `C`; `HTTPServer` breaks before the `S`, because an
/// uppercase letter followed by a lowercase one starts a new word; letters and digits always
/// break apart.
fn breaks_before(previous: Class, current: Class, next: Option<Class>) -> bool {
    match (previous, current) {
        (Class::Separator, _) => false,
        (Class::Lower, Class::Upper) => true,
        (Class::Upper, Class::Upper) => next == Some(Class::Lower),
        (a, b) => a != b && !(a == Class::Upper && b == Class::Lower),
    }
}

/// Reusable buffers of the scanner, so that indexing many symbols does not allocate per word.
#[derive(Debug, Default)]
pub(crate) struct Scratch {
    /// The lowercase word part being built.
    part: String,
    /// The parts of the current run, glued together.
    glue: String,
}

/// What [`scan`] reports.
#[derive(Debug, Clone, Copy)]
enum Event<'a> {
    /// A lowercase word part.
    Part(&'a str),
    /// The end of a run of letters and digits: its parts glued together, and how many there were.
    Run(&'a str, usize),
}

/// Walks `text` once and reports its lowercase word parts.
///
/// Text is cut into runs of letters and digits (any separator, and `_` is one, ends a run), and
/// each run into word parts (see [`breaks_before`]). Every part is reported in order, and each
/// run is reported after its last part.
fn scan(text: &str, scratch: &mut Scratch, mut emit: impl FnMut(Event<'_>)) {
    let Scratch { part, glue } = scratch;
    part.clear();
    glue.clear();
    let mut parts = 0_usize;
    let mut previous = Class::Separator;
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        let class = classify(ch);
        let next = chars.peek().map(|&c| classify(c));
        let ends_part = class == Class::Separator || breaks_before(previous, class, next);
        if ends_part && !part.is_empty() {
            emit(Event::Part(part));
            glue.push_str(part);
            part.clear();
            parts += 1;
        }
        if class == Class::Separator {
            if parts > 0 {
                emit(Event::Run(glue, parts));
                glue.clear();
                parts = 0;
            }
        } else if ch.is_ascii() {
            part.push(ch.to_ascii_lowercase());
        } else {
            part.extend(ch.to_lowercase());
        }
        previous = class;
    }
    if !part.is_empty() {
        emit(Event::Part(part));
        glue.push_str(part);
        parts += 1;
    }
    if parts > 0 {
        emit(Event::Run(glue, parts));
    }
}

/// Splits text into lowercase word parts, in order.
///
/// # Examples
/// ```text
/// parseConfig        -> parse, config
/// parse_config       -> parse, config
/// HTTPServer2Go      -> http, server, 2, go
/// ```
pub(crate) fn split_parts(text: &str) -> Vec<String> {
    let mut parts = Vec::new();
    scan(text, &mut Scratch::default(), |event| {
        if let Event::Part(part) = event {
            parts.push(part.to_owned());
        }
    });
    parts
}

/// Builds the text stored in a full-text column into `out`: the word parts separated by spaces,
/// and, for a word that was split (`parseConfig`), also the parts glued together
/// (`parseconfig`), so a query typed in one lowercase word still finds it.
///
/// `text` is cut to `max_bytes` first, and `out` is cleared first.
pub(crate) fn index_text_into(
    out: &mut String,
    text: &str,
    max_bytes: usize,
    scratch: &mut Scratch,
) {
    out.clear();
    scan(
        truncate_utf8(text, max_bytes),
        scratch,
        |event| match event {
            Event::Part(part) => {
                if !out.is_empty() {
                    out.push(' ');
                }
                out.push_str(part);
            }
            Event::Run(glue, parts) if parts > 1 => {
                out.push(' ');
                out.push_str(glue);
            }
            Event::Run(..) => {}
        },
    );
}

/// Builds the text stored in a full-text column; see [`index_text_into`].
pub(crate) fn index_text(text: &str, max_bytes: usize) -> String {
    let mut out = String::new();
    index_text_into(&mut out, text, max_bytes, &mut Scratch::default());
    out
}

/// How the terms of a query combine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Join {
    /// Every term must match.
    All,
    /// At least one term must match; rows matching more terms rank higher.
    Any,
}

/// Turns arbitrary text into a full-text query that is always valid, or `None` when the text has
/// no word in it.
///
/// Every term is a quoted word part; the last term also matches by prefix, so a half-typed name
/// finds its completions. Repeated terms are dropped and at most 32 terms are kept.
///
/// # Examples
/// ```text
/// parseConfig   -> "parse" AND "config"*
/// NEAR(a b) OR  -> "near" AND "a" AND "b" AND "or"*
/// ("unbalanced  -> "unbalanced"*
/// ```
pub(crate) fn match_expression(text: &str, join: Join) -> Option<String> {
    let parts = split_parts(truncate_utf8(text, MAX_QUERY_BYTES));
    let last = parts.last()?.clone();
    let mut terms: Vec<String> = Vec::new();
    for part in parts {
        if !terms.contains(&part) {
            terms.push(part);
        }
    }
    if terms.len() > MAX_QUERY_TERMS {
        terms.truncate(MAX_QUERY_TERMS - 1);
        if !terms.contains(&last) {
            terms.push(last.clone());
        }
    }
    let glue = match join {
        Join::All => " AND ",
        Join::Any => " OR ",
    };
    let rendered: Vec<String> = terms
        .iter()
        .map(|term| {
            let quoted = format!("\"{}\"", term.replace('"', "\"\""));
            if *term == last {
                format!("{quoted}*")
            } else {
                quoted
            }
        })
        .collect();
    Some(rendered.join(glue))
}

#[cfg(test)]
mod tests {
    use super::{Join, MAX_INDEXED_BYTES, index_text, match_expression, split_parts};

    /// Identifiers split at camel case, snake case, digits and acronyms.
    #[test]
    fn splits_identifiers() {
        let cases: &[(&str, &[&str])] = &[
            ("parseConfig", &["parse", "config"]),
            ("parse_config", &["parse", "config"]),
            ("ParseConfig", &["parse", "config"]),
            ("PARSE_CONFIG", &["parse", "config"]),
            ("HTTPServer", &["http", "server"]),
            ("XMLHttpRequest", &["xml", "http", "request"]),
            ("getHTTPResponseCode", &["get", "http", "response", "code"]),
            ("sha256sum", &["sha", "256", "sum"]),
            ("utf8", &["utf", "8"]),
            ("ID3", &["id", "3"]),
            ("a", &["a"]),
            ("Parser::parse", &["parser", "parse"]),
            ("models.User.save", &["models", "user", "save"]),
            ("kebab-case-name", &["kebab", "case", "name"]),
            ("日本語parse", &["日本語", "parse"]),
            ("ünïcodé", &["ünïcodé"]),
            ("", &[]),
            ("   ", &[]),
            ("!!!", &[]),
            ("___", &[]),
        ];
        for (input, expected) in cases {
            assert_eq!(split_parts(input), *expected, "input {input:?}");
        }
    }

    /// Split words also get their glued form, and unsplit words do not.
    #[test]
    fn index_text_adds_glued_words() {
        assert_eq!(
            index_text("parseConfig", MAX_INDEXED_BYTES),
            "parse config parseconfig"
        );
        assert_eq!(
            index_text("parse_config", MAX_INDEXED_BYTES),
            "parse config"
        );
        assert_eq!(index_text("fn parse()", MAX_INDEXED_BYTES), "fn parse");
        assert_eq!(index_text("", MAX_INDEXED_BYTES), "");
    }

    /// Long text is cut at a character boundary before it is split.
    #[test]
    fn index_text_is_bounded() {
        let long = "word ".repeat(10_000);
        let indexed = index_text(&long, 100);
        assert!(indexed.len() <= 100);
        let multibyte = "é".repeat(1000);
        assert!(index_text(&multibyte, 101).len() <= 101);
    }

    /// The same text yields the same query, made of quoted words with a prefix on the last.
    #[test]
    fn builds_queries() {
        assert_eq!(
            match_expression("parseConfig", Join::All).as_deref(),
            Some("\"parse\" AND \"config\"*")
        );
        assert_eq!(
            match_expression("parse config", Join::Any).as_deref(),
            Some("\"parse\" OR \"config\"*")
        );
        assert_eq!(match_expression("", Join::All), None);
        assert_eq!(match_expression("  ()\"'  ", Join::All), None);
        assert_eq!(
            match_expression("dup dup dup", Join::All).as_deref(),
            Some("\"dup\"*")
        );
    }

    /// Whatever the input, the query contains only quoted words, `*`, spaces and the two
    /// connectives, so it cannot carry syntax.
    #[test]
    fn queries_carry_no_syntax() {
        let hostile = [
            "\"",
            "\"\"",
            "a\"b",
            "NEAR(a b, 2)",
            "a AND",
            "OR OR",
            "NOT x",
            "x:y",
            "^x",
            "-x",
            "(",
            ")",
            "((()))",
            "a OR (b",
            "*",
            "**",
            "a*",
            "col:\"v\" AND",
            "{a b}",
            "\\",
            "'--",
            "\0",
            "a\0b",
            "\u{202e}",
            "🦀 crab",
            "name:foo NEAR/3 bar",
        ];
        for input in hostile {
            for join in [Join::All, Join::Any] {
                let Some(query) = match_expression(input, join) else {
                    continue;
                };
                let stripped = query.replace(" AND ", " ").replace(" OR ", " ");
                let mut in_quote = false;
                for ch in stripped.chars() {
                    match ch {
                        '"' => in_quote = !in_quote,
                        ' ' | '*' => assert!(!in_quote, "{input:?} -> {query}"),
                        other => {
                            assert!(in_quote && other.is_alphanumeric(), "{input:?} -> {query}");
                        }
                    }
                }
                assert!(!in_quote, "{input:?} -> {query}");
            }
        }
    }

    /// A query never has more than 32 distinct terms and always keeps the last one.
    #[test]
    fn query_terms_are_capped() {
        let mut words = String::new();
        for i in 0..255_u8 {
            words.push('t');
            words.push(char::from(b'a' + i % 26));
            words.push(char::from(b'a' + i / 26));
            words.push(' ');
        }
        words.push_str("final");
        let query = match_expression(&words, Join::All).unwrap_or_default();
        assert!(query.matches(" AND ").count() < 32);
        assert!(query.ends_with("\"final\"*"));
    }

    /// A slow, obvious implementation of the splitting rules, working on a vector of characters.
    fn reference_runs(text: &str) -> Vec<Vec<String>> {
        let chars: Vec<char> = text.chars().collect();
        let class_of = |c: char| {
            if c.is_lowercase() {
                'l'
            } else if c.is_uppercase() {
                'u'
            } else if c.is_numeric() {
                'd'
            } else if c.is_alphabetic() {
                'o'
            } else {
                's'
            }
        };
        let mut runs: Vec<Vec<String>> = Vec::new();
        let mut parts: Vec<String> = Vec::new();
        let mut current = String::new();
        for (i, &ch) in chars.iter().enumerate() {
            let class = class_of(ch);
            if class == 's' {
                if !current.is_empty() {
                    parts.push(std::mem::take(&mut current));
                }
                if !parts.is_empty() {
                    runs.push(std::mem::take(&mut parts));
                }
                continue;
            }
            let previous = if i == 0 { 's' } else { class_of(chars[i - 1]) };
            let next = chars.get(i + 1).map(|&c| class_of(c));
            let breaks = match (previous, class) {
                ('s', _) | ('u', 'l') => false,
                ('l', 'u') => true,
                ('u', 'u') => next == Some('l'),
                (a, b) => a != b,
            };
            if breaks && !current.is_empty() {
                parts.push(std::mem::take(&mut current));
            }
            current.extend(ch.to_lowercase());
        }
        if !current.is_empty() {
            parts.push(current);
        }
        if !parts.is_empty() {
            runs.push(parts);
        }
        runs
    }

    /// The single-pass scanner agrees with the obvious implementation on many random strings
    /// made of the characters that matter: cases, digits, separators and non-ASCII letters.
    #[test]
    fn scanner_matches_the_reference_implementation() {
        let alphabet: Vec<char> = "aAbBzZ09_ -.:Éé日本ǅß\u{0301}\u{0903}\n\""
            .chars()
            .collect();
        let mut state = 0x1234_5678_u64;
        for _ in 0..4_000 {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let length = usize::try_from((state >> 33) % 14).unwrap_or(0);
            let mut text = String::new();
            for _ in 0..length {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                let pick = usize::try_from((state >> 33) % alphabet.len() as u64).unwrap_or(0);
                text.push(alphabet[pick]);
            }
            let runs = reference_runs(&text);
            let expected_parts: Vec<String> = runs.iter().flatten().cloned().collect();
            assert_eq!(split_parts(&text), expected_parts, "text {text:?}");
            let mut expected_index = String::new();
            for parts in &runs {
                for part in parts {
                    if !expected_index.is_empty() {
                        expected_index.push(' ');
                    }
                    expected_index.push_str(part);
                }
                if parts.len() > 1 {
                    expected_index.push(' ');
                    expected_index.push_str(&parts.concat());
                }
            }
            assert_eq!(
                index_text(&text, MAX_INDEXED_BYTES),
                expected_index,
                "text {text:?}"
            );
        }
    }
}
