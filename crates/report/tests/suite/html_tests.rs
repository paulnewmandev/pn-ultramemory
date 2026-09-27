// SPDX-License-Identifier: Apache-2.0
//! Tests of the HTML report: structure, escaping, offline safety, themes, print, accessibility.

use pn_ultramemory_report::{Lang, ReportData, render_html};

use crate::fixtures::{NASTY, hostile, long_word, sample};
use crate::html_check::{Mode, check_markup};

/// Both languages of the realistic sample are valid HTML with the expected frame.
#[test]
fn sample_is_valid_html_in_both_languages() {
    for (lang, code) in [(Lang::En, "en"), (Lang::Es, "es")] {
        let html = render_html(&sample(), lang);
        let markup = check_markup(&html, Mode::Html).expect("valid HTML");
        assert!(html.contains(&format!("<html lang=\"{code}\">")));
        assert_eq!(markup.count("h1"), 1);
        assert_eq!(markup.count("main"), 1);
        assert_eq!(markup.count("header"), 1);
        assert_eq!(markup.count("footer"), 1);
        assert_eq!(markup.count("nav"), 1);
        assert!(markup.count("table") >= 4);
        assert!(markup.count("svg") >= 2, "language chart and graph");
        assert!(markup.count("section") >= 9);
    }
}

/// The page carries the required Content-Security-Policy and loads nothing from anywhere.
#[test]
fn page_is_offline_and_locked_down() {
    let html = render_html(&sample(), Lang::En);
    assert!(html.contains(
        "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'; img-src data:\">"
    ));
    for forbidden in [
        "<script",
        "<link",
        "<img",
        "<iframe",
        "src=",
        "@import",
        "url(",
        "http://",
        "https://",
        "javascript:",
        "onclick",
        "onload",
    ] {
        assert!(!html.contains(forbidden), "{forbidden} found");
    }
}

/// Light and dark themes, print rules and responsive rules are all present.
#[test]
fn page_has_themes_print_and_responsive_rules() {
    let html = render_html(&sample(), Lang::En);
    let markup = check_markup(&html, Mode::Html).expect("valid HTML");
    let css = markup.styles.join("\n");
    assert!(css.contains("@media (prefers-color-scheme:dark)"));
    assert!(css.contains("color-scheme:light dark"));
    assert!(css.contains("@media print"));
    assert!(css.contains("break-before:page"));
    assert!(css.contains("@media (max-width:640px)"));
    assert!(css.contains("overflow-x:auto"));
    assert!(
        html.contains("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">")
    );
    assert!(html.contains("<meta name=\"color-scheme\" content=\"light dark\">"));
}

/// Tables scroll inside labelled regions, and every header cell declares its scope.
#[test]
fn tables_are_accessible() {
    let html = render_html(&sample(), Lang::Es);
    let markup = check_markup(&html, Mode::Html).expect("valid HTML");
    assert!(markup.count("th") > 10);
    for (element, attribute, value) in &markup.attributes {
        if element == "th" && attribute == "scope" {
            assert!(value == "col" || value == "row", "{value}");
        }
    }
    let regions = markup
        .attributes
        .iter()
        .filter(|(_, a, v)| a == "role" && v == "region")
        .count();
    assert!(regions >= markup.count("table"));
    assert!(html.contains("<caption class=\"sr\">"));
}

/// Charts are images with a name; the graph names its content in the current language.
#[test]
fn charts_have_accessible_names() {
    let en = render_html(&sample(), Lang::En);
    assert!(en.contains("role=\"img\" aria-label=\"Files per language\""));
    assert!(en.contains("aria-label=\"Code graph with 4 nodes and 4 relationships\""));
    let es = render_html(&sample(), Lang::Es);
    assert!(es.contains("role=\"img\" aria-label=\"Archivos por lenguaje\""));
    assert!(es.contains("aria-label=\"Grafo de código con 4 nodos y 4 relaciones\""));
    assert!(en.contains("role=\"progressbar\""));
}

/// Numbers follow the language of the report.
#[test]
fn numbers_follow_the_language() {
    let en = render_html(&sample(), Lang::En);
    assert!(en.contains("<dd>71,240</dd>"));
    assert!(en.contains("75%"));
    let es = render_html(&sample(), Lang::Es);
    assert!(es.contains("<dd>71.240</dd>"));
    assert!(es.contains("75\u{a0}%"));
    assert!(es.contains("1.902.337"));
}

/// Every hostile string is escaped, and the result is still valid HTML.
#[test]
fn hostile_data_is_escaped() {
    for lang in [Lang::En, Lang::Es] {
        let html = render_html(&hostile(), lang);
        check_markup(&html, Mode::Html).expect("valid HTML despite hostile data");
        assert!(!html.contains("<script"), "raw script tag");
        assert!(!html.contains("alert(1)</script>"));
        assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
        assert!(html.contains("&quot;quoted&quot;"));
        assert!(html.contains("&#39;single&#39;"));
        assert!(
            html.contains("&amp;amp;"),
            "an existing entity is escaped, not interpreted"
        );
        assert!(html.contains("שלום עולם"), "right-to-left text is kept");
        assert!(html.contains('🙂'));
        assert!(!html.contains('\u{202e}'), "bidirectional override removed");
        assert!(!html.contains('\u{0}'));
    }
}

/// A single field at a time: each user-provided string reaches the page escaped.
#[test]
fn every_field_is_escaped() {
    let probe = "<i>x</i>\"&";
    let escaped = "&lt;i&gt;x&lt;/i&gt;&quot;&amp;";
    let mut data = sample();
    data.project = probe.into();
    data.generated_on = probe.into();
    data.tool_version = probe.into();
    data.languages[0].name = probe.into();
    data.modules[0].name = probe.into();
    data.hotspots[0].name = probe.into();
    data.hotspots[0].kind = probe.into();
    data.hotspots[0].path = probe.into();
    data.documentation.by_language[0].name = probe.into();
    data.documentation.undocumented_examples[0].name = probe.into();
    data.graph.nodes[0].label = probe.into();
    data.graph.nodes[0].group = probe.into();
    data.memories[0].kind = probe.into();
    data.memories[0].provenance = probe.into();
    data.memories[0].text = probe.into();
    data.notes = vec![probe.into()];
    let html = render_html(&data, Lang::En);
    check_markup(&html, Mode::Html).expect("valid HTML");
    assert!(!html.contains("<i>"), "unescaped markup");
    assert!(
        html.matches(escaped).count() >= 15,
        "{}",
        html.matches(escaped).count()
    );
}

/// Very long unbroken words neither break the page nor bloat it.
#[test]
fn long_words_are_clipped_and_wrap() {
    let mut data = sample();
    data.project = long_word(50_000);
    data.hotspots[0].name = long_word(50_000);
    data.memories[0].text = long_word(50_000);
    let html = render_html(&data, Lang::En);
    check_markup(&html, Mode::Html).expect("valid HTML");
    assert!(html.len() < 200_000, "{}", html.len());
    assert!(html.contains("overflow-wrap:anywhere"));
}

/// Empty data still renders every fixed section, in both languages.
#[test]
fn empty_report_renders() {
    for lang in [Lang::En, Lang::Es] {
        let html = render_html(&ReportData::default(), lang);
        let markup = check_markup(&html, Mode::Html).expect("valid HTML");
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
            assert!(markup.ids.contains(&id.to_owned()), "{id}");
        }
        assert!(!markup.ids.contains(&"usage".to_owned()));
        assert!(!markup.ids.contains(&"notes".to_owned()));
    }
}

