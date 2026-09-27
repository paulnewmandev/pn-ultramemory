// SPDX-License-Identifier: Apache-2.0
//! Insertion of documentation comments into source text, in each language's own syntax.
//!
//! # Role in the architecture
//! [`DocComments`] is the exit adapter that implements the [`DocInserter`] port of
//! `pn-ultramemory-core`. It reuses the extraction walker to find the declaration, so "already
//! documented" means exactly what the extractor reports as documentation, and it re-parses the
//! result so that a comment can never break the code it documents.
//!
//! # Comment styles
//! | Language | Style |
//! |---|---|
//! | Rust | `///` lines |
//! | Go | `//` lines, the text is only prefixed |
//! | JavaScript, TypeScript, TSX, Java, C, C++, PHP | `/** ... */` block with ` * ` lines; `*/` in the text is escaped |
//! | C# | `///` lines with the text inside `<summary>`, XML characters escaped |
//! | Ruby | `#` lines |
//! | Python | a `"""` docstring as the first statement of the body |
//!
//! # Invariants
//! * The comment sits at the declaration's indentation and above any decorators or attributes
//!   attached to it.
//! * Line endings are preserved: a file that uses CRLF gets CRLF in the inserted lines.
//! * On any refusal the caller keeps its original text: the function returns an error instead
//!   of a partly modified source, and it refuses when the number of syntax errors would grow.

use pn_ultramemory_core::{DocError, DocInserter, DocTarget, ExtractError, Language, SymbolKind};
use tree_sitter::Node;

use crate::extract::{SymbolExtra, choose_language, count_errors, extract_detailed, parse};

/// Inserts documentation comments in the syntax of each language.
///
/// # Examples
/// ```
/// use pn_ultramemory_core::{DocInserter, DocTarget, Language, SymbolKind};
/// use pn_ultramemory_index::DocComments;
///
/// let source = "fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n";
/// let target = DocTarget { name: "add", kind: SymbolKind::Function, line: 1 };
/// let documented = DocComments
///     .insert_doc(Language::Rust, source, &target, "Adds two numbers.")
///     .unwrap();
/// assert!(documented.starts_with("/// Adds two numbers.\nfn add"));
/// ```
#[derive(Debug, Default, Clone, Copy)]
pub struct DocComments;

/// The comment syntax used for a language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Style {
    /// `/// text` lines.
    Rust,
    /// `// text` lines.
    Go,
    /// `/** ... */` block.
    Block,
    /// `/// <summary>` lines.
    CSharp,
    /// `# text` lines.
    Ruby,
    /// A docstring inside the body.
    Python,
}

/// Returns the comment style of a language, or `None` when it has no inserter.
fn style_of(language: Language) -> Option<Style> {
    match language {
        Language::Rust => Some(Style::Rust),
        Language::Go => Some(Style::Go),
        Language::JavaScript
        | Language::TypeScript
        | Language::Tsx
        | Language::Java
        | Language::C
        | Language::Cpp
        | Language::Php => Some(Style::Block),
        Language::CSharp => Some(Style::CSharp),
        Language::Ruby => Some(Style::Ruby),
        Language::Python => Some(Style::Python),
        Language::Other(_) => None,
    }
}

/// Returns the line terminator a source uses.
fn line_ending(source: &str) -> &'static str {
    if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

/// Splits documentation text into lines: trailing whitespace is removed and blank lines at
/// both ends are dropped.
fn text_lines(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = text.lines().map(|l| l.trim_end().to_owned()).collect();
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    let skip = lines.iter().take_while(|l| l.is_empty()).count();
    lines.drain(..skip);
    lines
}

/// Prefixes each line with `marker` and a space; blank lines get the bare marker.
fn prefixed(indent: &str, marker: &str, lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .map(|l| {
            if l.is_empty() {
                format!("{indent}{marker}")
            } else {
                format!("{indent}{marker} {l}")
            }
        })
        .collect()
}

