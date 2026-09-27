// SPDX-License-Identifier: Apache-2.0
//! Walker rules for Ruby.
//!
//! # Role in the architecture
//! Implements [`Rules`] for the `tree-sitter-ruby` grammar. Qualified names use `.`.
//!
//! # Conventions
//! * Modules and classes open a member scope. A method after a bare `private` or `protected` is
//!   private until a bare `public` follows; `private def name` and `private :name` are
//!   understood too. Singleton methods (`def self.name`) and top-level methods are public.
//! * Constants (`NAME = value`) are listed as constants.
//! * Documentation is the run of `#` comment lines directly above the declaration; magic
//!   comments such as `# frozen_string_literal: true` are not documentation.
//! * `require`, `require_relative` and `load` with a string are imports; `include`, `extend`
//!   and `prepend` report the mixed-in module as an inheritance reference, and `X.new` is a use
//!   of the type `X`.

use pn_ultramemory_core::{RefKind, SymbolKind, Visibility};
use tree_sitter::Node;

use super::engine::{Decl, Role, Rules, ScopeKind, ScopeSpec, Walker};
use super::nodes::named_children;
use super::text::clean_comments;

/// The Ruby rules.
pub(super) struct RubyRules;

/// The shared instance of the Ruby rules.
pub(super) static RUBY: RubyRules = RubyRules;

/// Calls that only change visibility or define accessors; they are not reported as references.
const DIRECTIVES: &[&str] = &[
    "private",
    "protected",
    "public",
    "attr_reader",
    "attr_writer",
    "attr_accessor",
    "module_function",
    "private_constant",
    "private_class_method",
    "public_class_method",
];

/// Returns `true` for a comment that documents the declaration below it.
fn is_doc_comment(text: &str) -> bool {
    let body = text.trim_start_matches('#').trim();
    text.starts_with('#')
        && !text.starts_with("#!")
        && ![
            "frozen_string_literal",
            "encoding",
            "-*-",
            "typed:",
            "rubocop:",
            "coding",
        ]
        .iter()
        .any(|magic| body.starts_with(magic))
}

/// Returns the last segment of a constant path and the scope written before it.
fn constant_parts<'a>(walker: &Walker<'a, '_>, node: Node<'_>) -> (&'a str, Option<&'a str>) {
    if node.kind() == "scope_resolution" {
        if let Some(name) = node.child_by_field_name("name") {
            return (walker.text(name), walker.field_text(node, "scope"));
        }
    }
    (walker.text(node), None)
}

/// Returns the text of a string argument without its quotes.
fn string_argument<'a>(walker: &Walker<'a, '_>, arguments: Node<'_>) -> Option<&'a str> {
    let first = arguments.named_child(0)?;
    (first.kind() == "string").then(|| walker.text(first).trim_matches(['"', '\'']))
}

/// Returns where the header of a `def`, `class` or `module` ends.
fn header_end(node: Node<'_>) -> usize {
    if let Some(body) = node.child_by_field_name("body") {
        return body.start_byte();
    }
    ["parameters", "superclass", "name"]
        .iter()
        .filter_map(|f| node.child_by_field_name(f))
        .map(|n| n.end_byte())
        .max()
        .unwrap_or_else(|| node.end_byte())
}

impl RubyRules {
    /// Declares a module or a class.
    fn declare_namespace<'t>(walker: &mut Walker<'_, 't>, node: Node<'t>, kind: SymbolKind) {
        let Some(name) = node.child_by_field_name("name") else {
            return;
        };
        let (simple, _) = constant_parts(walker, name);
        let written = walker.text(name).replace("::", ".");
        let mut decl = Decl::new(node, simple, kind);
        decl.visibility = Visibility::Public;
        decl.name_node = Some(name);
        decl.qualified = Some(walker.qualify(&written));
        decl.sig_end = header_end(node);
        decl.scope = Some(ScopeSpec {
            kind: ScopeKind::Type,
            access: Visibility::Public,
            open_ended: false,
        });
        walker.declare(decl);
    }

    /// Declares an instance method, a singleton method or an endless method.
    fn declare_method<'t>(walker: &mut Walker<'_, 't>, node: Node<'t>) {
        let Some(name) = node.child_by_field_name("name") else {
            return;
        };
        let singleton = node.kind() == "singleton_method";
        let kind = if walker.scope().kind == ScopeKind::Type {
            SymbolKind::Method
        } else {
            SymbolKind::Function
        };
        let mut decl = Decl::new(node, walker.text(name), kind);
        decl.visibility = if singleton {
            Visibility::Public
        } else {
            explicit_visibility(walker).unwrap_or(walker.scope().access)
        };
        decl.name_node = Some(name);
        decl.sig_end = header_end(node);
        decl.scope = Some(ScopeSpec {
            kind: ScopeKind::Callable,
            access: Visibility::Public,
            open_ended: false,
        });
        walker.declare(decl);
    }

    /// Declares a constant assignment.
    fn declare_constant<'t>(walker: &mut Walker<'_, 't>, node: Node<'t>) {
        if walker.scope().kind == ScopeKind::Callable {
            return;
        }
        let Some(left) = node.child_by_field_name("left") else {
            return;
        };
        if left.kind() != "constant" || node.child_by_field_name("right").is_none() {
            return;
        }
        let mut decl = Decl::new(node, walker.text(left), SymbolKind::Constant);
        decl.visibility = Visibility::Public;
        decl.name_node = Some(left);
        decl.scope = Some(ScopeSpec {
            kind: ScopeKind::Callable,
            access: Visibility::Public,
            open_ended: false,
        });
        walker.declare(decl);
    }

    /// Applies `private`, `protected` and `public` written as bare statements or with symbols.
    fn declare_visibility_call<'t>(walker: &mut Walker<'_, 't>, node: Node<'t>) {
        let Some(method) = node.child_by_field_name("method") else {
            return;
        };
        let visibility = match walker.text(method) {
            "private" | "protected" => Visibility::Private,
            "public" => Visibility::Public,
            _ => return,
        };
        let Some(arguments) = node.child_by_field_name("arguments") else {
            return;
        };
        for argument in named_children(arguments) {
            if matches!(argument.kind(), "simple_symbol" | "string") {
                let name = walker.text(argument).trim_matches([':', '"', '\'']);
                if let Some(index) = walker.find_in_scope(name) {
                    walker.set_visibility(index, visibility);
                }
            }
        }
    }
}

