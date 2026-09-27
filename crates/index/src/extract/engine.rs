// SPDX-License-Identifier: Apache-2.0
//! The language-independent walker that turns a tree-sitter tree into symbols and references.
//!
//! # Role in the architecture
//! Every grammar-backed language implements [`Rules`]: two small hooks that recognise
//! declarations ([`Rules::declare`]) and references or imports ([`Rules::observe`]) on one node.
//! The [`Walker`] owns everything else: the iterative depth-first traversal, the scope stack
//! that gives symbols their parent and qualified name, the attachment of documentation
//! comments and attributes that precede a declaration, hashing, signatures and outlines.
//!
//! # Invariants
//! * The traversal never recurses: it uses a [`tree_sitter::TreeCursor`] and explicit stacks, so
//!   arbitrarily deep input cannot overflow the call stack. Nodes deeper than [`MAX_DEPTH`] are
//!   not descended into.
//! * Symbols are pushed in source order and a parent is always pushed before its children.
//! * Symbol nesting is bounded by [`MAX_SCOPES`], references by [`MAX_REFERENCES`], so the output
//!   size is bounded for any input.
//! * Nothing here panics: every slice is checked and every arithmetic operation saturates.

use std::collections::HashMap;
use std::ops::Range;

use pn_ultramemory_core::{
    RefKind, ReferenceDraft, Span, SymbolDraft, Visibility, hash_normalized,
};
use tree_sitter::Node;

pub(crate) use super::model::SymbolExtra;
pub(super) use super::model::{Decl, ParentRule, Role, Rules, Scope, ScopeKind, ScopeSpec};
use super::text::{
    collapse_whitespace, floor_boundary, is_simple_path, tidy_signature, to_u32,
    trim_signature_tail, truncate_chars,
};

/// Deepest node level the walker descends into.
pub(super) const MAX_DEPTH: usize = 4096;
/// Deepest nesting of symbol scopes; declarations nested deeper are not listed.
pub(super) const MAX_SCOPES: usize = 64;
/// Most references reported for one file.
pub(super) const MAX_REFERENCES: usize = 20_000;
/// Most callee names kept in the outline of a symbol.
const MAX_OUTLINE: usize = 24;
/// Longest signature, in characters.
const MAX_SIGNATURE_CHARS: usize = 240;
/// Most symbols reported for one file.
const MAX_SYMBOLS: usize = 200_000;
/// Most imports reported for one file.
const MAX_IMPORTS: usize = 50_000;
/// Longest name accepted for a reference, in bytes; longer names are noise.
const MAX_REF_NAME: usize = 256;
/// Most bytes of a declaration header read when building a signature.
const MAX_SIGNATURE_SOURCE: usize = 4096;

/// A run of comments and attributes immediately before the node being visited.
#[derive(Debug, Default)]
struct Frame {
    /// Byte ranges of the documentation comments in the run.
    docs: Vec<Range<usize>>,
    /// Start byte and row of the first element of the run.
    first: Option<(usize, u32)>,
    /// Start byte and row of the first attribute in the run.
    attr_start: Option<(usize, u32)>,
    /// Last source row covered by the run.
    last_row: u32,
    /// Last row of the previous sibling that is not part of the run.
    prev_row: Option<u32>,
}

impl Frame {
    /// Forgets everything, for a node whose children are about to be visited.
    fn reset(&mut self) {
        self.clear_run();
        self.prev_row = None;
    }

    /// Forgets the current run.
    fn clear_run(&mut self) {
        self.docs.clear();
        self.first = None;
        self.attr_start = None;
    }

    /// Returns `true` when the run ends on the row before `row` (or on `row` itself).
    fn attached_to(&self, row: u32) -> bool {
        self.first.is_some() && row >= self.last_row && row - self.last_row <= 1
    }
}

