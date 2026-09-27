// SPDX-License-Identifier: Apache-2.0
//! Walker rules for Python.
//!
//! # Role in the architecture
//! Implements [`Rules`] for the `tree-sitter-python` grammar. Qualified names use `.`.
//!
//! # Conventions
//! * Documentation is the docstring: the first statement of a function or class body when it is
//!   a plain string literal. Comments are never documentation.
//! * A decorated definition is one symbol whose span starts at its first decorator; the
//!   decorators are not part of the signature and their calls are owned by the definition.
//! * A leading underscore makes a name private, except for dunder names such as `__init__`.
//! * Module-level assignments to a simple name are listed: `UPPER_CASE` names as constants and
//!   the others as variables. Class-level assignments are listed only when they are constants.

use pn_ultramemory_core::{RefKind, SymbolKind, Visibility};
use tree_sitter::Node;

use super::engine::{Decl, Role, Rules, ScopeKind, ScopeSpec, Walker, for_each_descendant};
use super::nodes::named_children;
use super::text::{clean_comments, python_docstring};

/// The Python rules.
pub(super) struct PythonRules;

/// The shared instance of the Python rules.
pub(super) static PYTHON: PythonRules = PythonRules;

/// Built-in type names that are not worth reporting as type mentions.
const BUILTIN_TYPES: &[&str] = &[
    "int",
    "str",
    "float",
    "bool",
    "bytes",
    "list",
    "dict",
    "set",
    "tuple",
    "object",
    "complex",
    "type",
    "frozenset",
    "bytearray",
];

/// Visibility of a Python name: private with a leading underscore, except dunder names.
fn name_visibility(name: &str) -> Visibility {
    let dunder = name.len() > 4 && name.starts_with("__") && name.ends_with("__");
    if name.starts_with('_') && !dunder {
        Visibility::Private
    } else {
        Visibility::Public
    }
}

