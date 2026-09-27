// SPDX-License-Identifier: Apache-2.0
//! Walker rules for JavaScript, TypeScript and TSX.
//!
//! # Role in the architecture
//! One [`Rules`] implementation serves the three grammars: the TypeScript grammar only adds node
//! kinds, so the same `match` arms recognise both. Qualified names use `.`.
//!
//! # Conventions
//! * Visibility: `export` makes a declaration public; a top-level or namespace-level declaration
//!   that is not exported is private; members of a class or interface are public unless they
//!   are marked `private` or named with `#`. Nothing declared inside a function is public.
//! * Documentation is a `/** ... */` comment directly above the declaration (or above its
//!   `export` statement).
//! * `const NAME = <function>` and `let/var name = <function>` are functions; other top-level
//!   `const` declarations are constants and `let`/`var` declarations are variables. `require`
//!   calls are imports, not declarations.
//! * Decorators are not part of the signature.

use pn_ultramemory_core::{RefKind, SymbolKind, Visibility};
use tree_sitter::Node;

use super::engine::{Decl, Role, Rules, ScopeKind, ScopeSpec, Walker};
use super::nodes::{child_of_kind, has_child_kind, named_children, ranges_of_kinds};
use super::text::clean_comments;

/// The JavaScript and TypeScript rules.
pub(super) struct EcmaRules {
    /// Whether the grammar is TypeScript or TSX (it recognises type annotations).
    typescript: bool,
}

/// The shared instance of the JavaScript rules.
pub(super) static JAVASCRIPT: EcmaRules = EcmaRules { typescript: false };

/// The shared instance of the TypeScript and TSX rules.
pub(super) static TYPESCRIPT: EcmaRules = EcmaRules { typescript: true };

/// Node kinds that wrap a declaration and may carry its documentation comment.
const WRAPPERS: &[&str] = &[
    "export_statement",
    "ambient_declaration",
    "expression_statement",
];

/// Returns `true` for a documentation comment (`/** ... */`).
fn is_jsdoc(text: &str) -> bool {
    text.starts_with("/**") && !text.starts_with("/***") && text != "/**/"
}

/// Removes the quotes around a string literal.
fn unquote(text: &str) -> &str {
    text.trim().trim_matches(['"', '\'', '`'])
}

/// The wrapper nodes above the entered node and whether one of them is an `export`.
struct Wrapping<'t> {
    /// How many levels above the entered node the outermost wrapper is.
    up: usize,
    /// The outermost wrapper (or the entered node when there is none).
    outer: Node<'t>,
    /// Whether an `export` statement wraps the declaration.
    exported: bool,
    /// Decorators that sit on the wrapper instead of on the declaration.
    decorators: Vec<std::ops::Range<usize>>,
}

/// Looks at the nodes above `node` that belong to the same declaration.
///
/// `base_up` is how many levels above the entered node `node` sits (`0` for a function or class,
/// `1` for the statement above a variable declarator).
fn wrapping<'t>(walker: &Walker<'_, 't>, node: Node<'t>, base_up: usize) -> Wrapping<'t> {
    let mut extra = 0;
    let mut outer = node;
    let mut exported = false;
    let mut decorators = Vec::new();
    while extra < 3 {
        let Some(parent) = walker.ancestor(base_up + extra + 1) else {
            break;
        };
        if !WRAPPERS.contains(&parent.kind()) {
            break;
        }
        if parent.kind() == "expression_statement" && node.kind() != "internal_module" {
            break;
        }
        exported |= parent.kind() == "export_statement";
        decorators.extend(ranges_of_kinds(parent, &["decorator"]));
        outer = parent;
        extra += 1;
    }
    Wrapping {
        up: base_up + extra,
        outer,
        exported,
        decorators,
    }
}

/// Returns `true` when the node is a function-like value.
fn is_function_value(node: Node<'_>) -> bool {
    matches!(
        node.kind(),
        "arrow_function" | "function_expression" | "function" | "generator_function"
    )
}