/// Returns the last source row a node covers, ignoring a trailing line break.
pub(super) fn last_row(node: Node<'_>) -> u32 {
    let start = node.start_position();
    let end = node.end_position();
    let row = if end.column == 0 && end.row > start.row {
        end.row - 1
    } else {
        end.row
    };
    to_u32(row)
}

/// Where a declaration sits in the source, as computed from its node and the comments above it.
struct Placement {
    /// Byte where the symbol's span starts (its documentation comment or its first attribute).
    span_start: usize,
    /// 0-based row where the span starts.
    span_row: u32,
    /// Byte where the attributes attached to the declaration begin.
    attach_start: usize,
    /// 0-based row of `attach_start`.
    attach_row: u32,
    /// Byte just after the declaration, without trailing whitespace.
    end_byte: usize,
    /// 1-based last line of the declaration.
    end_line: u32,
    /// The attached documentation, cleaned.
    doc: Option<String>,
}

/// The state of one extraction.
pub(super) struct Walker<'a, 't> {
    /// The source text.
    pub(super) src: &'a str,
    /// The rules of the language being walked.
    rules: &'a dyn Rules,
    /// Symbols found so far.
    symbols: Vec<SymbolDraft>,
    /// Source position of each outline entry, parallel to the outlines of `symbols`.
    outline_positions: Vec<Vec<usize>>,
    /// Extra facts, parallel to `symbols`.
    extras: Vec<SymbolExtra>,
    /// References found so far.
    references: Vec<ReferenceDraft>,
    /// Imports found so far.
    imports: Vec<String>,
    /// Number of syntax errors seen.
    errors: u32,
    /// Whether errors are counted (false when the root reports none).
    count_errors: bool,
    /// Current node depth.
    depth: usize,
    /// The nodes from the root to the current node.
    path: Vec<Node<'t>>,
    /// The field name of each node on `path`.
    fields: Vec<Option<&'t str>>,
    /// Comment runs, one per depth.
    frames: Vec<Frame>,
    /// Active scopes, innermost last.
    scopes: Vec<Scope>,
    /// Qualified name to the first symbol that has it.
    by_qualified: HashMap<String, usize>,
    /// Ids of nodes that are already covered by a declaration.
    covered: Vec<usize>,
    /// Ids of nodes that must not be reported as references.
    suppressed: Vec<usize>,
}

impl<'a, 't> Walker<'a, 't> {
    /// Creates a walker over `src` with the given language rules.
    pub(super) fn new(src: &'a str, rules: &'a dyn Rules, root: Node<'t>) -> Self {
        Self {
            src,
            rules,
            symbols: Vec::new(),
            outline_positions: Vec::new(),
            extras: Vec::new(),
            references: Vec::new(),
            imports: Vec::new(),
            errors: 0,
            count_errors: root.has_error(),
            depth: 0,
            path: Vec::new(),
            fields: Vec::new(),
            frames: vec![Frame::default()],
            scopes: vec![Scope {
                kind: ScopeKind::File,
                symbol: None,
                prefix: String::new(),
                access: Visibility::Public,
                exit_depth: 0,
                outline: false,
            }],
            by_qualified: HashMap::new(),
            covered: Vec::new(),
            suppressed: Vec::new(),
        }
    }

    // ---- traversal -------------------------------------------------------------------------

    /// Walks the whole tree below `root`, iteratively.
    pub(super) fn run(&mut self, root: Node<'t>) {
        let mut cursor = root.walk();
        loop {
            let node = cursor.node();
            self.path.truncate(self.depth);
            self.fields.truncate(self.depth);
            self.path.push(node);
            self.fields.push(cursor.field_name());
            self.enter(node);
            if self.depth < MAX_DEPTH && cursor.goto_first_child() {
                self.depth += 1;
                if self.frames.len() <= self.depth {
                    self.frames.push(Frame::default());
                }
                self.enter_children(cursor.node().start_byte() == node.start_byte());
                continue;
            }
            loop {
                self.leave(cursor.node());
                if cursor.goto_next_sibling() {
                    break;
                }
                if !cursor.goto_parent() {
                    return;
                }
                self.depth = self.depth.saturating_sub(1);
            }
        }
    }

