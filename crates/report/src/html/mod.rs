// SPDX-License-Identifier: Apache-2.0
//! The self-contained HTML report.
//!
//! [`render`] assembles one HTML5 document: inline style sheet, inline SVG charts and graph, no
//! script and no external resource of any kind, guarded by a `Content-Security-Policy` that
//! forbids everything except inline styles and `data:` images. The page adapts to the light or
//! dark preference of the viewer, prints one section per page, scrolls tables horizontally on
//! small screens, and is navigable by headings, landmarks and a skip link.
//!
//! Invariants: all user-provided text is escaped; the document is well formed (balanced tags,
//! void elements only where allowed); the output is deterministic and depends on nothing but its
//! input.

mod css;
mod sections;
mod table;

use crate::content::{project_name, tool_version};
use crate::i18n::{Key, t, tf};
use crate::lang::Lang;
use crate::model::ReportData;
use crate::text::{escaped, push_escaped};

/// The Content-Security-Policy of the page: only inline styles and `data:` images.
pub(crate) const CSP: &str = "default-src 'none'; style-src 'unsafe-inline'; img-src data:";

/// Renders the report as one self-contained HTML document.
pub(crate) fn render(data: &ReportData, lang: Lang) -> String {
    let project = project_name(data, lang);
    let version = tool_version(data);
    let title = t(lang, Key::DocTitle);

    let mut out = String::with_capacity(24_000);
    out.push_str("<!DOCTYPE html>\n<html lang=\"");
    out.push_str(lang.code());
    out.push_str("\">\n<head>\n<meta charset=\"utf-8\">\n");
    out.push_str("<meta http-equiv=\"Content-Security-Policy\" content=\"");
    out.push_str(CSP);
    out.push_str("\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    out.push_str("<meta name=\"color-scheme\" content=\"light dark\">\n");
    out.push_str("<meta name=\"generator\" content=\"pn-ultramemory ");
    push_escaped(&mut out, &version);
    out.push_str("\">\n<title>");
    push_escaped(&mut out, &project);
    out.push_str(" - ");
    push_escaped(&mut out, title);
    out.push_str("</title>\n<style>");
    out.push_str(&css::stylesheet());
    out.push_str("</style>\n</head>\n<body>\n");
    out.push_str("<a class=\"skip\" href=\"#main\">");
    push_escaped(&mut out, t(lang, Key::SkipToContent));
    out.push_str("</a>\n<div class=\"page\">\n");

    title_block(&mut out, data, lang, &project, &version);
    navigation(&mut out, data, lang);

    out.push_str("<main id=\"main\">\n");
    sections::summary(&mut out, data, lang);
    sections::how_to_read(&mut out, lang);
    sections::languages(&mut out, data, lang);
    sections::modules(&mut out, data, lang);
    sections::hotspots(&mut out, data, lang);
    sections::documentation(&mut out, data, lang);
    sections::code_graph(&mut out, data, lang);
    sections::memories(&mut out, data, lang);
    sections::usage(&mut out, data, lang);
    sections::notes(&mut out, data, lang);
    out.push_str("\n</main>\n<footer><p>");
    push_escaped(&mut out, &tf(lang, Key::Footer, &[&version]));
    out.push_str("</p></footer>\n</div>\n</body>\n</html>\n");
    out
}

/// Writes the title block: heading, project, date and tool version.
fn title_block(out: &mut String, data: &ReportData, lang: Lang, project: &str, version: &str) {
    out.push_str("<header class=\"hero\">\n<p class=\"eyebrow\">pn-ultramemory</p>\n<h1>");
    push_escaped(out, t(lang, Key::DocTitle));
    out.push_str("</h1>\n<p class=\"project\"><bdi>");
    push_escaped(out, project);
    out.push_str("</bdi></p>\n<dl class=\"meta\">");
    let generated = crate::text::single_line(&data.generated_on, 40);
    if !generated.is_empty() {
        out.push_str("<div><dt>");
        push_escaped(out, t(lang, Key::GeneratedOn));
        out.push_str("</dt><dd>");
        push_escaped(out, &generated);
        out.push_str("</dd></div>");
    }
    out.push_str("<div><dt>");
    push_escaped(out, t(lang, Key::ToolVersion));
    out.push_str("</dt><dd>pn-ultramemory ");
    push_escaped(out, version);
    out.push_str("</dd></div></dl>\n</header>\n");
}