impl EcmaRules {
    /// Builds the visibility of a top-level declaration.
    fn top_visibility(walker: &Walker<'_, '_>, exported: bool) -> Visibility {
        if exported && walker.scope().kind != ScopeKind::Callable {
            Visibility::Public
        } else {
            Visibility::Private
        }
    }

    /// Finishes and reports a declaration that may be wrapped by `export`.
    fn finish_wrapped<'t>(
        walker: &mut Walker<'_, 't>,
        node: Node<'t>,
        wrapping: &Wrapping<'t>,
        mut decl: Decl<'t>,
    ) {
        decl.outer = wrapping.outer;
        decl.outer_up = wrapping.up;
        decl.sig_start = Some(wrapping.outer.start_byte().min(node.start_byte()));
        decl.sig_skip.extend(wrapping.decorators.iter().cloned());
        decl.sig_skip.extend(ranges_of_kinds(node, &["decorator"]));
        walker.declare(decl);
    }

    /// Declares a function, class, interface, type alias, enum or namespace.
    fn declare_named<'t>(walker: &mut Walker<'_, 't>, node: Node<'t>, kind: SymbolKind) {
        let Some(name) = node.child_by_field_name("name") else {
            return;
        };
        let wrapping = wrapping(walker, node, 0);
        let text = unquote(walker.text(name));
        let mut decl = Decl::new(node, text, kind);
        decl.visibility = Self::top_visibility(walker, wrapping.exported);
        decl.name_node = Some(name);
        decl.sig_end = node
            .child_by_field_name("body")
            .map_or_else(|| node.end_byte(), |b| b.start_byte());
        decl.scope = Some(match kind {
            SymbolKind::Function => ScopeSpec {
                kind: ScopeKind::Callable,
                access: Visibility::Public,
                open_ended: false,
            },
            SymbolKind::Module => ScopeSpec {
                kind: ScopeKind::Module,
                access: Visibility::Public,
                open_ended: false,
            },
            _ => ScopeSpec {
                kind: ScopeKind::Type,
                access: Visibility::Public,
                open_ended: false,
            },
        });
        Self::finish_wrapped(walker, node, &wrapping, decl);
    }

    /// Declares a class that is exported behind decorators (`@Component(...) export class X`).
    ///
    /// The declaration is made when the walk enters the `export` statement, so that the calls in
    /// the decorators are owned by the class.
    fn declare_decorated_export<'t>(walker: &mut Walker<'_, 't>, export: Node<'t>) {
        let Some(class) = export.child_by_field_name("declaration") else {
            return;
        };
        if !matches!(
            class.kind(),
            "class_declaration" | "abstract_class_declaration"
        ) {
            return;
        }
        let Some(name) = class.child_by_field_name("name") else {
            return;
        };
        let mut decl = Decl::new(class, walker.text(name), SymbolKind::Class);
        decl.outer = export;
        decl.visibility = Self::top_visibility(walker, true);
        decl.name_node = Some(name);
        decl.sig_start = Some(export.start_byte());
        decl.sig_end = class
            .child_by_field_name("body")
            .map_or_else(|| class.end_byte(), |b| b.start_byte());
        decl.sig_skip = ranges_of_kinds(export, &["decorator"]);
        decl.sig_skip.extend(ranges_of_kinds(class, &["decorator"]));
        decl.scope = Some(ScopeSpec {
            kind: ScopeKind::Type,
            access: Visibility::Public,
            open_ended: false,
        });
        decl.covers.push(class);
        walker.declare(decl);
    }

    /// Declares a method of a class or an interface.
    fn declare_method<'t>(walker: &mut Walker<'_, 't>, node: Node<'t>) {
        let Some(name) = node.child_by_field_name("name") else {
            return;
        };
        if name.kind() == "computed_property_name" {
            return;
        }
        let text = unquote(walker.text(name));
        let mut decl = Decl::new(node, text, SymbolKind::Method);
        decl.visibility = member_visibility(walker, node, text);
        decl.name_node = Some(name);
        decl.sig_end = node
            .child_by_field_name("body")
            .map_or_else(|| node.end_byte(), |b| b.start_byte());
        decl.sig_skip = ranges_of_kinds(node, &["decorator"]);
        decl.scope = Some(ScopeSpec {
            kind: ScopeKind::Callable,
            access: Visibility::Public,
            open_ended: false,
        });
        walker.declare(decl);
    }

    /// Declares a class field whose value is a function as a method.
    fn declare_field<'t>(walker: &mut Walker<'_, 't>, node: Node<'t>) {
        let Some(value) = node.child_by_field_name("value") else {
            return;
        };
        if !is_function_value(value) {
            return;
        }
        let Some(name) = node
            .child_by_field_name("name")
            .or_else(|| node.child_by_field_name("property"))
        else {
            return;
        };
        let text = unquote(walker.text(name));
        let mut decl = Decl::new(node, text, SymbolKind::Method);
        decl.visibility = member_visibility(walker, node, text);
        decl.name_node = Some(name);
        decl.sig_end = value
            .child_by_field_name("body")
            .map_or_else(|| node.end_byte(), |b| b.start_byte());
        decl.sig_skip = ranges_of_kinds(node, &["decorator"]);
        decl.scope = Some(ScopeSpec {
            kind: ScopeKind::Callable,
            access: Visibility::Public,
            open_ended: false,
        });
        walker.declare(decl);
    }

    /// Declares one variable of a top-level `const`, `let` or `var` statement.
    fn declare_variable<'t>(walker: &mut Walker<'_, 't>, node: Node<'t>) {
        let Some(statement) = walker.ancestor(1) else {
            return;
        };
        if !matches!(
            statement.kind(),
            "lexical_declaration" | "variable_declaration"
        ) {
            return;
        }
        if matches!(walker.scope().kind, ScopeKind::Callable | ScopeKind::Type) {
            return;
        }
        let Some(name) = node.child_by_field_name("name") else {
            return;
        };
        let value = node.child_by_field_name("value");
        if name.kind() != "identifier" || value.is_some_and(is_require_call) {
            return;
        }
        let function = value.filter(|v| is_function_value(*v));
        let kind = if function.is_some() {
            SymbolKind::Function
        } else if walker.text(statement).starts_with("const") {
            SymbolKind::Constant
        } else {
            SymbolKind::Variable
        };
        let wrapping = wrapping(walker, statement, 1);
        let mut decl = Decl::new(node, walker.text(name), kind);
        decl.visibility = Self::top_visibility(walker, wrapping.exported);
        decl.name_node = Some(name);
        decl.scope = Some(ScopeSpec {
            kind: ScopeKind::Callable,
            access: Visibility::Public,
            open_ended: false,
        });
        if statement.named_child_count() == 1 {
            decl.outer = wrapping.outer;
            decl.outer_up = wrapping.up;
            decl.sig_start = Some(wrapping.outer.start_byte());
            decl.sig_skip.extend(wrapping.decorators.iter().cloned());
        }
        decl.sig_end = match function {
            Some(f) => f
                .child_by_field_name("body")
                .map_or_else(|| f.end_byte(), |b| b.start_byte()),
            None => decl.outer.end_byte(),
        };
        walker.declare(decl);
    }

    /// Declares `exports.name = function () {}` and `module.exports.name = () => {}`.
    fn declare_commonjs<'t>(walker: &mut Walker<'_, 't>, node: Node<'t>) {
        if walker.scope().kind != ScopeKind::File {
            return;
        }
        let Some(assignment) = child_of_kind(node, "assignment_expression") else {
            return;
        };
        let (Some(left), Some(right)) = (
            assignment.child_by_field_name("left"),
            assignment.child_by_field_name("right"),
        ) else {
            return;
        };
        if left.kind() != "member_expression" || !is_function_value(right) {
            return;
        }
        let object = walker.field_text(left, "object").unwrap_or("");
        let Some(property) = left.child_by_field_name("property") else {
            return;
        };
        if object != "exports" && object != "module.exports" {
            return;
        }
        let mut decl = Decl::new(node, walker.text(property), SymbolKind::Function);
        decl.visibility = Visibility::Public;
        decl.name_node = Some(property);
        decl.sig_end = right
            .child_by_field_name("body")
            .map_or_else(|| right.end_byte(), |b| b.start_byte());
        decl.scope = Some(ScopeSpec {
            kind: ScopeKind::Callable,
            access: Visibility::Public,
            open_ended: false,
        });
        walker.declare(decl);
    }
}

