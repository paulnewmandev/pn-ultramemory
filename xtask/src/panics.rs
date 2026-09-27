// SPDX-License-Identifier: Apache-2.0
//! The panic ratchet: library code must not abort the process.
//!
//! # The invariant
//! A memory for coding agents runs inside somebody else's editor session. A panic there is not a
//! stack trace in a terminal, it is a tool that disappeared mid-task. Library code therefore returns
//! errors, and every construct that can panic is either removed or recorded.
//!
//! # Relationship to clippy
//! Clippy already denies `unwrap`, `expect`, `panic!`, `todo!`, `unimplemented!` and `dbg!` across
//! the workspace, and this guard records them too, so the two agree. Its own contribution is the
//! two things those lints do not cover:
//! * **indexing** with `[...]`, which panics on an out-of-range index or a missing map key;
//! * **integer `as` casts**, which silently truncate.
//!
//! # What this guard does not prove
//! * It has **no type information**. It cannot tell a slice index from a map index, nor a widening
//!   cast from a truncating one, so it records every integer cast and every index expression. That is
//!   deliberate: over-reporting into a baseline is safe, under-reporting is not.
//! * It does not prove the absence of panics. Arithmetic overflow, division by zero, `RefCell`
//!   borrows, stack exhaustion and panics inside dependencies are all invisible to it.
//! * It keys a finding by the enclosing function, so two identical expressions in one function
//!   collapse into one entry.
//! * It skips test code, where panicking is the normal way to report a failed assertion. Doctests
//!   live inside comments, so they are invisible to the parser and are skipped for free.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use syn::spanned::Spanned as _;
use syn::visit::Visit;

use crate::ratchet::{self, Baseline};
use crate::source::{self, SourceFile, TestOnly};

/// The baseline file this guard owns.
const BASELINE: &str = "panics.txt";

/// The explanatory comment block written at the top of the baseline.
const HEADER: &[&str] = &[
    "SPDX-License-Identifier: Apache-2.0",
    "",
    "Baseline of the panic ratchet: constructs in non-test code that can abort the process.",
    "One finding per line, as `<path>` TAB `<expression>` TAB `<enclosing function>`.",
    "There is deliberately no line number, so a finding does not move when the code above it is",
    "edited. The enclosing function is part of the key instead.",
    "",
    "This file may only SHRINK. A construct not listed here fails the build. A line listed here",
    "whose file is no longer analysed also fails the build, because `fixed` and `invisible to",
    "the analyser` are indistinguishable from this file's point of view.",
    "",
    "Run the guard:  cargo run -p xtask -- panics",
    "Regenerate:     cargo run -p xtask -- panics --update",
    "Sort order:     plain byte order. Post-process with LC_ALL=C or comm(1) will lie.",
    "",
    "What this guard does NOT prove:",
    "  * that the code cannot panic: overflow, division by zero and dependencies are not seen;",
    "  * that a recorded cast truncates, or that a recorded index is out of range. The guard has",
    "    no type information, so it records every integer cast and every index expression.",
];

/// Method names whose call can panic.
const PANICKING_METHODS: &[&str] = &["expect", "unwrap"];

/// Macro names that panic by definition.
const PANICKING_MACROS: &[&str] = &["panic", "todo", "unimplemented", "unreachable"];

/// Primitive integer types a cast can truncate into.
const INTEGER_TYPES: &[&str] = &[
    "i128", "i16", "i32", "i64", "i8", "isize", "u128", "u16", "u32", "u64", "u8", "usize",
];

/// How many characters of an expression are kept in a baseline entry.
const EXPRESSION_LIMIT: usize = 120;

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

    println!("panic ratchet");
    ratchet::print_scope(outcome.analysed, test_only.len(), baseline.len());
    println!("  panicking sites:   {}", outcome.sites);
    for (kind, count) in &outcome.by_kind {
        println!("    {kind}: {count}");
    }
    println!("  findings (unique): {}", outcome.findings.len());

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
    verdict.report("panics");
    Ok(!verdict.failed())
}

/// Everything one run of the guard learned.
#[derive(Default)]
struct Outcome {
    /// How many files were inspected.
    analysed: usize,
    /// How many panicking constructs were seen, before identical keys collapsed.
    sites: usize,
    /// Findings, keyed by path, expression and enclosing function.
    findings: BTreeSet<String>,
    /// The anchors the guard actually enumerated, for the ratchet comparison.
    anchors: BTreeSet<String>,
    /// Counts per kind of construct, for the summary.
    by_kind: BTreeMap<&'static str, usize>,
}

