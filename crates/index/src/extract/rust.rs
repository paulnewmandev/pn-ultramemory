// SPDX-License-Identifier: Apache-2.0
//! Walker rules for Rust.
//!
//! # Role in the architecture
//! Implements [`Rules`] for the `tree-sitter-rust` grammar. Qualified names use `::`.
//!
//! # Conventions
//! * `impl` blocks are not symbols. They open a scope named after the implemented type: the
//!   methods inside become `Type::method` with the type symbol as parent when it is declared
//!   earlier in the same file, and `impl Trait for Type` reports `Trait` as an inheritance
//!   reference owned by the type.
//! * Any `pub` form makes a symbol public. Members of a public trait inherit its visibility.
//! * Documentation is the run of `///` lines or `/** */` blocks directly above the item (with
//!   attributes allowed in between); `//!` inner documentation belongs to the enclosing module and
//!   is not attached to items.

use pn_ultramemory_core::{RefKind, SymbolKind, Visibility};
use tree_sitter::Node;

use super::engine::{Decl, Role, Rules, ScopeKind, ScopeSpec, Walker};
use super::nodes::{has_child_kind, named_children};
use super::text::clean_comments;

/// The Rust rules.
pub(super) struct RustRules;

/// The shared instance of the Rust rules.
pub(super) static RUST: RustRules = RustRules;

/// Returns `true` for an outer documentation comment (`///` or `/** */`).
fn is_outer_doc(text: &str) -> bool {
    if let Some(rest) = text.strip_prefix("///") {
        return !rest.starts_with('/');
    }
    text.starts_with("/**") && !text.starts_with("/***") && text != "/**/"
}

/// Returns the visibility written on an item: `Public` for any `pub` form.
fn written_visibility(node: Node<'_>) -> Option<Visibility> {
    has_child_kind(node, "visibility_modifier").then_some(Visibility::Public)
}

