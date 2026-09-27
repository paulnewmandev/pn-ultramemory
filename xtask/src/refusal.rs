// SPDX-License-Identifier: Apache-2.0
//! The refusal ratchet: every error message must name a way forward.
//!
//! # The invariant
//! When this tool refuses to do something, the person or the agent reading the refusal must be able
//! to tell what to do next. A message that only says what went wrong leaves the reader to guess, and
//! an agent that guesses burns tokens and, worse, invents.
//!
//! # What is collected
//! Messages are collected by **carrier** rather than by constructor, because the same sentence
//! reaches a user through several routes:
//! * a `#[error("...")]` attribute (`thiserror`);
//! * a literal handed to `format!`, `write!` or `writeln!` inside the `Display` implementation of a
//!   type whose name ends in `Error`;
//! * a literal handed to a constructor whose path ends in one of a fixed list of names such as
//!   `::invalid` or `::NotFound`;
//! * a literal that initialises an explanatory field named `reason`, `hint` or `detail`.
//!
//! # What this guard does not prove
//! * It does not prove the continuation is **correct**, only that one is named. A message telling
//!   the reader to run a command that does not exist satisfies it.
//! * It does not see messages assembled at run time from pieces, nor messages that come from a
//!   dependency, nor text in data files.
//! * It reads only the four carriers above. A message reaching a user by any other route is
//!   invisible to it.
//! * Keys carry no line number, so two identical messages in one file collapse into one entry.
//! * It skips test code, because a message a test prints cannot reach a user.

mod message;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use proc_macro2::{TokenStream, TokenTree};
use syn::visit::Visit;

use crate::ratchet::{self, Baseline};
use crate::source::{self, SourceFile, TestOnly};

/// The baseline file this guard owns.
const BASELINE: &str = "refusal.txt";

/// The explanatory comment block written at the top of the baseline.
const HEADER: &[&str] = &[
    "SPDX-License-Identifier: Apache-2.0",
    "",
    "Baseline of the refusal ratchet: error messages that do not yet name a way forward.",
    "One finding per line, as `<crate>/<path>` TAB `<message, whitespace-normalised>`.",
    "There is deliberately no line number: a finding must not move when the line above it is",
    "edited, or the guard breaks on unrelated changes and somebody switches it off.",
    "",
    "This file may only SHRINK. A message not listed here fails the build. A line listed here",
    "whose file is no longer analysed also fails the build, because `fixed` and `invisible to",
    "the analyser` are indistinguishable from this file's point of view.",
    "",
    "Run the guard:  cargo run -p xtask -- refusal",
    "Regenerate:     cargo run -p xtask -- refusal --update",
    "Sort order:     plain byte order. Post-process with LC_ALL=C or comm(1) will lie.",
    "",
    "What this guard does NOT prove:",
    "  * that a named continuation is the right one, only that one is named;",
    "  * anything about messages built at run time, or coming from a dependency, or held in data;",
    "  * anything about messages that reach a user by a carrier this guard does not read.",
];

/// Constructor path endings that mark a call as raising a refusal.
const CONSTRUCTOR_TAILS: &[&str] = &[
    "Backend",
    "Corrupt",
    "Invalid",
    "Io",
    "NotFound",
    "Parse",
    "Rejected",
    "Unsupported",
    "failure",
    "invalid",
];

/// Struct field names whose literal initialiser is an explanation shown to a reader.
const EXPLANATORY_FIELDS: &[&str] = &["detail", "hint", "reason"];

/// Macro names whose first literal argument is a message shown to a reader.
const MESSAGE_MACROS: &[&str] = &["format", "write", "writeln"];

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

    println!("refusal ratchet");
    ratchet::print_scope(outcome.analysed, test_only.len(), baseline.len());
    println!("  message sites:     {}", outcome.total);
    println!("  with continuation: {}", outcome.with_continuation);
    println!("  by design:         {}", outcome.by_design);
    for (carrier, count) in &outcome.by_carrier {
        println!("    via {carrier}: {count}");
    }
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
    verdict.report("refusal");
    if !outcome.problems.is_empty() {
        println!(
            "\nProblems that no baseline can excuse ({}):",
            outcome.problems.len()
        );
        for problem in &outcome.problems {
            println!("  ! {problem}");
        }
    }
    Ok(!verdict.failed() && outcome.problems.is_empty())
}