/// Returns `true` for `require('x')`.
fn is_require_call(node: Node<'_>) -> bool {
    node.kind() == "call_expression"
        && node
            .child_by_field_name("function")
            .is_some_and(|f| f.kind() == "identifier")
}

/// Visibility of a class or interface member.
fn member_visibility(walker: &Walker<'_, '_>, node: Node<'_>, name: &str) -> Visibility {
    if name.starts_with('#') {
        return Visibility::Private;
    }
    if let Some(modifier) = child_of_kind(node, "accessibility_modifier") {
        if walker.text(modifier).trim() == "private" {
            return Visibility::Private;
        }
    }
    Visibility::Public
}

impl Rules for EcmaRules {
    fn separator(&self) -> &'static str {
        "."
    }

    fn role(&self, src: &str, node: Node<'_>) -> Role {
        match node.kind() {
            "comment" if src.get(node.byte_range()).is_some_and(is_jsdoc) => Role::Doc,
            // A decorator that stands before a class member is a sibling of the member.
            "decorator" => Role::Attribute,
            _ => Role::Other,
        }
    }

    fn clean_doc(&self, comments: &[&str]) -> Option<String> {
        clean_comments(comments)
    }

    fn declare<'t>(&self, walker: &mut Walker<'_, 't>, node: Node<'t>) {
        match node.kind() {
            "function_declaration" | "generator_function_declaration" | "function_signature" => {
                EcmaRules::declare_named(walker, node, SymbolKind::Function);
            }
            "export_statement" if has_child_kind(node, "decorator") => {
                EcmaRules::declare_decorated_export(walker, node);
            }
            "class_declaration" | "abstract_class_declaration" => {
                EcmaRules::declare_named(walker, node, SymbolKind::Class);
            }
            "interface_declaration" => {
                EcmaRules::declare_named(walker, node, SymbolKind::Interface);
            }
            "type_alias_declaration" => EcmaRules::declare_named(walker, node, SymbolKind::Type),
            "enum_declaration" => EcmaRules::declare_named(walker, node, SymbolKind::Enum),
            "internal_module" | "module" => {
                EcmaRules::declare_named(walker, node, SymbolKind::Module);
            }
            "method_definition" | "method_signature" | "abstract_method_signature" => {
                EcmaRules::declare_method(walker, node);
            }
            "field_definition" | "public_field_definition" => {
                EcmaRules::declare_field(walker, node);
            }
            "variable_declarator" => EcmaRules::declare_variable(walker, node),
            "expression_statement" => EcmaRules::declare_commonjs(walker, node),
            _ => {}
        }
    }

    fn observe<'t>(&self, walker: &mut Walker<'_, 't>, node: Node<'t>) {
        match node.kind() {
            "call_expression" => observe_call(walker, node),
            "new_expression" => {
                if let Some(constructor) = node.child_by_field_name("constructor") {
                    report_expression(walker, constructor, RefKind::Type);
                }
            }
            "import_statement" | "export_statement" => {
                if let Some(source) = node.child_by_field_name("source") {
                    walker.import(unquote(walker.text(source)));
                } else if let Some(clause) = child_of_kind(node, "import_require_clause") {
                    if let Some(source) = clause.child_by_field_name("source") {
                        walker.import(unquote(walker.text(source)));
                    }
                }
            }
            "class_heritage" => {
                if child_of_kind(node, "extends_clause").is_none()
                    && child_of_kind(node, "implements_clause").is_none()
                {
                    for child in named_children(node) {
                        report_expression(walker, child, RefKind::Inherit);
                    }
                }
            }
            "extends_clause" => {
                if let Some(value) = node.child_by_field_name("value") {
                    report_expression(walker, value, RefKind::Inherit);
                }
            }
            "implements_clause" | "extends_type_clause" => {
                for child in named_children(node) {
                    report_type_node(walker, child, RefKind::Inherit);
                }
            }
            "decorator" => {
                if let Some(expression) = node.named_child(0) {
                    if expression.kind() == "identifier" {
                        report_expression(walker, expression, RefKind::Call);
                    }
                }
            }
            "jsx_opening_element" | "jsx_self_closing_element" => {
                if let Some(name) = node.child_by_field_name("name") {
                    let capitalized = walker
                        .text(name)
                        .chars()
                        .next()
                        .is_some_and(char::is_uppercase);
                    if capitalized {
                        report_expression(walker, name, RefKind::Type);
                    }
                }
            }
            "type_identifier" if self.typescript => observe_type_identifier(walker, node),
            _ => {}
        }
    }
}