/// Returns the identifier node that names the type an `impl` block is for.
fn impl_target(mut node: Node<'_>) -> Option<Node<'_>> {
    for _ in 0..16 {
        match node.kind() {
            "scoped_type_identifier" => node = node.child_by_field_name("name")?,
            "generic_type" | "reference_type" | "pointer_type" | "array_type" | "slice_type" => {
                node = node
                    .child_by_field_name("type")
                    .or_else(|| node.child_by_field_name("element"))?;
            }
            // A plain type identifier, or anything the rules do not look inside.
            _ => return Some(node),
        }
    }
    None
}

/// Joins two path segments with `::`, skipping empty ones.
fn join_path(prefix: &str, tail: &str) -> String {
    match (prefix.is_empty(), tail.is_empty()) {
        (true, _) => tail.to_owned(),
        (_, true) => prefix.to_owned(),
        _ => format!("{prefix}::{tail}"),
    }
}

/// Expands the argument of a `use` declaration into one path per imported item.
fn use_paths(walker: &Walker<'_, '_>, argument: Node<'_>) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![(argument, String::new())];
    while let Some((node, prefix)) = stack.pop() {
        if out.len() >= 256 {
            break;
        }
        match node.kind() {
            "scoped_use_list" => {
                let path = walker.field_text(node, "path").unwrap_or("");
                if let Some(list) = node.child_by_field_name("list") {
                    stack.push((list, join_path(&prefix, path)));
                }
            }
            "use_list" => {
                for child in named_children(node).into_iter().rev() {
                    stack.push((child, prefix.clone()));
                }
            }
            "use_as_clause" => {
                if let Some(path) = node.child_by_field_name("path") {
                    stack.push((path, prefix));
                }
            }
            "self" => {
                if !prefix.is_empty() {
                    out.push(prefix);
                }
            }
            _ => out.push(join_path(&prefix, walker.text(node))),
        }
    }
    out
}

impl RustRules {
    /// Declares an `impl` block as a virtual scope.
    fn declare_impl<'t>(walker: &mut Walker<'_, 't>, node: Node<'t>) {
        let target = node.child_by_field_name("type").and_then(impl_target);
        let name = target.map_or_else(String::new, |t| {
            super::text::collapse_whitespace(walker.text(t))
        });
        let resolved = walker
            .lookup(&walker.qualify(&name))
            .or_else(|| walker.lookup(&name))
            .filter(|i| {
                walker.symbol(*i).is_some_and(|s| {
                    matches!(
                        s.kind,
                        SymbolKind::Struct
                            | SymbolKind::Enum
                            | SymbolKind::Class
                            | SymbolKind::Interface
                            | SymbolKind::Type
                    )
                })
            });
        let (symbol, prefix) = match resolved {
            Some(i) => (
                Some(i),
                walker
                    .symbol(i)
                    .map_or_else(String::new, |s| s.qualified_name.clone()),
            ),
            None => (walker.scope().symbol, walker.qualify(&name)),
        };
        if let Some(t) = target {
            walker.suppress(t);
        }
        walker.push_virtual_scope(prefix, symbol, ScopeKind::Type, Visibility::Private);
    }

    /// Declares a function or a method.
    fn declare_function<'t>(walker: &mut Walker<'_, 't>, node: Node<'t>) {
        let Some(name) = node.child_by_field_name("name") else {
            return;
        };
        let scope_kind = walker.scope().kind;
        let kind = if scope_kind == ScopeKind::Type {
            SymbolKind::Method
        } else {
            SymbolKind::Function
        };
        let mut decl = Decl::new(node, walker.text(name), kind);
        decl.visibility = member_visibility(walker, node);
        decl.name_node = Some(name);
        decl.sig_end = node
            .child_by_field_name("body")
            .map_or_else(|| node.end_byte(), |b| b.start_byte());
        decl.scope = Some(ScopeSpec {
            kind: ScopeKind::Callable,
            access: Visibility::Private,
            open_ended: false,
        });
        walker.declare(decl);
    }

    /// Declares a struct, union, enum, trait, module, type alias, constant or macro.
    fn declare_item<'t>(walker: &mut Walker<'_, 't>, node: Node<'t>, kind: SymbolKind) {
        let Some(name) = node.child_by_field_name("name") else {
            return;
        };
        if matches!(node.kind(), "const_item" | "static_item")
            && walker.scope().kind == ScopeKind::Callable
        {
            return;
        }
        let visibility = member_visibility(walker, node);
        let mut decl = Decl::new(node, walker.text(name), kind);
        decl.visibility = visibility;
        decl.name_node = Some(name);
        decl.sig_end = match node.kind() {
            "macro_definition" => name.end_byte(),
            "struct_item" => match node.child_by_field_name("body") {
                Some(body) if body.kind() == "ordered_field_declaration_list" => node.end_byte(),
                Some(body) => body.start_byte(),
                None => node.end_byte(),
            },
            _ => node
                .child_by_field_name("body")
                .map_or_else(|| node.end_byte(), |b| b.start_byte()),
        };
        decl.scope = match node.kind() {
            "mod_item" => Some(ScopeSpec {
                kind: ScopeKind::Module,
                access: Visibility::Private,
                open_ended: false,
            }),
            "trait_item" => Some(ScopeSpec {
                kind: ScopeKind::Type,
                access: visibility,
                open_ended: false,
            }),
            "struct_item" | "union_item" | "enum_item" => Some(ScopeSpec {
                kind: ScopeKind::Type,
                access: Visibility::Private,
                open_ended: false,
            }),
            "const_item" | "static_item" | "type_item" => Some(ScopeSpec {
                kind: ScopeKind::Callable,
                access: Visibility::Private,
                open_ended: false,
            }),
            _ => None,
        };
        walker.declare(decl);
    }
}

/// Visibility of an item: `pub` forms are public, members of a public trait inherit it.
fn member_visibility(walker: &Walker<'_, '_>, node: Node<'_>) -> Visibility {
    written_visibility(node).unwrap_or_else(|| {
        let scope = walker.scope();
        if scope.kind == ScopeKind::Type && scope.access == Visibility::Public {
            Visibility::Public
        } else {
            Visibility::Private
        }
    })
}

/// Returns the node that names a callee and the qualifier written before it.
fn callee<'a, 't>(
    walker: &Walker<'a, 't>,
    function: Node<'t>,
) -> Option<(Node<'t>, Option<&'a str>)> {
    let mut node = function;
    for _ in 0..8 {
        match node.kind() {
            "identifier" => return Some((node, None)),
            "scoped_identifier" => {
                let name = node.child_by_field_name("name")?;
                // `Vec::<u8>::new` has a generic path: the qualifier is the type without its
                // arguments.
                let path = node.child_by_field_name("path").map(|p| {
                    if p.kind() == "generic_type" {
                        p.child_by_field_name("type").map_or("", |t| walker.text(t))
                    } else {
                        walker.text(p)
                    }
                });
                return Some((name, path));
            }
            "field_expression" => {
                let field = node.child_by_field_name("field")?;
                return Some((field, walker.field_text(node, "value")));
            }
            "generic_function" => node = node.child_by_field_name("function")?,
            _ => return None,
        }
    }
    None
}