/// Everything one run of the guard learned.
#[derive(Default)]
struct Outcome {
    /// How many files were inspected for findings.
    analysed: usize,
    /// How many message sites were collected in total.
    total: usize,
    /// How many of them name a continuation.
    with_continuation: usize,
    /// How many were excused by a by-design marker.
    by_design: usize,
    /// Findings, keyed by `<crate>/<path>` and the message text.
    findings: BTreeSet<String>,
    /// The anchors the guard actually enumerated, for the ratchet comparison.
    anchors: BTreeSet<String>,
    /// Counts per carrier, for the summary.
    by_carrier: BTreeMap<&'static str, usize>,
    /// Failures that a baseline cannot excuse: malformed or unused by-design markers.
    problems: Vec<String>,
}

/// Collects every message site in `files`, skipping test code.
///
/// A malformed or unused by-design marker is recorded in [`Outcome::problems`] rather than returned
/// as an error, so that one broken suppression does not hide the rest of the report.
fn collect(files: &[SourceFile], test_only: &TestOnly) -> Outcome {
    let mut outcome = Outcome::default();
    for file in files {
        if test_only.contains(&file.rel) {
            continue;
        }
        let anchor = format!("{}/{}", file.crate_name, file.in_crate);
        outcome.analysed += 1;
        outcome.anchors.insert(anchor.clone());
        let mut markers = match message::markers(&file.text) {
            Ok(markers) => markers,
            Err(problems) => {
                for problem in problems {
                    outcome.problems.push(format!("{}: {problem}", file.rel));
                }
                Vec::new()
            }
        };
        let mut collector = Collector {
            file,
            in_error_display: false,
            sites: Vec::new(),
        };
        collector.visit_file(&file.ast);
        let sites = collector.sites;
        for site in &sites {
            outcome.total += 1;
            *outcome.by_carrier.entry(site.carrier).or_insert(0) += 1;
            if message::names_continuation(&site.text) {
                outcome.with_continuation += 1;
                continue;
            }
            if let Some(marker) = markers.iter_mut().find(|marker| marker.covers(site.line)) {
                marker.used = true;
                outcome.by_design += 1;
                continue;
            }
            outcome
                .findings
                .insert(ratchet::key(&anchor, &[&site.text]));
        }
        for marker in &markers {
            if !marker.used {
                outcome.problems.push(format!(
                    "{}:{}: `refusal:by-design {}: {}` excuses nothing. The message sites within \
                     three lines below it either already name a continuation or were not \
                     collected. Remove the marker or move it beside the message it is about.",
                    file.rel,
                    marker.line,
                    marker.shape.name(),
                    marker.reason
                ));
            }
        }
    }
    outcome
}

/// One collected message site.
struct Site {
    /// The message text, whitespace-normalised.
    text: String,
    /// 1-based line the literal starts on, used only to match a by-design marker.
    line: usize,
    /// Which carrier it was found on, for the summary.
    carrier: &'static str,
}

/// Walks one file gathering message sites.
struct Collector<'a> {
    /// The file being walked, for span resolution.
    file: &'a SourceFile,
    /// True while inside the `Display` implementation of a type whose name ends in `Error`.
    in_error_display: bool,
    /// The sites found so far.
    sites: Vec<Site>,
}

impl Collector<'_> {
    /// Records `literal` as a message site if it carries prose rather than only placeholders.
    fn push(&mut self, literal: &syn::LitStr, carrier: &'static str) {
        let text = source::normalise_whitespace(&literal.value());
        if !message::carries_prose(&text) {
            return;
        }
        self.sites.push(Site {
            text,
            line: self.file.span_line(literal.span()),
            carrier,
        });
    }

    /// Records every string literal in `tokens`, however deeply grouped.
    fn push_tokens(&mut self, tokens: TokenStream, carrier: &'static str) {
        for literal in string_literals(tokens) {
            self.push(&literal, carrier);
        }
    }
}