    /// Prepares the comment run for the children of the node just entered.
    ///
    /// The first child of a node that starts at the same byte (Ruby's `body_statement`, whose
    /// comments precede it in the enclosing node) inherits the run of comments that precedes its
    /// parent; every other child starts with an empty run.
    fn enter_children(&mut self, first_child_shares_start: bool) {
        let (outer, inner) = self.frames.split_at_mut(self.depth);
        let parent = &outer[self.depth - 1];
        let child = &mut inner[0];
        child.reset();
        if first_child_shares_start && parent.first.is_some() {
            child.docs.clone_from(&parent.docs);
            child.first = parent.first;
            child.attr_start = parent.attr_start;
            child.last_row = parent.last_row;
        }
    }

    /// Handles a node that the traversal has just entered.
    fn enter(&mut self, node: Node<'t>) {
        if self.count_errors && (node.is_error() || node.is_missing()) {
            self.errors = self.errors.saturating_add(1);
        }
        if !node.is_named() {
            return;
        }
        let rules = self.rules;
        let id = node.id();
        if let Some(pos) = self.covered.iter().position(|c| *c == id) {
            self.covered.swap_remove(pos);
        } else {
            rules.declare(self, node);
        }
        rules.observe(self, node);
    }

    /// Handles a node whose subtree has just been fully visited.
    fn leave(&mut self, node: Node<'t>) {
        let depth = self.depth;
        while self.scopes.len() > 1 && self.scopes.last().is_some_and(|s| s.exit_depth >= depth) {
            self.scopes.pop();
        }
        if !node.is_named() {
            return;
        }
        if !self.suppressed.is_empty() {
            let id = node.id();
            self.suppressed.retain(|s| *s != id);
        }
        let role = self.rules.role(self.src, node);
        let start_row = to_u32(node.start_position().row);
        let end_row = last_row(node);
        let frame = &mut self.frames[depth];
        let trailing = frame.prev_row == Some(start_row);
        match role {
            Role::Doc | Role::Attribute if !(trailing && role == Role::Doc) => {
                if frame.first.is_some() && start_row > frame.last_row.saturating_add(1) {
                    frame.clear_run();
                }
                if frame.first.is_none() {
                    frame.first = Some((node.start_byte(), start_row));
                }
                if role == Role::Doc {
                    frame.docs.push(node.byte_range());
                } else if frame.attr_start.is_none() {
                    frame.attr_start = Some((node.start_byte(), start_row));
                }
                frame.last_row = end_row;
            }
            _ => {
                frame.clear_run();
                frame.prev_row = Some(end_row);
            }
        }
    }

    // ---- queries for the rules --------------------------------------------------------------

    /// Returns the source text of a node, or an empty string if its range is invalid.
    pub(super) fn text(&self, node: Node<'_>) -> &'a str {
        self.src.get(node.byte_range()).unwrap_or("")
    }

    /// Returns the text of the child of `node` with the given field name.
    pub(super) fn field_text(&self, node: Node<'_>, name: &str) -> Option<&'a str> {
        node.child_by_field_name(name).map(|n| self.text(n))
    }

    /// Returns the innermost scope.
    pub(super) fn scope(&self) -> &Scope {
        // The file scope is never popped, so the stack is never empty.
        &self.scopes[self.scopes.len() - 1]
    }

    /// Returns the innermost scope, mutably.
    pub(super) fn scope_mut(&mut self) -> &mut Scope {
        let last = self.scopes.len() - 1;
        &mut self.scopes[last]
    }

    /// Returns the ancestor `up` levels above the current node (`1` is the parent).
    pub(super) fn ancestor(&self, up: usize) -> Option<Node<'t>> {
        self.depth
            .checked_sub(up)
            .and_then(|i| self.path.get(i))
            .copied()
    }

    /// Returns the field name under which the node `up` levels above the current node
    /// (`0` is the current node itself) sits in its parent.
    pub(super) fn field_of(&self, up: usize) -> Option<&'t str> {
        self.depth
            .checked_sub(up)
            .and_then(|i| self.fields.get(i))
            .copied()
            .flatten()
    }