impl Rules for RustRules {
    fn separator(&self) -> &'static str {
        "::"
    }

    fn role(&self, src: &str, node: Node<'_>) -> Role {
        match node.kind() {
            "attribute_item" => Role::Attribute,
            "line_comment" | "block_comment" => {
                if src.get(node.byte_range()).is_some_and(is_outer_doc) {
                    Role::Doc
                } else {
                    Role::Other
                }
            }
            _ => Role::Other,
        }
    }

    fn clean_doc(&self, comments: &[&str]) -> Option<String> {
        clean_comments(comments)
    }

    fn declare<'t>(&self, walker: &mut Walker<'_, 't>, node: Node<'t>) {
        match node.kind() {
            "function_item" | "function_signature_item" => Self::declare_function(walker, node),
            "struct_item" | "union_item" => Self::declare_item(walker, node, SymbolKind::Struct),
            "enum_item" => Self::declare_item(walker, node, SymbolKind::Enum),
            "trait_item" => Self::declare_item(walker, node, SymbolKind::Interface),
            "mod_item" => Self::declare_item(walker, node, SymbolKind::Module),
            "const_item" | "static_item" => Self::declare_item(walker, node, SymbolKind::Constant),
            "type_item" => Self::declare_item(walker, node, SymbolKind::Type),
            "macro_definition" => Self::declare_item(walker, node, SymbolKind::Macro),
            "impl_item" => Self::declare_impl(walker, node),
            _ => {}
        }
    }

    fn observe<'t>(&self, walker: &mut Walker<'_, 't>, node: Node<'t>) {
        match node.kind() {
            "call_expression" => {
                if let Some((name, qualifier)) = node
                    .child_by_field_name("function")
                    .and_then(|f| callee(walker, f))
                {
                    walker.reference(walker.text(name), RefKind::Call, name, qualifier);
                }
            }
            "macro_invocation" => {
                let Some(macro_name) = node.child_by_field_name("macro") else {
                    return;
                };
                if macro_name.kind() == "scoped_identifier" {
                    let name = macro_name.child_by_field_name("name");
                    let path = walker.field_text(macro_name, "path");
                    if let Some(name) = name {
                        walker.reference(walker.text(name), RefKind::Call, name, path);
                    }
                } else {
                    walker.reference(walker.text(macro_name), RefKind::Call, node, None);
                }
            }
            "use_declaration" => {
                if let Some(argument) = node.child_by_field_name("argument") {
                    for path in use_paths(walker, argument) {
                        walker.import(&path);
                    }
                }
            }
            "type_identifier" => observe_type(walker, node),
            _ => {}
        }
    }
}

/// Reports a type mention, or an inheritance reference when the node names a trait in an
/// `impl Trait for Type` header or in the supertrait list of a trait.
fn observe_type<'t>(walker: &mut Walker<'_, 't>, node: Node<'t>) {
    if walker.is_suppressed(node) {
        return;
    }
    let name = walker.text(node);
    if name.len() <= 1 || name == "Self" {
        return;
    }
    let parent = walker.ancestor(1);
    let parent_kind = parent.map_or("", |p| p.kind());
    let mut qualifier = None;
    let mut kind = RefKind::Type;
    let mut anchor_up = 1;
    if parent_kind == "scoped_type_identifier" && walker.field_of(0) == Some("name") {
        qualifier = parent.and_then(|p| walker.field_text(p, "path"));
        anchor_up = 2;
    } else if parent_kind == "generic_type" && walker.field_of(0) == Some("type") {
        anchor_up = 2;
    }
    let anchor = walker.ancestor(anchor_up);
    let anchor_field = walker.field_of(anchor_up.saturating_sub(1));
    match anchor.map(|a| a.kind()) {
        Some("impl_item") if anchor_field == Some("trait") => kind = RefKind::Inherit,
        Some("trait_bounds")
            if walker
                .ancestor(anchor_up + 1)
                .is_some_and(|t| t.kind() == "trait_item") =>
        {
            kind = RefKind::Inherit;
        }
        _ => {}
    }
    walker.reference(name, kind, node, qualifier);
}
