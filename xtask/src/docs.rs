// SPDX-License-Identifier: Apache-2.0
//! The documentation ratchet: a docstring must say more than the item's own name.
//!
//! # The invariant
//! The compiler already refuses an undocumented item. It cannot refuse a docstring that only repeats
//! the name, and a function `parse_config` documented as "Parses config" costs a reader a line and
//! teaches nothing. This guard finds those.
//!
//! # The heuristic
//! The first sentence of the docstring is split into words. Each word is reduced to a crude stem, and
//! so is each word of the item's own name, split at underscores and at case boundaries. The docstring
//! is a finding when every one of its words is either a word of the name or a common function word.
//!
//! # What this guard does not prove
//! * It cannot judge whether a docstring is **true**, **current** or **useful**; only that it adds a
//!   word. "Parses the configuration into a banana" passes.
//! * The stemmer is three suffix rules, not linguistics. It will accept a restatement dressed in an
//!   unusual inflection and, more rarely, flag a short but genuine sentence. That is what the
//!   baseline and the `# Errors`-style longer docstring are for.
//! * It reads only the first sentence. Everything after it is unexamined.
//! * It reads `crates/*/src` only, so examples, tests and benchmarks are not covered.

use std::collections::BTreeSet;
use std::path::Path;

use syn::visit::Visit;

use crate::ratchet::{self, Baseline};
use crate::source::{self, SourceFile, TestOnly};

/// The baseline file this guard owns.
const BASELINE: &str = "docs.txt";

/// The explanatory comment block written at the top of the baseline.
const HEADER: &[&str] = &[
    "SPDX-License-Identifier: Apache-2.0",
    "",
    "Baseline of the documentation ratchet: public items whose first documented sentence only",
    "restates the item's own name.",
    "One finding per line, as `<path>` TAB `<item path>`.",
    "There is deliberately no line number: the item path is the stable identity, so a finding",
    "survives an edit above it and survives the item moving inside its file.",
    "",
    "This file may only SHRINK. A new restating docstring fails the build. A line listed here",
    "whose file is no longer analysed also fails the build, because `rewritten` and `no longer",
    "seen by the analyser` are indistinguishable from this file's point of view.",
    "",
    "Run the guard:  cargo run -p xtask -- docs",
    "Regenerate:     cargo run -p xtask -- docs --update",
    "Sort order:     plain byte order. Post-process with LC_ALL=C or comm(1) will lie.",
    "",
    "What this guard does NOT prove:",
    "  * that a docstring is true, current or useful, only that it adds a word to the name;",
    "  * anything about the sentences after the first one;",
    "  * anything about items that are not public, nor about examples, tests or benchmarks.",
];

/// Words that carry no information about an item and so never rescue a docstring.
const STOP_WORDS: &[&str] = &[
    "a", "all", "an", "and", "any", "are", "as", "at", "be", "by", "each", "every", "for", "from",
    "given", "has", "if", "in", "into", "is", "it", "its", "must", "new", "of", "on", "one", "or",
    "self", "that", "the", "their", "this", "to", "value", "when", "which", "with", "within",
];

/// Runs the guard, returning whether it passed.
///
/// # Errors
/// Returns a message when the sources cannot be enumerated, a file does not parse, or the baseline
/// cannot be read or written.
pub(crate) fn run(root: &Path, update: bool) -> Result<bool, String> {
    let files = source::collect_crate_sources(root)?;
    let test_only = source::classify_test_only(&files);
    let outcome = collect(&files, &test_only);
    let baseline = Baseline::load(root, BASELINE)?;

    println!("documentation ratchet");
    ratchet::print_scope(outcome.analysed, test_only.len(), baseline.len());
    println!("  public items:      {}", outcome.items);
    println!("  findings:          {}", outcome.findings.len());

    if update {
        baseline.write(&outcome.findings, HEADER)?;
        println!(
            "\nWrote .ratchet/{BASELINE} with {} entries (was {}).",
            outcome.findings.len(),
            baseline.len()
        );
        return Ok(true);
    }

    let verdict = baseline.compare(&outcome.findings, &outcome.anchors);
    verdict.report("docs");
    Ok(!verdict.failed())
}

