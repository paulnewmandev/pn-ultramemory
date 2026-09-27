// SPDX-License-Identifier: Apache-2.0
//! Helpers shared by the integration tests of `pn-ultramemory-toon`.
//!
//! # Role in the architecture
//! Test-only support code: a deterministic pseudo-random generator (no external crate), the JSON
//! equality rule of section 2 of the TOON specification, and an independent oracle that predicts how
//! tabular and keyed forms reorder object keys, so that round-trip tests can compare exactly.
//!
//! # Invariants
//! * The generator is seeded explicitly, so every run produces the same values.
//! * [`check_model_eq`] is order sensitive for object keys and value equal for numbers
//!   (integer-valued numbers equal their integer form, `-0` equals `0`).
//! * [`normalize`] re-implements the detection rules of sections 9.3 and 9.5 on purpose, so it is
//!   an oracle for the encoder rather than a copy of it.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers"
)]

pub(crate) mod generator;

use serde_json::{Map, Number, Value};

/// A small deterministic pseudo-random generator (splitmix64).
pub(crate) struct Rng(u64);

impl Rng {
    /// Creates a generator from a seed.
    pub(crate) fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// Returns the next 64 random bits.
    pub(crate) fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Returns a value in `0..len` (`len` must be non-zero).
    pub(crate) fn index(&mut self, len: usize) -> usize {
        let len = u64::try_from(len).unwrap();
        usize::try_from(self.next_u64() % len).unwrap()
    }

    /// Returns `true` with probability `numerator / denominator`.
    pub(crate) fn chance(&mut self, numerator: u64, denominator: u64) -> bool {
        self.next_u64() % denominator < numerator
    }

    /// Returns a reference to a random element of `items`.
    pub(crate) fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.index(items.len())]
    }

    /// Shuffles `items` in place (Fisher-Yates).
    pub(crate) fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let j = self.index(i + 1);
            items.swap(i, j);
        }
    }
}

/// A number reduced to its mathematical identity for comparison.
#[derive(Debug, PartialEq, Eq)]
enum Canonical {
    /// An integer, including integer-valued floats below `1e38`.
    Int(i128),
    /// Any other finite float, by its bit pattern.
    Float(u64),
}

/// Reduces a JSON number to the identity used by the JSON-model equality of section 2.
#[allow(
    clippy::cast_possible_truncation,
    reason = "integer-valued floats are range-checked"
)]
fn canonical(n: &Number) -> Canonical {
    if let Some(u) = n.as_u64() {
        return Canonical::Int(i128::from(u));
    }
    if let Some(i) = n.as_i64() {
        return Canonical::Int(i128::from(i));
    }
    let f = n.as_f64().unwrap();
    if f.fract().classify() == std::num::FpCategory::Zero && f.abs() < 1e38 {
        return Canonical::Int(f as i128);
    }
    Canonical::Float(f.to_bits())
}

/// Checks the JSON-model equality of section 2, with object key order significant.
///
/// # Errors
///
/// Returns a description of the first difference, with a path such as `$.a[2].b`.
pub(crate) fn check_model_eq(expected: &Value, actual: &Value) -> Result<(), String> {
    eq(expected, actual, "$")
}

/// Recursive worker of [`check_model_eq`].
fn eq(expected: &Value, actual: &Value, path: &str) -> Result<(), String> {
    match (expected, actual) {
        (Value::Null, Value::Null) => Ok(()),
        (Value::Bool(a), Value::Bool(b)) if a == b => Ok(()),
        (Value::String(a), Value::String(b)) if a == b => Ok(()),
        (Value::Number(a), Value::Number(b)) if canonical(a) == canonical(b) => Ok(()),
        (Value::Array(a), Value::Array(b)) => {
            if a.len() != b.len() {
                return Err(format!("{path}: array length {} != {}", a.len(), b.len()));
            }
            for (i, (x, y)) in a.iter().zip(b).enumerate() {
                eq(x, y, &format!("{path}[{i}]"))?;
            }
            Ok(())
        }
        (Value::Object(a), Value::Object(b)) => {
            if !a.keys().eq(b.keys()) {
                let (ka, kb): (Vec<_>, Vec<_>) = (a.keys().collect(), b.keys().collect());
                return Err(format!("{path}: keys {ka:?} != {kb:?}"));
            }
            for (k, x) in a {
                eq(x, &b[k], &format!("{path}.{k}"))?;
            }
            Ok(())
        }
        _ => Err(format!("{path}: {expected} != {actual}")),
    }
}