    /// Returns the symbol drafted so far at `index`.
    pub(super) fn symbol(&self, index: usize) -> Option<&SymbolDraft> {
        self.symbols.get(index)
    }

    /// Changes the visibility of an already reported symbol.
    pub(super) fn set_visibility(&mut self, index: usize, visibility: Visibility) {
        if let Some(symbol) = self.symbols.get_mut(index) {
            symbol.visibility = visibility;
        }
    }

    /// Finds the most recent symbol named `name` that is declared directly in the current scope.
    pub(super) fn find_in_scope(&self, name: &str) -> Option<usize> {
        let owner = self.scope().symbol;
        let floor = owner.map_or(0, |i| i + 1);
        (floor..self.symbols.len())
            .rev()
            .find(|i| self.symbols[*i].parent == owner && self.symbols[*i].name == name)
    }

    /// Finds the first symbol with the given qualified name.
    pub(super) fn lookup(&self, qualified: &str) -> Option<usize> {
        self.by_qualified.get(qualified).copied()
    }

    /// Builds the qualified name of `name` in the current scope.
    pub(super) fn qualify(&self, name: &str) -> String {
        let prefix = &self.scope().prefix;
        if prefix.is_empty() {
            name.to_owned()
        } else {
            format!("{prefix}{}{name}", self.rules.separator())
        }
    }

    /// Returns `true` when `node` is reported as neither a type mention nor a reference.
    pub(super) fn is_suppressed(&self, node: Node<'_>) -> bool {
        self.suppressed.contains(&node.id())
    }

    /// Marks `node` so that [`Rules::observe`] implementations skip it.
    pub(super) fn suppress(&mut self, node: Node<'_>) {
        if self.suppressed.len() < 64 {
            self.suppressed.push(node.id());
        }
    }

    // ---- reports from the rules -------------------------------------------------------------

    /// Records an import target after collapsing its whitespace.
    pub(super) fn import(&mut self, target: &str) {
        if self.imports.len() >= MAX_IMPORTS {
            return;
        }
        let target = collapse_whitespace(target);
        if !target.is_empty() && target.len() <= 512 {
            self.imports.push(target);
        }
    }

    /// Records a reference found at `node`, owned by the innermost symbol.
    ///
    /// A [`RefKind::Call`] also extends the outline of every enclosing symbol.
    pub(super) fn reference(
        &mut self,
        name: &str,
        kind: RefKind,
        node: Node<'_>,
        qualifier: Option<&str>,
    ) {
        if name.is_empty() || name.len() > MAX_REF_NAME || name.chars().any(char::is_whitespace) {
            return;
        }
        if kind == RefKind::Call {
            self.extend_outlines(name, node.start_byte());
        }
        if self.references.len() >= MAX_REFERENCES {
            return;
        }
        let qualifier = qualifier
            .map(str::trim)
            .filter(|q| is_simple_path(q))
            .map(str::to_owned);
        self.references.push(ReferenceDraft {
            name: name.to_owned(),
            kind,
            line: to_u32(node.start_position().row) + 1,
            owner: self.scope().symbol,
            qualifier,
        });
    }

