// SPDX-License-Identifier: Apache-2.0
//! Helpers shared by the integration tests of `pn-ultramemory-index`: fixture loading, symbol
//! and reference lookup, and the structural invariants every extraction must satisfy.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests unwrap and panic to fail loudly"
)]

use pn_ultramemory_core::{
    Extractor, FileExtract, Language, RefKind, ReferenceDraft, SymbolDraft, SymbolKind,
    hash_normalized,
};
use pn_ultramemory_index::TreeSitterExtractor;

/// Reads a fixture from `tests/fixtures`.
pub(crate) fn fixture(name: &str) -> String {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {path}: {e}"))
}

/// Extracts a source, checks the structural invariants and returns the result.
pub(crate) fn extract(language: Language, source: &str) -> FileExtract {
    let file = TreeSitterExtractor::new()
        .extract(language, source)
        .unwrap_or_else(|e| panic!("extraction failed: {e}"));
    assert_invariants(&file, source);
    let again = TreeSitterExtractor::new()
        .extract(language, source)
        .unwrap();
    assert_eq!(file, again, "extraction must be deterministic");
    file
}

/// Extracts a fixture that must parse without any syntax error.
pub(crate) fn extract_clean(language: Language, name: &str) -> (FileExtract, String) {
    let source = fixture(name);
    let file = extract(language, &source);
    assert_eq!(file.parse_errors, 0, "{name} must parse cleanly");
    (file, source)
}

/// Returns the first symbol with the given qualified name.
pub(crate) fn symbol<'a>(file: &'a FileExtract, qualified: &str) -> &'a SymbolDraft {
    file.symbols
        .iter()
        .find(|s| s.qualified_name == qualified)
        .unwrap_or_else(|| {
            let known: Vec<_> = file.symbols.iter().map(|s| &s.qualified_name).collect();
            panic!("no symbol `{qualified}` among {known:?}")
        })
}

/// Returns the symbol with the given qualified name and kind.
pub(crate) fn symbol_kind<'a>(
    file: &'a FileExtract,
    qualified: &str,
    kind: SymbolKind,
) -> &'a SymbolDraft {
    file.symbols
        .iter()
        .find(|s| s.qualified_name == qualified && s.kind == kind)
        .unwrap_or_else(|| panic!("no {kind} `{qualified}`"))
}

/// Returns the index of the symbol with the given qualified name.
pub(crate) fn index_of(file: &FileExtract, qualified: &str) -> usize {
    file.symbols
        .iter()
        .position(|s| s.qualified_name == qualified)
        .unwrap_or_else(|| panic!("no symbol `{qualified}`"))
}

/// Returns the references with the given name and kind.
pub(crate) fn refs<'a>(
    file: &'a FileExtract,
    name: &str,
    kind: RefKind,
) -> Vec<&'a ReferenceDraft> {
    file.references
        .iter()
        .filter(|r| r.name == name && r.kind == kind)
        .collect()
}

/// Returns the references with the given name and kind that are owned by `owner`.
pub(crate) fn refs_owned_by<'a>(
    file: &'a FileExtract,
    name: &str,
    kind: RefKind,
    owner: &str,
) -> Vec<&'a ReferenceDraft> {
    let owner = index_of(file, owner);
    refs(file, name, kind)
        .into_iter()
        .filter(|r| r.owner == Some(owner))
        .collect()
}

/// Asserts that a reference exists, with the given owner and qualifier. When several symbols
/// share the owner's qualified name (a prototype and its definition), any of them may own it.
pub(crate) fn assert_ref(
    file: &FileExtract,
    name: &str,
    kind: RefKind,
    owner: Option<&str>,
    qualifier: Option<&str>,
) {
    let owners: Vec<Option<usize>> = match owner {
        Some(wanted) => file
            .symbols
            .iter()
            .enumerate()
            .filter(|(_, s)| s.qualified_name == wanted)
            .map(|(i, _)| Some(i))
            .collect(),
        None => vec![None],
    };
    assert!(!owners.is_empty(), "no symbol `{owner:?}`");
    let found = file.references.iter().any(|r| {
        r.name == name
            && r.kind == kind
            && owners.contains(&r.owner)
            && r.qualifier.as_deref() == qualifier
    });
    assert!(
        found,
        "missing {kind:?} reference `{name}` owner={owner:?} qualifier={qualifier:?} in {:?}",
        file.references
            .iter()
            .filter(|r| r.name == name)
            .collect::<Vec<_>>()
    );
}

/// Byte offsets of the first character of every line, for constant-time line lookups.
fn line_starts(source: &str) -> Vec<usize> {
    let mut starts = vec![0];
    starts.extend(source.match_indices('\n').map(|(i, _)| i + 1));
    starts
}

/// Returns the number of the line (1-based) that holds the byte offset.
fn line_of(starts: &[usize], byte: usize) -> u32 {
    u32::try_from(starts.partition_point(|s| *s <= byte)).unwrap()
}

/// Checks the invariants that hold for every extraction of every language.
pub(crate) fn assert_invariants(file: &FileExtract, source: &str) {
    let starts = line_starts(source);
    for (index, symbol) in file.symbols.iter().enumerate() {
        let context = format!("symbol #{index} `{}`", symbol.qualified_name);
        assert!(!symbol.name.is_empty(), "{context}: empty name");
        assert!(
            symbol.qualified_name.ends_with(&symbol.name),
            "{context}: qualified name must end with the name"
        );
        assert!(
            !symbol.signature.contains('\n'),
            "{context}: signature spans lines"
        );
        assert!(
            symbol.signature.chars().count() <= 240,
            "{context}: signature too long"
        );
        assert_eq!(
            symbol.sig_hash,
            hash_normalized(&symbol.signature),
            "{context}: sig_hash"
        );
        assert!(symbol.outline.len() <= 24, "{context}: outline too long");
        let mut sorted = symbol.outline.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            symbol.outline.len(),
            "{context}: outline has duplicates"
        );
        if let Some(parent) = symbol.parent {
            assert!(parent < index, "{context}: parent must come first");
        }
        let (start, end) = (
            symbol.span.start_byte as usize,
            symbol.span.end_byte as usize,
        );
        assert!(
            start < end && end <= source.len(),
            "{context}: span out of range"
        );
        assert!(
            source.is_char_boundary(start) && source.is_char_boundary(end),
            "{context}: span splits a character"
        );
        let slice = &source[start..end];
        assert!(
            slice.contains(&symbol.name),
            "{context}: span does not contain the name"
        );
        assert_eq!(
            symbol.span.start_line,
            line_of(&starts, start),
            "{context}: start line"
        );
        assert_eq!(
            symbol.span.end_line,
            line_of(&starts, end - 1),
            "{context}: end line"
        );
        if symbol.doc.is_none() {
            assert_eq!(
                symbol.body_hash,
                hash_normalized(slice),
                "{context}: body_hash"
            );
        }
    }
    for reference in &file.references {
        assert!(!reference.name.is_empty());
        assert!(reference.line >= 1 && reference.line <= file.line_count.max(1));
        if let Some(owner) = reference.owner {
            assert!(owner < file.symbols.len(), "owner out of range");
        }
    }
    assert!(file.references.len() <= 20_000);
}
