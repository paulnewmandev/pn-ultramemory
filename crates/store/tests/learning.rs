// SPDX-License-Identifier: Apache-2.0
//! Learning and metadata tests: utility states, the signal history, co-access with decay, the
//! reset, and the key/value store.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // Test code: a failed check aborts the test.

mod common;

use common::{NOW, func, id_of, put, store};
use pn_ultramemory_core::{
    DEFAULT_HALF_LIFE_SECS, MemoryFilter, MemoryId, SignalKind, Storage, StorageError, SymbolId,
    Target, UtilityState, decay_factor,
};

/// Two symbols to attach learning to.
fn two_symbols() -> (pn_ultramemory_store::SqliteStorage, SymbolId, SymbolId) {
    let store = store();
    put(
        &store,
        "src/a.rs",
        vec![func("alpha"), func("beta")],
        vec![],
    );
    let alpha = id_of(&store, "src/a.rs", "alpha");
    let beta = id_of(&store, "src/a.rs", "beta");
    (store, alpha, beta)
}

/// Compares two floats with a tolerance.
fn close(left: f64, right: f64) -> bool {
    (left - right).abs() < 1e-9
}

/// A utility state is stored as given and replaced as a whole.
#[test]
fn utility_states_round_trip() {
    let (store, alpha, beta) = two_symbols();
    let a = Target::Symbol(alpha);
    let b = Target::Symbol(beta);
    let m = Target::Memory(MemoryId(alpha.0));
    assert!(store.utility_states(&[a, b, m]).unwrap().is_empty());

    let first = UtilityState {
        alpha: 1.5,
        beta: 0.25,
        updated_at: 100,
    };
    store.put_utility_state(a, first).unwrap();
    store
        .put_utility_state(
            m,
            UtilityState {
                alpha: 9.0,
                beta: 8.0,
                updated_at: 7,
            },
        )
        .unwrap();
    // The same raw id under another kind of target is another target.
    let found = store.utility_states(&[b, a, m]).unwrap();
    assert_eq!(found.len(), 2);
    assert_eq!(found[0], (a, first));
    assert_eq!(found[1].0, m);
    assert!(close(found[1].1.alpha, 9.0));

    // Replaced, not merged.
    let second = UtilityState {
        alpha: 0.0,
        beta: 4.0,
        updated_at: 200,
    };
    store.put_utility_state(a, second).unwrap();
    assert_eq!(store.utility_states(&[a]).unwrap(), [(a, second)]);
    // A target asked for twice is answered once.
    assert_eq!(store.utility_states(&[a, a, a]).unwrap().len(), 1);
    assert!(store.utility_states(&[]).unwrap().is_empty());
    assert_eq!(store.learning_status().unwrap().tracked_targets, 2);
}

/// Numbers that are not finite are refused and leave the stored state alone.
#[test]
fn non_finite_utility_is_refused() {
    let (store, alpha, _) = two_symbols();
    let target = Target::Symbol(alpha);
    let good = UtilityState {
        alpha: 1.0,
        beta: 1.0,
        updated_at: 1,
    };
    store.put_utility_state(target, good).unwrap();
    for (a, b) in [
        (f64::NAN, 0.0),
        (0.0, f64::INFINITY),
        (f64::NEG_INFINITY, 1.0),
    ] {
        let result = store.put_utility_state(
            target,
            UtilityState {
                alpha: a,
                beta: b,
                updated_at: 2,
            },
        );
        assert!(matches!(result, Err(StorageError::Backend(_))), "{a} {b}");
    }
    assert_eq!(store.utility_states(&[target]).unwrap(), [(target, good)]);
}

/// Signals are counted, and logging one does not change any utility state.
#[test]
fn signals_are_logged_and_counted() {
    let (store, alpha, beta) = two_symbols();
    for kind in SignalKind::ALL {
        store.log_signal(Target::Symbol(alpha), kind, NOW).unwrap();
    }
    store
        .log_signal(Target::Symbol(beta), SignalKind::Used, NOW + 1)
        .unwrap();
    store
        .log_signal(Target::Memory(MemoryId(3)), SignalKind::Ignored, NOW + 2)
        .unwrap();
    let status = store.learning_status().unwrap();
    assert_eq!(status.signals, 7);
    assert_eq!(status.tracked_targets, 0);
    assert_eq!(status.coaccess_pairs, 0);
}