    /// Adds a callee name to the outline of every symbol whose body contains the call.
    ///
    /// Outlines are ordered by the position of the callee name in the source, which differs from
    /// the order of the walk for chained calls (`a.b().c()` enters the call of `c` first).
    fn extend_outlines(&mut self, name: &str, position: usize) {
        for scope in &self.scopes {
            if !scope.outline {
                continue;
            }
            let Some(index) = scope.symbol else {
                continue;
            };
            let (Some(symbol), Some(positions)) = (
                self.symbols.get_mut(index),
                self.outline_positions.get_mut(index),
            ) else {
                continue;
            };
            if let Some(at) = symbol.outline.iter().position(|n| n == name) {
                if position >= positions[at] {
                    continue;
                }
                symbol.outline.remove(at);
                positions.remove(at);
            }
            let at = positions.partition_point(|p| *p < position);
            if at >= MAX_OUTLINE {
                continue;
            }
            symbol.outline.insert(at, name.to_owned());
            positions.insert(at, position);
            symbol.outline.truncate(MAX_OUTLINE);
            positions.truncate(MAX_OUTLINE);
        }
    }

    /// Opens a scope that is not backed by a symbol of its own (a Rust `impl` block).
    pub(super) fn push_virtual_scope(
        &mut self,
        prefix: String,
        symbol: Option<usize>,
        kind: ScopeKind,
        access: Visibility,
    ) {
        if self.scopes.len() >= MAX_SCOPES {
            return;
        }
        self.scopes.push(Scope {
            kind,
            symbol,
            prefix,
            access,
            exit_depth: self.depth,
            outline: false,
        });
    }

    /// Opens a scope that is transparent: it keeps the owner and prefix of the enclosing scope
    /// but can change the kind or the access of what it contains.
    pub(super) fn push_transparent_scope(&mut self, kind: ScopeKind, access: Visibility) {
        if self.scopes.len() >= MAX_SCOPES {
            return;
        }
        let (symbol, prefix) = {
            let outer = self.scope();
            (outer.symbol, outer.prefix.clone())
        };
        self.scopes.push(Scope {
            kind,
            symbol,
            prefix,
            access,
            exit_depth: self.depth,
            outline: false,
        });
    }

    /// Reports a declaration. Returns the index of the new symbol, or `None` when the limits on
    /// nesting or on the number of symbols were reached.
    pub(super) fn declare(&mut self, decl: Decl<'t>) -> Option<usize> {
        if self.symbols.len() >= MAX_SYMBOLS
            || self.scopes.len() >= MAX_SCOPES
            || decl.name.trim().is_empty()
        {
            return None;
        }
        let index = self.symbols.len();
        let qualified = decl
            .qualified
            .clone()
            .unwrap_or_else(|| self.qualify(&decl.name));
        let parent = match decl.parent {
            ParentRule::Scope => self.scope().symbol,
            ParentRule::Explicit(parent) => parent,
        };
        let place = self.place(&decl);
        let signature = self.signature(&decl);
        let body = self
            .src
            .get(place.attach_start..place.end_byte)
            .unwrap_or("");
        let draft = SymbolDraft {
            name: decl.name.clone(),
            qualified_name: qualified.clone(),
            kind: decl.kind,
            sig_hash: hash_normalized(&signature),
            signature,
            doc: place.doc.clone(),
            visibility: decl.visibility,
            span: Span {
                start_line: place.span_row + 1,
                end_line: place.end_line,
                start_byte: to_u32(place.span_start),
                end_byte: to_u32(place.end_byte),
            },
            parent,
            outline: Vec::new(),
            body_hash: hash_normalized(body),
        };
        let name_line = decl
            .name_node
            .map_or(decl.core.start_position().row, |n| n.start_position().row);
        self.extras.push(SymbolExtra {
            decl_line: to_u32(decl.core.start_position().row) + 1,
            name_line: to_u32(name_line) + 1,
            attach_start: place.attach_start,
            attach_line: place.attach_row + 1,
            core_start: decl.core.start_byte(),
            core_end: decl.core.end_byte(),
            has_doc: place.doc.is_some(),
        });
        self.symbols.push(draft);
        self.outline_positions.push(Vec::new());
        self.by_qualified.entry(qualified.clone()).or_insert(index);
        if let Some(name_node) = decl.name_node {
            self.suppress(name_node);
        }
        self.covered.extend(decl.covers.iter().map(Node::id));
        if let Some(spec) = decl.scope {
            let exit_depth = if spec.open_ended {
                self.depth.saturating_sub(1)
            } else {
                self.depth
            };
            self.scopes.push(Scope {
                kind: spec.kind,
                symbol: Some(index),
                prefix: qualified,
                access: spec.access,
                exit_depth,
                outline: true,
            });
        }
        Some(index)
    }