/// Collects every panicking construct in the non-test parts of `files`.
fn collect(files: &[SourceFile], test_only: &TestOnly) -> Outcome {
    let mut outcome = Outcome::default();
    for file in files {
        if test_only.contains(&file.rel) || is_non_library(&file.rel) {
            continue;
        }
        outcome.analysed += 1;
        outcome.anchors.insert(file.rel.clone());
        let mut collector = Collector {
            file,
            scope: Vec::new(),
            found: Vec::new(),
        };
        collector.visit_file(&file.ast);
        for site in collector.found {
            outcome.sites += 1;
            *outcome.by_kind.entry(site.kind).or_insert(0) += 1;
            outcome
                .findings
                .insert(ratchet::key(&file.rel, &[&site.expression, &site.scope]));
        }
    }
    outcome
}

/// Returns true when a path is a test, an example or a benchmark rather than library code.
fn is_non_library(rel: &str) -> bool {
    rel.split('/')
        .any(|part| matches!(part, "tests" | "examples" | "benches"))
}

/// One collected panicking construct.
struct Site {
    /// The source text of the expression, normalised and shortened.
    expression: String,
    /// The enclosing function, as a path such as `DecodeError::new`.
    scope: String,
    /// Which kind of construct it is, for the summary.
    kind: &'static str,
}

/// Walks one file gathering panicking constructs, tracking the enclosing function.
struct Collector<'a> {
    /// The file being walked, for span resolution.
    file: &'a SourceFile,
    /// The module, type and function names enclosing the current position.
    scope: Vec<String>,
    /// The sites found so far.
    found: Vec<Site>,
}

impl Collector<'_> {
    /// Records one construct, resolving its text from the source and naming its function.
    fn push(&mut self, span: proc_macro2::Span, kind: &'static str) {
        let text = self.file.span_text(span);
        let expression = if text.is_empty() {
            format!("<{kind}>")
        } else {
            source::truncate(&text, EXPRESSION_LIMIT)
        };
        let scope = if self.scope.is_empty() {
            "<item initialiser>".to_owned()
        } else {
            self.scope.join("::")
        };
        self.found.push(Site {
            expression,
            scope,
            kind,
        });
    }

    /// Runs `body` with `name` pushed onto the scope stack.
    fn within<F: FnOnce(&mut Self)>(&mut self, name: String, body: F) {
        self.scope.push(name);
        body(self);
        self.scope.pop();
    }
}

impl<'ast> Visit<'ast> for Collector<'_> {
    /// Skips modules compiled only for tests, and names the others.
    fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
        if source::is_test_gated(&node.attrs) {
            return;
        }
        self.within(node.ident.to_string(), |this| {
            syn::visit::visit_item_mod(this, node);
        });
    }

    /// Skips test functions, and names the others.
    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        if source::is_test_gated(&node.attrs) {
            return;
        }
        self.within(node.sig.ident.to_string(), |this| {
            syn::visit::visit_item_fn(this, node);
        });
    }

    /// Names the type an implementation block is for, so a method key reads `Type::method`.
    fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
        if source::is_test_gated(&node.attrs) {
            return;
        }
        self.within(source::type_name(&node.self_ty), |this| {
            syn::visit::visit_item_impl(this, node);
        });
    }

    /// Skips test methods, and names the others.
    fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
        if source::is_test_gated(&node.attrs) {
            return;
        }
        self.within(node.sig.ident.to_string(), |this| {
            syn::visit::visit_impl_item_fn(this, node);
        });
    }

    /// Names a default method body inside a trait definition.
    fn visit_trait_item_fn(&mut self, node: &'ast syn::TraitItemFn) {
        if source::is_test_gated(&node.attrs) {
            return;
        }
        self.within(node.sig.ident.to_string(), |this| {
            syn::visit::visit_trait_item_fn(this, node);
        });
    }

    /// Records `unwrap()` and `expect(...)`.
    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        if PANICKING_METHODS.contains(&node.method.to_string().as_str()) {
            self.push(node.span(), "unwrap or expect");
        }
        syn::visit::visit_expr_method_call(self, node);
    }

    /// Records the macros that panic by definition.
    fn visit_macro(&mut self, node: &'ast syn::Macro) {
        if node
            .path
            .segments
            .last()
            .is_some_and(|segment| PANICKING_MACROS.contains(&segment.ident.to_string().as_str()))
        {
            self.push(node.span(), "panicking macro");
        }
        syn::visit::visit_macro(self, node);
    }

    /// Records indexing, which panics on an out-of-range index or a missing key.
    fn visit_expr_index(&mut self, node: &'ast syn::ExprIndex) {
        self.push(node.span(), "indexing");
        syn::visit::visit_expr_index(self, node);
    }

    /// Records integer casts, which truncate silently.
    fn visit_expr_cast(&mut self, node: &'ast syn::ExprCast) {
        if is_integer_type(&node.ty) && !is_integer_literal(&node.expr) {
            self.push(node.span(), "integer cast");
        }
        syn::visit::visit_expr_cast(self, node);
    }
}

