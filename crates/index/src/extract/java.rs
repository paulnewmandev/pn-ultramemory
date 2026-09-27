// SPDX-License-Identifier: Apache-2.0
//! Walker rules for Java.
//!
//! # Role in the architecture
//! Implements [`Rules`] for the `tree-sitter-java` grammar. Qualified names use `.`, and the
//! `package` declaration does not create a symbol.
//!
//! # Conventions
//! * `public` makes a symbol public; anything else is private, except the members of an
//!   interface or annotation type, which are implicitly public.
//! * Records are reported as structs, annotation types as interfaces.
//! * Fields are listed only when they are constants (`static final`, or any field of an
//!   interface).
//! * Documentation is the Javadoc block directly above the declaration; annotations are not part
//!   of the signature.

use pn_ultramemory_core::{RefKind, SymbolKind, Visibility};
use tree_sitter::Node;

use super::engine::{Decl, Role, Rules, ScopeKind, ScopeSpec, Walker};
use super::nodes::{child_of_kind, has_child_kind, named_children, ranges_of_kinds};
use super::text::clean_comments;

/// The Java rules.
pub(super) struct JavaRules;

/// The shared instance of the Java rules.
pub(super) static JAVA: JavaRules = JavaRules;

/// Kinds of annotation nodes.
const ANNOTATIONS: &[&str] = &["marker_annotation", "annotation"];

/// Returns `true` for a Javadoc comment.
fn is_javadoc(text: &str) -> bool {
    text.starts_with("/**") && !text.starts_with("/***") && text != "/**/"
}

/// Returns the modifiers of a declaration.
fn modifiers(node: Node<'_>) -> Option<Node<'_>> {
    child_of_kind(node, "modifiers")
}

/// Returns `true` when the declaration carries the keyword `word`.
fn has_modifier(node: Node<'_>, word: &str) -> bool {
    modifiers(node).is_some_and(|m| has_child_kind(m, word))
}

/// Visibility of a member: `public`, or implicitly public inside an interface.
fn visibility_of(walker: &Walker<'_, '_>, node: Node<'_>) -> Visibility {
    let scope = walker.scope();
    if has_modifier(node, "public")
        || (scope.kind == ScopeKind::Type && scope.access == Visibility::Public)
    {
        Visibility::Public
    } else {
        Visibility::Private
    }
}

/// The annotation ranges of a declaration, to be dropped from its signature.
fn annotation_ranges(node: Node<'_>) -> Vec<std::ops::Range<usize>> {
    modifiers(node).map_or_else(Vec::new, |m| ranges_of_kinds(m, ANNOTATIONS))
}

impl JavaRules {
    /// Declares a type: class, interface, enum, record or annotation type.
    fn declare_type<'t>(walker: &mut Walker<'_, 't>, node: Node<'t>, kind: SymbolKind) {
        let Some(name) = node.child_by_field_name("name") else {
            return;
        };
        let interface_like = matches!(
            node.kind(),
            "interface_declaration" | "annotation_type_declaration"
        );
        let mut decl = Decl::new(node, walker.text(name), kind);
        decl.visibility = visibility_of(walker, node);
        decl.name_node = Some(name);
        decl.sig_end = node
            .child_by_field_name("body")
            .map_or_else(|| node.end_byte(), |b| b.start_byte());
        decl.sig_skip = annotation_ranges(node);
        decl.scope = Some(ScopeSpec {
            kind: ScopeKind::Type,
            access: if interface_like {
                Visibility::Public
            } else {
                Visibility::Private
            },
            open_ended: false,
        });
        walker.declare(decl);
    }

    /// Declares a method or a constructor.
    fn declare_method<'t>(walker: &mut Walker<'_, 't>, node: Node<'t>) {
        let Some(name) = node.child_by_field_name("name") else {
            return;
        };
        let mut decl = Decl::new(node, walker.text(name), SymbolKind::Method);
        decl.visibility = visibility_of(walker, node);
        decl.name_node = Some(name);
        decl.sig_end = node
            .child_by_field_name("body")
            .map_or_else(|| node.end_byte(), |b| b.start_byte());
        decl.sig_skip = annotation_ranges(node);
        decl.scope = Some(ScopeSpec {
            kind: ScopeKind::Callable,
            access: Visibility::Private,
            open_ended: false,
        });
        walker.declare(decl);
    }

    /// Declares the constants of a field declaration.
    fn declare_field<'t>(walker: &mut Walker<'_, 't>, node: Node<'t>) {
        let in_interface =
            walker.scope().kind == ScopeKind::Type && walker.scope().access == Visibility::Public;
        let constant =
            in_interface || (has_modifier(node, "static") && has_modifier(node, "final"));
        if !constant || walker.scope().kind != ScopeKind::Type {
            return;
        }
        let mut cursor = node.walk();
        let declarators: Vec<Node<'t>> = node
            .children_by_field_name("declarator", &mut cursor)
            .collect();
        let visibility = visibility_of(walker, node);
        let single = declarators.len() == 1;
        for declarator in declarators {
            let Some(name) = declarator.child_by_field_name("name") else {
                continue;
            };
            let mut decl = Decl::new(node, walker.text(name), SymbolKind::Constant);
            decl.visibility = visibility;
            decl.name_node = Some(name);
            decl.sig_skip = annotation_ranges(node);
            if single {
                decl.scope = Some(ScopeSpec {
                    kind: ScopeKind::Callable,
                    access: Visibility::Private,
                    open_ended: false,
                });
            }
            walker.declare(decl);
        }
    }
}