/// Co-access is unordered, adds up, and reads back decayed to the time asked.
#[test]
fn coaccess_accumulates_and_decays() {
    let (store, alpha, beta) = two_symbols();
    store.bump_coaccess(alpha, beta, 2.0, 0).unwrap();
    // The pair has no direction.
    let from_alpha = store.coaccess_neighbors(alpha, 10, 0).unwrap();
    let from_beta = store.coaccess_neighbors(beta, 10, 0).unwrap();
    assert_eq!(from_alpha, [(beta, 2.0)]);
    assert_eq!(from_beta, [(alpha, 2.0)]);
    assert_eq!(store.learning_status().unwrap().coaccess_pairs, 1);

    // One half-life later the stored strength counts half.
    let later = store
        .coaccess_neighbors(alpha, 10, DEFAULT_HALF_LIFE_SECS)
        .unwrap();
    assert!(close(later[0].1, 1.0), "{later:?}");
    let much_later = store
        .coaccess_neighbors(alpha, 10, 2 * DEFAULT_HALF_LIFE_SECS)
        .unwrap();
    assert!(close(much_later[0].1, 0.5));

    // Bumping after a half-life decays what was stored first, then adds (in either order of ids).
    store
        .bump_coaccess(beta, alpha, 1.0, DEFAULT_HALF_LIFE_SECS)
        .unwrap();
    let now = store
        .coaccess_neighbors(alpha, 10, DEFAULT_HALF_LIFE_SECS)
        .unwrap();
    assert!(close(now[0].1, 2.0), "{now:?}");
    assert_eq!(store.learning_status().unwrap().coaccess_pairs, 1);
    let expected = 2.0 * decay_factor(DEFAULT_HALF_LIFE_SECS, DEFAULT_HALF_LIFE_SECS);
    assert!(close(
        store
            .coaccess_neighbors(beta, 10, 2 * DEFAULT_HALF_LIFE_SECS)
            .unwrap()[0]
            .1,
        expected
    ));
}

/// Neighbors come strongest first, ties by id, limited, and a symbol with none has none.
#[test]
fn coaccess_neighbors_are_ranked() {
    let store = store();
    let (hub, weak, strong, tie_low, tie_high) = (
        SymbolId(100),
        SymbolId(1),
        SymbolId(2),
        SymbolId(3),
        SymbolId(4),
    );
    store.bump_coaccess(hub, weak, 1.0, 0).unwrap();
    store.bump_coaccess(strong, hub, 5.0, 0).unwrap();
    store.bump_coaccess(hub, tie_high, 3.0, 0).unwrap();
    store.bump_coaccess(tie_low, hub, 3.0, 0).unwrap();
    let ranked: Vec<_> = store
        .coaccess_neighbors(hub, 10, 0)
        .unwrap()
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    assert_eq!(ranked, [strong, tie_low, tie_high, weak]);
    assert_eq!(store.coaccess_neighbors(hub, 2, 0).unwrap().len(), 2);
    assert!(store.coaccess_neighbors(hub, 0, 0).unwrap().is_empty());
    assert!(
        store
            .coaccess_neighbors(SymbolId(555), 10, 0)
            .unwrap()
            .is_empty()
    );
}

/// Edge cases of co-access: a symbol with itself, bad numbers, negative weights, time going back.
#[test]
fn coaccess_edge_cases() {
    let (store, alpha, beta) = two_symbols();
    store.bump_coaccess(alpha, alpha, 5.0, 0).unwrap();
    assert_eq!(store.learning_status().unwrap().coaccess_pairs, 0);
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(matches!(
            store.bump_coaccess(alpha, beta, bad, 0),
            Err(StorageError::Backend(_))
        ));
    }
    assert_eq!(store.learning_status().unwrap().coaccess_pairs, 0);

    store.bump_coaccess(alpha, beta, 1.0, 1_000).unwrap();
    // A clock that went backwards does not amplify or wipe anything.
    store.bump_coaccess(alpha, beta, 1.0, 10).unwrap();
    let strength = store.coaccess_neighbors(alpha, 10, 1_000).unwrap()[0].1;
    assert!(close(strength, 2.0), "{strength}");
    // A negative weight lowers the strength but never below zero, and a zero pair is not listed.
    store.bump_coaccess(alpha, beta, -10.0, 1_000).unwrap();
    assert!(
        store
            .coaccess_neighbors(alpha, 10, 1_000)
            .unwrap()
            .is_empty()
    );
    store.bump_coaccess(alpha, beta, 0.5, 1_000).unwrap();
    assert!(close(
        store.coaccess_neighbors(alpha, 10, 1_000).unwrap()[0].1,
        0.5
    ));
}