impl<'ast> Visit<'ast> for Collector<'_> {
    /// Skips modules compiled only for tests.
    fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
        if source::is_test_gated(&node.attrs) {
            return;
        }
        syn::visit::visit_item_mod(self, node);
    }

    /// Skips test functions.
    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        if source::is_test_gated(&node.attrs) {
            return;
        }
        syn::visit::visit_item_fn(self, node);
    }

    /// Skips test methods inside an implementation block.
    fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
        if source::is_test_gated(&node.attrs) {
            return;
        }
        syn::visit::visit_impl_item_fn(self, node);
    }

    /// Notes whether this implementation block is the `Display` of an error type.
    fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
        if source::is_test_gated(&node.attrs) {
            return;
        }
        let outer = self.in_error_display;
        if is_error_display(node) {
            self.in_error_display = true;
        }
        syn::visit::visit_item_impl(self, node);
        self.in_error_display = outer;
    }

    /// Collects the literals of a `#[error("...")]` attribute.
    fn visit_attribute(&mut self, node: &'ast syn::Attribute) {
        if node.path().is_ident("error") {
            if let syn::Meta::List(list) = &node.meta {
                self.push_tokens(list.tokens.clone(), "error attribute");
            }
        }
        syn::visit::visit_attribute(self, node);
    }

    /// Collects the literals of a formatting macro inside an error `Display` implementation.
    fn visit_macro(&mut self, node: &'ast syn::Macro) {
        if self.in_error_display && last_segment_is(&node.path, MESSAGE_MACROS) {
            self.push_tokens(node.tokens.clone(), "error Display");
        }
        syn::visit::visit_macro(self, node);
    }

    /// Collects the literal arguments of a refusal constructor.
    fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
        if let syn::Expr::Path(path) = node.func.as_ref() {
            if path.path.segments.len() >= 2 && last_segment_is(&path.path, CONSTRUCTOR_TAILS) {
                for argument in &node.args {
                    if let syn::Expr::Macro(inner) = argument {
                        if last_segment_is(&inner.mac.path, &["concat", "format"]) {
                            self.push_tokens(inner.mac.tokens.clone(), "constructor");
                            continue;
                        }
                    }
                    if let Some(text) = literal_initialiser(argument) {
                        self.push(text, "constructor");
                    }
                }
            }
        }
        syn::visit::visit_expr_call(self, node);
    }

    /// Collects the literal initialiser of an explanatory field.
    fn visit_expr_struct(&mut self, node: &'ast syn::ExprStruct) {
        for field in &node.fields {
            if let syn::Member::Named(name) = &field.member {
                if EXPLANATORY_FIELDS.contains(&name.to_string().as_str()) {
                    if let Some(literal) = literal_initialiser(&field.expr) {
                        self.push(literal, "explanatory field");
                    }
                }
            }
        }
        syn::visit::visit_expr_struct(self, node);
    }
}

/// Returns the string literal an explanatory field is initialised with, if it is one.
///
/// A bare literal counts, and so does a literal turned into a `String` by `to_owned`, `to_string`
/// or `into`, which is how a literal usually reaches an owned field.
fn literal_initialiser(expr: &syn::Expr) -> Option<&syn::LitStr> {
    match expr {
        syn::Expr::Lit(literal) => match &literal.lit {
            syn::Lit::Str(text) => Some(text),
            _ => None,
        },
        syn::Expr::MethodCall(call)
            if matches!(
                call.method.to_string().as_str(),
                "to_owned" | "to_string" | "into"
            ) =>
        {
            literal_initialiser(&call.receiver)
        }
        _ => None,
    }
}

/// Returns true when `impl` block implements `Display` for a type whose name ends in `Error`.
fn is_error_display(node: &syn::ItemImpl) -> bool {
    let Some((path, _)) = &node.trait_ else {
        return false;
    };
    if !last_segment_is(path, &["Display"]) {
        return false;
    }
    match node.self_ty.as_ref() {
        syn::Type::Path(type_path) => type_path
            .path
            .segments
            .last()
            .is_some_and(|segment| segment.ident.to_string().ends_with("Error")),
        _ => false,
    }
}

/// Returns true when the last segment of `path` is one of `names`.
fn last_segment_is(path: &syn::Path, names: &[&str]) -> bool {
    path.segments
        .last()
        .is_some_and(|segment| names.contains(&segment.ident.to_string().as_str()))
}

/// Extracts every string literal from `tokens`, descending into delimited groups.
fn string_literals(tokens: TokenStream) -> Vec<syn::LitStr> {
    let mut found = Vec::new();
    let mut stack: Vec<TokenStream> = vec![tokens];
    while let Some(stream) = stack.pop() {
        for token in stream {
            match token {
                TokenTree::Group(group) => stack.push(group.stream()),
                TokenTree::Literal(literal) => {
                    let single = TokenStream::from(TokenTree::Literal(literal));
                    if let Ok(text) = syn::parse2::<syn::LitStr>(single) {
                        found.push(text);
                    }
                }
                _ => {}
            }
        }
    }
    found
}

#[cfg(test)]
mod tests;
