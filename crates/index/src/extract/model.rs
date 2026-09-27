// SPDX-License-Identifier: Apache-2.0
//! The vocabulary shared by the walker and the language rules: what a rules module reports
//! ([`Decl`]), how scopes are described ([`Scope`], [`ScopeSpec`], [`ScopeKind`]), how sibling
//! nodes are classified for comment attachment ([`Role`]), the facts kept for the documentation
//! inserter ([`SymbolExtra`]) and the [`Rules`] trait itself.
//!
//! # Role in the architecture
//! Pure data and one trait: nothing here walks a tree. The walker in [`super::engine`] consumes
//! these types, and each language module produces them.

use std::ops::Range;

use pn_ultramemory_core::{SymbolKind, Visibility};
use tree_sitter::Node;

use super::engine::Walker;

/// What a sibling node is, for the purpose of attaching it to the declaration that follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Role {
    /// A documentation comment.
    Doc,
    /// An attribute or decorator that is a sibling of the declaration it belongs to.
    Attribute,
    /// Anything else; it ends any run of comments before it.
    Other,
}

/// The kind of region a scope covers, which decides what may be declared inside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ScopeKind {
    /// The whole file.
    File,
    /// A module, namespace or package body.
    Module,
    /// The body of a class, struct, trait, interface or enum: members live here.
    Type,
    /// The body of a function or method: only nested functions and types are listed.
    Callable,
}

/// A region of the file that owns declarations.
#[derive(Debug, Clone)]
pub(super) struct Scope {
    /// What kind of region it is.
    pub(super) kind: ScopeKind,
    /// Symbol that owns declarations directly inside, and owner of references made there.
    pub(super) symbol: Option<usize>,
    /// Qualified-name prefix for declarations directly inside; empty at file level.
    pub(super) prefix: String,
    /// Visibility given to members declared next; access labels (`private:`) change it.
    pub(super) access: Visibility,
    /// Node depth at which the scope ends.
    pub(super) exit_depth: usize,
    /// Whether callee names seen inside also go to the outline of `symbol`.
    pub(super) outline: bool,
}

/// Where the parent of a declared symbol comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ParentRule {
    /// The symbol that owns the current scope.
    Scope,
    /// A symbol chosen by the rules (a method's receiver type), or none.
    Explicit(Option<usize>),
}

/// How a declaration opens a scope for what it contains.
#[derive(Debug, Clone)]
pub(super) struct ScopeSpec {
    /// The kind of the new scope.
    pub(super) kind: ScopeKind,
    /// Visibility of members that carry no explicit modifier.
    pub(super) access: Visibility,
    /// Set for declarations whose scope lasts until the end of the enclosing node
    /// (`namespace X;` statements) instead of ending with the declaration node.
    pub(super) open_ended: bool,
}

/// A declaration recognised by a language's [`Rules::declare`].
#[derive(Debug, Clone)]
pub(super) struct Decl<'t> {
    /// Simple name.
    pub(super) name: String,
    /// Kind of symbol.
    pub(super) kind: SymbolKind,
    /// Visibility of the symbol.
    pub(super) visibility: Visibility,
    /// The node that decides the extent of the symbol and is preceded by its comments. It is the
    /// entered node, or an ancestor of it when [`Decl::outer_up`] is not zero.
    pub(super) outer: Node<'t>,
    /// How many levels above the entered node `outer` sits.
    pub(super) outer_up: usize,
    /// Node where the declaration proper starts (after wrappers such as `export`).
    pub(super) core: Node<'t>,
    /// Byte where the signature starts; defaults to the start of `core`.
    pub(super) sig_start: Option<usize>,
    /// Byte where the signature ends (usually the start of the body).
    pub(super) sig_end: usize,
    /// Byte ranges inside the signature that are dropped (annotations, decorators).
    pub(super) sig_skip: Vec<Range<usize>>,
    /// Documentation found inside the declaration (a Python docstring).
    pub(super) inline_doc: Option<String>,
    /// The node holding the name; it is not reported as a type mention.
    pub(super) name_node: Option<Node<'t>>,
    /// Scope opened for the children, if the declaration is a container.
    pub(super) scope: Option<ScopeSpec>,
    /// Inner nodes that must not be declared again when the walk reaches them.
    pub(super) covers: Vec<Node<'t>>,
    /// Replaces the qualified name derived from the current scope.
    pub(super) qualified: Option<String>,
    /// Where the parent of the symbol comes from.
    pub(super) parent: ParentRule,
    /// Replaces the end of the symbol's extent (used by open-ended scopes).
    pub(super) end_override: Option<usize>,
}

impl<'t> Decl<'t> {
    /// Creates a declaration whose entered node and declaration proper are both `node`.
    pub(super) fn new(node: Node<'t>, name: impl Into<String>, kind: SymbolKind) -> Self {
        Self {
            name: name.into(),
            kind,
            visibility: Visibility::Unknown,
            outer: node,
            outer_up: 0,
            core: node,
            sig_start: None,
            sig_end: node.end_byte(),
            sig_skip: Vec::new(),
            inline_doc: None,
            name_node: None,
            scope: None,
            covers: Vec::new(),
            qualified: None,
            parent: ParentRule::Scope,
            end_override: None,
        }
    }
}

/// Extra facts about a symbol that the documentation inserter needs but [`SymbolDraft`] omits.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SymbolExtra {
    /// 1-based line where the declaration proper starts.
    pub(crate) decl_line: u32,
    /// 1-based line of the name.
    pub(crate) name_line: u32,
    /// Byte where attributes or decorators attached to the declaration begin (or the declaration
    /// itself when there are none): where documentation is inserted.
    pub(crate) attach_start: usize,
    /// 1-based line of `attach_start`.
    pub(crate) attach_line: u32,
    /// Start byte of the declaration proper.
    pub(crate) core_start: usize,
    /// End byte of the declaration proper.
    pub(crate) core_end: usize,
    /// Whether documentation is attached to the symbol.
    pub(crate) has_doc: bool,
}

/// The two hooks a language supplies to the walker.
pub(super) trait Rules: Sync {
    /// Separator between the parts of a qualified name (`::` or `.`).
    fn separator(&self) -> &'static str;

    /// Classifies a sibling node for comment attachment.
    fn role(&self, src: &str, node: Node<'_>) -> Role;

    /// Turns the raw texts of a run of attached comments into a documentation string.
    fn clean_doc(&self, comments: &[&str]) -> Option<String>;

    /// Recognises a declaration at `node` and reports it with [`Walker::declare`].
    fn declare<'t>(&self, walker: &mut Walker<'_, 't>, node: Node<'t>);

    /// Reports the references and imports found at `node` with [`Walker::reference`] and
    /// [`Walker::import`].
    fn observe<'t>(&self, walker: &mut Walker<'_, 't>, node: Node<'t>);
}