/// Escapes the characters that are special in an XML documentation comment.
fn escape_xml(line: &str) -> String {
    line.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Renders the documentation as the lines to insert, at the given indentation.
fn render(style: Style, indent: &str, lines: &[String]) -> Vec<String> {
    match style {
        Style::Rust => prefixed(indent, "///", lines),
        Style::Go => prefixed(indent, "//", lines),
        Style::Ruby => prefixed(indent, "#", lines),
        Style::CSharp => {
            let escaped: Vec<String> = lines.iter().map(|l| escape_xml(l)).collect();
            let mut out = vec![format!("{indent}/// <summary>")];
            out.extend(prefixed(indent, "///", &escaped));
            out.push(format!("{indent}/// </summary>"));
            out
        }
        Style::Block => {
            let escaped: Vec<String> = lines.iter().map(|l| l.replace("*/", "*\\/")).collect();
            let mut out = vec![format!("{indent}/**")];
            out.extend(prefixed(indent, " *", &escaped));
            out.push(format!("{indent} */"));
            out
        }
        Style::Python => docstring_lines(indent, lines),
    }
}

/// Builds the docstring lines for Python at the given indentation.
fn docstring_lines(indent: &str, lines: &[String]) -> Vec<String> {
    let escaped: Vec<String> = lines
        .iter()
        .map(|l| l.replace('\\', "\\\\").replace("\"\"\"", "\\\"\\\"\\\""))
        .collect();
    if let [only] = escaped.as_slice() {
        let body = match only.strip_suffix('"') {
            Some(head) => format!("{head}\\\""),
            None => only.clone(),
        };
        return vec![format!("{indent}\"\"\"{body}\"\"\"")];
    }
    let mut out = Vec::with_capacity(escaped.len() + 1);
    for (i, line) in escaped.iter().enumerate() {
        let text = match (i, line.is_empty()) {
            (0, _) => format!("{indent}\"\"\"{line}"),
            (_, true) => String::new(),
            (_, false) => format!("{indent}{line}"),
        };
        out.push(text);
    }
    out.push(format!("{indent}\"\"\""));
    out
}

/// Where and how to insert: the byte offset of the start of a line and the indentation.
struct Insertion {
    /// Byte offset of the start of the line that the comment goes before.
    at: usize,
    /// The indentation of that line.
    indent: String,
}

/// Finds the insertion point that puts a comment before the line holding `byte`.
///
/// # Errors
/// [`DocError::Invalid`] when something other than whitespace comes before `byte` on its line.
fn insertion_before(source: &str, byte: usize) -> Result<Insertion, DocError> {
    let at = source
        .get(..byte)
        .map_or(0, |s| s.rfind('\n').map_or(0, |i| i + 1));
    let prefix = source.get(at..byte).unwrap_or("");
    if !prefix.chars().all(|c| c == ' ' || c == '\t') {
        return Err(DocError::Invalid(
            "the declaration does not start its line".to_owned(),
        ));
    }
    Ok(Insertion {
        at,
        indent: prefix.to_owned(),
    })
}

/// Finds the symbol that `target` refers to.
fn find_symbol(
    symbols: &[pn_ultramemory_core::SymbolDraft],
    extras: &[SymbolExtra],
    target: &DocTarget<'_>,
) -> Option<usize> {
    let mut best: Option<(usize, bool)> = None;
    for (index, (symbol, extra)) in symbols.iter().zip(extras).enumerate() {
        if symbol.name != target.name {
            continue;
        }
        let on_line = [
            extra.decl_line,
            extra.name_line,
            extra.attach_line,
            symbol.span.start_line,
        ]
        .contains(&target.line);
        if !on_line {
            continue;
        }
        let same_kind = symbol.kind == target.kind || kinds_compatible(symbol.kind, target.kind);
        if best.is_none_or(|(_, best_kind)| same_kind && !best_kind) {
            best = Some((index, same_kind));
        }
    }
    best.map(|(index, _)| index)
}

/// Returns `true` when two kinds can describe the same declaration (a method may be reported
/// as a function by a caller that does not know its parent).
fn kinds_compatible(a: SymbolKind, b: SymbolKind) -> bool {
    matches!(
        (a, b),
        (SymbolKind::Function, SymbolKind::Method) | (SymbolKind::Method, SymbolKind::Function)
    )
}

/// Finds the declaration node of a Python symbol from the byte range recorded by the extractor.
fn python_definition<'t>(root: Node<'t>, extra: &SymbolExtra) -> Option<Node<'t>> {
    let mut node = root.descendant_for_byte_range(extra.core_start, extra.core_end)?;
    loop {
        if node.start_byte() == extra.core_start
            && node.end_byte() == extra.core_end
            && matches!(node.kind(), "function_definition" | "class_definition")
        {
            return Some(node);
        }
        node = node.parent()?;
    }
}