/// Reports a call, a `require`, or a dynamic `import`.
fn observe_call<'t>(walker: &mut Walker<'_, 't>, node: Node<'t>) {
    let Some(function) = node.child_by_field_name("function") else {
        return;
    };
    match function.kind() {
        "identifier" => {
            let name = walker.text(function);
            if name == "require" {
                if let Some(arguments) = node.child_by_field_name("arguments") {
                    if let Some(first) = arguments.named_child(0) {
                        if is_plain_string(walker, first) {
                            walker.import(unquote(walker.text(first)));
                            return;
                        }
                    }
                }
            }
            walker.reference(name, RefKind::Call, function, None);
        }
        "import" => {
            if let Some(arguments) = node.child_by_field_name("arguments") {
                if let Some(first) = arguments.named_child(0) {
                    if is_plain_string(walker, first) {
                        walker.import(unquote(walker.text(first)));
                    }
                }
            }
        }
        "member_expression" => {
            if let Some(property) = function.child_by_field_name("property") {
                let qualifier = walker.field_text(function, "object");
                walker.reference(walker.text(property), RefKind::Call, property, qualifier);
            }
        }
        _ => {}
    }
}

/// Returns `true` for a string literal, or a template literal without substitutions.
fn is_plain_string(walker: &Walker<'_, '_>, node: Node<'_>) -> bool {
    match node.kind() {
        "string" => true,
        "template_string" => !walker.text(node).contains("${"),
        _ => false,
    }
}

