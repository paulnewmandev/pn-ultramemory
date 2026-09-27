// SPDX-License-Identifier: Apache-2.0
//! A small markup validator for the tests: tag balance, void elements, escaping and accessibility.
//!
//! It is deliberately strict about the constructs the renderers produce and knows nothing about
//! the renderers, so it can catch mistakes in them. It checks that: the document starts with the
//! HTML5 doctype (HTML mode); tags are balanced and properly nested; void elements are never
//! closed and other elements only self-close inside `<svg>`; attribute values are quoted; text
//! and attribute values contain no raw `<` or `>` and every `&` starts a known entity; `id`
//! values are unique and every `href="#..."` or `aria-labelledby` target exists; headings never
//! skip a level and there is exactly one `<h1>`; table and list parts have valid parents; every
//! `role="img"` has an `aria-label` and a `<title>` as its first child; and no script, link,
//! image, frame or external URL is present.

use std::collections::{BTreeMap, BTreeSet};

/// Which markup dialect to validate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    /// A complete HTML5 document.
    Html,
    /// A standalone XML document such as an SVG file.
    Xml,
}

/// What the validator learned about a document.
#[derive(Debug, Default)]
pub(crate) struct Markup {
    /// How many times each element name occurs.
    pub(crate) tags: BTreeMap<String, usize>,
    /// Every `id` value.
    pub(crate) ids: Vec<String>,
    /// The levels of the headings, in document order.
    pub(crate) headings: Vec<u8>,
    /// All text nodes, concatenated.
    pub(crate) text: String,
    /// The content of the style elements.
    pub(crate) styles: Vec<String>,
    /// Every `(element, attribute, value)` triple.
    pub(crate) attributes: Vec<(String, String, String)>,
}

impl Markup {
    /// Returns how many elements have this name.
    pub(crate) fn count(&self, name: &str) -> usize {
        self.tags.get(name).copied().unwrap_or(0)
    }
}