/// Writes the table of contents as a row of links to the sections that will be present.
fn navigation(out: &mut String, data: &ReportData, lang: Lang) {
    let mut entries = vec![
        ("summary", Key::SecSummary),
        ("howto", Key::SecHowTo),
        ("languages", Key::SecLanguages),
        ("modules", Key::SecModules),
        ("hotspots", Key::SecHotspots),
        ("documentation", Key::SecDocs),
        ("graph", Key::SecGraph),
        ("memories", Key::SecMemories),
    ];
    if data.usage.is_some() {
        entries.push(("usage", Key::SecUsage));
    }
    if !data.notes.is_empty() {
        entries.push(("notes", Key::SecNotes));
    }
    out.push_str("<nav class=\"toc\" aria-label=\"");
    out.push_str(&escaped(t(lang, Key::Contents)));
    out.push_str("\"><ol>");
    for (id, key) in entries {
        out.push_str("<li><a href=\"#");
        out.push_str(id);
        out.push_str("\">");
        push_escaped(out, t(lang, key));
        out.push_str("</a></li>");
    }
    out.push_str("</ol></nav>\n");
}

#[cfg(test)]
mod tests {
    use super::{CSP, render};
    use crate::lang::Lang;
    use crate::model::{ReportData, Summary, Usage};

    /// A minimal report renders the document frame, the policy and the language attribute.
    #[test]
    fn document_frame() {
        let data = ReportData {
            project: "demo".into(),
            generated_on: "2026-09-25".into(),
            tool_version: "1.2.36".into(),
            summary: Summary {
                files: 1234,
                ..Summary::default()
            },
            ..ReportData::default()
        };
        let en = render(&data, Lang::En);
        assert!(en.starts_with("<!DOCTYPE html>\n<html lang=\"en\">"));
        assert!(en.contains(&format!("content=\"{CSP}\"")));
        assert!(en.contains("<title>demo - Code report</title>"));
        assert!(en.contains("<dd>1,234</dd>"));
        assert!(
            en.contains("Generated locally by pn-ultramemory 1.2.36. No data left this machine.")
        );
        let es = render(&data, Lang::Es);
        assert!(es.contains("<html lang=\"es\">"));
        assert!(es.contains("<title>demo - Informe de código</title>"));
        assert!(es.contains("<dd>1.234</dd>"));
        assert!(es.contains(
            "Generado localmente por pn-ultramemory 1.2.36. Ningún dato salió de esta máquina."
        ));
    }

    /// The usage and notes sections appear only when there is something to show.
    #[test]
    fn optional_sections() {
        let mut data = ReportData::default();
        let plain = render(&data, Lang::En);
        assert!(!plain.contains("id=\"usage\""));
        assert!(!plain.contains("id=\"notes\""));
        data.usage = Some(Usage {
            recalls: 7,
            avg_budget_percent: 42,
            ..Usage::default()
        });
        data.notes = vec!["remember me".into()];
        let full = render(&data, Lang::En);
        assert!(full.contains("id=\"usage\""));
        assert!(full.contains("nothing is sent anywhere"));
        assert!(full.contains("<a href=\"#notes\">Notes</a>"));
        assert!(full.contains("remember me"));
    }

    /// An empty report renders every fixed section with a "no data" message.
    #[test]
    fn empty_report_renders() {
        let html = render(&ReportData::default(), Lang::Es);
        for id in [
            "summary",
            "howto",
            "languages",
            "modules",
            "hotspots",
            "documentation",
            "graph",
            "memories",
        ] {
            assert!(html.contains(&format!("id=\"{id}\"")), "{id}");
        }
        assert!(html.contains("Sin datos."));
        assert!(html.contains("(proyecto sin nombre)"));
    }

    /// The same input gives the same bytes.
    #[test]
    fn render_is_deterministic() {
        let data = ReportData {
            project: "x".into(),
            ..ReportData::default()
        };
        assert_eq!(render(&data, Lang::En), render(&data, Lang::En));
    }
}