/// Returns `true` for null, booleans, numbers and strings.
fn is_primitive(value: &Value) -> bool {
    !matches!(value, Value::Array(_) | Value::Object(_))
}

/// Oracle for the uniform-columns rule of sections 9.3 and 9.5.
fn uniform(objects: &[&Map<String, Value>]) -> bool {
    let Some(first) = objects.first() else {
        return false;
    };
    if first.is_empty() {
        return false;
    }
    let same_keys = objects
        .iter()
        .all(|o| o.len() == first.len() && o.keys().all(|k| first.contains_key(k)));
    if !same_keys {
        return false;
    }
    first.keys().all(|key| {
        let column: Vec<&Value> = objects.iter().map(|o| &o[key]).collect();
        if column.iter().all(|v| is_primitive(v)) {
            return true;
        }
        let nested: Option<Vec<&Map<String, Value>>> = column
            .iter()
            .map(|v| v.as_object().filter(|m| !m.is_empty()))
            .collect();
        nested.is_some_and(|inner| uniform(&inner))
    })
}

/// Rebuilds `object` with the key order of `template`, recursively for nested groups.
fn reorder(object: &Map<String, Value>, template: &Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    for (key, template_value) in template {
        let value = &object[key];
        let rebuilt = match (value, template_value) {
            (Value::Object(inner), Value::Object(inner_template)) => {
                Value::Object(reorder(inner, inner_template))
            }
            _ => value.clone(),
        };
        out.insert(key.clone(), rebuilt);
    }
    out
}

/// Where a value sits, as far as the choice of form is concerned.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pos {
    /// The document root or an object field.
    Open,
    /// An element of an array in list form (never tabular, never keyed).
    Item,
}

/// Predicts the value a decoder returns for the encoder's output of `value`: tabular arrays and
/// keyed objects list their keys in the order of their first element or entry (section 2).
pub(crate) fn normalize(value: &Value) -> Value {
    norm(value, Pos::Open)
}

/// Recursive worker of [`normalize`].
fn norm(value: &Value, pos: Pos) -> Value {
    match value {
        Value::Array(items) => {
            let objects: Option<Vec<&Map<String, Value>>> =
                items.iter().map(Value::as_object).collect();
            if let Some(objects) = objects.filter(|o| pos == Pos::Open && uniform(o)) {
                let template = objects[0];
                return Value::Array(
                    objects
                        .iter()
                        .map(|o| Value::Object(reorder(o, template)))
                        .collect(),
                );
            }
            Value::Array(items.iter().map(|item| norm(item, Pos::Item)).collect())
        }
        Value::Object(map) => {
            let entries: Option<Vec<&Map<String, Value>>> =
                map.values().map(Value::as_object).collect();
            if let Some(entries) =
                entries.filter(|e| pos == Pos::Open && map.len() >= 2 && uniform(e))
            {
                let template = entries[0];
                let mut out = Map::new();
                for (key, entry) in map.keys().zip(entries) {
                    out.insert(key.clone(), Value::Object(reorder(entry, template)));
                }
                return Value::Object(out);
            }
            Value::Object(
                map.iter()
                    .map(|(k, v)| (k.clone(), norm(v, Pos::Open)))
                    .collect(),
            )
        }
        other => other.clone(),
    }
}
