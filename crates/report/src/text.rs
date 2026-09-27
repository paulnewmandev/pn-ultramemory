// SPDX-License-Identifier: Apache-2.0
//! Sanitizing and escaping of user-provided text.
//!
//! Every string that comes from the repository (names, paths, memory text, notes) is untrusted:
//! it can contain markup, quotes, control characters, right-to-left overrides or megabytes of
//! text. Renderers first pass it through [`single_line`], which produces something that is safe
//! to measure and lay out, and then through the escaper of the target format.
//!
//! Invariants: no function here panics; [`single_line`] returns text without control characters
//! or newlines and never longer than the requested number of characters plus the ellipsis; the
//! escapers only ever grow the text.

/// Clip length for short names (symbols, modules, languages, graph labels).
pub(crate) const CLIP_NAME: usize = 160;
/// Clip length for file paths.
pub(crate) const CLIP_PATH: usize = 240;
/// Clip length for free text (memories and notes).
pub(crate) const CLIP_TEXT: usize = 600;

/// The ellipsis appended to clipped text.
pub(crate) const ELLIPSIS: char = '\u{2026}';

/// Returns `true` for the bidirectional override and isolate controls, which could visually
/// reorder the text around them.
const fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

/// Collapses `text` to a single line and clips it to `max_chars` characters.
///
/// Runs of whitespace and control characters become one space, leading and trailing whitespace
/// is removed, bidirectional override controls are dropped, and text longer than `max_chars` ends
/// with an ellipsis. Only as much of the input as needed is scanned, so a huge string costs
/// little.
///
/// # Examples
///
/// ```text
/// single_line("a \n\t b", 10) == "a b"
/// single_line("abcdef", 3) == "abc…"
/// ```
pub(crate) fn single_line(text: &str, max_chars: usize) -> String {
    let mut out = String::new();
    let mut count = 0usize;
    let mut pending_space = false;
    for c in text.chars() {
        if is_bidi_control(c) {
            continue;
        }
        if c.is_whitespace() || c.is_control() {
            pending_space = count > 0;
            continue;
        }
        if count >= max_chars {
            out.push(ELLIPSIS);
            return out;
        }
        if pending_space {
            if count + 1 >= max_chars {
                out.push(ELLIPSIS);
                return out;
            }
            out.push(' ');
            count += 1;
            pending_space = false;
        }
        out.push(c);
        count += 1;
    }
    out
}

/// Appends `text` to `out` with the five characters that matter in HTML and XML escaped.
///
/// The result is safe inside element content and inside single- or double-quoted attributes.
pub(crate) fn push_escaped(out: &mut String, text: &str) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
}

/// Returns `text` escaped for HTML and XML (see [`push_escaped`]).
pub(crate) fn escaped(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    push_escaped(&mut out, text);
    out
}

/// Escapes text for a Markdown table cell or paragraph so it is displayed literally.
///
/// Markup characters are backslash-escaped, `<`, `>` and `&` become entities (so raw HTML is
/// never produced), `@` is escaped so nobody is mentioned by accident, and a leading list or
/// ordered-list marker is neutralized. The input should already be a single line.
pub(crate) fn md_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    for (index, c) in text.chars().enumerate() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '\\' | '`' | '*' | '_' | '[' | ']' | '|' | '~' | '#' | '@' => {
                out.push('\\');
                out.push(c);
            }
            '-' | '+' if index == 0 => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    let digits = out.chars().take_while(char::is_ascii_digit).count();
    let marker = out.chars().nth(digits);
    let after = out.chars().nth(digits + 1);
    if digits > 0 && matches!(marker, Some('.' | ')')) && matches!(after, None | Some(' ')) {
        out.insert(digits, '\\');
    }
    out
}