/// Returns the visibility given by `private def name` / `public def name`, if the method being
/// declared is the argument of such a call.
fn explicit_visibility(walker: &Walker<'_, '_>) -> Option<Visibility> {
    let arguments = walker.ancestor(1).filter(|a| a.kind() == "argument_list")?;
    let call = walker.ancestor(2).filter(|c| c.kind() == "call")?;
    let _ = arguments;
    match walker.field_text(call, "method")? {
        "private" | "protected" => Some(Visibility::Private),
        "public" => Some(Visibility::Public),
        _ => None,
    }
}

impl Rules for RubyRules {
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
            "module" => Self::declare_namespace(walker, node, SymbolKind::Module),
            "class" => Self::declare_namespace(walker, node, SymbolKind::Class),
            "method" | "singleton_method" => Self::declare_method(walker, node),
            "assignment" => Self::declare_constant(walker, node),
            "singleton_class" => {
                walker.push_transparent_scope(ScopeKind::Type, Visibility::Public);
            }
            "call" => Self::declare_visibility_call(walker, node),
            "identifier" => {
                let access = match walker.text(node) {
                    "private" | "protected" => Visibility::Private,
                    "public" => Visibility::Public,
                    _ => return,
                };
                let bare = walker
                    .ancestor(1)
                    .is_some_and(|p| p.kind() == "body_statement");
                if bare && walker.scope().kind == ScopeKind::Type {
                    walker.scope_mut().access = access;
                }
            }
            _ => {}
        }
    }

    fn observe<'t>(&self, walker: &mut Walker<'_, 't>, node: Node<'t>) {
        match node.kind() {
            "call" => observe_call(walker, node),
            "class" => {
                if let Some(superclass) = node.child_by_field_name("superclass") {
                    if let Some(base) = superclass.named_child(0) {
                        if matches!(base.kind(), "constant" | "scope_resolution") {
                            let (name, qualifier) = constant_parts(walker, base);
                            walker.reference(name, RefKind::Inherit, base, qualifier);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

/// Reports a call: imports, mixins, instantiations and ordinary calls.
fn observe_call<'t>(walker: &mut Walker<'_, 't>, node: Node<'t>) {
    let Some(method) = node.child_by_field_name("method") else {
        return;
    };
    let name = walker.text(method);
    let receiver = node.child_by_field_name("receiver");
    let arguments = node.child_by_field_name("arguments");
    if receiver.is_none() {
        match name {
            "require" | "require_relative" | "load" => {
                if let Some(target) = arguments.and_then(|a| string_argument(walker, a)) {
                    walker.import(target);
                }
                return;
            }
            "include" | "extend" | "prepend" => {
                for argument in arguments.map(named_children).unwrap_or_default() {
                    if matches!(argument.kind(), "constant" | "scope_resolution") {
                        let (text, qualifier) = constant_parts(walker, argument);
                        walker.reference(text, RefKind::Inherit, argument, qualifier);
                    }
                }
                return;
            }
            _ if DIRECTIVES.contains(&name) => return,
            _ => {}
        }
    }
    if name == "new" {
        if let Some(receiver) =
            receiver.filter(|r| matches!(r.kind(), "constant" | "scope_resolution"))
        {
            let (text, qualifier) = constant_parts(walker, receiver);
            walker.reference(text, RefKind::Type, receiver, qualifier);
            return;
        }
    }
    let qualifier = receiver.map(|r| walker.text(r));
    walker.reference(name, RefKind::Call, method, qualifier);
}
