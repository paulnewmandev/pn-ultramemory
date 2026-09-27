// SPDX-License-Identifier: Apache-2.0
//! The style sheet of the HTML report.
//!
//! One inline sheet drives everything: CSS variables carry the palette, `prefers-color-scheme`
//! switches them for dark mode, a small responsive layer keeps tables scrollable on phones, and a
//! print layer forces the light palette, removes shadows and starts every section on a new page.
//!
//! Invariants: the sheet needs no font, image or script from anywhere; text and background
//! colors keep at least a 4.5:1 contrast ratio in both themes; the sheet is a pure function of
//! nothing, so it is byte-identical on every run.

use std::fmt::Write as _;

use crate::graph::palette;
use crate::graph::svg::{GRAPH_CSS, group_class_css, vars_dark, vars_light};

/// Page variables of the light theme.
const LIGHT: &str = "--bg:#f4f6f8;--surface:#ffffff;--text:#14181f;--muted:#4b5563;\
--border:#d5dbe3;--accent:#0f6b63;--accent-fg:#ffffff;--accent-soft:#e3f3f1;--track:#dde3ea;\
--head:#eaeff4;--zebra:#f7f9fb;--warn:#8a4b00;--warn-bg:#fff2dc;--bad:#a12727;--bad-bg:#fde8e8;\
--shadow:0 1px 2px rgba(15,23,42,.08),0 6px 18px rgba(15,23,42,.06);";

/// Page variables of the dark theme.
const DARK: &str = "--bg:#0d1117;--surface:#161b22;--text:#e6edf3;--muted:#a3adba;\
--border:#30363d;--accent:#3ecfc2;--accent-fg:#06201d;--accent-soft:#12332f;--track:#2b323b;\
--head:#1d242d;--zebra:#1a2029;--warn:#f0b35a;--warn-bg:#3a2a10;--bad:#ff9b9b;--bad-bg:#3d1717;\
--shadow:none;";

