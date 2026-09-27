// SPDX-License-Identifier: Apache-2.0
//! Walker rules for PHP.
//!
//! # Role in the architecture
//! Implements [`Rules`] for the `tree-sitter-php` grammar. Qualified names use `.` (namespaces
//! are separated by `.` in qualified names, not by the backslash of the source).
//!
//! # Conventions
//! * `private` and `protected` make a member private; everything else is public.
//! * Namespaces open a module scope; `namespace App;` without a body lasts until the end of the
//!   file.
//! * Traits are reported as interfaces; `use Trait;` inside a class is an inheritance reference.
//! * Constants are listed; properties are not.
//! * Documentation is the `PHPDoc` block directly above the declaration; attributes are not part
//!   of the signature.
//! * Imports are the `use` statements and the string argument of `require` and `include`.

use pn_ultramemory_core::{RefKind, SymbolKind, Visibility};
use tree_sitter::Node;

use super::engine::{Decl, Role, Rules, ScopeKind, ScopeSpec, Walker};
use super::nodes::{child_of_kind, child_of_kinds, named_children, ranges_of_kinds};
use super::text::clean_comments;

/// The PHP rules.
pub(super) struct PhpRules;

/// The shared instance of the PHP rules.
pub(super) static PHP: PhpRules = PhpRules;

/// Returns `true` for a `PHPDoc` comment.
fn is_phpdoc(text: &str) -> bool {
    text.starts_with("/**") && !text.starts_with("/***") && text != "/**/"
}

/// Visibility from the `visibility_modifier` child, `Public` when there is none.
fn visibility_of(walker: &Walker<'_, '_>, node: Node<'_>) -> Visibility {
    match child_of_kind(node, "visibility_modifier").map(|m| walker.text(m).trim()) {
        Some("private" | "protected") => Visibility::Private,
        _ => Visibility::Public,
    }
}

/// Returns the last segment of a name and the namespace written before it.
fn name_parts<'a>(walker: &Walker<'a, '_>, node: Node<'_>) -> (&'a str, Option<&'a str>) {
    match node.kind() {
        "qualified_name" => {
            let prefix = node.child_by_field_name("prefix");
            let text = walker.text(node);
            let name = text.rsplit('\\').next().unwrap_or(text);
            let prefix = prefix
                .map(|p| walker.text(p).trim_matches('\\'))
                .filter(|p| !p.is_empty());
            (name, prefix)
        }
        _ => (walker.text(node), None),
    }
}

impl PhpRules {
    /// Declares a class, interface, trait or enum.
    fn declare_type<'t>(walker: &mut Walker<'_, 't>, node: Node<'t>, kind: SymbolKind) {
        let Some(name) = node.child_by_field_name("name") else {
            return;
        };
        let mut decl = Decl::new(node, walker.text(name), kind);
        decl.visibility = Visibility::Public;
        decl.name_node = Some(name);
        decl.sig_end = node
            .child_by_field_name("body")
            .map_or_else(|| node.end_byte(), |b| b.start_byte());
        decl.sig_skip = ranges_of_kinds(node, &["attribute_list"]);
        decl.scope = Some(ScopeSpec {
            kind: ScopeKind::Type,
            access: Visibility::Public,
            open_ended: false,
        });
        walker.declare(decl);
    }

    /// Declares a function or a method.
    fn declare_function<'t>(walker: &mut Walker<'_, 't>, node: Node<'t>) {
        let Some(name) = node.child_by_field_name("name") else {
            return;
        };
        let method = node.kind() == "method_declaration";
        let mut decl = Decl::new(
            node,
            walker.text(name),
            if method {
                SymbolKind::Method
            } else {
                SymbolKind::Function
            },
        );
        decl.visibility = visibility_of(walker, node);
        decl.name_node = Some(name);
        decl.sig_end = node
            .child_by_field_name("body")
            .map_or_else(|| node.end_byte(), |b| b.start_byte());
        decl.sig_skip = ranges_of_kinds(node, &["attribute_list"]);
        decl.scope = Some(ScopeSpec {
            kind: ScopeKind::Callable,
            access: Visibility::Public,
            open_ended: false,
        });
        walker.declare(decl);
    }

    /// Declares a namespace, with or without a body.
    fn declare_namespace<'t>(walker: &mut Walker<'_, 't>, node: Node<'t>) {
        let Some(name) = node.child_by_field_name("name") else {
            return;
        };
        let body = node.child_by_field_name("body");
        let written = walker.text(name);
        let simple = written.rsplit('\\').next().unwrap_or(written);
        let mut decl = Decl::new(node, simple, SymbolKind::Module);
        decl.visibility = Visibility::Public;
        decl.name_node = Some(name);
        decl.qualified = Some(walker.qualify(&written.replace('\\', ".")));
        decl.sig_end = body.map_or_else(|| node.end_byte(), |b| b.start_byte());
        decl.scope = Some(ScopeSpec {
            kind: ScopeKind::Module,
            access: Visibility::Public,
            open_ended: body.is_none(),
        });
        if body.is_none() {
            decl.end_override = Some(walker.src.len());
        }
        walker.declare(decl);
    }

    /// Declares the constants of a `const` declaration.
    fn declare_const<'t>(walker: &mut Walker<'_, 't>, node: Node<'t>) {
        if walker.scope().kind == ScopeKind::Callable {
            return;
        }
        let visibility = visibility_of(walker, node);
        let elements: Vec<Node<'t>> = named_children(node)
            .into_iter()
            .filter(|c| c.kind() == "const_element")
            .collect();
        for element in elements {
            let Some(name) = child_of_kind(element, "name") else {
                continue;
            };
            let mut decl = Decl::new(node, walker.text(name), SymbolKind::Constant);
            decl.visibility = visibility;
            decl.name_node = Some(name);
            walker.declare(decl);
        }
    }
}