/// Everything one run of the guard learned.
#[derive(Default)]
struct Outcome {
    /// How many files were inspected.
    analysed: usize,
    /// How many public items were examined.
    items: usize,
    /// Findings, keyed by path and item path.
    findings: BTreeSet<String>,
    /// The anchors the guard actually enumerated, for the ratchet comparison.
    anchors: BTreeSet<String>,
}

/// Collects every public item whose first documented sentence only restates its name.
fn collect(files: &[SourceFile], test_only: &TestOnly) -> Outcome {
    let mut outcome = Outcome::default();
    for file in files {
        if test_only.contains(&file.rel) {
            continue;
        }
        outcome.analysed += 1;
        outcome.anchors.insert(file.rel.clone());
        let mut collector = Collector {
            scope: Vec::new(),
            items: 0,
            found: Vec::new(),
        };
        collector.visit_file(&file.ast);
        outcome.items += collector.items;
        for item in collector.found {
            outcome.findings.insert(ratchet::key(&file.rel, &[&item]));
        }
    }
    outcome
}

/// Walks one file gathering public items with a restating docstring.
struct Collector {
    /// The module and type names enclosing the current position.
    scope: Vec<String>,
    /// How many public items were examined.
    items: usize,
    /// The item paths found so far.
    found: Vec<String>,
}

impl Collector {
    /// Examines one public item, recording it when its docstring only restates its name.
    fn examine(&mut self, name: &str, attrs: &[syn::Attribute]) {
        self.items += 1;
        if restates(name, &first_sentence(attrs)) {
            let mut path = self.scope.clone();
            path.push(name.to_owned());
            self.found.push(path.join("::"));
        }
    }

    /// Runs `body` with `name` pushed onto the scope stack.
    fn within<F: FnOnce(&mut Self)>(&mut self, name: String, body: F) {
        self.scope.push(name);
        body(self);
        self.scope.pop();
    }
}

impl<'ast> Visit<'ast> for Collector {
    /// Examines a public module and walks into it; skips a test module.
    fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
        if source::is_test_gated(&node.attrs) {
            return;
        }
        let name = node.ident.to_string();
        if is_public(&node.vis) {
            self.examine(&name, &node.attrs);
        }
        self.within(name, |this| syn::visit::visit_item_mod(this, node));
    }

    /// Examines a public free function.
    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        if source::is_test_gated(&node.attrs) {
            return;
        }
        if is_public(&node.vis) {
            self.examine(&node.sig.ident.to_string(), &node.attrs);
        }
    }

    /// Examines a public struct and its public named fields.
    fn visit_item_struct(&mut self, node: &'ast syn::ItemStruct) {
        if !is_public(&node.vis) {
            return;
        }
        let name = node.ident.to_string();
        self.examine(&name, &node.attrs);
        self.within(name, |this| {
            for field in &node.fields {
                if let Some(ident) = &field.ident {
                    if is_public(&field.vis) {
                        this.examine(&ident.to_string(), &field.attrs);
                    }
                }
            }
        });
    }

    /// Examines a public enum and every one of its variants.
    fn visit_item_enum(&mut self, node: &'ast syn::ItemEnum) {
        if !is_public(&node.vis) {
            return;
        }
        let name = node.ident.to_string();
        self.examine(&name, &node.attrs);
        self.within(name, |this| {
            for variant in &node.variants {
                this.examine(&variant.ident.to_string(), &variant.attrs);
            }
        });
    }

    /// Examines a public trait and every one of its items.
    fn visit_item_trait(&mut self, node: &'ast syn::ItemTrait) {
        if !is_public(&node.vis) {
            return;
        }
        let name = node.ident.to_string();
        self.examine(&name, &node.attrs);
        self.within(name, |this| {
            for item in &node.items {
                match item {
                    syn::TraitItem::Fn(function) => {
                        this.examine(&function.sig.ident.to_string(), &function.attrs);
                    }
                    syn::TraitItem::Const(constant) => {
                        this.examine(&constant.ident.to_string(), &constant.attrs);
                    }
                    syn::TraitItem::Type(alias) => {
                        this.examine(&alias.ident.to_string(), &alias.attrs);
                    }
                    _ => {}
                }
            }
        });
    }

    /// Examines the public methods and associated items of an inherent implementation block.
    fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
        if source::is_test_gated(&node.attrs) || node.trait_.is_some() {
            return;
        }
        self.within(source::type_name(&node.self_ty), |this| {
            for item in &node.items {
                match item {
                    syn::ImplItem::Fn(function) if is_public(&function.vis) => {
                        if !source::is_test_gated(&function.attrs) {
                            this.examine(&function.sig.ident.to_string(), &function.attrs);
                        }
                    }
                    syn::ImplItem::Const(constant) if is_public(&constant.vis) => {
                        this.examine(&constant.ident.to_string(), &constant.attrs);
                    }
                    _ => {}
                }
            }
        });
    }

    /// Examines a public type alias.
    fn visit_item_type(&mut self, node: &'ast syn::ItemType) {
        if is_public(&node.vis) {
            self.examine(&node.ident.to_string(), &node.attrs);
        }
    }

    /// Examines a public constant.
    fn visit_item_const(&mut self, node: &'ast syn::ItemConst) {
        if is_public(&node.vis) {
            self.examine(&node.ident.to_string(), &node.attrs);
        }
    }

    /// Examines a public static.
    fn visit_item_static(&mut self, node: &'ast syn::ItemStatic) {
        if is_public(&node.vis) {
            self.examine(&node.ident.to_string(), &node.attrs);
        }
    }
}