/// Layout and component rules.
const BASE: &str = "\
*{box-sizing:border-box}\
html{-webkit-text-size-adjust:100%}\
body{margin:0;background:var(--bg);color:var(--text);\
font:16px/1.55 system-ui,-apple-system,\"Segoe UI\",Roboto,\"Helvetica Neue\",Arial,sans-serif}\
.skip{position:absolute;left:-999px;top:0;background:var(--accent);color:var(--accent-fg);\
padding:.5rem 1rem;border-radius:0 0 8px 0;z-index:10}\
.skip:focus{left:0}\
:focus-visible{outline:3px solid var(--accent);outline-offset:2px}\
.page{max-width:72rem;margin:0 auto;padding:1.5rem 1rem 3rem}\
.hero{background:var(--surface);border:1px solid var(--border);\
border-left:6px solid var(--accent);border-radius:10px;padding:1.5rem 1.75rem;\
box-shadow:var(--shadow)}\
.eyebrow{margin:0;font-size:.8rem;letter-spacing:.12em;text-transform:uppercase;\
color:var(--accent);font-weight:700}\
h1{font-size:clamp(1.6rem,4vw,2.4rem);line-height:1.15;margin:.25rem 0 .5rem}\
.project{margin:0 0 1rem;font-size:1.25rem;font-weight:600;overflow-wrap:anywhere}\
.meta{display:flex;flex-wrap:wrap;gap:.5rem 2rem;margin:0}\
.meta div{min-width:0}\
.meta dt{font-size:.78rem;color:var(--muted);text-transform:uppercase;letter-spacing:.06em}\
.meta dd{margin:0;font-weight:600;overflow-wrap:anywhere}\
nav.toc{margin:1rem 0}\
nav.toc ol{list-style:none;display:flex;flex-wrap:wrap;gap:.4rem;padding:0;margin:0}\
nav.toc a{display:inline-block;padding:.25rem .7rem;border:1px solid var(--border);\
border-radius:999px;background:var(--surface);color:var(--text);text-decoration:none;\
font-size:.9rem}\
nav.toc a:hover,nav.toc a:focus-visible{border-color:var(--accent);color:var(--accent)}\
section{background:var(--surface);border:1px solid var(--border);border-radius:10px;\
padding:1.25rem 1.5rem;margin:1rem 0;box-shadow:var(--shadow)}\
h2{font-size:1.35rem;margin:0 0 .75rem;padding-bottom:.4rem;border-bottom:2px solid var(--border)}\
h3{font-size:1.05rem;margin:1.25rem 0 .5rem}\
p{margin:.5rem 0;overflow-wrap:anywhere}\
.lead{color:var(--muted)}\
.cards{display:grid;grid-template-columns:repeat(auto-fit,minmax(9.5rem,1fr));gap:.75rem;margin:0}\
.card{display:flex;flex-direction:column-reverse;justify-content:flex-end;\
background:var(--accent-soft);border:1px solid var(--border);border-radius:8px;\
padding:.75rem .9rem}\
.card dt{font-size:.82rem;color:var(--muted)}\
.card dd{margin:0;font-size:1.6rem;font-weight:700;font-variant-numeric:tabular-nums;\
overflow-wrap:anywhere}\
.card.warn{background:var(--warn-bg)}.card.warn dd{color:var(--warn)}\
.card.bad{background:var(--bad-bg)}.card.bad dd{color:var(--bad)}\
.scroll{overflow-x:auto;-webkit-overflow-scrolling:touch;border:1px solid var(--border);\
border-radius:8px}\
table{border-collapse:collapse;width:100%;font-size:.92rem}\
th,td{padding:.45rem .7rem;text-align:left;vertical-align:top;\
border-bottom:1px solid var(--border);overflow-wrap:anywhere}\
thead th{background:var(--head);font-weight:700}\
tbody tr:nth-child(even){background:var(--zebra)}\
tbody tr:last-child>*{border-bottom:0}\
th[scope=row]{font-weight:600}\
.num{text-align:right;font-variant-numeric:tabular-nums;white-space:nowrap}\
.mono{font-family:ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;font-size:.88em}\
.badge{display:inline-block;padding:.05rem .5rem;border-radius:999px;font-size:.78rem;\
font-weight:700;border:1px solid var(--border);white-space:nowrap}\
.badge.stale{background:var(--warn-bg);color:var(--warn);border-color:var(--warn)}\
.badge.ok{background:var(--accent-soft);color:var(--accent)}\
.bar{height:.7rem;background:var(--track);border-radius:999px;overflow:hidden}\
.bar>span{display:block;height:100%;background:var(--accent);border-radius:999px}\
.bar.sm{height:.5rem;min-width:6rem}\
.cov{display:flex;align-items:center;gap:.6rem}.cov .bar{flex:1}\
.cov .pct{min-width:3.6rem;text-align:right;font-variant-numeric:tabular-nums}\
.big{font-size:2.4rem;font-weight:800;font-variant-numeric:tabular-nums;line-height:1;margin:.25rem 0}\
.note{color:var(--muted);font-size:.88rem}\
figure{margin:0}\
figcaption{color:var(--muted);font-size:.88rem;margin-top:.4rem}\
.chart{display:block;width:100%;height:auto;max-width:44rem}\
.chart .lbl{fill:var(--text);font:13px system-ui,-apple-system,\"Segoe UI\",Roboto,Arial,sans-serif}\
.chart .val{fill:var(--muted);font:12px system-ui,-apple-system,\"Segoe UI\",Roboto,Arial,sans-serif}\
.chart .bx{fill:var(--accent)}\
.graph{display:block;width:100%;min-width:44rem;height:auto}\
.legend{list-style:none;display:flex;flex-wrap:wrap;gap:.4rem 1.2rem;padding:0;margin:.5rem 0}\
.legend li{display:flex;align-items:center;gap:.4rem;overflow-wrap:anywhere}\
.sw{width:.9rem;height:.9rem;border-radius:50%;display:inline-block;flex:none}\
ul.notes{padding-left:1.25rem}ul.notes li{margin:.25rem 0;overflow-wrap:anywhere}\
.sr{position:absolute;width:1px;height:1px;margin:-1px;padding:0;overflow:hidden;\
clip:rect(0 0 0 0);white-space:nowrap;border:0}\
footer{margin:1.5rem 0 0;color:var(--muted);font-size:.9rem;text-align:center}\
@media (max-width:640px){.page{padding:.75rem .5rem 2rem}.hero,section{padding:1rem}\
.big{font-size:2rem}h2{font-size:1.2rem}}";