/// Returns true when `ty` is one of the primitive integer types a cast can truncate into.
fn is_integer_type(ty: &syn::Type) -> bool {
    match ty {
        syn::Type::Path(path) => path
            .path
            .segments
            .last()
            .is_some_and(|segment| INTEGER_TYPES.contains(&segment.ident.to_string().as_str())),
        _ => false,
    }
}

/// Returns true when `expr` is an integer literal, whose value the compiler already range-checks.
fn is_integer_literal(expr: &syn::Expr) -> bool {
    match expr {
        syn::Expr::Lit(literal) => matches!(literal.lit, syn::Lit::Int(_)),
        syn::Expr::Unary(unary) => is_integer_literal(&unary.expr),
        syn::Expr::Paren(paren) => is_integer_literal(&paren.expr),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Loads one fixture as if it were a source file of a crate called `fx`.
    fn analyse(name: &str) -> Outcome {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("cannot read fixture {name}: {err}"));
        let files =
            vec![SourceFile::from_text(format!("crates/fx/src/{name}"), text).expect("parses")];
        let test_only = source::classify_test_only(&files);
        collect(&files, &test_only)
    }

    /// Returns the findings of one fixture as plain strings.
    fn findings(name: &str) -> Vec<String> {
        analyse(name).findings.into_iter().collect()
    }

    /// Every kind of panicking construct is collected, keyed by expression and function.
    #[test]
    fn collects_every_kind() {
        let outcome = analyse("panics_sites.rs");
        let kinds: Vec<&str> = outcome.by_kind.keys().copied().collect();
        assert_eq!(
            kinds,
            vec![
                "indexing",
                "integer cast",
                "panicking macro",
                "unwrap or expect"
            ]
        );
    }

    /// A finding names the function it sits in, and carries no line number.
    #[test]
    fn keys_by_function_not_by_line() {
        let found = findings("panics_sites.rs");
        assert!(
            found
                .iter()
                .any(|entry| entry.ends_with("\tself.rows[index]\tTable::cell")),
            "{found:#?}"
        );
        for entry in &found {
            assert_eq!(entry.matches('\t').count(), 2, "{entry}");
            assert!(!entry.contains(":1"), "{entry}");
        }
    }

    /// Casts to an integer type are recorded; casts of a literal, and casts to a float, are not.
    #[test]
    fn records_only_integer_casts_of_non_literals() {
        let found = findings("panics_casts.rs");
        assert!(found.iter().any(|entry| entry.contains("total as u32")));
        assert!(!found.iter().any(|entry| entry.contains("as f64")));
        assert!(!found.iter().any(|entry| entry.contains("7 as u8")));
    }

    /// Test modules, test functions, and files under `tests`, `examples` or `benches` are skipped.
    #[test]
    fn skips_test_code() {
        let outcome = analyse("panics_test_code.rs");
        assert!(outcome.findings.is_empty(), "{:#?}", outcome.findings);
        assert!(is_non_library("crates/toon/tests/roundtrip.rs"));
        assert!(is_non_library("crates/store/examples/bench.rs"));
        assert!(!is_non_library("crates/store/src/db.rs"));
    }

    /// `unwrap_or` and `expect_err` are different methods and are not recorded.
    #[test]
    fn similar_method_names_are_not_recorded() {
        let found = findings("panics_lookalikes.rs");
        assert!(found.is_empty(), "{found:#?}");
    }
}