/// Reports an identifier or a member expression (`a.B`) with the given kind.
fn report_expression<'t>(walker: &mut Walker<'_, 't>, node: Node<'t>, kind: RefKind) {
    match node.kind() {
        "identifier" | "type_identifier" => {
            walker.suppress(node);
            walker.reference(walker.text(node), kind, node, None);
        }
        "member_expression" | "nested_identifier" | "nested_type_identifier" => {
            let name = node
                .child_by_field_name("property")
                .or_else(|| node.child_by_field_name("name"));
            let object = node
                .child_by_field_name("object")
                .or_else(|| node.child_by_field_name("module"));
            if let Some(name) = name {
                walker.suppress(name);
                let qualifier = object.map(|o| walker.text(o));
                walker.reference(walker.text(name), kind, name, qualifier);
            }
        }
        "call_expression" => {
            if let Some(function) = node.child_by_field_name("function") {
                report_expression(walker, function, kind);
            }
        }
        _ => {}
    }
}

/// Reports a type node of an `implements` or interface `extends` list.
fn report_type_node<'t>(walker: &mut Walker<'_, 't>, node: Node<'t>, kind: RefKind) {
    match node.kind() {
        "generic_type" => {
            if let Some(name) = node.child_by_field_name("name") {
                report_expression(walker, name, kind);
            }
        }
        _ => report_expression(walker, node, kind),
    }
}

/// Reports a type mention.
fn observe_type_identifier<'t>(walker: &mut Walker<'_, 't>, node: Node<'t>) {
    if walker.is_suppressed(node) {
        return;
    }
    let name = walker.text(node);
    if name.len() <= 1 {
        return;
    }
    let mut qualifier = None;
    if let Some(parent) = walker.ancestor(1) {
        match parent.kind() {
            "nested_type_identifier" if walker.field_of(0) == Some("name") => {
                qualifier = walker.field_text(parent, "module");
            }
            "type_parameter" if walker.field_of(0) == Some("name") => return,
            _ => {}
        }
    }
    walker.reference(name, RefKind::Type, node, qualifier);
}
