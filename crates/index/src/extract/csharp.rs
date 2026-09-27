// SPDX-License-Identifier: Apache-2.0
//! Walker rules for C#.
//!
//! # Role in the architecture
//! Implements [`Rules`] for the `tree-sitter-c-sharp` grammar. Qualified names use `.`.
//!
//! # Conventions
//! * `public` makes a symbol public; anything else is private, except the members of an
//!   interface, which are implicitly public.
//! * Block namespaces open a module scope; a file-scoped `namespace X;` opens a scope that lasts
//!   until the end of the file.
//! * Fields are listed only when they are constants (`const`, or `static readonly`); properties
//!   and events are not listed.
//! * Documentation is the run of `///` lines directly above the declaration; the text inside
//!   `<summary>` is the documentation, with XML entities unescaped.

use pn_ultramemory_core::{RefKind, SymbolKind, Visibility};
use tree_sitter::Node;

use super::engine::{Decl, Role, Rules, ScopeKind, ScopeSpec, Walker, for_each_descendant};
use super::nodes::{named_children, ranges_of_kinds};
use super::text::{clean_comments, xml_doc_text};

/// The C# rules.
pub(super) struct CSharpRules;

/// The shared instance of the C# rules.
pub(super) static CSHARP: CSharpRules = CSharpRules;

/// Returns `true` for an XML documentation comment (`///` or `/** */`).
fn is_doc_comment(text: &str) -> bool {
    if let Some(rest) = text.strip_prefix("///") {
        return !rest.starts_with('/');
    }
    text.starts_with("/**") && !text.starts_with("/***") && text != "/**/"
}

/// Returns `true` when the declaration carries the modifier `word`.
fn has_modifier(walker: &Walker<'_, '_>, node: Node<'_>, word: &str) -> bool {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .any(|c| c.kind() == "modifier" && walker.text(c).trim() == word)
}

/// Visibility of a member: `public`, or implicitly public inside an interface.
fn visibility_of(walker: &Walker<'_, '_>, node: Node<'_>) -> Visibility {
    let scope = walker.scope();
    if has_modifier(walker, node, "public")
        || (scope.kind == ScopeKind::Type && scope.access == Visibility::Public)
    {
        Visibility::Public
    } else {
        Visibility::Private
    }
}

impl CSharpRules {
    /// Declares a class, struct, interface, enum, record or delegate.
    fn declare_type<'t>(walker: &mut Walker<'_, 't>, node: Node<'t>, kind: SymbolKind) {
        let Some(name) = node.child_by_field_name("name") else {
            return;
        };
        let mut decl = Decl::new(node, walker.text(name), kind);
        decl.visibility = visibility_of(walker, node);
        decl.name_node = Some(name);
        decl.sig_end = node
            .child_by_field_name("body")
            .map_or_else(|| node.end_byte(), |b| b.start_byte());
        decl.sig_skip = ranges_of_kinds(node, &["attribute_list"]);
        if kind != SymbolKind::Type {
            decl.scope = Some(ScopeSpec {
                kind: ScopeKind::Type,
                access: if node.kind() == "interface_declaration" {
                    Visibility::Public
                } else {
                    Visibility::Private
                },
                open_ended: false,
            });
        }
        walker.declare(decl);
    }

    /// Declares a namespace, block-scoped or file-scoped.
    fn declare_namespace<'t>(walker: &mut Walker<'_, 't>, node: Node<'t>) {
        let Some(name) = node.child_by_field_name("name") else {
            return;
        };
        let file_scoped = node.kind() == "file_scoped_namespace_declaration";
        let mut decl = Decl::new(node, walker.text(name), SymbolKind::Module);
        decl.visibility = Visibility::Public;
        decl.name_node = Some(name);
        decl.sig_end = node
            .child_by_field_name("body")
            .map_or_else(|| node.end_byte(), |b| b.start_byte());
        decl.scope = Some(ScopeSpec {
            kind: ScopeKind::Module,
            access: Visibility::Private,
            open_ended: file_scoped,
        });
        if file_scoped {
            decl.end_override = Some(walker.src.len());
        }
        walker.declare(decl);
    }

    /// Declares a method, constructor, destructor or operator.
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
        decl.sig_skip = ranges_of_kinds(node, &["attribute_list"]);
        decl.scope = Some(ScopeSpec {
            kind: ScopeKind::Callable,
            access: Visibility::Private,
            open_ended: false,
        });
        walker.declare(decl);
    }

    /// Declares the constants of a field declaration.
    fn declare_field<'t>(walker: &mut Walker<'_, 't>, node: Node<'t>) {
        let constant = has_modifier(walker, node, "const")
            || (has_modifier(walker, node, "static") && has_modifier(walker, node, "readonly"));
        if !constant || walker.scope().kind != ScopeKind::Type {
            return;
        }
        let Some(declaration) = super::nodes::child_of_kind(node, "variable_declaration") else {
            return;
        };
        let visibility = visibility_of(walker, node);
        for declarator in named_children(declaration) {
            if declarator.kind() != "variable_declarator" {
                continue;
            }
            let Some(name) = declarator.child_by_field_name("name") else {
                continue;
            };
            let mut decl = Decl::new(node, walker.text(name), SymbolKind::Constant);
            decl.visibility = visibility;
            decl.name_node = Some(name);
            decl.sig_skip = ranges_of_kinds(node, &["attribute_list"]);
            walker.declare(decl);
        }
    }
}