/// Formats `text` as a Markdown inline code span that is safe inside a table cell.
///
/// The fence is one backtick longer than the longest run of backticks inside the text, pipes are
/// replaced by the look-alike `\u{2223}` because a code span cannot escape them portably, and
/// empty text yields an empty string.
pub(crate) fn md_code(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    let text = text.replace('|', "\u{2223}");
    let mut longest = 0usize;
    let mut run = 0usize;
    for c in text.chars() {
        if c == '`' {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    let fence = "`".repeat(longest + 1);
    let pad = if text.starts_with('`') || text.ends_with('`') {
        " "
    } else {
        ""
    };
    format!("{fence}{pad}{text}{pad}{fence}")
}

#[cfg(test)]
mod tests {
    use super::{ELLIPSIS, escaped, md_code, md_escape, single_line};

    /// Whitespace runs and control characters collapse to one space and the ends are trimmed.
    #[test]
    fn single_line_collapses_whitespace() {
        assert_eq!(single_line("  a \n\t b\r\n c  ", 50), "a b c");
        assert_eq!(single_line("a\u{0}b\u{7}c", 50), "a b c");
        assert_eq!(single_line("", 5), "");
        assert_eq!(single_line("   \n ", 5), "");
    }

    /// Text over the limit is clipped with an ellipsis, exact-length text is untouched.
    #[test]
    fn single_line_clips() {
        assert_eq!(single_line("abcdef", 3), format!("abc{ELLIPSIS}"));
        assert_eq!(single_line("abc", 3), "abc");
        assert_eq!(single_line("abc  ", 3), "abc");
        assert_eq!(single_line("ab cd", 3), format!("ab{ELLIPSIS}"));
        assert_eq!(single_line("abc", 0), format!("{ELLIPSIS}"));
    }

    /// Clipping counts characters, not bytes, so accents and emoji never split.
    #[test]
    fn single_line_counts_characters() {
        assert_eq!(single_line("ñandú🙂z", 5), format!("ñandú{ELLIPSIS}"));
        assert_eq!(single_line("ñandú🙂", 6), "ñandú🙂");
    }

    /// Bidirectional overrides are dropped so they cannot reorder surrounding text.
    #[test]
    fn single_line_drops_bidi_overrides() {
        assert_eq!(single_line("a\u{202e}b\u{2066}c", 10), "abc");
        assert_eq!(single_line("שלום", 10), "שלום");
    }

    /// A multi-megabyte input is clipped without scanning all of it into the output.
    #[test]
    fn single_line_handles_huge_input() {
        let huge = "x".repeat(5_000_000);
        let out = single_line(&huge, 100);
        assert_eq!(out.chars().count(), 101);
    }

    /// The five significant characters are escaped and nothing else changes.
    #[test]
    fn html_escaping() {
        assert_eq!(
            escaped("<script>alert(\"x\" & 'y')</script>"),
            "&lt;script&gt;alert(&quot;x&quot; &amp; &#39;y&#39;)&lt;/script&gt;"
        );
        assert_eq!(escaped("ñ 🙂 שלום"), "ñ 🙂 שלום");
    }

    /// Markdown escaping neutralizes markup, mentions, raw HTML and leading list markers.
    #[test]
    fn markdown_escaping() {
        assert_eq!(md_escape("a|b"), "a\\|b");
        assert_eq!(md_escape("<b>x</b>"), "&lt;b&gt;x&lt;/b&gt;");
        assert_eq!(
            md_escape("snake_case *x* `y` [z](u)"),
            "snake\\_case \\*x\\* \\`y\\` \\[z\\](u)"
        );
        assert_eq!(md_escape("# title"), "\\# title");
        assert_eq!(md_escape("- item"), "\\- item");
        assert_eq!(md_escape("1. item"), "1\\. item");
        assert_eq!(md_escape("1.2.36"), "1.2.36");
        assert_eq!(md_escape("2)"), "2\\)");
        assert_eq!(md_escape("@user"), "\\@user");
        assert_eq!(md_escape("plain text"), "plain text");
    }

    /// Code spans use a fence longer than any backtick run and never contain a pipe.
    #[test]
    fn markdown_code_spans() {
        assert_eq!(md_code("foo"), "`foo`");
        assert_eq!(md_code("a`b"), "``a`b``");
        assert_eq!(md_code("`a"), "`` `a ``");
        assert_eq!(md_code("a|b"), "`a\u{2223}b`");
        assert_eq!(md_code(""), "");
    }
}
