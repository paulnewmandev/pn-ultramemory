// SPDX-License-Identifier: Apache-2.0
//! Walker rules for C and C++.
//!
//! # Role in the architecture
//! One [`Rules`] implementation serves both grammars; the C++ grammar only adds node kinds.
//! Qualified names use `::` for C++ and `.` for C (where nothing is nested).
//!
//! # Conventions
//! * Functions are listed both as definitions and as prototypes (a header is mostly
//!   prototypes). A function defined in a class is a method, and so is an out-of-class
//!   definition such as `void Shape::area()`, whose parent is `Shape` when the class is declared
//!   earlier in the file.
//! * A `static` function or variable at file or namespace level is private, and so is a class
//!   member after `private:` or `protected:`; everything else is public.
//! * `#define` macros with a value or parameters are macros; include guards are not listed.
//! * Structs, unions, classes and enums are listed when they have a body. An anonymous type
//!   takes the name of its `typedef`.
//! * Documentation is a Doxygen comment (`/** */`, `/*! */`, `///` or `//!`) directly above the
//!   declaration or above its `template<...>` header.
//! * `#include` targets are reported without their quotes or angle brackets.

use pn_ultramemory_core::{RefKind, SymbolKind, Visibility};
use tree_sitter::Node;

use super::engine::{Decl, ParentRule, Role, Rules, ScopeKind, ScopeSpec, Walker};
use super::nodes::named_children;
use super::text::clean_comments;

/// The C and C++ rules.
pub(super) struct CFamilyRules {
    /// Whether the grammar is C++.
    cpp: bool,
}

/// The shared instance of the C rules.
pub(super) static C: CFamilyRules = CFamilyRules { cpp: false };

/// The shared instance of the C++ rules.
pub(super) static CPP: CFamilyRules = CFamilyRules { cpp: true };

/// Returns `true` for a Doxygen documentation comment.
fn is_doc_comment(text: &str) -> bool {
    if let Some(rest) = text.strip_prefix("///") {
        return !rest.starts_with('/');
    }
    text.starts_with("//!")
        || text.starts_with("/*!")
        || (text.starts_with("/**") && !text.starts_with("/***") && text != "/**/")
}

/// What a declarator resolves to.
struct Resolved<'t> {
    /// The node holding the declared name.
    name: Node<'t>,
    /// The scope written before the name (`A::B` in `A::B::f`), if any.
    scope: Option<String>,
    /// Whether a function declarator was passed on the way to the name.
    is_function: bool,
    /// Whether the function declarator names the function directly (not a function pointer).
    direct: bool,
}

/// Returns the inner declarator of a declarator node.
fn inner_declarator(node: Node<'_>) -> Option<Node<'_>> {
    node.child_by_field_name("declarator")
        .or_else(|| node.named_child(0))
}

/// Follows a declarator down to the declared name.
fn resolve<'t>(walker: &Walker<'_, 't>, declarator: Node<'t>) -> Option<Resolved<'t>> {
    let mut node = declarator;
    let mut is_function = false;
    let mut direct = false;
    let mut scope = String::new();
    for _ in 0..32 {
        match node.kind() {
            "function_declarator" => {
                is_function = true;
                node = inner_declarator(node)?;
                direct = matches!(
                    node.kind(),
                    "identifier"
                        | "field_identifier"
                        | "qualified_identifier"
                        | "destructor_name"
                        | "operator_name"
                        | "template_function"
                        | "template_method"
                        | "type_identifier"
                );
            }
            "pointer_declarator"
            | "reference_declarator"
            | "array_declarator"
            | "parenthesized_declarator"
            | "attributed_declarator"
            | "init_declarator" => {
                node = inner_declarator(node)?;
            }
            "qualified_identifier" => {
                if let Some(s) = node.child_by_field_name("scope") {
                    if !scope.is_empty() {
                        scope.push_str("::");
                    }
                    scope.push_str(walker.text(s));
                }
                node = node.child_by_field_name("name")?;
            }
            "template_function" | "template_method" => {
                node = node.child_by_field_name("name")?;
            }
            "identifier" | "field_identifier" | "type_identifier" | "operator_name"
            | "destructor_name" => {
                return Some(Resolved {
                    name: node,
                    scope: (!scope.is_empty()).then_some(scope),
                    is_function,
                    direct,
                });
            }
            _ => return None,
        }
    }
    None
}

