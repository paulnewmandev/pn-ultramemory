// SPDX-License-Identifier: Apache-2.0
//! Unit tests of the repository map: how a file is priced, which of its symbols it offers, how the
//! map is shrunk to fit a budget and what an empty map looks like.
//!
//! The end-to-end behaviour, including the budget guarantee over thousands of budgets, is checked
//! in `tests/graph_map.rs` against the real adapters.

use super::{
    Chosen, Entry, MAP_TOP_NAMES, MAP_TOP_SIGNATURES, MapFile, RepoMap, cheapest_step_down,
    compose, considered_files, measure_map, offers_for, row_tokens, shrink, symbol_key,
};
use pn_ultramemory_core::{FileId, Language, Span, SymbolId, SymbolKind, SymbolRecord, Visibility};

/// A file with `names` offerable names and `signatures` offerable signatures.
fn file(path: &str, names: usize, signatures: usize) -> MapFile {
    MapFile {
        path: path.to_owned(),
        language: "rust".to_owned(),
        lines: 120,
        symbols: 9,
        top: (0..names).map(|index| format!("name_{index}")).collect(),
        signatures: (0..signatures)
            .map(|index| {
                (
                    format!("name_{index}"),
                    format!("pub fn name_{index}(input: &str) -> bool"),
                )
            })
            .collect(),
    }
}

/// A symbol record with the fields the ordering looks at.
fn record(name: &str, line: u32, visibility: Visibility) -> SymbolRecord {
    SymbolRecord {
        id: SymbolId(1),
        file_id: FileId(1),
        path: "a.rs".to_owned(),
        language: Language::Rust,
        name: name.to_owned(),
        qualified_name: name.to_owned(),
        kind: SymbolKind::Function,
        signature: String::new(),
        doc: None,
        visibility,
        span: Span {
            start_line: line,
            end_line: line,
            start_byte: 0,
            end_byte: 0,
        },
        parent: None,
        outline: Vec::new(),
        sig_hash: 0,
        body_hash: 0,
        pagerank: 0.0,
    }
}

/// A file offers one, two or three ways of being shown, each dearer than the last.
#[test]
fn offers_grow_with_what_they_print() {
    assert_eq!(offers_for(&file("a.rs", 0, 0), 0).len(), 1);
    assert_eq!(offers_for(&file("a.rs", 2, 0), 0).len(), 2);
    let full = offers_for(&file("a.rs", 3, 5), 0);
    assert_eq!(full.len(), 3);
    assert!(full.windows(2).all(|pair| pair[0].tokens < pair[1].tokens));
    assert!(
        full.windows(2)
            .all(|pair| pair[0].utility < pair[1].utility)
    );
    assert_eq!(full.last().map(|offer| offer.names), Some(MAP_TOP_NAMES));
    let many = offers_for(&file("a.rs", 9, 30), 0);
    assert_eq!(
        many.last().map(|offer| offer.signatures),
        Some(MAP_TOP_SIGNATURES)
    );
}

/// The price of an offer is the estimate of the row it prints, so adding names costs tokens.
#[test]
fn rows_are_priced_from_their_text() {
    let file = file("crates/core/src/symbol.rs", 3, 0);
    assert!(row_tokens(&file, 0) < row_tokens(&file, 1));
    assert!(row_tokens(&file, 1) < row_tokens(&file, 3));
}

/// Symbols are offered most referenced first, then public ones, then by line and name.
#[test]
fn symbols_rank_by_degree_then_visibility() {
    let mut ranked = [
        (0, record("late_public", 50, Visibility::Public)),
        (0, record("early_private", 10, Visibility::Private)),
        (0, record("early_public", 10, Visibility::Public)),
        (7, record("hub", 90, Visibility::Private)),
    ];
    ranked.sort_by(|a, b| symbol_key(&a.1, a.0).cmp(&symbol_key(&b.1, b.0)));
    let order: Vec<&str> = ranked.iter().map(|(_, r)| r.name.as_str()).collect();
    assert_eq!(
        order,
        ["hub", "early_public", "late_public", "early_private"]
    );
}

/// The number of files whose symbols are read grows with the budget and stays bounded.
#[test]
fn considered_files_track_the_budget() {
    assert!(considered_files(100) < considered_files(3000));
    assert_eq!(considered_files(u32::MAX), super::MAX_CONSIDERED);
    assert!(considered_files(0) >= super::EXTRA_CONSIDERED);
}

/// Shrinking lowers the least valuable file first and eventually empties the map.
#[test]
fn shrinking_gives_up_the_cheapest_loss_first() {
    let entries: Vec<Entry> = [("hub.rs", 3.0), ("leaf.rs", 1.0)]
        .into_iter()
        .map(|(path, relevance)| {
            let file = file(path, 3, 4);
            let offers = offers_for(&file, 0);
            Entry {
                file,
                relevance,
                offers,
            }
        })
        .collect();
    let mut chosen: Chosen = vec![Some(2), Some(2)];
    assert!(shrink(&entries, &mut chosen, 1));
    assert_eq!(chosen, vec![Some(2), Some(1)]);
    while shrink(&entries, &mut chosen, 10_000) {}
    assert_eq!(chosen, vec![None, None]);
    assert_eq!(cheapest_step_down(&entries, &chosen), None);
}

/// An empty map still measures something, and the measure is far below any accepted budget.
#[test]
fn an_empty_map_is_tiny() {
    let map = compose(200, &[], &Chosen::new(), 7);
    assert!(map.files.is_empty());
    assert_eq!(map.omitted_files, 7);
    assert!(measure_map(&map) < 40, "{}", measure_map(&map));
    let value = RepoMap {
        budget: 200,
        used: 20,
        files: Vec::new(),
        omitted_files: 7,
    }
    .to_value();
    assert_eq!(value["map"]["omitted"], 7);
    assert!(value.get("files").is_none());
    assert!(value.get("signatures").is_none());
}