    /// Works out where a declaration sits: its extent, where its attributes begin, and the
    /// documentation attached to it.
    fn place(&self, decl: &Decl<'t>) -> Placement {
        let outer = decl.outer;
        let start_row = to_u32(outer.start_position().row);
        let raw_end = decl
            .end_override
            .unwrap_or_else(|| outer.end_byte())
            .min(self.src.len());
        let end_byte = self.trimmed_end(raw_end);
        // The last row is the row of the last character kept: line breaks trimmed from the end
        // (a node that includes its own line terminator, or blank lines a macro swallows) do
        // not count.
        let dropped = self
            .src
            .get(end_byte..raw_end)
            .map_or(0, |trimmed| trimmed.matches('\n').count());
        let raw_row = if decl.end_override.is_some() {
            self.src
                .get(..raw_end)
                .map_or(0, |head| head.matches('\n').count())
        } else {
            outer.end_position().row
        };
        let end_line = to_u32(raw_row.saturating_sub(dropped)) + 1;
        let run = &self.frames[self.depth.saturating_sub(decl.outer_up)];
        let attached = run.attached_to(start_row);
        let (span_start, span_row) = match run.first {
            Some((byte, row)) if attached => (byte.min(outer.start_byte()), row.min(start_row)),
            _ => (outer.start_byte(), start_row),
        };
        let (attach_start, attach_row) = match run.attr_start {
            Some((byte, row)) if attached => (byte.min(outer.start_byte()), row.min(start_row)),
            _ => (outer.start_byte(), start_row),
        };
        let doc = decl.inline_doc.clone().or_else(|| {
            if !attached || run.docs.is_empty() {
                return None;
            }
            let texts: Vec<&str> = run
                .docs
                .iter()
                .filter_map(|r| self.src.get(r.clone()))
                .collect();
            self.rules.clean_doc(&texts)
        });
        Placement {
            span_start,
            span_row,
            attach_start,
            attach_row,
            end_byte,
            end_line,
            doc,
        }
    }

    /// Returns the end of `end` after trimming trailing whitespace, never before the start.
    fn trimmed_end(&self, end: usize) -> usize {
        let end = end.min(self.src.len());
        self.src.as_bytes().get(..end).map_or(end, |b| {
            let mut e = b.len();
            while e > 0 && matches!(b[e - 1], b'\n' | b'\r' | b' ' | b'\t') {
                e -= 1;
            }
            e
        })
    }

    /// Builds the one-line signature of a declaration.
    fn signature(&self, decl: &Decl<'_>) -> String {
        let start = decl.sig_start.unwrap_or_else(|| decl.core.start_byte());
        let end = decl.sig_end.max(start).min(self.src.len());
        let end = end.min(start.saturating_add(MAX_SIGNATURE_SOURCE));
        let end = floor_boundary(self.src, end);
        let start = floor_boundary(self.src, start).min(end);
        let mut raw = String::new();
        let mut at = start;
        let mut skips: Vec<&Range<usize>> = decl.sig_skip.iter().collect();
        skips.sort_by_key(|r| r.start);
        for skip in skips {
            if skip.start >= end {
                break;
            }
            if skip.start > at {
                raw.push_str(self.src.get(at..skip.start).unwrap_or(""));
            }
            at = at.max(skip.end);
        }
        if at < end {
            raw.push_str(self.src.get(at..end).unwrap_or(""));
        }
        let collapsed = tidy_signature(&collapse_whitespace(&raw));
        let trimmed = trim_signature_tail(&collapsed);
        truncate_chars(trimmed.to_owned(), MAX_SIGNATURE_CHARS)
    }