/// Returns `true` when the node has a storage class specifier with the given text.
fn has_storage(walker: &Walker<'_, '_>, node: Node<'_>, word: &str) -> bool {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .any(|c| c.kind() == "storage_class_specifier" && walker.text(c).trim() == word)
}

/// Returns `true` when the node has a type qualifier with the given text.
fn has_qualifier(walker: &Walker<'_, '_>, node: Node<'_>, word: &str) -> bool {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .any(|c| c.kind() == "type_qualifier" && walker.text(c).trim() == word)
}

/// Returns `true` for a specifier that defines a type (it has a body).
fn is_type_specifier(node: Node<'_>) -> bool {
    matches!(
        node.kind(),
        "struct_specifier" | "union_specifier" | "class_specifier" | "enum_specifier"
    )
}

/// Maps a type specifier to the kind of symbol it declares.
fn specifier_kind(node: Node<'_>) -> SymbolKind {
    match node.kind() {
        "class_specifier" => SymbolKind::Class,
        "enum_specifier" => SymbolKind::Enum,
        _ => SymbolKind::Struct,
    }
}

/// Configures a declaration that sits directly under a `template<...>` header.
fn attach_template<'t>(walker: &Walker<'_, 't>, decl: &mut Decl<'t>) {
    if let Some(parent) = walker.ancestor(1) {
        if parent.kind() == "template_declaration" {
            decl.outer = parent;
            decl.outer_up = 1;
            decl.sig_start = Some(parent.start_byte());
        }
    }
}