/// Rules for printing and saving to PDF from the browser.
const PRINT: &str = "\
body{background:#fff;font-size:11pt}\
.page{max-width:none;padding:0}\
.hero{box-shadow:none;margin:0 0 1rem}\
section{box-shadow:none;padding:0;margin:0 0 1rem;border:0}\
main>section{break-before:page;page-break-before:always}\
main>section:first-of-type{break-before:auto;page-break-before:auto}\
nav.toc,.skip{display:none}\
tr,.card,figure{break-inside:avoid;page-break-inside:avoid}\
thead{display:table-header-group}\
.scroll{overflow:visible;border:0}\
.graph{min-width:0}\
*{-webkit-print-color-adjust:exact;print-color-adjust:exact}\
@page{margin:14mm}";

/// Returns the complete style sheet, ready to place inside a `<style>` element.
pub(super) fn stylesheet() -> String {
    let mut sheet = String::with_capacity(12_000);
    sheet.push_str(":root{color-scheme:light dark;");
    sheet.push_str(LIGHT);
    sheet.push_str(&vars_light());
    sheet.push_str("}@media (prefers-color-scheme:dark){:root{");
    sheet.push_str(DARK);
    sheet.push_str(&vars_dark());
    sheet.push_str("}}");
    sheet.push_str(BASE);
    sheet.push_str(GRAPH_CSS);
    sheet.push_str(&group_class_css());
    for index in 0..palette::COLORS {
        let _ = write!(sheet, ".sw.g{index}{{background:var(--g{index})}}");
    }
    sheet.push_str("@media print{:root{color-scheme:light;");
    sheet.push_str(LIGHT);
    sheet.push_str(&vars_light());
    sheet.push('}');
    sheet.push_str(PRINT);
    sheet.push('}');
    sheet
}

#[cfg(test)]
mod tests {
    use super::stylesheet;

    /// The sheet supports both themes, printing and small screens, and loads nothing.
    #[test]
    fn stylesheet_has_the_required_layers() {
        let css = stylesheet();
        assert!(css.contains("prefers-color-scheme:dark"));
        assert!(css.contains("@media print"));
        assert!(css.contains("max-width:640px"));
        assert!(css.contains("break-before:page"));
        assert!(css.contains("box-shadow:none"));
        assert!(css.contains("overflow-x:auto"));
        for forbidden in [
            "@import",
            "url(",
            "http:",
            "https:",
            "@font-face",
            "expression(",
        ] {
            assert!(!css.contains(forbidden), "{forbidden}");
        }
    }

    /// Braces are balanced, so the sheet parses.
    #[test]
    fn braces_are_balanced() {
        let css = stylesheet();
        let mut depth = 0i32;
        for c in css.chars() {
            match c {
                '{' => depth += 1,
                '}' => depth -= 1,
                _ => {}
            }
            assert!(depth >= 0);
        }
        assert_eq!(depth, 0);
    }

    /// The sheet is the same on every call.
    #[test]
    fn stylesheet_is_deterministic() {
        assert_eq!(stylesheet(), stylesheet());
    }
}