    // ---- results ----------------------------------------------------------------------------

    /// Consumes the walker and returns everything it found.
    pub(super) fn finish(self) -> Outcome {
        Outcome {
            symbols: self.symbols,
            extras: self.extras,
            references: self.references,
            imports: self.imports,
            errors: self.errors,
        }
    }
}

/// What a finished walk produced.
pub(super) struct Outcome {
    /// Symbols in source order.
    pub(super) symbols: Vec<SymbolDraft>,
    /// Extra facts, parallel to `symbols`.
    pub(super) extras: Vec<SymbolExtra>,
    /// References in source order.
    pub(super) references: Vec<ReferenceDraft>,
    /// Import targets in source order.
    pub(super) imports: Vec<String>,
    /// Number of syntax errors.
    pub(super) errors: u32,
}

/// Visits the descendants of `root` (not `root` itself) in document order without recursion.
///
/// `visit` returns `true` to descend into the node's children. At most `limit` nodes are
/// visited, so a hostile subtree cannot make a caller quadratic.
pub(super) fn for_each_descendant<'t>(
    root: Node<'t>,
    limit: usize,
    mut visit: impl FnMut(Node<'t>) -> bool,
) {
    let mut cursor = root.walk();
    if !cursor.goto_first_child() {
        return;
    }
    let mut visited = 0usize;
    loop {
        let node = cursor.node();
        visited += 1;
        if visited > limit {
            return;
        }
        if visit(node) && cursor.goto_first_child() {
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() || cursor.node() == root {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use tree_sitter::{Parser, Tree};

    use super::{for_each_descendant, last_row};

    /// Parses Rust source for the tests.
    fn parse(source: &str) -> Tree {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .expect("the Rust grammar loads");
        parser.parse(source, None).expect("the source parses")
    }

    /// Descendants come in document order, `false` prunes a subtree, and the limit stops the walk.
    #[test]
    fn descendants_are_ordered_pruned_and_bounded() {
        let tree = parse("fn a() { b(); }\nfn c() {}\n");
        let mut kinds = Vec::new();
        for_each_descendant(tree.root_node(), 1000, |n| {
            if n.is_named() {
                kinds.push(n.kind());
            }
            true
        });
        assert_eq!(kinds.first(), Some(&"function_item"));
        assert_eq!(kinds.iter().filter(|k| **k == "function_item").count(), 2);
        let call = kinds.iter().position(|k| *k == "call_expression").unwrap();
        let second = kinds.iter().rposition(|k| *k == "function_item").unwrap();
        assert!(call < second);

        let mut pruned = Vec::new();
        for_each_descendant(tree.root_node(), 1000, |n| {
            pruned.push(n.kind());
            n.kind() != "function_item"
        });
        assert_eq!(pruned, ["function_item", "function_item"]);

        let mut visited = 0;
        for_each_descendant(tree.root_node(), 5, |_| {
            visited += 1;
            true
        });
        assert_eq!(visited, 5);
    }

    /// A leaf has no descendants.
    #[test]
    fn leaf_has_no_descendants() {
        let tree = parse("x");
        let mut leaf = tree.root_node();
        while let Some(child) = leaf.child(0) {
            leaf = child;
        }
        let mut visited = 0;
        for_each_descendant(leaf, 10, |_| {
            visited += 1;
            true
        });
        assert_eq!(visited, 0);
    }

    /// A node that includes its trailing line break does not claim the next row.
    #[test]
    fn last_row_ignores_the_trailing_line_break() {
        let tree = parse("/// doc\nfn f() {}\n");
        let comment = tree.root_node().child(0).unwrap();
        assert_eq!(comment.kind(), "line_comment");
        assert_eq!(
            comment.end_position().row,
            1,
            "the comment node ends at the next row"
        );
        assert_eq!(last_row(comment), 0);
        let item = tree.root_node().child(1).unwrap();
        assert_eq!(last_row(item), 1);
    }
}