/// Visibility of a function or variable given its storage class and its scope.
fn visibility_of(walker: &Walker<'_, '_>, node: Node<'_>) -> Visibility {
    let scope = walker.scope();
    if scope.kind == ScopeKind::Type {
        return scope.access;
    }
    if has_storage(walker, node, "static") {
        Visibility::Private
    } else {
        Visibility::Public
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

impl CFamilyRules {
    /// Declares a function definition or a function prototype.
    fn declare_function<'t>(
        &self,
        walker: &mut Walker<'_, 't>,
        node: Node<'t>,
        resolved: &Resolved<'t>,
    ) {
        let name = walker.text(resolved.name);
        let in_type = walker.scope().kind == ScopeKind::Type;
        let kind = if self.cpp && (in_type || resolved.scope.is_some()) {
            SymbolKind::Method
        } else {
            SymbolKind::Function
        };
        let mut decl = Decl::new(node, name, kind);
        decl.visibility = visibility_of(walker, node);
        decl.name_node = Some(resolved.name);
        decl.sig_end = node
            .child_by_field_name("body")
            .map_or_else(|| node.end_byte(), |b| b.start_byte());
        decl.scope = Some(callable_scope());
        if let Some(scope) = &resolved.scope {
            let owner = walker.qualify(scope);
            decl.qualified = Some(format!("{owner}::{name}"));
            decl.parent = ParentRule::Explicit(
                walker
                    .lookup(&owner)
                    .or_else(|| walker.lookup(scope))
                    .filter(|i| {
                        walker.symbol(*i).is_some_and(|s| {
                            !matches!(s.kind, SymbolKind::Function | SymbolKind::Method)
                        })
                    }),
            );
        }
        attach_template(walker, &mut decl);
        walker.declare(decl);
    }

    /// Declares a `declaration` or `field_declaration`: a prototype or a file-level variable.
    fn declare_declaration<'t>(&self, walker: &mut Walker<'_, 't>, node: Node<'t>) {
        let scope_kind = walker.scope().kind;
        if scope_kind == ScopeKind::Callable {
            return;
        }
        let mut cursor = node.walk();
        let declarators: Vec<Node<'t>> = node
            .children_by_field_name("declarator", &mut cursor)
            .collect();
        for declarator in declarators {
            let Some(resolved) = resolve(walker, declarator) else {
                continue;
            };
            if resolved.is_function {
                if resolved.direct {
                    self.declare_function(walker, node, &resolved);
                }
                continue;
            }
            if scope_kind == ScopeKind::Type
                || has_storage(walker, node, "extern")
                || has_storage(walker, node, "typedef")
            {
                continue;
            }
            let constant =
                has_qualifier(walker, node, "const") || has_qualifier(walker, node, "constexpr");
            let kind = if constant {
                SymbolKind::Constant
            } else {
                SymbolKind::Variable
            };
            let mut decl = Decl::new(node, walker.text(resolved.name), kind);
            decl.visibility = visibility_of(walker, node);
            decl.name_node = Some(resolved.name);
            walker.declare(decl);
        }
    }

    /// Declares a struct, union, class or enum with a body.
    fn declare_specifier<'t>(walker: &mut Walker<'_, 't>, node: Node<'t>) {
        let (Some(name), Some(body)) = (
            node.child_by_field_name("name"),
            node.child_by_field_name("body"),
        ) else {
            return;
        };
        let kind = specifier_kind(node);
        let mut decl = Decl::new(node, walker.text(name), kind);
        decl.visibility = if walker.scope().kind == ScopeKind::Type {
            walker.scope().access
        } else {
            Visibility::Public
        };
        decl.name_node = Some(name);
        decl.sig_end = body.start_byte();
        decl.scope = Some(ScopeSpec {
            kind: ScopeKind::Type,
            access: if node.kind() == "class_specifier" {
                Visibility::Private
            } else {
                Visibility::Public
            },
            open_ended: false,
        });
        attach_template(walker, &mut decl);
        if let Some(parent) = walker.ancestor(1) {
            if parent.kind() == "type_definition" {
                decl.outer = parent;
                decl.outer_up = 1;
                decl.sig_start = Some(parent.start_byte());
                if let Some(alias) = parent
                    .child_by_field_name("declarator")
                    .and_then(|d| resolve(walker, d))
                {
                    walker.suppress(alias.name);
                }
            }
        }
        walker.declare(decl);
    }

    /// Declares a `typedef`: an anonymous struct or enum takes the typedef name, other
    /// definitions are left to their specifier, and everything else is a type alias.
    fn declare_typedef<'t>(walker: &mut Walker<'_, 't>, node: Node<'t>) {
        let Some(ty) = node.child_by_field_name("type") else {
            return;
        };
        let Some(resolved) = node
            .child_by_field_name("declarator")
            .and_then(|d| resolve(walker, d))
        else {
            return;
        };
        let defines = is_type_specifier(ty) && ty.child_by_field_name("body").is_some();
        if defines && ty.child_by_field_name("name").is_some() {
            return;
        }
        let kind = if defines {
            specifier_kind(ty)
        } else {
            SymbolKind::Type
        };
        let mut decl = Decl::new(node, walker.text(resolved.name), kind);
        decl.visibility = Visibility::Public;
        decl.name_node = Some(resolved.name);
        if let Some(body) = ty.child_by_field_name("body").filter(|_| defines) {
            decl.sig_skip.push(body.byte_range());
            decl.scope = Some(ScopeSpec {
                kind: ScopeKind::Type,
                access: Visibility::Public,
                open_ended: false,
            });
        }
        walker.declare(decl);
    }

    /// Declares a C++ namespace.
    fn declare_namespace<'t>(walker: &mut Walker<'_, 't>, node: Node<'t>) {
        let Some(name) = node.child_by_field_name("name") else {
            return;
        };
        let mut decl = Decl::new(node, walker.text(name), SymbolKind::Module);
        decl.visibility = Visibility::Public;
        decl.name_node = Some(name);
        decl.sig_end = node
            .child_by_field_name("body")
            .map_or_else(|| node.end_byte(), |b| b.start_byte());
        decl.scope = Some(ScopeSpec {
            kind: ScopeKind::Module,
            access: Visibility::Public,
            open_ended: false,
        });
        walker.declare(decl);
    }

    /// Declares a `#define` macro that has a value or parameters.
    fn declare_macro<'t>(walker: &mut Walker<'_, 't>, node: Node<'t>) {
        let Some(name) = node.child_by_field_name("name") else {
            return;
        };
        let function_like = node.kind() == "preproc_function_def";
        if !function_like && node.child_by_field_name("value").is_none() {
            return;
        }
        let mut decl = Decl::new(node, walker.text(name), SymbolKind::Macro);
        decl.visibility = Visibility::Public;
        decl.name_node = Some(name);
        if function_like {
            decl.sig_end = node
                .child_by_field_name("parameters")
                .map_or_else(|| name.end_byte(), |p| p.end_byte());
        }
        walker.declare(decl);
    }

    /// Declares a C++ alias (`using Name = ...;`).
    fn declare_alias<'t>(walker: &mut Walker<'_, 't>, node: Node<'t>) {
        let Some(name) = node.child_by_field_name("name") else {
            return;
        };
        let mut decl = Decl::new(node, walker.text(name), SymbolKind::Type);
        decl.visibility = visibility_of(walker, node);
        decl.name_node = Some(name);
        attach_template(walker, &mut decl);
        walker.declare(decl);
    }
}

