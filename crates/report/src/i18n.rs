// SPDX-License-Identifier: Apache-2.0
//! The table of every fixed string a report contains, in English and Spanish.
//!
//! Every renderer asks this module for its text through a [`Key`], so there is exactly one place
//! where wording lives and the compiler guarantees each key has both translations: the
//! `keys!` macro below defines the enum, the list of all keys and the two lookup arms from a
//! single entry. Strings may contain positional placeholders (`{0}`, `{1}`), which [`tf`] fills
//! in; the English and Spanish versions of a key must use the same placeholders (a test checks
//! it), but may order them differently.
//!
//! Spanish follows natural technical usage: inverted punctuation, correct accents and the words
//! developers actually use (`memorias`, `símbolos`, `grafo`).

use crate::lang::Lang;

/// Defines [`Key`], [`Key::ALL`] and [`Key::text`] from one list of `Name => (english, spanish)`.
macro_rules! keys {
    ($($(#[$meta:meta])* $name:ident => ($en:expr, $es:expr)),+ $(,)?) => {
        /// Identifies one fixed string of the report.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub(crate) enum Key {
            $($(#[$meta])* $name),+
        }

        impl Key {
            /// Every key, in declaration order.
            #[cfg(test)]
            pub(crate) const ALL: &'static [Key] = &[$(Key::$name),+];

            /// Returns the text of the key in `lang`.
            pub(crate) const fn text(self, lang: Lang) -> &'static str {
                match (self, lang) {
                    $((Key::$name, Lang::En) => $en, (Key::$name, Lang::Es) => $es),+
                }
            }
        }
    };
}

keys! {
    /// Title of the report.
    DocTitle => ("Code report", "Informe de código"),
    /// Label of the project name in the title block.
    Project => ("Project", "Proyecto"),
    /// Label of the generation date.
    GeneratedOn => ("Generated on", "Generado el"),
    /// Label of the tool version.
    ToolVersion => ("Tool version", "Versión de la herramienta"),
    /// Shown when the project name is empty.
    UnnamedProject => ("(unnamed project)", "(proyecto sin nombre)"),
    /// Label of the table of contents.
    Contents => ("Contents", "Contenido"),
    /// Text of the skip link.
    SkipToContent => ("Skip to the report", "Saltar al informe"),
    /// Heading of the executive summary.
    SecSummary => ("Executive summary", "Resumen ejecutivo"),
    /// Heading of the reading guide.
    SecHowTo => ("How to read this report", "Cómo leer este informe"),
    /// Heading of the languages section.
    SecLanguages => ("Languages", "Lenguajes"),
    /// Heading of the modules section.
    SecModules => ("Modules", "Módulos"),
    /// Heading of the hotspots section.
    SecHotspots => ("Hotspots", "Puntos calientes"),
    /// Heading of the documentation section.
    SecDocs => ("Documentation coverage", "Cobertura de la documentación"),
    /// Heading of the graph section.
    SecGraph => ("Code graph", "Grafo de código"),
    /// Heading of the memories section.
    SecMemories => ("Memories", "Memorias"),
    /// Heading of the usage section.
    SecUsage => ("Usage and token efficiency", "Uso y eficiencia de tokens"),
    /// Heading of the notes section.
    SecNotes => ("Notes", "Notas"),
    /// Card label: files.
    MetricFiles => ("Files", "Archivos"),
    /// Card label: lines of code.
    MetricLines => ("Lines of code", "Líneas de código"),
    /// Card label: symbols.
    MetricSymbols => ("Symbols", "Símbolos"),
    /// Card label: relationships.
    MetricRelationships => ("Relationships", "Relaciones"),
    /// Card label: memories.
    MetricMemories => ("Memories", "Memorias"),
    /// Card label: stale memories.
    MetricStale => ("Stale memories", "Memorias obsoletas"),
    /// Card label: files with parse errors.
    MetricParseErrors => ("Files with parse errors", "Archivos con errores de análisis"),
    /// The reading guide paragraph.
    HowToRead => (
        "This report summarizes the indexed repository. Every number comes from the local code \
         graph: symbols are functions, types and other named items, and relationships are calls, \
         imports and inheritance between them. Hotspots are the most referenced symbols, so a \
         change there reaches the most code. Stale memories describe code that changed after \
         they were saved and should be reviewed before they are trusted.",
        "Este informe resume el repositorio indexado. Todas las cifras provienen del grafo de \
         código local: los símbolos son funciones, tipos y otros elementos con nombre, y las \
         relaciones son llamadas, importaciones y herencia entre ellos. Los puntos calientes son \
         los símbolos más referenciados, así que un cambio en ellos alcanza más código. Las \
         memorias obsoletas describen código que cambió después de guardarlas y conviene \
         revisarlas antes de fiarse de ellas."
    ),
    /// Column: language.
    ColLanguage => ("Language", "Lenguaje"),
    /// Column: files.
    ColFiles => ("Files", "Archivos"),
    /// Column: symbols.
    ColSymbols => ("Symbols", "Símbolos"),
    /// Column: share of files.
    ColShare => ("Share of files", "Proporción de archivos"),
    /// Column: module.
    ColModule => ("Module", "Módulo"),
    /// Column: incoming relationships.
    ColIncoming => ("Incoming", "Entrantes"),
    /// Column: outgoing relationships.
    ColOutgoing => ("Outgoing", "Salientes"),
    /// Column: symbol.
    ColSymbol => ("Symbol", "Símbolo"),
    /// Column: kind.
    ColKind => ("Kind", "Tipo"),
    /// Column: location.
    ColLocation => ("Location", "Ubicación"),
    /// Column: callers.
    ColCallers => ("Callers", "Llamadores"),
    /// Column: callees.
    ColCallees => ("Callees", "Llamados"),
    /// Column: identifier.
    ColId => ("ID", "ID"),
    /// Column: provenance.
    ColProvenance => ("Provenance", "Procedencia"),
    /// Column: status.
    ColStatus => ("Status", "Estado"),
    /// Column: text.
    ColText => ("Text", "Texto"),
    /// Column: public symbols.
    ColPublic => ("Public symbols", "Símbolos públicos"),
    /// Column: documented symbols.
    ColDocumented => ("Documented", "Documentados"),
    /// Column: coverage.
    ColCoverage => ("Coverage", "Cobertura"),
    /// Column: name.
    ColName => ("Name", "Nombre"),
    /// Column: group.
    ColGroup => ("Group", "Grupo"),
    /// Column: weight.
    ColWeight => ("Weight", "Peso"),
    /// Column of the summary table in Markdown: metric.
    ColMetric => ("Metric", "Métrica"),
    /// Column of the summary table in Markdown: value.
    ColValue => ("Value", "Valor"),
    /// Memory status: stale.
    StatusStale => ("Stale", "Obsoleta"),
    /// Memory status: still valid.
    StatusCurrent => ("Current", "Vigente"),
    /// Truncation note; placeholders: rows shown, rows in total.
    ShowingOf => ("Showing {0} of {1}.", "Mostrando {0} de {1}."),
    /// Shown instead of an empty table.
    NoData => ("No data.", "Sin datos."),
    /// Introduction of the hotspots table.
    HotspotsIntro => (
        "The most referenced symbols. A change to one of them reaches the most code.",
        "Los símbolos más referenciados. Un cambio en cualquiera de ellos alcanza más código."
    ),
    /// Introduction of the modules table.
    ModulesIntro => (
        "Incoming and outgoing count the relationships that cross the module boundary.",
        "Entrantes y salientes cuentan las relaciones que cruzan el límite del módulo."
    ),
    /// Label of the overall coverage figure.
    DocOverall => ("Overall coverage", "Cobertura global"),
    /// Coverage sentence; placeholders: documented, public symbols.
    DocSentence => (
        "{0} of {1} public symbols have a documentation comment.",
        "{0} de {1} símbolos públicos tienen un comentario de documentación."
    ),
    /// Heading of the per-language coverage table.
    DocByLanguage => ("Coverage by language", "Cobertura por lenguaje"),
    /// Heading of the list of undocumented symbols.
    DocUndocumented => (
        "Public symbols without documentation (examples)",
        "Símbolos públicos sin documentación (ejemplos)"
    ),
    /// Introduction of the graph.
    GraphIntro => (
        "Each circle is a node, sized by its weight and colored by group. Arrows point from the \
         source to the target, and thicker lines mean heavier relationships.",
        "Cada círculo es un nodo: su tamaño indica el peso y su color el grupo. Las flechas van \
         del origen al destino, y las líneas más gruesas indican relaciones más pesadas."
    ),
    /// Heading of the legend.
    GraphLegend => ("Legend", "Leyenda"),
    /// Accessible name of the graph; placeholders: nodes, relationships.
    GraphAlt => (
        "Code graph with {0} nodes and {1} relationships",
        "Grafo de código con {0} nodos y {1} relaciones"
    ),
    /// Truncation note of the graph; placeholders: nodes shown, nodes total, edges shown, edges total.
    GraphShowing => (
        "Showing the {0} heaviest nodes of {1} and {2} of {3} relationships.",
        "Se muestran los {0} nodos de mayor peso de {1} y {2} de {3} relaciones."
    ),
    /// Note inside a capped graph image; placeholder: hidden nodes.
    GraphMore => ("+{0} more nodes not shown", "+{0} nodos más sin mostrar"),
    /// Name of the group of nodes that have none.
    GraphUngrouped => ("Ungrouped", "Sin grupo"),
    /// Legend entry for dashed edges.
    GraphDashed => (
        "Dashed lines mark relationships with lower confidence.",
        "Las líneas discontinuas marcan relaciones de menor confianza."
    ),
    /// Accessible name of the language chart.
    ChartFiles => ("Files per language", "Archivos por lenguaje"),
    /// The one-sentence privacy statement of the usage section.
    UsageIntro => (
        "These are local counts kept on this machine; nothing is sent anywhere.",
        "Son recuentos locales guardados en esta máquina; no se envía nada a ningún sitio."
    ),
    /// Usage: recall calls.
    UsageRecalls => ("Recalls", "Recuperaciones"),
    /// Usage: expand calls.
    UsageExpands => ("Expansions", "Expansiones"),
    /// Usage: impact calls.
    UsageImpacts => ("Impact analyses", "Análisis de impacto"),
    /// Usage: remember calls.
    UsageRemembers => ("Memories saved", "Memorias guardadas"),
    /// Usage: average tokens served.
    UsageAvgTokens => ("Average tokens served", "Promedio de tokens servidos"),
    /// Usage: average share of the budget.
    UsageAvgBudget => (
        "Average share of the token budget used",
        "Promedio del presupuesto de tokens utilizado"
    ),
    /// Usage: total tokens served.
    UsageTotalTokens => ("Total tokens served", "Total de tokens servidos"),
    /// Footer; placeholder: tool version.
    Footer => (
        "Generated locally by pn-ultramemory {0}. No data left this machine.",
        "Generado localmente por pn-ultramemory {0}. Ningún dato salió de esta máquina."
    ),
    /// Page counter of the PDF; placeholders: page number, page count.
    PageOf => ("Page {0} of {1}", "Página {0} de {1}"),
}

/// Returns the text of `key` in `lang`.
pub(crate) const fn t(lang: Lang, key: Key) -> &'static str {
    key.text(lang)
}

/// Returns the text of `key` in `lang` with `{0}`, `{1}`, ... replaced by `args`.
///
/// A placeholder without a matching argument is left as it is, and arguments are inserted
/// verbatim (never re-scanned), so an argument that itself contains `{0}` is harmless.
pub(crate) fn tf(lang: Lang, key: Key, args: &[&str]) -> String {
    fill(key.text(lang), args)
}

/// Replaces `{n}` placeholders in `template` with `args[n]`.
fn fill(template: &str, args: &[&str]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let replaced = after.find('}').and_then(|close| {
            let index: usize = after[..close].parse().ok()?;
            let arg = args.get(index)?;
            Some((close, *arg))
        });
        if let Some((close, arg)) = replaced {
            out.push_str(arg);
            rest = &after[close + 1..];
        } else {
            out.push('{');
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::{Key, fill, t, tf};
    use crate::lang::Lang;

    /// Lists the placeholders (`{0}`, `{1}`, ...) of a template, in order of appearance.
    fn placeholders(template: &str) -> Vec<String> {
        let mut found = Vec::new();
        let mut rest = template;
        while let Some(open) = rest.find('{') {
            let after = &rest[open + 1..];
            let close = after.find('}').unwrap_or(0);
            found.push(after[..close].to_owned());
            rest = &after[close + 1..];
        }
        found.sort();
        found
    }

    /// Every key has a non-empty English and Spanish text.
    #[test]
    fn every_key_has_both_translations() {
        assert!(Key::ALL.len() > 60, "the table looks truncated");
        for key in Key::ALL {
            for lang in [Lang::En, Lang::Es] {
                let text = t(lang, *key);
                assert!(!text.trim().is_empty(), "{key:?} is empty in {lang:?}");
                assert_eq!(
                    text,
                    text.trim(),
                    "{key:?} has stray whitespace in {lang:?}"
                );
                assert!(
                    !text.contains("  "),
                    "{key:?} has a double space in {lang:?}"
                );
            }
        }
    }

    /// Both languages of a key use the same placeholders, and placeholders are well formed.
    #[test]
    fn placeholders_match_between_languages() {
        for key in Key::ALL {
            let en = placeholders(t(Lang::En, *key));
            let es = placeholders(t(Lang::Es, *key));
            assert_eq!(en, es, "{key:?}");
            for name in en {
                assert!(
                    name.parse::<usize>().is_ok(),
                    "{key:?} has placeholder {name:?}"
                );
            }
        }
    }

    /// Only a small allow-list of keys may read the same in both languages.
    #[test]
    fn spanish_is_really_translated() {
        let same: Vec<Key> = Key::ALL
            .iter()
            .copied()
            .filter(|key| t(Lang::En, *key) == t(Lang::Es, *key))
            .collect();
        assert_eq!(same, vec![Key::ColId]);
    }

    /// Spanish text keeps its accents and inverted punctuation where the words need them.
    #[test]
    fn spanish_has_accents() {
        let must_contain = [
            (Key::DocTitle, "código"),
            (Key::SecHowTo, "Cómo"),
            (Key::SecModules, "Módulos"),
            (Key::MetricSymbols, "Símbolos"),
            (Key::MetricLines, "Líneas"),
            (Key::PageOf, "Página"),
            (Key::Footer, "Ningún"),
            (Key::ToolVersion, "Versión"),
            (Key::ColLocation, "Ubicación"),
            (Key::MetricParseErrors, "análisis"),
        ];
        for (key, word) in must_contain {
            assert!(
                t(Lang::Es, key).contains(word),
                "{key:?} should contain {word}"
            );
        }
    }

    /// Placeholders are filled, missing arguments are kept, and arguments are not re-scanned.
    #[test]
    fn fill_replaces_placeholders() {
        assert_eq!(
            tf(Lang::En, Key::ShowingOf, &["25", "5,000"]),
            "Showing 25 of 5,000."
        );
        assert_eq!(
            tf(Lang::Es, Key::ShowingOf, &["25", "5.000"]),
            "Mostrando 25 de 5.000."
        );
        assert_eq!(fill("a {1} b {0}", &["x", "y"]), "a y b x");
        assert_eq!(fill("a {2}", &["x"]), "a {2}");
        assert_eq!(fill("{0}", &["{0}"]), "{0}");
        assert_eq!(fill("{", &[]), "{");
        assert_eq!(fill("{x} {", &["a"]), "{x} {");
        assert_eq!(fill("no placeholders", &["a"]), "no placeholders");
    }

    /// The footer and the page counter interpolate as documented in both languages.
    #[test]
    fn footer_and_pager_interpolate() {
        assert_eq!(
            tf(Lang::En, Key::Footer, &["1.2.3"]),
            "Generated locally by pn-ultramemory 1.2.3. No data left this machine."
        );
        assert_eq!(tf(Lang::Es, Key::PageOf, &["2", "9"]), "Página 2 de 9");
    }
}
