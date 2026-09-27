// SPDX-License-Identifier: Apache-2.0
//! Reference-implementation tests of edge resolution: random projects are stored, resolved, edited
//! and resolved again, and after every step the edges in the store must equal what a small,
//! obviously correct implementation of the documented rules computes from the same project.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // Test code: a failed check aborts the test.

mod common;

use std::collections::BTreeMap;

use common::{DraftExt, EdgeView, edges_of, put_in, store, sym};
use pn_ultramemory_core::{
    Confidence, Language, RefKind, ReferenceDraft, ResolveScope, Storage, SymbolDraft, SymbolKind,
};
use pn_ultramemory_store::SqliteStorage;

/// Resolves everything.
fn resolve_all(store: &SqliteStorage) -> u64 {
    store
        .resolve_edges(&ResolveScope::All)
        .unwrap()
        .edges_written
}

/// A small deterministic pseudo-random generator, so failures reproduce.
struct Rng(u64);

impl Rng {
    /// The next raw value.
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }

    /// A value in `0..bound` (`bound` must be positive).
    fn below(&mut self, bound: usize) -> usize {
        let bound = u64::try_from(bound).unwrap();
        usize::try_from(self.next() % bound).unwrap()
    }
}

/// A symbol of the model project.
#[derive(Clone)]
struct ModelSymbol {
    /// Qualified name.
    qualified: String,
    /// Simple name.
    name: String,
    /// First line.
    line: u32,
    /// Bumped when the declaration is edited without changing what it is called.
    version: u32,
}

/// A reference of the model project.
#[derive(Clone)]
struct ModelRef {
    /// Index of the owner symbol, or none.
    owner: Option<usize>,
    /// The referenced name.
    name: String,
    /// The reference kind.
    kind: RefKind,
    /// The line.
    line: u32,
    /// The qualifier, if any.
    qualifier: Option<String>,
}

/// A file of the model project.
#[derive(Clone)]
struct ModelFile {
    /// Its path.
    path: String,
    /// Its symbols in source order.
    symbols: Vec<ModelSymbol>,
    /// Its references.
    refs: Vec<ModelRef>,
    /// The language it is written in.
    language: Language,
}

/// The languages files are written in: six languages in four families, so that names are often
/// shared across families and within one.
const LANGUAGES: [Language; 6] = [
    Language::Rust,
    Language::Python,
    Language::TypeScript,
    Language::JavaScript,
    Language::C,
    Language::Cpp,
];

/// The pool of names; small on purpose, so that ambiguity is common.
const POOL: [&str; 9] = [
    "new", "run", "parse", "load", "save", "render", "draw", "init", "stop",
];

/// The directories files are spread over.
const DIRS: [&str; 5] = ["src/core", "src/ui", "src/ui/widgets", "lib", "tests"];

/// Generates the symbols of a file.
fn random_symbols(rng: &mut Rng) -> Vec<ModelSymbol> {
    let count = 2 + rng.below(6);
    (0..count)
        .map(|i| {
            let name = POOL[rng.below(POOL.len())].to_owned();
            let qualified = if rng.below(2) == 0 {
                name.clone()
            } else {
                format!("Type{}::{name}", rng.below(3))
            };
            ModelSymbol {
                qualified,
                name,
                line: u32::try_from(1 + i * 10 + rng.below(4)).unwrap(),
                version: 0,
            }
        })
        .collect()
}

/// Generates the references of a file with `symbol_count` symbols.
fn random_refs(rng: &mut Rng, symbol_count: usize) -> Vec<ModelRef> {
    let count = 3 + rng.below(12);
    (0..count)
        .map(|_| {
            let owner = if rng.below(8) == 0 {
                None
            } else {
                Some(rng.below(symbol_count))
            };
            let qualifier = match rng.below(4) {
                0 => Some("self".to_owned()),
                1 => Some("other".to_owned()),
                _ => None,
            };
            let kind = [
                RefKind::Call,
                RefKind::Call,
                RefKind::Type,
                RefKind::Inherit,
            ][rng.below(4)];
            ModelRef {
                owner,
                name: POOL[rng.below(POOL.len())].to_owned(),
                kind,
                line: u32::try_from(1 + rng.below(70)).unwrap(),
                qualifier,
            }
        })
        .collect()
}