/// Reports the names of an `extends`, `implements` or trait `use` clause as inheritance.
fn report_inherit<'t>(walker: &mut Walker<'_, 't>, node: Node<'t>) {
    for child in named_children(node) {
        if matches!(child.kind(), "name" | "qualified_name") {
            let (name, qualifier) = name_parts(walker, child);
            walker.suppress(child);
            walker.reference(name, RefKind::Inherit, child, qualifier);
        }
    }
}

/// Reports the targets of a `use` statement, expanding a group.
fn report_use<'t>(walker: &mut Walker<'_, 't>, node: Node<'t>) {
    let prefix = child_of_kind(node, "namespace_name").map(|p| walker.text(p).to_owned());
    let mut clauses = Vec::new();
    for child in named_children(node) {
        match child.kind() {
            "namespace_use_clause" => clauses.push(child),
            "namespace_use_group" => clauses.extend(
                named_children(child)
                    .into_iter()
                    .filter(|c| c.kind() == "namespace_use_clause"),
            ),
            _ => {}
        }
    }
    for clause in clauses {
        let target = child_of_kinds(clause, &["qualified_name", "name"]);
        if let Some(target) = target {
            let text = walker.text(target);
            match &prefix {
                Some(prefix) => walker.import(&format!("{prefix}\\{text}")),
                None => walker.import(text),
            }
        }
    }
}

impl Rules for PhpRules {
    fn separator(&self) -> &'static str {
        "."
    }

    fn role(&self, src: &str, node: Node<'_>) -> Role {
        if node.kind() == "comment" && src.get(node.byte_range()).is_some_and(is_phpdoc) {
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
            "namespace_definition" => Self::declare_namespace(walker, node),
            "class_declaration" => Self::declare_type(walker, node, SymbolKind::Class),
            "interface_declaration" | "trait_declaration" => {
                Self::declare_type(walker, node, SymbolKind::Interface);
            }
            "enum_declaration" => Self::declare_type(walker, node, SymbolKind::Enum),
            "function_definition" | "method_declaration" => Self::declare_function(walker, node),
            "const_declaration" => Self::declare_const(walker, node),
            _ => {}
        }
    }

    fn observe<'t>(&self, walker: &mut Walker<'_, 't>, node: Node<'t>) {
        match node.kind() {
            "function_call_expression" => {
                if let Some(function) = node.child_by_field_name("function") {
                    if matches!(function.kind(), "name" | "qualified_name") {
                        let (name, qualifier) = name_parts(walker, function);
                        walker.reference(name, RefKind::Call, function, qualifier);
                    }
                }
            }
            "member_call_expression" | "nullsafe_member_call_expression" => {
                if let Some(name) = node.child_by_field_name("name") {
                    let qualifier = walker.field_text(node, "object");
                    walker.reference(walker.text(name), RefKind::Call, name, qualifier);
                }
            }
            "scoped_call_expression" => {
                if let Some(name) = node.child_by_field_name("name") {
                    let qualifier = walker.field_text(node, "scope");
                    walker.reference(walker.text(name), RefKind::Call, name, qualifier);
                }
            }
            "object_creation_expression" => {
                if let Some(target) = child_of_kinds(node, &["name", "qualified_name"]) {
                    let (name, qualifier) = name_parts(walker, target);
                    walker.suppress(target);
                    walker.reference(name, RefKind::Type, target, qualifier);
                }
            }
            "base_clause" | "class_interface_clause" | "use_declaration" => {
                report_inherit(walker, node);
            }
            "attribute" => {
                if let Some(target) = child_of_kinds(node, &["name", "qualified_name"]) {
                    let (name, qualifier) = name_parts(walker, target);
                    walker.reference(name, RefKind::Type, target, qualifier);
                }
            }
            "named_type" => {
                if let Some(target) = child_of_kinds(node, &["name", "qualified_name"]) {
                    let (name, qualifier) = name_parts(walker, target);
                    if !matches!(name, "self" | "static" | "parent") {
                        walker.reference(name, RefKind::Type, target, qualifier);
                    }
                }
            }
            "namespace_use_declaration" => report_use(walker, node),
            "require_expression"
            | "require_once_expression"
            | "include_expression"
            | "include_once_expression" => {
                if let Some(target) = node.named_child(0) {
                    if matches!(target.kind(), "string" | "encapsed_string") {
                        walker.import(walker.text(target).trim_matches(['"', '\'']));
                    }
                }
            }
            _ => {}
        }
    }
}