/// Reports the names in a type expression: identifiers, qualified and generic names.
fn report_type<'t>(walker: &mut Walker<'_, 't>, node: Node<'t>, kind: RefKind) {
    let mut found: Vec<(Node<'t>, Option<&str>)> = Vec::new();
    let mut visit = |n: Node<'t>| match n.kind() {
        "identifier" => {
            found.push((n, None));
            false
        }
        "qualified_name" => {
            if let Some(name) = n.child_by_field_name("name") {
                found.push((name, walker.field_text(n, "qualifier")));
            }
            false
        }
        "predefined_type" => false,
        _ => true,
    };
    if matches!(node.kind(), "identifier" | "qualified_name") {
        if visit(node) {
            for_each_descendant(node, 64, visit);
        }
    } else {
        for_each_descendant(node, 128, visit);
    }
    for (name_node, qualifier) in found {
        let name = walker.text(name_node);
        if name.len() > 1 && !walker.is_suppressed(name_node) {
            if kind == RefKind::Inherit {
                walker.suppress(name_node);
            }
            walker.reference(name, kind, name_node, qualifier);
        }
    }
}

impl Rules for CSharpRules {
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
        clean_comments(comments).map(|text| xml_doc_text(&text))
    }

    fn declare<'t>(&self, walker: &mut Walker<'_, 't>, node: Node<'t>) {
        match node.kind() {
            "namespace_declaration" | "file_scoped_namespace_declaration" => {
                Self::declare_namespace(walker, node);
            }
            "class_declaration" | "record_declaration" => {
                Self::declare_type(walker, node, SymbolKind::Class);
            }
            "struct_declaration" | "record_struct_declaration" => {
                Self::declare_type(walker, node, SymbolKind::Struct);
            }
            "interface_declaration" => Self::declare_type(walker, node, SymbolKind::Interface),
            "enum_declaration" => Self::declare_type(walker, node, SymbolKind::Enum),
            "delegate_declaration" => Self::declare_type(walker, node, SymbolKind::Type),
            "method_declaration" | "constructor_declaration" | "destructor_declaration" => {
                Self::declare_method(walker, node);
            }
            "field_declaration" => Self::declare_field(walker, node),
            _ => {}
        }
    }

    fn observe<'t>(&self, walker: &mut Walker<'_, 't>, node: Node<'t>) {
        match node.kind() {
            "invocation_expression" => {
                let Some(function) = node.child_by_field_name("function") else {
                    return;
                };
                match function.kind() {
                    "identifier" => {
                        walker.reference(walker.text(function), RefKind::Call, function, None);
                    }
                    "generic_name" => {
                        if let Some(name) = named_children(function).into_iter().next() {
                            walker.reference(walker.text(name), RefKind::Call, name, None);
                        }
                    }
                    "member_access_expression" => {
                        if let Some(name) = function.child_by_field_name("name") {
                            let qualifier = walker.field_text(function, "expression");
                            let name = if name.kind() == "generic_name" {
                                named_children(name).into_iter().next().unwrap_or(name)
                            } else {
                                name
                            };
                            walker.reference(walker.text(name), RefKind::Call, name, qualifier);
                        }
                    }
                    _ => {}
                }
            }
            "base_list" => {
                for child in named_children(node) {
                    report_type(walker, child, RefKind::Inherit);
                }
            }
            "object_creation_expression"
            | "parameter"
            | "variable_declaration"
            | "property_declaration"
            | "typeof_expression"
            | "cast_expression"
            | "indexer_declaration"
            | "catch_declaration" => {
                if let Some(ty) = node.child_by_field_name("type") {
                    report_type(walker, ty, RefKind::Type);
                }
            }
            "method_declaration"
            | "delegate_declaration"
            | "local_function_statement"
            | "operator_declaration" => {
                if let Some(ty) = node.child_by_field_name("returns") {
                    report_type(walker, ty, RefKind::Type);
                }
            }
            "attribute" => {
                if let Some(name) = node.child_by_field_name("name") {
                    report_type(walker, name, RefKind::Type);
                }
            }
            "using_directive" => {
                if let Some(target) = named_children(node).into_iter().last() {
                    walker.import(walker.text(target));
                }
            }
            _ => {}
        }
    }
}
