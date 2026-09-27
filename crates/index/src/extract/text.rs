// SPDX-License-Identifier: Apache-2.0
//! Small text helpers shared by the extractors: whitespace collapsing, bounded truncation and
//! documentation-comment cleaning.
//!
//! Everything here is a pure function over `&str`, linear in the input size, and never panics:
//! every slice is taken on a character boundary that was checked first.

/// Converts a length or offset to `u32`, saturating instead of wrapping.
pub(crate) fn to_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// Returns the largest character boundary of `text` that is at most `index`.
pub(crate) fn floor_boundary(text: &str, index: usize) -> usize {
    let mut index = index.min(text.len());
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

/// Collapses every run of whitespace into one space and trims both ends.
pub(crate) fn collapse_whitespace(text: &str) -> String {
    let mut out = String::with_capacity(text.len().min(256));
    let mut pending = false;
    for c in text.chars() {
        if c.is_whitespace() {
            pending = !out.is_empty();
        } else {
            if pending {
                out.push(' ');
                pending = false;
            }
            out.push(c);
        }
    }
    out
}

/// Removes the spaces that line breaks leave inside brackets of a collapsed signature: after `(`
/// or `[`, before `)`, `]` and `,`, and a trailing comma before a closing bracket.
pub(crate) fn tidy_signature(text: &str) -> String {
    text.replace("( ", "(")
        .replace("[ ", "[")
        .replace(" ,", ",")
        .replace(" )", ")")
        .replace(" ]", "]")
        .replace(",)", ")")
        .replace(",]", "]")
}

/// Shortens `text` to at most `max` characters, ending with `...` when it was cut.
pub(crate) fn truncate_chars(text: String, max: usize) -> String {
    if text.chars().count() <= max {
        return text;
    }
    let keep = max.saturating_sub(3);
    let mut out: String = text.chars().take(keep).collect();
    out.push_str("...");
    out
}

/// Removes the tokens that open a body from the end of a signature: `{`, `:`, `;`, `=>` and `=`.
pub(crate) fn trim_signature_tail(text: &str) -> &str {
    let mut text = text.trim_end();
    loop {
        let trimmed = text
            .strip_suffix('{')
            .or_else(|| text.strip_suffix(':'))
            .or_else(|| text.strip_suffix(';'))
            .or_else(|| text.strip_suffix("=>"))
            .or_else(|| text.strip_suffix('='))
            .map(str::trim_end);
        match trimmed {
            Some(shorter) => text = shorter,
            None => return text,
        }
    }
}

/// Returns `true` when `text` looks like a plain dotted or scoped path such as `self`, `a.b`,
/// `Foo::bar`, `$this` or `pkg\Name`, and is short enough to be a useful qualifier.
pub(crate) fn is_simple_path(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 120
        && text.chars().all(|c| {
            c.is_alphanumeric() || matches!(c, '_' | '.' | ':' | '$' | '\\' | '-' | '>' | '@' | '#')
        })
}

/// Removes the comment marker of one comment and returns its text lines.
///
/// Handles `///`, `//!`, `//`, `#`, `--`, `;`, `/** ... */`, `/*! ... */` and `/* ... */`. A block comment
/// loses its leading `*` decoration on every line.
fn strip_comment_markers(comment: &str, out: &mut Vec<String>) {
    let trimmed = comment.trim();
    if let Some(rest) = trimmed.strip_prefix("/*") {
        let rest = rest.strip_suffix("*/").unwrap_or(rest);
        let rest = rest
            .strip_prefix('*')
            .or_else(|| rest.strip_prefix('!'))
            .unwrap_or(rest);
        for line in rest.lines() {
            let line = line.trim();
            let line = line.strip_prefix('*').unwrap_or(line);
            out.push(strip_one_space(line).trim_end().to_owned());
        }
        return;
    }
    for line in trimmed.lines() {
        let line = line.trim_start();
        let body = if let Some(rest) = line.strip_prefix("///") {
            rest
        } else if let Some(rest) = line.strip_prefix("//!") {
            rest
        } else if let Some(rest) = line.strip_prefix("//") {
            rest
        } else if let Some(rest) = line.strip_prefix('#') {
            rest.trim_start_matches('#')
        } else if let Some(rest) = line.strip_prefix("--") {
            rest.trim_start_matches('-')
        } else if let Some(rest) = line.strip_prefix(';') {
            rest.trim_start_matches(';')
        } else {
            line
        };
        out.push(strip_one_space(body).trim_end().to_owned());
    }
}

/// Removes a single leading space, when there is one.
fn strip_one_space(text: &str) -> &str {
    text.strip_prefix(' ').unwrap_or(text)
}

/// Cleans the raw text of a run of documentation comments into one documentation string.
///
/// Markers are removed, leading and trailing blank lines are dropped, and `None` is returned
/// when nothing is left.
pub(crate) fn clean_comments(comments: &[&str]) -> Option<String> {
    let mut lines = Vec::new();
    for comment in comments {
        strip_comment_markers(comment, &mut lines);
    }
    join_doc_lines(lines)
}

/// Joins doc lines, trimming blank lines at both ends. Returns `None` for an empty result.
pub(crate) fn join_doc_lines(mut lines: Vec<String>) -> Option<String> {
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
    let skip = lines.iter().take_while(|l| l.trim().is_empty()).count();
    let text = lines[skip..].join("\n");
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

/// Extracts the readable text of a C# XML documentation comment.
///
/// When a `<summary>` element is present its content is returned; otherwise the text is
/// returned as it is. The five predefined XML entities are unescaped.
pub(crate) fn xml_doc_text(text: &str) -> String {
    let inner = match (text.find("<summary>"), text.find("</summary>")) {
        (Some(open), Some(close)) if open + 9 <= close => &text[open + 9..close],
        _ => text,
    };
    let lines: Vec<String> = inner.lines().map(|l| l.trim().to_owned()).collect();
    let joined = join_doc_lines(lines).unwrap_or_default();
    joined
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// Removes up to `count` leading whitespace characters from `line`.
fn dedent(line: &str, count: usize) -> &str {
    let mut cut = 0;
    for (taken, (index, c)) in line.char_indices().enumerate() {
        if taken >= count || !c.is_whitespace() {
            cut = index;
            return &line[cut..];
        }
        cut = index + c.len_utf8();
    }
    &line[cut..]
}

/// Cleans the content of a Python string literal used as a docstring.
///
/// The first line is trimmed and the following lines are dedented by their common indentation,
/// as `inspect.cleandoc` does. Escaped backslashes and quotes are unescaped for non-raw strings.
pub(crate) fn python_docstring(content: &str, raw: bool) -> Option<String> {
    let mut lines = content.lines();
    let first = lines.next().unwrap_or("").trim().to_owned();
    let rest: Vec<&str> = lines.collect();
    let indent = rest
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.chars().take_while(|c| c.is_whitespace()).count())
        .min()
        .unwrap_or(0);
    let mut out = vec![first];
    out.extend(rest.iter().map(|l| dedent(l, indent).trim_end().to_owned()));
    let text = join_doc_lines(out)?;
    if raw {
        Some(text)
    } else {
        Some(text.replace("\\\"", "\"").replace("\\\\", "\\"))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        clean_comments, collapse_whitespace, floor_boundary, is_simple_path, python_docstring,
        to_u32, trim_signature_tail, truncate_chars, xml_doc_text,
    };

    /// Line-break artifacts are removed from inside brackets.
    #[test]
    fn signature_is_tidied() {
        assert_eq!(
            super::tidy_signature("fn add( &mut self, key: &str, item: T, ) -> R"),
            "fn add(&mut self, key: &str, item: T) -> R"
        );
        assert_eq!(super::tidy_signature("f(a , b)"), "f(a, b)");
    }

    /// Whitespace runs collapse to one space and the ends are trimmed.
    #[test]
    fn collapse_handles_mixed_whitespace() {
        assert_eq!(collapse_whitespace("  a \t\n b  "), "a b");
        assert_eq!(collapse_whitespace(""), "");
        assert_eq!(collapse_whitespace(" \n "), "");
    }

    /// Truncation keeps short text and marks the cut on long text, counting characters.
    #[test]
    fn truncate_counts_characters() {
        assert_eq!(truncate_chars("abc".to_owned(), 5), "abc");
        let long = "é".repeat(300);
        let cut = truncate_chars(long, 240);
        assert_eq!(cut.chars().count(), 240);
        assert!(cut.ends_with("..."));
    }

    /// Body openers are removed from the tail of a signature, repeatedly.
    #[test]
    fn signature_tail_is_trimmed() {
        assert_eq!(trim_signature_tail("fn f() {"), "fn f()");
        assert_eq!(trim_signature_tail("def f():"), "def f()");
        assert_eq!(trim_signature_tail("const f = (a) =>"), "const f = (a)");
        assert_eq!(trim_signature_tail("struct A;"), "struct A");
        assert_eq!(trim_signature_tail("const X: usize ="), "const X: usize");
        assert_eq!(trim_signature_tail("plain"), "plain");
    }

    /// Simple qualifiers are accepted and expressions are rejected.
    #[test]
    fn simple_paths() {
        assert!(is_simple_path("self"));
        assert!(is_simple_path("a.b.c"));
        assert!(is_simple_path("Foo::bar"));
        assert!(is_simple_path("$this"));
        assert!(!is_simple_path("a.b()"));
        assert!(!is_simple_path("x[0]"));
        assert!(!is_simple_path(""));
    }

    /// Offsets convert with saturation and boundaries never split a character.
    #[test]
    fn numeric_and_boundary_helpers() {
        assert_eq!(to_u32(5), 5);
        assert_eq!(floor_boundary("aé", 2), 1);
        assert_eq!(floor_boundary("abc", 99), 3);
    }

    /// Every comment style loses its markers.
    #[test]
    fn comment_markers_are_removed() {
        assert_eq!(
            clean_comments(&["/// One.\n", "/// Two."]).as_deref(),
            Some("One.\nTwo.")
        );
        assert_eq!(
            clean_comments(&["/**\n * Block doc.\n *\n * More.\n */"]).as_deref(),
            Some("Block doc.\n\nMore.")
        );
        assert_eq!(clean_comments(&["# ruby doc"]).as_deref(), Some("ruby doc"));
        assert_eq!(clean_comments(&["// go doc"]).as_deref(), Some("go doc"));
        assert_eq!(clean_comments(&["///"]), None);
        assert_eq!(clean_comments(&["/** */"]), None);
    }

    /// The summary element of a C# comment is returned, entities unescaped.
    #[test]
    fn xml_summary_is_extracted() {
        assert_eq!(
            xml_doc_text(
                "<summary>\nA &lt;b&gt; service.\n</summary>\n<param name=\"x\">y</param>"
            ),
            "A <b> service."
        );
        assert_eq!(xml_doc_text("plain text"), "plain text");
    }

    /// Docstrings are dedented like `inspect.cleandoc`.
    #[test]
    fn docstring_is_dedented() {
        assert_eq!(
            python_docstring("Foo doc.\n\n    More.\n    ", false).as_deref(),
            Some("Foo doc.\n\nMore.")
        );
        assert_eq!(python_docstring("  ", false), None);
        assert_eq!(
            python_docstring("a \\\\ b", false).as_deref(),
            Some("a \\ b")
        );
        assert_eq!(
            python_docstring("a \\\\ b", true).as_deref(),
            Some("a \\\\ b")
        );
    }
}
