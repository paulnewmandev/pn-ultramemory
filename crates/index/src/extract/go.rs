// SPDX-License-Identifier: Apache-2.0
//! Walker rules for Go.
//!
//! # Role in the architecture
//! Implements [`Rules`] for the `tree-sitter-go` grammar. Qualified names use `.`, and the
//! package clause does not create a symbol.
//!
//! # Conventions
//! * A name that starts with an uppercase letter is public; anything else is private.
//! * A method is qualified by its receiver type (`Circle.Area`) and its parent is the type when
//!   the type is declared earlier in the same file.
//! * Documentation is the run of `//` lines (or a `/* */` block) directly above the declaration;
//!   compiler directives such as `//go:generate` are not documentation.
//! * Embedded struct fields and embedded interfaces are reported as inheritance references.
//! * Constants and variables are listed only at package level.

use pn_ultramemory_core::{RefKind, SymbolKind, Visibility};
use tree_sitter::Node;

use super::engine::{Decl, ParentRule, Role, Rules, ScopeKind, ScopeSpec, Walker};
use super::nodes::{child_of_kind, has_child_kind, named_children};
use super::text::clean_comments;

/// The Go rules.
pub(super) struct GoRules;

/// The shared instance of the Go rules.
pub(super) static GO: GoRules = GoRules;

/// Predeclared identifiers that are not reported as type mentions or calls.
const PREDECLARED: &[&str] = &[
    "bool",
    "byte",
    "complex64",
    "complex128",
    "error",
    "float32",
    "float64",
    "int",
    "int8",
    "int16",
    "int32",
    "int64",
    "rune",
    "string",
    "uint",
    "uint8",
    "uint16",
    "uint32",
    "uint64",
    "uintptr",
    "any",
    "comparable",
    "make",
    "new",
    "len",
    "cap",
    "append",
    "copy",
    "delete",
    "close",
    "panic",
    "recover",
    "print",
    "println",
    "complex",
    "real",
    "imag",
    "min",
    "max",
    "clear",
];

/// Returns `true` for a documentation comment: a plain `//` or `/* */` comment that is not a
/// compiler directive.
fn is_doc_comment(text: &str) -> bool {
    if let Some(rest) = text.strip_prefix("//") {
        let word: String = rest
            .chars()
            .take_while(|c| c.is_ascii_lowercase() || *c == '_')
            .collect();
        let directive = !word.is_empty() && rest[word.len()..].starts_with(':');
        return !directive && !rest.starts_with("line ");
    }
    text.starts_with("/*")
}

/// Visibility of a Go name: exported names start with an uppercase letter.
fn name_visibility(name: &str) -> Visibility {
    if name.chars().next().is_some_and(char::is_uppercase) {
        Visibility::Public
    } else {
        Visibility::Private
    }
}