/// Elements that never have content or an end tag.
const VOID: [&str; 14] = [
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

/// Returns `Err` when `text` contains a raw `<`, `>` or an `&` that does not start a known entity.
fn check_text(text: &str, context: &str) -> Result<(), String> {
    let mut rest = text;
    while let Some(position) = rest.find(['<', '>', '&']) {
        let found = &rest[position..];
        if !found.starts_with('&') {
            return Err(format!("raw {:?} in {context}", &found[..1]));
        }
        let end = found
            .find(';')
            .ok_or_else(|| format!("unterminated entity in {context}"))?;
        let entity = &found[..=end];
        if !matches!(
            entity,
            "&amp;" | "&lt;" | "&gt;" | "&quot;" | "&#39;" | "&nbsp;"
        ) {
            return Err(format!("unknown entity {entity} in {context}"));
        }
        rest = &found[end + 1..];
    }
    Ok(())
}

/// The attributes of a start tag, the index just after its `>`, and whether it self-closes.
type ParsedTag = (Vec<(String, String)>, usize, bool);

/// Parses the attributes of a start tag whose name has been read; returns them and the index just
/// after the closing `>`, and whether the tag self-closes.
fn parse_attributes(source: &str, mut at: usize) -> Result<ParsedTag, String> {
    let bytes = source.as_bytes();
    let mut attributes = Vec::new();
    loop {
        while at < bytes.len() && bytes[at].is_ascii_whitespace() {
            at += 1;
        }
        match bytes.get(at) {
            None => return Err("unterminated tag".into()),
            Some(b'>') => return Ok((attributes, at + 1, false)),
            Some(b'/') if bytes.get(at + 1) == Some(&b'>') => {
                return Ok((attributes, at + 2, true));
            }
            _ => {}
        }
        let start = at;
        while at < bytes.len() && !matches!(bytes[at], b'=' | b' ' | b'>' | b'/' | b'\t' | b'\n') {
            at += 1;
        }
        let name = source[start..at].to_owned();
        if name.is_empty() {
            return Err(format!(
                "bad attribute near {}",
                &source[start..(start + 20).min(source.len())]
            ));
        }
        if bytes.get(at) == Some(&b'=') {
            at += 1;
            let quote = *bytes.get(at).ok_or("missing attribute value")?;
            if quote != b'"' && quote != b'\'' {
                return Err(format!("unquoted value for {name}"));
            }
            let value_start = at + 1;
            let close = source[value_start..]
                .find(char::from(quote))
                .ok_or_else(|| format!("unterminated value for {name}"))?;
            let value = source[value_start..value_start + close].to_owned();
            check_text(&value, &format!("attribute {name}"))?;
            attributes.push((name, value));
            at = value_start + close + 1;
        } else {
            attributes.push((name, String::new()));
        }
    }
}

/// Checks that the document starts the way its dialect requires.
fn check_prologue(source: &str, mode: Mode) -> Result<(), String> {
    match mode {
        Mode::Html => {
            if !source.starts_with("<!DOCTYPE html>\n<html lang=\"") {
                return Err("missing doctype or lang attribute".into());
            }
        }
        Mode::Xml => {
            if !source.starts_with(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\"",
            ) {
                return Err("missing XML declaration or namespace".into());
            }
        }
    }
    Ok(())
}

/// Records a start tag in the statistics: element count, attributes, ids and heading levels.
fn record_start_tag(markup: &mut Markup, name: &str, attributes: &[(String, String)]) {
    *markup.tags.entry(name.to_owned()).or_insert(0) += 1;
    for (attribute, value) in attributes {
        markup
            .attributes
            .push((name.to_owned(), attribute.clone(), value.clone()));
        if attribute == "id" {
            markup.ids.push(value.clone());
        }
    }
    if let Some(level) = name
        .strip_prefix('h')
        .and_then(|digit| digit.parse::<u8>().ok())
    {
        if (1..=6).contains(&level) && name.len() == 2 {
            markup.headings.push(level);
        }
    }
}

/// Validates `source` and returns what it found.
pub(crate) fn check_markup(source: &str, mode: Mode) -> Result<Markup, String> {
    let mut markup = Markup::default();
    let mut at = 0usize;
    let mut stack: Vec<String> = Vec::new();
    check_prologue(source, mode)?;
    let mut expect_title = false;
    while at < source.len() {
        let Some(open) = source[at..].find('<').map(|p| p + at) else {
            let text = &source[at..];
            check_text(text, "trailing text")?;
            markup.text.push_str(text);
            break;
        };
        let text = &source[at..open];
        check_text(text, "text")?;
        markup.text.push_str(text);
        let rest = &source[open..];
        if rest.starts_with("<!--") {
            let end = rest.find("-->").ok_or("unterminated comment")?;
            at = open + end + 3;
            continue;
        }
        if rest.starts_with("<!DOCTYPE") {
            at = open + rest.find('>').ok_or("unterminated doctype")? + 1;
            continue;
        }
        if rest.starts_with("<?") {
            at = open + rest.find("?>").ok_or("unterminated declaration")? + 2;
            continue;
        }
        if let Some(closing) = rest.strip_prefix("</") {
            let end = closing.find('>').ok_or("unterminated end tag")?;
            let name = closing[..end].trim().to_ascii_lowercase();
            match stack.pop() {
                Some(expected) if expected == name => {}
                other => return Err(format!("end tag </{name}> does not match {other:?}")),
            }
            at = open + 2 + end + 1;
            continue;
        }
        let name_len = rest[1..]
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == ':'))
            .ok_or("unterminated tag")?;
        if name_len == 0 {
            return Err(format!("stray '<' near {:?}", &rest[..rest.len().min(30)]));
        }
        let name = rest[1..=name_len].to_ascii_lowercase();
        let (attributes, after, self_closing) = parse_attributes(source, open + 1 + name_len)?;
        if expect_title && name != "title" {
            return Err(format!(
                "role=img element must start with <title>, found <{name}>"
            ));
        }
        expect_title = false;
        check_parent(&name, &stack)?;
        record_start_tag(&mut markup, &name, &attributes);
        let attribute = |wanted: &str| {
            attributes
                .iter()
                .find(|(n, _)| n == wanted)
                .map(|(_, v)| v.as_str())
        };
        if attribute("role") == Some("img") {
            if attribute("aria-label").is_none_or(str::is_empty) {
                return Err("role=img without aria-label".into());
            }
            expect_title = true;
        }
        if name == "th" && attribute("scope").is_none() {
            return Err("<th> without scope".into());
        }
        at = after;
        if VOID.contains(&name.as_str()) {
            if self_closing && mode == Mode::Html && !stack.iter().any(|open| open == "svg") {
                // `<meta ... />` is legal HTML5, but the renderers never write it.
                return Err(format!("void element <{name}> written as self-closing"));
            }
            continue;
        }
        if self_closing {
            if mode == Mode::Html && !stack.iter().any(|open| open == "svg") {
                return Err(format!("<{name}/> self-closes outside svg"));
            }
            continue;
        }
        if name == "style" {
            let end = source[at..].find("</style>").ok_or("unterminated style")?;
            markup.styles.push(source[at..at + end].to_owned());
            at += end + "</style>".len();
            continue;
        }
        stack.push(name);
    }
    if !stack.is_empty() {
        return Err(format!("unclosed elements: {stack:?}"));
    }
    verify_references(&markup, mode)?;
    Ok(markup)
}

