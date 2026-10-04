// SPDX-License-Identifier: Apache-2.0
//! The brain view: one self-contained HTML page that draws a repository as a brain of particles.
//!
//! Every symbol is a particle, every relationship a fibre, every memory a beacon over the code it
//! describes; a person flies through it, searches it and opens any particle to see its signature,
//! what calls it, what it calls and what was decided about it.
//!
//! # What the page may do
//! Unlike the report, this page runs a script: it is an interactive scene, and no static drawing
//! of thousands of particles is readable. What it may *not* do is reach anything. Its
//! `Content-Security-Policy` allows only the inline script and styles it carries, and names no
//! source for connections, images, fonts or frames, so the browser itself refuses any request the
//! page could make. The repository's data stays in the file.
//!
//! # Safety
//! The data arrives as JSON text and is embedded in a `<script type="application/json">` block.
//! Every `<`, `>` and `&` in it is written as a JSON unicode escape, so no text from the
//! repository can close the block or open a tag, and the script writes every string it shows with
//! `textContent`.

use crate::lang::Lang;

/// The style sheet of the page.
const CSS: &str = include_str!("brain.css");

/// The script that draws and drives the scene.
const SCRIPT: &str = include_str!("brain.js");

/// The policy that keeps the page from reaching anything: inline script and styles only, and no
/// source at all for connections, images, fonts, frames or forms.
const POLICY: &str = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; \
                      img-src data:; base-uri 'none'; form-action 'none'";

/// JSON text made safe to sit inside an HTML script block.
///
/// `<`, `>` and `&` only ever appear inside JSON strings, where a unicode escape means the same
/// character, so the value is unchanged and the HTML parser sees no markup.
fn embeddable(json: &str) -> String {
    let mut out = String::with_capacity(json.len() + json.len() / 32);
    for c in json.chars() {
        match c {
            '<' => out.push_str("\\u003c"),
            '>' => out.push_str("\\u003e"),
            '&' => out.push_str("\\u0026"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            other => out.push(other),
        }
    }
    out
}

/// Text made safe to sit between HTML tags.
fn text(raw: &str) -> String {
    raw.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Renders the brain view for the data the engine built, as one HTML document.
///
/// `data` is the JSON of `pn_ultramemory_engine::Brain::to_value`; `title` names the repository.
/// The page is the same for any data, so a malformed `data` gives a page that says the index is
/// empty rather than an error.
pub(crate) fn render(data: &str, title: &str, lang: Lang) -> String {
    let code = match lang {
        Lang::En => "en",
        Lang::Es => "es",
    };
    let title = text(title);
    let mut page = String::with_capacity(CSS.len() + SCRIPT.len() + data.len() + 2048);
    page.push_str("<!doctype html>\n<html lang=\"");
    page.push_str(code);
    page.push_str("\">\n<head>\n<meta charset=\"utf-8\">\n");
    page.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    page.push_str("<meta http-equiv=\"Content-Security-Policy\" content=\"");
    page.push_str(POLICY);
    page.push_str("\">\n<meta name=\"referrer\" content=\"no-referrer\">\n<title>");
    page.push_str(&title);
    page.push_str(" · brain</title>\n<style>\n");
    page.push_str(CSS);
    page.push_str("</style>\n</head>\n<body>\n");
    page.push_str(concat!(
        "<canvas id=\"scene\" aria-label=\"brain\"></canvas>\n",
        "<div id=\"vignette\"></div>\n",
        "<div id=\"labels\"></div>\n",
        "<button id=\"menu\" class=\"glass\" aria-label=\"menu\">&#9776;</button>\n",
        "<header id=\"brand\" class=\"glass\"><div class=\"mark\"><i></i>pn-ultramemory · brain</div>",
        "<div id=\"repo\" class=\"repo\"></div><div id=\"stats\" class=\"stats\"></div></header>\n",
        "<div id=\"searchbox\" class=\"glass\"><span class=\"icon\">&#9906;</span>",
        "<input id=\"search\" type=\"search\" autocomplete=\"off\" spellcheck=\"false\">",
        "<kbd>/</kbd><ul id=\"results\"></ul><ul id=\"history\"></ul></div>\n",
        "<aside id=\"legend\" class=\"glass\"><h3 id=\"regions-title\"></h3><ul id=\"regions\"></ul>",
        "<h3 id=\"view-title\"></h3><div id=\"toggles\"></div><p id=\"legend-note\" class=\"note\"></p></aside>\n",
        "<aside id=\"panel\" class=\"glass\"></aside>\n",
        "<div id=\"note\" class=\"glass\"></div>\n",
        "<div id=\"hint\" class=\"glass\"></div>\n",
        "<div id=\"toast\" class=\"glass\"></div>\n",
        "<div id=\"message\" class=\"glass\"></div>\n",
    ));
    page.push_str("<script id=\"brain-data\" type=\"application/json\">");
    page.push_str(&embeddable(data));
    page.push_str("</script>\n<script>\n");
    page.push_str(SCRIPT);
    page.push_str("</script>\n</body>\n</html>\n");
    page
}

#[cfg(test)]
mod tests {
    use super::{embeddable, render};
    use crate::lang::Lang;

    /// Text from the repository cannot close the data block or open a tag.
    #[test]
    fn data_cannot_break_out_of_its_block() {
        let data = r#"{"repo":"</script><script>alert(1)</script>","nodes":[]}"#;
        let page = render(data, "<b>x</b>", Lang::En);
        assert!(!page.contains("</script><script>alert"));
        assert!(page.contains("\\u003c/script\\u003e"));
        assert!(page.contains("<title>&lt;b&gt;x&lt;/b&gt; · brain</title>"));
    }

    /// The escaped text still parses as the same JSON value.
    #[test]
    fn escaping_keeps_the_value() {
        let raw = "{\"a\":\"<&>\u{2028}\"}";
        let safe = embeddable(raw);
        assert!(!safe.contains('<') && !safe.contains('>') && !safe.contains('&'));
        assert_eq!(safe, "{\"a\":\"\\u003c\\u0026\\u003e\\u2028\"}");
    }

    /// The page forbids every request and names its language.
    #[test]
    fn the_page_is_sealed_and_labelled() {
        let page = render("{}", "demo", Lang::Es);
        assert!(page.starts_with("<!doctype html>\n<html lang=\"es\">"));
        assert!(page.contains("default-src 'none'"));
        assert!(!page.contains("connect-src"));
        assert!(!page.contains("http://") && !page.contains("https://"));
    }
}
