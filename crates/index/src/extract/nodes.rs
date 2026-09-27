// SPDX-License-Identifier: Apache-2.0
//! Helpers for looking at the direct children of a tree-sitter node.
//!
//! All helpers use a single cursor pass over the children, so they are linear in the number of
//! children, and none of them recurses.

use tree_sitter::Node;

/// Returns the first direct child of `node` whose kind is `kind`.
pub(super) fn child_of_kind<'t>(node: Node<'t>, kind: &str) -> Option<Node<'t>> {
    let mut cursor = node.walk();
    node.children(&mut cursor).find(|c| c.kind() == kind)
}

/// Returns the first direct child of `node` whose kind is one of `kinds`.
pub(super) fn child_of_kinds<'t>(node: Node<'t>, kinds: &[&str]) -> Option<Node<'t>> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|c| kinds.contains(&c.kind()))
}

/// Returns `true` when `node` has a direct child of kind `kind`.
pub(super) fn has_child_kind(node: Node<'_>, kind: &str) -> bool {
    child_of_kind(node, kind).is_some()
}

/// Returns all named direct children of `node`, in order.
pub(super) fn named_children(node: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
}

/// Returns the byte ranges of the direct children of `node` whose kind is in `kinds`.
pub(super) fn ranges_of_kinds(node: Node<'_>, kinds: &[&str]) -> Vec<std::ops::Range<usize>> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .filter(|c| kinds.contains(&c.kind()))
        .map(|c| c.byte_range())
        .collect()
}

#[cfg(test)]
mod tests {
    use tree_sitter::{Node, Parser, Tree};

    use super::{child_of_kind, child_of_kinds, has_child_kind, named_children, ranges_of_kinds};

    /// Parses Rust source for the tests.
    fn parse(source: &str) -> Tree {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .expect("the Rust grammar loads");
        parser.parse(source, None).expect("the source parses")
    }

    /// Returns the first function of a parsed file.
    fn first_item(tree: &Tree) -> Node<'_> {
        tree.root_node().named_child(0).expect("one item")
    }

    /// Child lookups find the first match, several kinds at once, and report absence.
    #[test]
    fn child_lookups() {
        let tree = parse("pub async fn f(a: u8) -> u8 { a }");
        let item = first_item(&tree);
        assert_eq!(
            child_of_kind(item, "visibility_modifier").unwrap().kind(),
            "visibility_modifier"
        );
        assert!(has_child_kind(item, "parameters"));
        assert!(!has_child_kind(item, "where_clause"));
        assert_eq!(
            child_of_kinds(item, &["where_clause", "block"])
                .unwrap()
                .kind(),
            "block"
        );
        assert!(child_of_kinds(item, &["struct_item"]).is_none());
    }

    /// Named children skip punctuation, and ranges follow the requested kinds.
    #[test]
    fn named_children_and_ranges() {
        let source = "fn f(a: u8, b: u8) {}";
        let tree = parse(source);
        let item = first_item(&tree);
        let parameters = child_of_kind(item, "parameters").unwrap();
        let named = named_children(parameters);
        assert_eq!(named.len(), 2);
        let ranges = ranges_of_kinds(parameters, &["parameter"]);
        assert_eq!(ranges.len(), 2);
        assert_eq!(&source[ranges[0].clone()], "a: u8");
        assert_eq!(&source[ranges[1].clone()], "b: u8");
    }
}