/// Returns the identifier node of a type expression, looking through pointers and generics.
fn base_type_node(mut node: Node<'_>) -> Option<Node<'_>> {
    for _ in 0..8 {
        match node.kind() {
            "type_identifier" => return Some(node),
            "pointer_type" | "parenthesized_type" => node = node.named_child(0)?,
            "generic_type" => node = node.child_by_field_name("type")?,
            "qualified_type" => return node.child_by_field_name("name"),
            _ => return None,
        }
    }
    None
}

/// Returns the receiver type node of a method declaration.
fn receiver_type(receiver: Node<'_>) -> Option<Node<'_>> {
    let parameter = child_of_kind(receiver, "parameter_declaration")?;
    base_type_node(parameter.child_by_field_name("type")?)
}

impl GoRules {
    /// Declares a function.
    fn declare_function<'t>(walker: &mut Walker<'_, 't>, node: Node<'t>) {
        let Some(name) = node.child_by_field_name("name") else {
            return;
        };
        let text = walker.text(name);
        let mut decl = Decl::new(node, text, SymbolKind::Function);
        decl.visibility = name_visibility(text);
        decl.name_node = Some(name);
        decl.sig_end = node
            .child_by_field_name("body")
            .map_or_else(|| node.end_byte(), |b| b.start_byte());
        decl.scope = Some(callable_scope());
        walker.declare(decl);
    }

    /// Declares a method, qualified by its receiver type.
    fn declare_method<'t>(walker: &mut Walker<'_, 't>, node: Node<'t>) {
        let Some(name) = node.child_by_field_name("name") else {
            return;
        };
        let text = walker.text(name);
        let receiver = node
            .child_by_field_name("receiver")
            .and_then(receiver_type)
            .map(|t| walker.text(t));
        let mut decl = Decl::new(node, text, SymbolKind::Method);
        decl.visibility = name_visibility(text);
        decl.name_node = Some(name);
        decl.sig_end = node
            .child_by_field_name("body")
            .map_or_else(|| node.end_byte(), |b| b.start_byte());
        decl.scope = Some(callable_scope());
        if let Some(receiver) = receiver {
            let type_qualified = walker.qualify(receiver);
            decl.qualified = Some(format!("{type_qualified}.{text}"));
            decl.parent = ParentRule::Explicit(walker.lookup(&type_qualified).filter(|i| {
                walker
                    .symbol(*i)
                    .is_some_and(|s| s.kind != SymbolKind::Function && s.kind != SymbolKind::Method)
            }));
        }
        walker.declare(decl);
    }

    /// Declares a type specification (`type Name ...`).
    fn declare_type<'t>(walker: &mut Walker<'_, 't>, node: Node<'t>) {
        let Some(name) = node.child_by_field_name("name") else {
            return;
        };
        let text = walker.text(name);
        let underlying = node.child_by_field_name("type");
        let (kind, sig_end) = match underlying.map(|t| t.kind()) {
            Some("struct_type") => (
                SymbolKind::Struct,
                underlying
                    .and_then(|t| child_of_kind(t, "field_declaration_list"))
                    .map_or(node.end_byte(), |b| b.start_byte()),
            ),
            Some("interface_type") => (
                SymbolKind::Interface,
                underlying.map_or(node.end_byte(), |t| {
                    walker
                        .text(t)
                        .find('{')
                        .map_or(node.end_byte(), |i| t.start_byte() + i)
                }),
            ),
            _ => (SymbolKind::Type, node.end_byte()),
        };
        let mut decl = Decl::new(node, text, kind);
        decl.visibility = name_visibility(text);
        decl.name_node = Some(name);
        decl.sig_end = sig_end;
        decl.scope = Some(ScopeSpec {
            kind: if matches!(kind, SymbolKind::Type) {
                ScopeKind::Callable
            } else {
                ScopeKind::Type
            },
            access: Visibility::Public,
            open_ended: false,
        });
        if let Some(parent) = walker.ancestor(1) {
            if parent.kind() == "type_declaration" && is_single_spec(parent) {
                decl.outer = parent;
                decl.outer_up = 1;
                decl.sig_start = Some(parent.start_byte());
            }
        }
        walker.declare(decl);
    }

    /// Declares the names of a constant or variable specification.
    fn declare_value_spec<'t>(walker: &mut Walker<'_, 't>, node: Node<'t>) {
        if walker.scope().kind == ScopeKind::Callable {
            return;
        }
        let kind = if node.kind() == "const_spec" {
            SymbolKind::Constant
        } else {
            SymbolKind::Variable
        };
        let mut cursor = node.walk();
        let names: Vec<Node<'t>> = node
            .children_by_field_name("name", &mut cursor)
            .filter(|n| n.kind() == "identifier")
            .collect();
        let single_form = walker
            .ancestor(1)
            .filter(|d| matches!(d.kind(), "const_declaration" | "var_declaration"))
            .filter(|d| is_single_spec(*d));
        let several = names.len() > 1;
        for name in names {
            let text = walker.text(name);
            if text == "_" {
                continue;
            }
            let mut decl = Decl::new(node, text, kind);
            decl.visibility = name_visibility(text);
            decl.name_node = Some(name);
            if let Some(declaration) = single_form {
                decl.outer = declaration;
                decl.outer_up = 1;
            }
            decl.sig_start = Some(decl.outer.start_byte());
            decl.sig_end = decl.outer.end_byte();
            if !several {
                decl.scope = Some(callable_scope());
            }
            walker.declare(decl);
        }
    }

    /// Declares a method of an interface.
    fn declare_interface_method<'t>(walker: &mut Walker<'_, 't>, node: Node<'t>) {
        let Some(name) = node.child_by_field_name("name") else {
            return;
        };
        let text = walker.text(name);
        let mut decl = Decl::new(node, text, SymbolKind::Method);
        decl.visibility = name_visibility(text);
        decl.name_node = Some(name);
        walker.declare(decl);
    }
}