/// Checks that table, list and definition parts sit inside valid parents.
fn check_parent(name: &str, stack: &[String]) -> Result<(), String> {
    let parent = stack.last().map(String::as_str);
    let valid = match name {
        "li" => matches!(parent, Some("ul" | "ol")),
        "tr" => matches!(parent, Some("thead" | "tbody" | "tfoot")),
        "td" | "th" => parent == Some("tr"),
        "thead" | "tbody" | "tfoot" | "caption" => parent == Some("table"),
        "dt" | "dd" => matches!(parent, Some("dl" | "div")),
        "figcaption" => parent == Some("figure"),
        _ => true,
    };
    let allowed_child = match parent {
        Some("ul" | "ol") => name == "li",
        Some("table") => matches!(name, "caption" | "thead" | "tbody" | "tfoot" | "colgroup"),
        Some("thead" | "tbody" | "tfoot") => name == "tr",
        Some("tr") => matches!(name, "td" | "th"),
        _ => true,
    };
    if valid && allowed_child {
        Ok(())
    } else {
        Err(format!("<{name}> inside <{parent:?}>"))
    }
}

/// Checks ids, links, headings and forbidden elements.
fn verify_references(markup: &Markup, mode: Mode) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for id in &markup.ids {
        if !seen.insert(id.as_str()) {
            return Err(format!("duplicate id {id}"));
        }
    }
    for (element, attribute, value) in &markup.attributes {
        match attribute.as_str() {
            "href" if mode == Mode::Html => {
                let target = value
                    .strip_prefix('#')
                    .ok_or_else(|| format!("external href {value} on <{element}>"))?;
                if !seen.contains(target) {
                    return Err(format!("href to missing id {target}"));
                }
            }
            "aria-labelledby" => {
                if !seen.contains(value.as_str()) {
                    return Err(format!("aria-labelledby to missing id {value}"));
                }
            }
            "src" | "srcset" | "action" | "formaction" | "data" | "poster" | "xlink:href" => {
                return Err(format!("resource attribute {attribute} on <{element}>"));
            }
            name if name.starts_with("on")
                && name.len() > 2
                && name[2..].chars().all(|c| c.is_ascii_lowercase()) =>
            {
                return Err(format!("event handler {name} on <{element}>"));
            }
            _ => {}
        }
    }
    for forbidden in [
        "script",
        "link",
        "img",
        "iframe",
        "object",
        "embed",
        "form",
        "video",
        "audio",
        "use",
        "image",
        "foreignobject",
    ] {
        if markup.count(forbidden) > 0 {
            return Err(format!("forbidden element <{forbidden}>"));
        }
    }
    for style in &markup.styles {
        for forbidden in [
            "@import",
            "url(",
            "expression(",
            "javascript:",
            "@font-face",
        ] {
            if style.contains(forbidden) {
                return Err(format!("forbidden {forbidden} in style"));
            }
        }
    }
    if mode == Mode::Html {
        if markup.headings.first() != Some(&1)
            || markup
                .headings
                .iter()
                .copied()
                .filter(|level| *level == 1)
                .count()
                != 1
        {
            return Err(format!(
                "exactly one h1 first expected, got {:?}",
                markup.headings
            ));
        }
        for pair in markup.headings.windows(2) {
            if pair[1] > pair[0] + 1 {
                return Err(format!("heading level skipped: {pair:?}"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Mode, check_markup};

    /// Wraps a body in a minimal valid document.
    fn page(body: &str) -> String {
        format!(
            "<!DOCTYPE html>\n<html lang=\"en\"><head><title>t</title></head><body><h1>x</h1>{body}</body></html>"
        )
    }

    /// The validator accepts a valid page and rejects each class of mistake it is meant to catch.
    #[test]
    fn validator_catches_mistakes() {
        assert!(check_markup(&page("<p>fine &amp; good</p>"), Mode::Html).is_ok());
        let bad = [
            "<p>unclosed",
            "<p></div>",
            "<p>a < b</p>",
            "<p>a > b</p>",
            "<p>a & b</p>",
            "<p>&bogus;</p>",
            "<script>x</script>",
            "<a href=\"http://x\">y</a>",
            "<a href=\"#missing\">y</a>",
            "<p id=\"a\"></p><p id=\"a\"></p>",
            "<br></br>",
            "<div/>",
            "<p class=unquoted>x</p>",
            "<h3>skipped</h3>",
            "<h1>second</h1>",
            "<table><tr><td>x</td></tr></table>",
            "<table><tbody><tr><th>x</th></tr></tbody></table>",
            "<ul><p>x</p><li>y</li></ul>",
            "<svg role=\"img\"><g/></svg>",
            "<svg role=\"img\" aria-label=\"a\"><g/></svg>",
            "<p onclick=\"x()\">y</p>",
            "<img alt=\"\">",
        ];
        for body in bad {
            assert!(
                check_markup(&page(body), Mode::Html).is_err(),
                "should reject {body}"
            );
        }
        assert!(check_markup("<html>", Mode::Html).is_err());
    }

    /// Valid inline SVG, tables and lists pass.
    #[test]
    fn validator_accepts_valid_constructs() {
        let body = "<svg role=\"img\" aria-label=\"a\"><title>a</title><rect/><path d=\"M0 0\"/></svg>\
            <table><caption>c</caption><thead><tr><th scope=\"col\">h</th></tr></thead><tbody><tr><td>d</td></tr></tbody></table>\
            <ul><li>x</li></ul><dl><div><dt>a</dt><dd>b</dd></div></dl><h2 id=\"s\">s</h2><a href=\"#s\">go</a><br><style>p{color:red}</style>";
        let markup = check_markup(&page(body), Mode::Html).expect("valid");
        assert_eq!(markup.count("rect"), 1);
        assert_eq!(markup.styles, ["p{color:red}"]);
    }

    /// XML mode requires the declaration and namespace and allows self-closing tags anywhere.
    #[test]
    fn xml_mode() {
        let svg = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\" role=\"img\" aria-label=\"a\"><title>a</title><g><rect/></g></svg>\n";
        assert!(check_markup(svg, Mode::Xml).is_ok());
        assert!(check_markup(&svg.replace("</g>", ""), Mode::Xml).is_err());
        assert!(check_markup("<svg></svg>", Mode::Xml).is_err());
    }
}