/// Plans the insertion of a docstring: the point before the first statement of the body.
///
/// # Errors
/// [`DocError::Invalid`] when the body starts on the header line or cannot be found.
fn python_insertion(
    source: &str,
    tree: &tree_sitter::Tree,
    extra: &SymbolExtra,
) -> Result<Insertion, DocError> {
    let definition = python_definition(tree.root_node(), extra).ok_or_else(|| {
        DocError::Invalid("only functions and classes can hold a docstring".to_owned())
    })?;
    let body = definition
        .child_by_field_name("body")
        .ok_or_else(|| DocError::Invalid("the declaration has no body".to_owned()))?;
    if let Some(colon) = body.prev_sibling() {
        if colon.end_position().row == body.start_position().row {
            return Err(DocError::Invalid(
                "the body starts on the same line as the header".to_owned(),
            ));
        }
    }
    let mut cursor = body.walk();
    let first = body
        .named_children(&mut cursor)
        .find(|c| c.kind() != "comment")
        .ok_or_else(|| DocError::Invalid("the body is empty".to_owned()))?;
    insertion_before(source, first.start_byte())
}

impl DocInserter for DocComments {
    fn insert_doc(
        &self,
        language: Language,
        source: &str,
        target: &DocTarget<'_>,
        text: &str,
    ) -> Result<String, DocError> {
        let style = style_of(language).ok_or(DocError::Unsupported(language))?;
        let language = choose_language(language, source);
        let lines = text_lines(text);
        if lines.is_empty() {
            return Err(DocError::Invalid(
                "the documentation text is empty".to_owned(),
            ));
        }
        let (file, extras, tree) = extract_detailed(language, source).map_err(|e| match e {
            ExtractError::Unsupported(l) => DocError::Unsupported(l),
            ExtractError::Parse(reason) => DocError::Invalid(reason),
        })?;
        let index = find_symbol(&file.symbols, &extras, target).ok_or(DocError::TargetNotFound)?;
        let extra = extras[index];
        if extra.has_doc {
            return Err(DocError::Invalid(
                "the declaration already has documentation".to_owned(),
            ));
        }
        let insertion = if style == Style::Python {
            python_insertion(source, &tree, &extra)?
        } else {
            insertion_before(source, extra.attach_start)?
        };
        let eol = line_ending(source);
        let block = render(style, &insertion.indent, &lines);
        let mut result = String::with_capacity(source.len() + text.len() + 64);
        result.push_str(&source[..insertion.at]);
        for line in &block {
            result.push_str(line);
            result.push_str(eol);
        }
        result.push_str(&source[insertion.at..]);
        let after = parse(language, &result).map_err(|e| DocError::Invalid(e.to_string()))?;
        if count_errors(&after) > count_errors(&tree) {
            return Err(DocError::Invalid(
                "the inserted comment would introduce syntax errors".to_owned(),
            ));
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use pn_ultramemory_core::{DocError, SymbolKind};

    use super::{
        Style, docstring_lines, escape_xml, insertion_before, kinds_compatible, line_ending,
        render, style_of, text_lines,
    };
    use pn_ultramemory_core::Language;

    /// Converts string literals to owned lines.
    fn lines(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    /// Blank lines at both ends and trailing whitespace are removed; inner blanks are kept.
    #[test]
    fn text_is_split_and_trimmed() {
        assert_eq!(text_lines("\n\n  a  \n\nb\t\n\n"), ["  a", "", "b"]);
        assert!(text_lines(" \n \t\n").is_empty());
        assert_eq!(text_lines("one\r\ntwo\r\n"), ["one", "two"]);
    }

    /// Every language has a style except the fallback ones.
    #[test]
    fn styles_by_language() {
        for language in Language::GRAMMAR_BACKED {
            assert!(style_of(language).is_some(), "{language}");
        }
        assert_eq!(style_of(Language::Other("lua")), None);
        assert_eq!(style_of(Language::Rust), Some(Style::Rust));
        assert_eq!(style_of(Language::Tsx), Some(Style::Block));
    }

    /// Each style renders its own markers, with the indentation on every line.
    #[test]
    fn rendering() {
        let text = lines(&["First.", "", "Second."]);
        assert_eq!(
            render(Style::Rust, "  ", &text),
            ["  /// First.", "  ///", "  /// Second."]
        );
        assert_eq!(
            render(Style::Go, "", &text),
            ["// First.", "//", "// Second."]
        );
        assert_eq!(
            render(Style::Ruby, "\t", &text),
            ["\t# First.", "\t#", "\t# Second."]
        );
        assert_eq!(
            render(Style::Block, " ", &text),
            [" /**", "  * First.", "  *", "  * Second.", "  */"]
        );
        assert_eq!(
            render(Style::CSharp, "", &lines(&["a < b & c"])),
            ["/// <summary>", "/// a &lt; b &amp; c", "/// </summary>"]
        );
        assert_eq!(
            render(Style::Block, "", &lines(&["a */ b"]))[1],
            " * a *\\/ b"
        );
    }

    /// Docstrings are single-line when possible and escape quotes and backslashes.
    #[test]
    fn docstring_rendering() {
        assert_eq!(
            docstring_lines("    ", &lines(&["One."])),
            ["    \"\"\"One.\"\"\""]
        );
        assert_eq!(
            docstring_lines("  ", &lines(&["One.", "", "Two."])),
            ["  \"\"\"One.", "", "  Two.", "  \"\"\""]
        );
        assert_eq!(
            docstring_lines("", &lines(&["a\\b"])),
            ["\"\"\"a\\\\b\"\"\""]
        );
        assert_eq!(
            docstring_lines("", &lines(&["say \"hi\""])),
            ["\"\"\"say \"hi\\\"\"\"\""]
        );
        assert_eq!(
            docstring_lines("", &lines(&["x \"\"\" y"])),
            ["\"\"\"x \\\"\\\"\\\" y\"\"\""]
        );
    }

    /// XML escaping covers the three characters that matter.
    #[test]
    fn xml_escaping() {
        assert_eq!(escape_xml("<a & b>"), "&lt;a &amp; b&gt;");
        assert_eq!(escape_xml("plain"), "plain");
    }

    /// The line ending of a file is CRLF as soon as it contains one.
    #[test]
    fn line_endings() {
        assert_eq!(line_ending("a\nb\n"), "\n");
        assert_eq!(line_ending("a\r\nb\r\n"), "\r\n");
        assert_eq!(line_ending(""), "\n");
    }

    /// The insertion point is the start of the line, and only whitespace may precede the byte.
    #[test]
    fn insertion_points() {
        let source = "fn a() {}\n    fn b() {}\n\tfn c() {} fn d() {}\n";
        let b = insertion_before(source, source.find("fn b").unwrap()).unwrap();
        assert_eq!((b.at, b.indent.as_str()), (10, "    "));
        let c = insertion_before(source, source.find("fn c").unwrap()).unwrap();
        assert_eq!(c.indent, "\t");
        let d = insertion_before(source, source.find("fn d").unwrap());
        assert!(matches!(d, Err(DocError::Invalid(_))));
        let first = insertion_before(source, 0).unwrap();
        assert_eq!((first.at, first.indent.as_str()), (0, ""));
    }

    /// A function and a method may stand for each other; other kinds may not.
    #[test]
    fn compatible_kinds() {
        assert!(kinds_compatible(SymbolKind::Function, SymbolKind::Method));
        assert!(kinds_compatible(SymbolKind::Method, SymbolKind::Function));
        assert!(!kinds_compatible(SymbolKind::Class, SymbolKind::Function));
    }
}