/// The scope opened by a function body.
fn callable_scope() -> ScopeSpec {
    ScopeSpec {
        kind: ScopeKind::Callable,
        access: Visibility::Public,
        open_ended: false,
    }
}

/// Returns `true` for a declaration written without parentheses and with one specification.
fn is_single_spec(declaration: Node<'_>) -> bool {
    !has_child_kind(declaration, "(")
}

impl Rules for GoRules {
    fn separator(&self) -> &'static str {
        "."
    }

    fn role(&self, src: &str, node: Node<'_>) -> Role {
        if node.kind() == "comment" && src.get(node.byte_range()).is_some_and(is_doc_comment) {
            Role::Doc
        } else {
            Role::Other
        }
    }

    fn clean_doc(&self, comments: &[&str]) -> Option<String> {
        clean_comments(comments)
    }

    fn declare<'t>(&self, walker: &mut Walker<'_, 't>, node: Node<'t>) {
        match node.kind() {
            "function_declaration" => Self::declare_function(walker, node),
            "method_declaration" => Self::declare_method(walker, node),
            "type_spec" | "type_alias" => Self::declare_type(walker, node),
            "const_spec" | "var_spec" => Self::declare_value_spec(walker, node),
            "method_elem" => Self::declare_interface_method(walker, node),
            _ => {}
        }
    }

    fn observe<'t>(&self, walker: &mut Walker<'_, 't>, node: Node<'t>) {
        match node.kind() {
            "call_expression" => {
                let Some(function) = node.child_by_field_name("function") else {
                    return;
                };
                match function.kind() {
                    "identifier" => {
                        let name = walker.text(function);
                        if !PREDECLARED.contains(&name) {
                            walker.reference(name, RefKind::Call, function, None);
                        }
                    }
                    "selector_expression" => {
                        if let Some(field) = function.child_by_field_name("field") {
                            let q = walker.field_text(function, "operand");
                            walker.reference(walker.text(field), RefKind::Call, field, q);
                        }
                    }
                    _ => {}
                }
            }
            "field_declaration" => {
                if node.child_by_field_name("name").is_none() {
                    if let Some(ty) = node.child_by_field_name("type") {
                        report_embedded(walker, ty);
                    }
                }
            }
            "type_elem" => {
                for child in named_children(node) {
                    report_embedded(walker, child);
                }
            }
            "import_spec" => {
                if let Some(path) = node.child_by_field_name("path") {
                    walker.import(walker.text(path).trim_matches(['"', '`']));
                }
            }
            "type_identifier" => observe_type(walker, node),
            _ => {}
        }
    }
}

/// Reports an embedded type as an inheritance reference.
fn report_embedded<'t>(walker: &mut Walker<'_, 't>, node: Node<'t>) {
    let Some(name) = base_type_node(node) else {
        return;
    };
    let text = walker.text(name);
    if PREDECLARED.contains(&text) {
        return;
    }
    let qualifier = name
        .parent()
        .filter(|p| p.kind() == "qualified_type")
        .and_then(|p| walker.field_text(p, "package"));
    walker.suppress(name);
    walker.reference(text, RefKind::Inherit, name, qualifier);
}

/// Reports a type mention.
fn observe_type<'t>(walker: &mut Walker<'_, 't>, node: Node<'t>) {
    if walker.is_suppressed(node) {
        return;
    }
    let name = walker.text(node);
    if name.len() <= 1 || PREDECLARED.contains(&name) {
        return;
    }
    let qualifier = walker
        .ancestor(1)
        .filter(|p| p.kind() == "qualified_type" && walker.field_of(0) == Some("name"))
        .and_then(|p| walker.field_text(p, "package"));
    walker.reference(name, RefKind::Type, node, qualifier);
}