/// Reports the names of a base class list.
fn report_bases<'t>(walker: &mut Walker<'_, 't>, node: Node<'t>) {
    for child in named_children(node) {
        let (name, qualifier) = match child.kind() {
            "type_identifier" => (Some(child), None),
            "qualified_identifier" => (
                child.child_by_field_name("name"),
                walker.field_text(child, "scope"),
            ),
            "template_type" => (child.child_by_field_name("name"), None),
            _ => (None, None),
        };
        let Some(mut name) = name else {
            continue;
        };
        if name.kind() == "template_type" {
            let Some(inner) = name.child_by_field_name("name") else {
                continue;
            };
            name = inner;
        }
        walker.suppress(name);
        walker.reference(walker.text(name), RefKind::Inherit, name, qualifier);
    }
}

/// Returns the node that names the callee of a call expression.
fn callee<'a, 't>(
    walker: &Walker<'a, 't>,
    function: Node<'t>,
) -> Option<(Node<'t>, Option<&'a str>)> {
    let mut node = function;
    let mut qualifier = None;
    for _ in 0..8 {
        match node.kind() {
            "identifier" | "field_identifier" => return Some((node, qualifier)),
            "field_expression" => {
                qualifier = walker.field_text(node, "argument");
                node = node.child_by_field_name("field")?;
            }
            "qualified_identifier" => {
                qualifier = walker.field_text(node, "scope");
                node = node.child_by_field_name("name")?;
            }
            "template_function" | "template_method" => node = node.child_by_field_name("name")?,
            _ => return None,
        }
    }
    None
}

impl Rules for CFamilyRules {
    fn separator(&self) -> &'static str {
        if self.cpp { "::" } else { "." }
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
            "function_definition" => {
                if let Some(resolved) = node
                    .child_by_field_name("declarator")
                    .and_then(|d| resolve(walker, d))
                {
                    if resolved.is_function {
                        self.declare_function(walker, node, &resolved);
                    }
                }
            }
            "declaration" | "field_declaration" => self.declare_declaration(walker, node),
            "struct_specifier" | "union_specifier" | "class_specifier" | "enum_specifier" => {
                Self::declare_specifier(walker, node);
            }
            "type_definition" => Self::declare_typedef(walker, node),
            "namespace_definition" => Self::declare_namespace(walker, node),
            "preproc_def" | "preproc_function_def" => Self::declare_macro(walker, node),
            "alias_declaration" => Self::declare_alias(walker, node),
            "access_specifier" if walker.scope().kind == ScopeKind::Type => {
                let access = if walker.text(node).trim() == "public" {
                    Visibility::Public
                } else {
                    Visibility::Private
                };
                walker.scope_mut().access = access;
            }
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
            "base_class_clause" => report_bases(walker, node),
            "preproc_include" => {
                if let Some(path) = node.child_by_field_name("path") {
                    let text = walker.text(path).trim();
                    walker.import(text.trim_matches(['<', '>', '"']));
                }
            }
            "type_identifier" => {
                if walker.is_suppressed(node) {
                    return;
                }
                let name = walker.text(node);
                if name.len() <= 1 {
                    return;
                }
                let qualifier = walker
                    .ancestor(1)
                    .filter(|p| p.kind() == "qualified_identifier")
                    .filter(|_| walker.field_of(0) == Some("name"))
                    .and_then(|p| walker.field_text(p, "scope"));
                walker.reference(name, RefKind::Type, node, qualifier);
            }
            _ => {}
        }
    }
}