/// Returns `true` for names written in `UPPER_SNAKE_CASE`.
fn is_constant_name(name: &str) -> bool {
    name.chars().any(|c| c.is_ascii_uppercase())
        && name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// Returns the docstring of a function or class body, if its first statement is one.
fn docstring(walker: &Walker<'_, '_>, body: Node<'_>) -> Option<String> {
    let mut cursor = body.walk();
    let first = body
        .named_children(&mut cursor)
        .find(|c| c.kind() != "comment")?;
    if first.kind() != "expression_statement" {
        return None;
    }
    let mut inner = first.walk();
    let literal = first.named_children(&mut inner).next()?;
    if literal.kind() != "string" || first.named_child_count() != 1 {
        return None;
    }
    let children = named_children(literal);
    let start = children.iter().find(|c| c.kind() == "string_start")?;
    let end = children.iter().rev().find(|c| c.kind() == "string_end")?;
    let prefix = walker.text(*start);
    let prefix = prefix.trim_end_matches(['"', '\'']).to_ascii_lowercase();
    if prefix.contains(['f', 'b']) {
        return None;
    }
    let content = walker.src.get(start.end_byte()..end.start_byte())?;
    python_docstring(content, prefix.contains('r'))
}

impl PythonRules {
    /// Declares a function or a class. `outer` is the node being entered (a decorated
    /// definition or the definition itself) and `core` the definition.
    fn declare_definition<'t>(walker: &mut Walker<'_, 't>, outer: Node<'t>, core: Node<'t>) {
        let Some(name) = core.child_by_field_name("name") else {
            return;
        };
        let text = walker.text(name);
        let is_class = core.kind() == "class_definition";
        let kind = if is_class {
            SymbolKind::Class
        } else if walker.scope().kind == ScopeKind::Type {
            SymbolKind::Method
        } else {
            SymbolKind::Function
        };
        let mut decl = Decl::new(core, text, kind);
        decl.outer = outer;
        decl.visibility = name_visibility(text);
        decl.name_node = Some(name);
        let body = core.child_by_field_name("body");
        decl.sig_end = body.map_or_else(|| core.end_byte(), |b| b.start_byte());
        decl.inline_doc = body.and_then(|b| docstring(walker, b));
        decl.scope = Some(ScopeSpec {
            kind: if is_class {
                ScopeKind::Type
            } else {
                ScopeKind::Callable
            },
            access: Visibility::Public,
            open_ended: false,
        });
        if outer.id() != core.id() {
            decl.covers.push(core);
        }
        walker.declare(decl);
    }

    /// Declares a module-level or class-level assignment to a simple name.
    fn declare_assignment<'t>(walker: &mut Walker<'_, 't>, node: Node<'t>) {
        let scope_kind = walker.scope().kind;
        let owner_ok = walker.ancestor(1).is_some_and(|p| p.kind() == "module")
            || (scope_kind == ScopeKind::Type
                && walker.ancestor(1).is_some_and(|p| p.kind() == "block"));
        if !owner_ok {
            return;
        }
        let Some(assignment) = super::nodes::child_of_kind(node, "assignment") else {
            return;
        };
        let (Some(left), Some(_)) = (
            assignment.child_by_field_name("left"),
            assignment.child_by_field_name("right"),
        ) else {
            return;
        };
        if left.kind() != "identifier" {
            return;
        }
        let name = walker.text(left);
        let kind = if is_constant_name(name) {
            SymbolKind::Constant
        } else if scope_kind == ScopeKind::Type {
            return;
        } else {
            SymbolKind::Variable
        };
        let mut decl = Decl::new(node, name, kind);
        decl.visibility = name_visibility(name);
        decl.name_node = Some(left);
        decl.scope = Some(ScopeSpec {
            kind: ScopeKind::Callable,
            access: Visibility::Public,
            open_ended: false,
        });
        walker.declare(decl);
    }

    /// Reports the base classes of a class definition.
    fn observe_class<'t>(walker: &mut Walker<'_, 't>, node: Node<'t>) {
        let Some(bases) = node.child_by_field_name("superclasses") else {
            return;
        };
        for base in named_children(bases) {
            match base.kind() {
                "identifier" => walker.reference(walker.text(base), RefKind::Inherit, base, None),
                "attribute" => {
                    if let Some(attr) = base.child_by_field_name("attribute") {
                        let q = walker.field_text(base, "object");
                        walker.reference(walker.text(attr), RefKind::Inherit, attr, q);
                    }
                }
                "keyword_argument" => {
                    if let Some(value) = base.child_by_field_name("value") {
                        if value.kind() == "identifier" {
                            walker.reference(walker.text(value), RefKind::Type, value, None);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// Reports the names used in a type annotation.
    fn observe_annotation<'t>(walker: &mut Walker<'_, 't>, node: Node<'t>) {
        let mut found: Vec<(Node<'t>, Option<&str>)> = Vec::new();
        for_each_descendant(node, 256, |n| match n.kind() {
            "type" | "string" => false,
            "identifier" => {
                found.push((n, None));
                false
            }
            "attribute" => {
                if let Some(attr) = n.child_by_field_name("attribute") {
                    found.push((attr, walker.field_text(n, "object")));
                }
                false
            }
            _ => true,
        });
        if node.named_child_count() == 1 {
            if let Some(only) = node.named_child(0) {
                if only.kind() == "identifier" {
                    found.push((only, None));
                }
            }
        }
        found.dedup_by_key(|(n, _)| n.id());
        for (name_node, qualifier) in found {
            let name = walker.text(name_node);
            if !BUILTIN_TYPES.contains(&name) {
                walker.reference(name, RefKind::Type, name_node, qualifier);
            }
        }
    }

    /// Reports the targets of an `import` or `from ... import` statement.
    fn observe_import(walker: &mut Walker<'_, '_>, node: Node<'_>) {
        if node.kind() == "import_statement" {
            for child in named_children(node) {
                let target = match child.kind() {
                    "dotted_name" => Some(child),
                    "aliased_import" => child.child_by_field_name("name"),
                    _ => None,
                };
                if let Some(target) = target {
                    walker.import(walker.text(target));
                }
            }
            return;
        }
        let Some(module) = node.child_by_field_name("module_name") else {
            return;
        };
        let module_text = walker.text(module);
        if !module_text.chars().all(|c| c == '.') {
            walker.import(module_text);
            return;
        }
        let mut any = false;
        for child in named_children(node) {
            let name = match child.kind() {
                "dotted_name" if child.id() != module.id() => Some(child),
                "aliased_import" => child.child_by_field_name("name"),
                _ => None,
            };
            if let Some(name) = name {
                any = true;
                walker.import(&format!("{module_text}{}", walker.text(name)));
            }
        }
        if !any {
            walker.import(module_text);
        }
    }
}

impl Rules for PythonRules {
    fn separator(&self) -> &'static str {
        "."
    }

    fn role(&self, _src: &str, _node: Node<'_>) -> Role {
        Role::Other
    }

    fn clean_doc(&self, comments: &[&str]) -> Option<String> {
        clean_comments(comments)
    }

    fn declare<'t>(&self, walker: &mut Walker<'_, 't>, node: Node<'t>) {
        match node.kind() {
            "decorated_definition" => {
                if let Some(definition) = node.child_by_field_name("definition") {
                    Self::declare_definition(walker, node, definition);
                }
            }
            "function_definition" | "class_definition" => {
                Self::declare_definition(walker, node, node);
            }
            "expression_statement" => Self::declare_assignment(walker, node),
            _ => {}
        }
    }

    fn observe<'t>(&self, walker: &mut Walker<'_, 't>, node: Node<'t>) {
        match node.kind() {
            "call" => {
                let Some(function) = node.child_by_field_name("function") else {
                    return;
                };
                match function.kind() {
                    "identifier" => {
                        walker.reference(walker.text(function), RefKind::Call, function, None);
                    }
                    "attribute" => {
                        if let Some(attr) = function.child_by_field_name("attribute") {
                            let q = walker.field_text(function, "object");
                            walker.reference(walker.text(attr), RefKind::Call, attr, q);
                        }
                    }
                    _ => {}
                }
            }
            "decorator" => {
                let Some(expression) = node.named_child(0) else {
                    return;
                };
                match expression.kind() {
                    "identifier" => {
                        walker.reference(walker.text(expression), RefKind::Call, expression, None);
                    }
                    "attribute" => {
                        if let Some(attr) = expression.child_by_field_name("attribute") {
                            let q = walker.field_text(expression, "object");
                            walker.reference(walker.text(attr), RefKind::Call, attr, q);
                        }
                    }
                    _ => {}
                }
            }
            "class_definition" => Self::observe_class(walker, node),
            "type" => Self::observe_annotation(walker, node),
            "import_statement" | "import_from_statement" => Self::observe_import(walker, node),
            _ => {}
        }
    }
}