/// Truncated tables say how many rows are shown, in both languages.
#[test]
fn truncation_notes() {
    let mut data = sample();
    data.hotspots = crate::fixtures::huge().hotspots;
    assert!(render_html(&data, Lang::En).contains("Showing 25 of 5,000."));
    assert!(render_html(&data, Lang::Es).contains("Mostrando 25 de 5.000."));
    data.memories = crate::fixtures::huge().memories;
    let html = render_html(&data, Lang::En);
    assert_eq!(html.matches("Showing 25 of 2,000.").count(), 1);
    let markup = check_markup(&html, Mode::Html).expect("valid HTML");
    assert!(markup.count("tr") < 200);
}

/// The footer states that the report was generated locally, in both languages.
#[test]
fn footer_sentence() {
    assert!(
        render_html(&sample(), Lang::En)
            .contains("Generated locally by pn-ultramemory 1.2.36. No data left this machine.")
    );
    assert!(render_html(&sample(), Lang::Es).contains(
        "Generado localmente por pn-ultramemory 1.2.36. Ningún dato salió de esta máquina."
    ));
}

/// The usage section explains that the numbers are local counts.
#[test]
fn usage_section_states_privacy() {
    let en = render_html(&sample(), Lang::En);
    assert!(en.contains("These are local counts kept on this machine; nothing is sent anywhere."));
    assert!(en.contains("Average share of the token budget used"));
    let es = render_html(&sample(), Lang::Es);
    assert!(es.contains("no se envía nada a ningún sitio"));
    let mut without = sample();
    without.usage = None;
    assert!(!render_html(&without, Lang::En).contains("local counts"));
}

/// Rendering twice gives identical bytes.
#[test]
fn html_is_deterministic() {
    for data in [sample(), hostile()] {
        assert_eq!(render_html(&data, Lang::En), render_html(&data, Lang::En));
        assert_eq!(render_html(&data, Lang::Es), render_html(&data, Lang::Es));
    }
}

/// The hostile text constant really contains the dangerous pieces the tests rely on.
#[test]
fn nasty_constant_is_nasty() {
    assert!(NASTY.contains("<script>"));
    assert!(NASTY.contains('\u{0}'));
    assert!(NASTY.contains('🙂'));
}