/// Reports every type named in a `superclass`, `super_interfaces` or `extends_interfaces` node.
fn report_supertypes<'t>(walker: &mut Walker<'_, 't>, node: Node<'t>) {
    let mut pending = named_children(node);
    pending.reverse();
    while let Some(child) = pending.pop() {
        match child.kind() {
            "type_list" => {
                let mut inner = named_children(child);
                inner.reverse();
                pending.extend(inner);
            }
            "type_identifier" => {
                walker.suppress(child);
                walker.reference(walker.text(child), RefKind::Inherit, child, None);
            }
            "generic_type" | "scoped_type_identifier" => {
                if let Some(name) = last_type_identifier(child) {
                    walker.suppress(name);
                    walker.reference(walker.text(name), RefKind::Inherit, name, None);
                }
            }
            _ => {}
        }
    }
}

/// Returns the last `type_identifier` among the direct children of a type node, looking through
/// a generic type's own type.
fn last_type_identifier(node: Node<'_>) -> Option<Node<'_>> {
    let mut node = node;
    for _ in 0..8 {
        match node.kind() {
            "type_identifier" => return Some(node),
            "generic_type" => node = named_children(node).into_iter().next()?,
            "scoped_type_identifier" => node = named_children(node).into_iter().last()?,
            _ => return None,
        }
    }
    None
}

impl Rules for JavaRules {
    fn separator(&self) -> &'static str {
        "."
    }

    fn role(&self, src: &str, node: Node<'_>) -> Role {
        if node.kind() == "block_comment" && src.get(node.byte_range()).is_some_and(is_javadoc) {
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
            "class_declaration" => Self::declare_type(walker, node, SymbolKind::Class),
            "interface_declaration" | "annotation_type_declaration" => {
                Self::declare_type(walker, node, SymbolKind::Interface);
            }
            "enum_declaration" => Self::declare_type(walker, node, SymbolKind::Enum),
            "record_declaration" => Self::declare_type(walker, node, SymbolKind::Struct),
            "method_declaration"
            | "constructor_declaration"
            | "compact_constructor_declaration" => {
                Self::declare_method(walker, node);
            }
            "field_declaration" | "constant_declaration" => Self::declare_field(walker, node),
            _ => {}
        }
    }

    fn observe<'t>(&self, walker: &mut Walker<'_, 't>, node: Node<'t>) {
        match node.kind() {
            "method_invocation" => {
                if let Some(name) = node.child_by_field_name("name") {
                    let qualifier = walker.field_text(node, "object");
                    walker.reference(walker.text(name), RefKind::Call, name, qualifier);
                }
            }
            "superclass" | "super_interfaces" | "extends_interfaces" => {
                report_supertypes(walker, node);
            }
            "marker_annotation" | "annotation" => {
                if let Some(name) = node.child_by_field_name("name") {
                    let text = walker.text(name);
                    let simple = text.rsplit('.').next().unwrap_or(text);
                    walker.reference(simple, RefKind::Type, name, None);
                }
            }
            "import_declaration" => {
                let path = child_of_kind(node, "scoped_identifier")
                    .or_else(|| child_of_kind(node, "identifier"))
                    .map(|p| walker.text(p));
                if let Some(path) = path {
                    if has_child_kind(node, "asterisk") {
                        walker.import(&format!("{path}.*"));
                    } else {
                        walker.import(path);
                    }
                }
            }
            "type_identifier" => {
                if walker.is_suppressed(node) {
                    return;
                }
                let name = walker.text(node);
                if name.len() > 1 {
                    walker.reference(name, RefKind::Type, node, None);
                }
            }
            _ => {}
        }
    }
}