/// Returns true when a visibility makes the item part of the crate's public surface.
fn is_public(vis: &syn::Visibility) -> bool {
    matches!(vis, syn::Visibility::Public(_))
}

/// Joins the `#[doc]` attributes of an item and returns its first sentence.
///
/// A sentence ends at the first full stop followed by whitespace or the end of the text, or at the
/// first blank line, whichever comes first.
fn first_sentence(attrs: &[syn::Attribute]) -> String {
    let mut text = String::new();
    for attr in attrs {
        if !attr.path().is_ident("doc") {
            continue;
        }
        let syn::Meta::NameValue(pair) = &attr.meta else {
            continue;
        };
        let syn::Expr::Lit(literal) = &pair.value else {
            continue;
        };
        let syn::Lit::Str(line) = &literal.lit else {
            continue;
        };
        let line = line.value();
        if line.trim().is_empty() {
            break;
        }
        text.push_str(line.trim());
        text.push(' ');
    }
    let trimmed = text.trim();
    match trimmed.find(". ") {
        Some(at) => trimmed.get(..at).unwrap_or(trimmed).to_owned(),
        None => trimmed.trim_end_matches('.').to_owned(),
    }
}

/// Returns true when every informative word of `sentence` is already a word of `name`.
///
/// An empty sentence restates vacuously, which is the correct verdict: a docstring that says nothing
/// is the worst case of saying only the name.
fn restates(name: &str, sentence: &str) -> bool {
    let own: BTreeSet<String> = split_identifier(name).into_iter().map(stem).collect();
    for word in sentence.split(|ch: char| !ch.is_alphanumeric()) {
        let lowered = word.to_lowercase();
        if lowered.is_empty() || STOP_WORDS.contains(&lowered.as_str()) {
            continue;
        }
        if !own.contains(&stem(lowered)) {
            return false;
        }
    }
    true
}