/// Resetting forgets learning and touches nothing else.
#[test]
fn reset_learning_keeps_symbols_and_memories() {
    let (store, alpha, beta) = two_symbols();
    let memory = store
        .add_memory(
            &pn_ultramemory_core::NewMemory {
                kind: pn_ultramemory_core::MemoryKind::Fact,
                text: "kept".into(),
                provenance: pn_ultramemory_core::Provenance::User,
                about: vec![alpha],
            },
            5,
        )
        .unwrap();
    store
        .put_utility_state(
            Target::Symbol(alpha),
            UtilityState {
                alpha: 3.0,
                beta: 1.0,
                updated_at: 5,
            },
        )
        .unwrap();
    store
        .put_utility_state(
            Target::Memory(memory.id),
            UtilityState {
                alpha: 1.0,
                beta: 0.0,
                updated_at: 5,
            },
        )
        .unwrap();
    store
        .log_signal(Target::Symbol(alpha), SignalKind::Useful, 6)
        .unwrap();
    store.bump_coaccess(alpha, beta, 1.0, 7).unwrap();
    store.set_meta("root", "/repo").unwrap();
    let before = store.stats().unwrap();
    let status = store.learning_status().unwrap();
    assert_eq!(
        (
            status.signals,
            status.tracked_targets,
            status.coaccess_pairs
        ),
        (1, 2, 1)
    );

    store.reset_learning().unwrap();

    let status = store.learning_status().unwrap();
    assert_eq!(
        (
            status.signals,
            status.tracked_targets,
            status.coaccess_pairs
        ),
        (0, 0, 0)
    );
    assert!(
        store
            .utility_states(&[Target::Symbol(alpha)])
            .unwrap()
            .is_empty()
    );
    assert!(store.coaccess_neighbors(alpha, 10, 7).unwrap().is_empty());
    assert_eq!(store.stats().unwrap(), before);
    assert_eq!(store.symbol(alpha).unwrap().unwrap().name, "alpha");
    assert_eq!(
        store.list_memories(&MemoryFilter::default()).unwrap().len(),
        1
    );
    assert_eq!(store.get_meta("root").unwrap().as_deref(), Some("/repo"));
    // And it can be reset again, and learning starts over.
    store.reset_learning().unwrap();
    store.bump_coaccess(alpha, beta, 1.0, 8).unwrap();
    assert_eq!(store.learning_status().unwrap().coaccess_pairs, 1);
}

/// Metadata: missing keys, replacement, and awkward keys and values.
#[test]
fn metadata_round_trips() {
    let store = store();
    assert_eq!(store.get_meta("root").unwrap(), None);
    store.set_meta("root", "/repo").unwrap();
    assert_eq!(store.get_meta("root").unwrap().as_deref(), Some("/repo"));
    store.set_meta("root", "/elsewhere").unwrap();
    assert_eq!(
        store.get_meta("root").unwrap().as_deref(),
        Some("/elsewhere")
    );
    let long_key = "k".repeat(10_000);
    let long_value = "v".repeat(1_000_000);
    for (key, value) in [
        ("", ""),
        ("last index", "2026-09-25T10:00:00Z"),
        ("'; DROP TABLE meta; --", "\"quoted\" \\ \n \0 日本語"),
        (long_key.as_str(), long_value.as_str()),
    ] {
        store.set_meta(key, value).unwrap();
        assert_eq!(store.get_meta(key).unwrap().as_deref(), Some(value));
    }
    assert_eq!(
        store.get_meta("Root").unwrap(),
        None,
        "keys are case sensitive"
    );
}