/// Turns a model file into the drafts the store takes.
fn drafts(model: &ModelFile) -> (Vec<SymbolDraft>, Vec<ReferenceDraft>) {
    let symbols = model
        .symbols
        .iter()
        .map(|s| {
            sym(&s.qualified, SymbolKind::Function)
                .lines(s.line, s.line + 2)
                .body(&format!("version {}", s.version))
        })
        .collect();
    let refs = model
        .refs
        .iter()
        .map(|r| ReferenceDraft {
            name: r.name.clone(),
            kind: r.kind,
            line: r.line,
            owner: r.owner,
            qualifier: r.qualifier.clone(),
        })
        .collect();
    (symbols, refs)
}

/// Stores a model file and returns the outcome.
fn store_model(store: &SqliteStorage, model: &ModelFile) -> pn_ultramemory_core::UpsertOutcome {
    let (symbols, refs) = drafts(model);
    put_in(store, &model.path, model.language, symbols, refs)
}

/// The kind of edge a reference kind produces.
fn edge_kind_of(kind: RefKind) -> &'static str {
    match kind {
        RefKind::Call => "calls",
        RefKind::Type => "uses",
        RefKind::Inherit => "inherits",
    }
}

/// The reference implementation of the resolution rules, written for clarity, not speed.
fn oracle(store: &SqliteStorage, files: &[ModelFile]) -> Vec<EdgeView> {
    // Identities are read back from the store, in source order, like the tie-breaks need.
    let ids: Vec<Vec<i64>> = files
        .iter()
        .map(|f| {
            store
                .symbols_in_file(&f.path)
                .unwrap()
                .iter()
                .map(|s| s.id.0)
                .collect()
        })
        .collect();
    let shared_prefix =
        |a: &str, b: &str| a.bytes().zip(b.bytes()).take_while(|(x, y)| x == y).count();
    let mut edges: BTreeMap<(i64, i64, &'static str), (Confidence, u32)> = BTreeMap::new();
    let mut describe: BTreeMap<i64, String> = BTreeMap::new();
    for (fi, file) in files.iter().enumerate() {
        for (si, symbol) in file.symbols.iter().enumerate() {
            describe.insert(ids[fi][si], format!("{}::{}", file.path, symbol.qualified));
        }
    }
    for (fi, file) in files.iter().enumerate() {
        for reference in &file.refs {
            let Some(owner) = reference.owner else {
                continue;
            };
            let owner_id = ids[fi][owner];
            let excluded = file.symbols[owner].name == reference.name
                && reference.qualifier.as_deref().is_some_and(|q| {
                    !["self", "this", "Self", "cls", "$this", "static"].contains(&q)
                });
            // (file index, symbol index) of every candidate.
            let mut candidates: Vec<(usize, usize)> = Vec::new();
            for (cf, other) in files.iter().enumerate() {
                for (cs, symbol) in other.symbols.iter().enumerate() {
                    if symbol.name == reference.name
                        && other.language.family() == file.language.family()
                        && !(excluded && ids[cf][cs] == owner_id)
                    {
                        candidates.push((cf, cs));
                    }
                }
            }
            let same: Vec<(usize, usize)> =
                candidates.iter().copied().filter(|c| c.0 == fi).collect();
            let (chosen, confidence): (Vec<(usize, usize)>, Confidence) = if !same.is_empty() {
                let mut same = same;
                same.sort_by_key(|&(cf, cs)| {
                    let distance =
                        (i64::from(files[cf].symbols[cs].line) - i64::from(reference.line)).abs();
                    (distance, ids[cf][cs])
                });
                same.truncate(4);
                (same, Confidence::Resolved)
            } else if candidates.len() == 1 {
                (candidates, Confidence::Heuristic)
            } else if candidates.len() >= 2 {
                candidates.sort_by_key(|&(cf, cs)| {
                    (
                        std::cmp::Reverse(shared_prefix(&files[cf].path, &file.path)),
                        files[cf].path.clone(),
                        ids[cf][cs],
                    )
                });
                candidates.truncate(4);
                (candidates, Confidence::Guess)
            } else {
                continue;
            };
            for (cf, cs) in chosen {
                let key = (owner_id, ids[cf][cs], edge_kind_of(reference.kind));
                let entry = edges.entry(key).or_insert((confidence, reference.line));
                entry.0 = entry.0.max(confidence);
                entry.1 = entry.1.min(reference.line);
            }
        }
    }
    let mut view: Vec<EdgeView> = edges
        .into_iter()
        .map(|((src, dst, kind), (confidence, line))| {
            (
                describe[&src].clone(),
                describe[&dst].clone(),
                kind,
                confidence,
                line,
            )
        })
        .collect();
    view.sort();
    view
}

/// Generates a project of `count` files.
fn random_project(rng: &mut Rng, count: usize) -> Vec<ModelFile> {
    (0..count)
        .map(|i| {
            let symbols = random_symbols(rng);
            let refs = random_refs(rng, symbols.len());
            ModelFile {
                path: format!("{}/file{i}.rs", DIRS[rng.below(DIRS.len())]),
                symbols,
                refs,
                language: LANGUAGES[rng.below(LANGUAGES.len())],
            }
        })
        .collect()
}

/// The full resolve agrees with the reference implementation on many random projects.
#[test]
fn full_resolve_matches_the_reference_rules() {
    let mut seen = std::collections::BTreeSet::new();
    let mut total = 0;
    for seed in 1..=25 {
        let mut rng = Rng(seed);
        let count = 3 + rng.below(12);
        let project = random_project(&mut rng, count);
        let store = store();
        for model in &project {
            store_model(&store, model);
        }
        resolve_all(&store);
        let expected = oracle(&store, &project);
        assert_eq!(edges_of(&store), expected, "seed {seed}");
        total += expected.len();
        seen.extend(expected.iter().map(|edge| edge.3));
    }
    // The generated projects must exercise every tier, or the comparison proves little.
    assert!(total > 300, "only {total} edges were generated");
    assert!(seen.contains(&Confidence::Resolved));
    assert!(seen.contains(&Confidence::Heuristic));
    assert!(seen.contains(&Confidence::Guess));
}

/// After random edits and removals, a partial resolve agrees with the reference rules, whether
/// or not the caller passes the changed names.
#[test]
fn touching_resolve_matches_the_reference_rules_after_edits() {
    for seed in 100..118 {
        let mut rng = Rng(seed);
        let count = 4 + rng.below(8);
        let mut project = random_project(&mut rng, count);
        let store = store();
        for model in &project {
            store_model(&store, model);
        }
        resolve_all(&store);
        for round in 0..12 {
            let index = rng.below(project.len());
            match rng.below(9) {
                0 => {
                    // Rewrite the file from scratch (symbols and references).
                    project[index].symbols = random_symbols(&mut rng);
                    project[index].refs = random_refs(&mut rng, project[index].symbols.len());
                }
                1 => {
                    // Only the references change.
                    project[index].refs = random_refs(&mut rng, project[index].symbols.len());
                }
                2 => {
                    // A file disappears.
                    if project.len() > 2 {
                        project.remove(index);
                        let present: Vec<String> = project.iter().map(|f| f.path.clone()).collect();
                        store.remove_files_not_in(&present, 5).unwrap();
                        store
                            .resolve_edges(&ResolveScope::Touching {
                                file_ids: vec![],
                                names: vec![],
                            })
                            .unwrap();
                        assert_eq!(
                            edges_of(&store),
                            oracle(&store, &project),
                            "seed {seed} round {round}"
                        );
                    }
                    continue;
                }
                3 => {
                    // A symbol is dropped and its references are re-generated.
                    if project[index].symbols.len() > 1 {
                        let victim = rng.below(project[index].symbols.len());
                        project[index].symbols.remove(victim);
                        project[index].refs = random_refs(&mut rng, project[index].symbols.len());
                    }
                }
                4 => {
                    // A symbol is added at the end; the references stay as they were.
                    let mut extra = random_symbols(&mut rng);
                    extra.truncate(1);
                    for symbol in &mut extra {
                        symbol.line += 200;
                    }
                    project[index].symbols.extend(extra);
                }
                5 | 6 => {
                    // Only modified: declarations change, names and set of symbols do not.
                    for symbol in &mut project[index].symbols {
                        if rng.below(2) == 0 {
                            symbol.version += 1;
                        }
                    }
                }
                7 => {
                    // The same file is now written in another language, so it moves to
                    // another family (or not).
                    project[index].language = LANGUAGES[rng.below(LANGUAGES.len())];
                }
                _ => {
                    // Only moved: the same declarations on other lines.
                    let shift = u32::try_from(1 + rng.below(30)).unwrap();
                    for symbol in &mut project[index].symbols {
                        symbol.line += shift;
                    }
                }
            }
            let outcome = store_model(&store, &project[index]);
            let names = if round % 2 == 0 {
                outcome.changed_names
            } else {
                vec![]
            };
            store
                .resolve_edges(&ResolveScope::Touching {
                    file_ids: vec![outcome.file_id.unwrap()],
                    names,
                })
                .unwrap();
            assert_eq!(
                edges_of(&store),
                oracle(&store, &project),
                "seed {seed} round {round}"
            );
        }
        // And a full resolve changes nothing.
        let partial = edges_of(&store);
        resolve_all(&store);
        assert_eq!(edges_of(&store), partial, "seed {seed}");
    }
}

/// A name for a dense project: a third of them come from the small shared pool, the rest from a
/// wide pool where most names are unique or nearly so.
fn dense_name(rng: &mut Rng) -> String {
    if rng.below(3) == 0 {
        POOL[rng.below(POOL.len())].to_owned()
    } else {
        format!("rare_{}", rng.below(220))
    }
}

/// Generates a larger file: many symbols and many references.
fn dense_file(rng: &mut Rng, index: usize) -> ModelFile {
    let symbol_count = 15 + rng.below(12);
    let symbols: Vec<ModelSymbol> = (0..symbol_count)
        .map(|i| {
            let name = dense_name(rng);
            ModelSymbol {
                qualified: if rng.below(4) == 0 {
                    format!("Owner{}::{name}", rng.below(3))
                } else {
                    name.clone()
                },
                name,
                line: u32::try_from(1 + i * 9 + rng.below(3)).unwrap(),
                version: 0,
            }
        })
        .collect();
    let refs = (0..40 + rng.below(40))
        .map(|_| ModelRef {
            owner: Some(rng.below(symbol_count)),
            name: dense_name(rng),
            kind: [RefKind::Call, RefKind::Type, RefKind::Inherit][rng.below(3)],
            line: u32::try_from(1 + rng.below(300)).unwrap(),
            qualifier: [
                None,
                None,
                None,
                Some("self".to_owned()),
                Some("x".to_owned()),
            ][rng.below(5)]
            .clone(),
        })
        .collect();
    ModelFile {
        path: format!("{}/dense{index}.rs", DIRS[rng.below(DIRS.len())]),
        symbols,
        refs,
        language: LANGUAGES[rng.below(LANGUAGES.len())],
    }
}

/// A larger project, with a mix of unique and shared names, resolves like the reference rules,
/// in full and after a batch of files is rewritten and resolved partially.
#[test]
fn dense_project_matches_the_reference_rules() {
    for seed in [7, 8, 9] {
        let mut rng = Rng(seed);
        let mut project: Vec<ModelFile> = (0..30).map(|i| dense_file(&mut rng, i)).collect();
        let store = store();
        for model in &project {
            store_model(&store, model);
        }
        resolve_all(&store);
        assert_eq!(
            edges_of(&store),
            oracle(&store, &project),
            "seed {seed}, full"
        );

        let mut touched = Vec::new();
        let mut names = Vec::new();
        for index in [3, 11, 19, 27] {
            project[index] = ModelFile {
                path: project[index].path.clone(),
                ..dense_file(&mut rng, index)
            };
            let outcome = store_model(&store, &project[index]);
            touched.push(outcome.file_id.unwrap());
            names.extend(outcome.changed_names);
        }
        store
            .resolve_edges(&ResolveScope::Touching {
                file_ids: touched,
                names,
            })
            .unwrap();
        assert_eq!(
            edges_of(&store),
            oracle(&store, &project),
            "seed {seed}, partial"
        );
        let confidences: std::collections::BTreeSet<_> =
            edges_of(&store).iter().map(|edge| edge.3).collect();
        assert_eq!(confidences.len(), 3, "seed {seed}: every tier is exercised");
    }
}