/// Splits an identifier into lowercase words at underscores and at case boundaries.
///
/// A run of capitals stays together, so `MAX_TOKENS` yields `max`, `tokens` and `HttpServer` yields
/// `http`, `server`, rather than one word per letter.
fn split_identifier(name: &str) -> Vec<String> {
    let chars: Vec<char> = name.chars().collect();
    let mut words = Vec::new();
    let mut current = String::new();
    for (index, ch) in chars.iter().enumerate() {
        if *ch == '_' || *ch == '-' {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            continue;
        }
        if ch.is_uppercase() && !current.is_empty() {
            let previous = index.checked_sub(1).and_then(|at| chars.get(at).copied());
            let next = chars.get(index + 1).copied();
            let after_lower = previous.is_some_and(|prev| prev.is_lowercase() || prev.is_numeric());
            let before_lower =
                previous.is_some_and(char::is_uppercase) && next.is_some_and(char::is_lowercase);
            if after_lower || before_lower {
                words.push(std::mem::take(&mut current));
            }
        }
        for lower in ch.to_lowercase() {
            current.push(lower);
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

/// Reduces a word to a crude stem, so that "parses", "parsing" and "parse" all compare equal.
///
/// One inflectional suffix is removed, then a trailing `e`, which is what makes "parses" and "parse"
/// meet in the middle at "pars". It is three rules and a trim, not linguistics; the module
/// documentation says what that costs.
fn stem(word: impl Into<String>) -> String {
    let word = word.into();
    let length = word.chars().count();
    let mut stripped = word.clone();
    for (suffix, replacement, minimum) in [
        ("ies", "y", 4),
        ("ing", "", 5),
        ("es", "", 4),
        ("ed", "", 4),
        ("s", "", 3),
    ] {
        if length >= minimum {
            if let Some(kept) = word.strip_suffix(suffix) {
                stripped = format!("{kept}{replacement}");
                break;
            }
        }
    }
    match stripped.strip_suffix('e') {
        Some(shorter) if shorter.chars().count() >= 2 => shorter.to_owned(),
        _ => stripped,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Loads one fixture and returns the item paths it flags.
    fn findings(name: &str) -> Vec<String> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("cannot read fixture {name}: {err}"));
        let files =
            vec![SourceFile::from_text(format!("crates/fx/src/{name}"), text).expect("parses")];
        let test_only = source::classify_test_only(&files);
        collect(&files, &test_only)
            .findings
            .into_iter()
            .filter_map(|entry| entry.split_once('\t').map(|(_, item)| item.to_owned()))
            .collect()
    }

    /// An identifier splits at underscores and at case boundaries.
    #[test]
    fn splits_identifiers() {
        assert_eq!(split_identifier("parse_config"), vec!["parse", "config"]);
        assert_eq!(split_identifier("DecodeError"), vec!["decode", "error"]);
        assert_eq!(
            split_identifier("to_toon_string"),
            vec!["to", "toon", "string"]
        );
        assert_eq!(split_identifier("MAX_TOKENS"), vec!["max", "tokens"]);
        assert_eq!(split_identifier("HttpServer"), vec!["http", "server"]);
    }

    /// The stemmer folds the inflections a docstring actually uses onto the name's own form.
    #[test]
    fn stems_common_inflections() {
        for (inflected, base) in [
            ("parses", "parse"),
            ("parsing", "parse"),
            ("applies", "apply"),
            ("decoded", "decode"),
            ("matches", "match"),
            ("tokens", "token"),
            ("uses", "use"),
        ] {
            assert_eq!(stem(inflected), stem(base), "{inflected} against {base}");
        }
        assert_eq!(stem("is"), "is", "a two-letter word is left alone");
        assert_ne!(stem("parse"), stem("render"));
    }

    /// A docstring that only restates the name is flagged; one that adds a word is not.
    #[test]
    fn detects_restatement() {
        assert!(restates("parse_config", "Parses config"));
        assert!(restates("parse_config", "Parses the config."));
        assert!(restates("line", ""));
        assert!(!restates(
            "parse_config",
            "Parses the configuration file into settings"
        ));
        assert!(!restates("line", "Returns the 1-based line number"));
    }

    /// Only the first sentence is read, and it ends at the first full stop or blank line.
    #[test]
    fn reads_only_the_first_sentence() {
        let attrs: syn::ItemFn = syn::parse_str(
            "/// Parses config. Then it validates the schema thoroughly.\nfn parse_config() {}",
        )
        .expect("parses");
        assert_eq!(first_sentence(&attrs.attrs), "Parses config");
    }

    /// Public items of every shape are examined, and restating docstrings are flagged by item path.
    #[test]
    fn flags_restating_public_items() {
        let mut found = findings("docs_restating.rs");
        found.sort();
        assert_eq!(
            found,
            vec![
                "Capsule::token_count",
                "TokenBudget",
                "TokenBudget::limit",
                "parse_config",
            ]
        );
    }

    /// A docstring that adds information is never flagged, however short it is.
    #[test]
    fn leaves_informative_docstrings_alone() {
        assert!(findings("docs_informative.rs").is_empty());
    }
}
