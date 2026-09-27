// SPDX-License-Identifier: Apache-2.0
//! Deterministic random JSON generator for the property tests.
//!
//! # Role in the architecture
//! Builds values that exercise every form of the format: hostile strings and keys, numbers of every
//! magnitude, primitive arrays, uniform arrays of objects with nested groups (eligible for tabular
//! form), keyed objects, and near-misses that must fall back to list form.
//!
//! # Invariants
//! * The output depends only on the seed, so failures are reproducible.
//! * `value(depth)` never nests containers deeper than `depth` levels.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::unreadable_literal,
    reason = "the generator deliberately builds numbers from raw random bits and digit strings"
)]
#![allow(clippy::unwrap_used, reason = "test code")]

use super::Rng;
use serde_json::{Map, Number, Value, json};

/// Strings chosen to break naive quoting: delimiters, quotes, escapes, structure, numbers, spaces.
pub(crate) const NASTY_STRINGS: &[&str] = &[
    "",
    " ",
    "  ",
    " a",
    "a ",
    "\t",
    "a\tb",
    "\ta",
    "true",
    "false",
    "null",
    "TRUE",
    "Null",
    "42",
    "-3.14",
    "05",
    "+1",
    "1e-6",
    "1E5",
    "0x10",
    "1.",
    ".5",
    "-",
    "--",
    "- item",
    "-1",
    "#",
    "#tag",
    "a#b",
    "a:b",
    ":",
    "key: value",
    "a, b",
    "a,b",
    ",",
    "a|b",
    "|",
    "[]",
    "[3]",
    "[2]: x,y",
    "[",
    "]",
    "{}",
    "{a}",
    "a{b}",
    "}",
    "\"",
    "\"quoted\"",
    "\"unterminated",
    "back\\slash",
    "\\",
    "\\n",
    "\\u0041",
    "a\"b\\c",
    "line1\nline2",
    "\n",
    "cr\rlf",
    "\r",
    "\u{0}",
    "\u{1}",
    "\u{1f}",
    "\u{7f}",
    "\u{85}",
    "\u{a0}",
    "\u{feff}",
    "x\u{feff}",
    "\u{2028}",
    "é",
    "你好",
    "🚀",
    "a 🚀 b",
    "  lead",
    "trail  ",
    "null ",
    " null",
    "Infinity",
    "NaN",
    "12abc",
    "١٢٣",
    "a-b",
    "a.b",
    "_x",
    "A1",
    "__proto__",
    "constructor",
];

/// Characters used to assemble random strings and keys.
const ALPHABET: &[char] = &[
    'a', 'b', 'Z', '0', '1', '9', ' ', ',', ':', '|', '\t', '"', '\\', '[', ']', '{', '}', '-',
    '#', '.', '_', '\n', '\r', 'é', '世', '🚀', '\u{0}', '\u{7f}', 'e', 'E', '+', 't', 'r', 'u',
    'n', 'l', 'f',
];

/// Keys chosen to break naive key handling.
pub(crate) const NASTY_KEYS: &[&str] = &[
    "id",
    "name",
    "a",
    "b",
    "c",
    "x",
    "y",
    "user.name",
    "_x",
    "A1",
    "",
    " ",
    "my-key",
    "full name",
    "a:b",
    "a,b",
    "a|b",
    "a\tb",
    "[x]",
    "{y}",
    "-lead",
    "#c",
    "123",
    "1e5",
    "true",
    "null",
    "café",
    "名前",
    "q\"uote",
    "back\\slash",
    "line\nbreak",
    "x[2]",
    "k:",
    ":k",
    "__proto__",
    "constructor",
    "\u{feff}k",
    "a b",
    "é",
    "🚀",
];

/// The shape of a column in a uniform group of objects.
#[derive(Clone)]
enum Shape {
    /// A primitive column.
    Leaf,
    /// A nested object column with its own columns.
    Group(Vec<(String, Shape)>),
}

/// The random value generator.
pub(crate) struct Gen {
    /// The underlying deterministic random source.
    pub(crate) rng: Rng,
}

impl Gen {
    /// Creates a generator from a seed.
    pub(crate) fn new(seed: u64) -> Self {
        Self {
            rng: Rng::new(seed),
        }
    }

    /// Returns a random string, often one of the nasty ones.
    fn string(&mut self) -> String {
        if self.rng.chance(1, 2) {
            return (*self.rng.pick(NASTY_STRINGS)).to_string();
        }
        let len = self.rng.index(13);
        (0..len).map(|_| *self.rng.pick(ALPHABET)).collect()
    }

    /// Returns a random key, often one of the nasty ones.
    fn key(&mut self) -> String {
        if self.rng.chance(2, 3) {
            return (*self.rng.pick(NASTY_KEYS)).to_string();
        }
        let len = self.rng.index(6);
        (0..len).map(|_| *self.rng.pick(ALPHABET)).collect()
    }

    /// Returns a random JSON number covering small and huge integers and floats.
    pub(crate) fn number(&mut self) -> Value {
        let float = |f: f64| Number::from_f64(f).map_or(Value::Null, Value::Number);
        match self.rng.index(10) {
            0 => json!(self.rng.index(200) as i64 - 100),
            1 => json!(self.rng.next_u64() as i64),
            2 => json!(self.rng.next_u64()),
            3 => float(f64::from_bits(self.rng.next_u64())),
            4 => float(f64::from(self.rng.next_u64() as i32) / 1000.0),
            5 => {
                let specials = [
                    0.0,
                    -0.0,
                    1.0,
                    -1.5,
                    0.1,
                    1e-6,
                    1e-7,
                    0.000_001_5,
                    1e20,
                    1e21,
                    1e22,
                    1e300,
                    5e-324,
                    f64::MAX,
                    f64::MIN_POSITIVE,
                    123_456_789.123,
                    9_007_199_254_740_993.0,
                    1.152_921_504_606_847e18,
                    0.3333333333333333,
                ];
                float(*self.rng.pick(&specials))
            }
            6 => {
                let base = 2_f64.powi(i32::try_from(self.rng.index(80)).unwrap());
                float(base + (self.rng.index(5) as f64) - 2.0)
            }
            7 => {
                let mantissa = (self.rng.next_u64() >> 11) as f64 / 9_007_199_254_740_992.0;
                let exponent = i32::try_from(self.rng.index(600)).unwrap() - 300;
                float(mantissa * 10_f64.powi(exponent))
            }
            8 => float((self.rng.index(2_000_000) as f64 - 1_000_000.0) * 1e10),
            _ => float((self.rng.next_u64() >> 11) as f64 / 9_007_199_254_740_992.0),
        }
    }

    /// Returns a random primitive.
    fn primitive(&mut self) -> Value {
        match self.rng.index(10) {
            0 => Value::Null,
            1 => Value::Bool(self.rng.chance(1, 2)),
            2..=4 => self.number(),
            _ => Value::String(self.string()),
        }
    }

    /// Returns a random column shape with up to `depth` levels of nested groups.
    fn shape(&mut self, depth: usize) -> Vec<(String, Shape)> {
        let count = 1 + self.rng.index(4);
        let mut columns: Vec<(String, Shape)> = Vec::new();
        for _ in 0..count {
            let key = self.key();
            if columns.iter().any(|(k, _)| *k == key) {
                continue;
            }
            let shape = if depth > 0 && self.rng.chance(1, 4) {
                Shape::Group(self.shape(depth - 1))
            } else {
                Shape::Leaf
            };
            columns.push((key, shape));
        }
        columns
    }

    /// Builds one object of a uniform group, with its keys in a random order.
    fn instantiate(&mut self, shape: &[(String, Shape)]) -> Map<String, Value> {
        let mut order: Vec<usize> = (0..shape.len()).collect();
        self.rng.shuffle(&mut order);
        let mut object = Map::new();
        for i in order {
            let (key, column) = &shape[i];
            let value = match column {
                Shape::Leaf => self.primitive(),
                Shape::Group(inner) => Value::Object(self.instantiate(inner)),
            };
            object.insert(key.clone(), value);
        }
        object
    }

    /// Breaks the uniformity of one object in `objects` in a random way.
    fn sabotage(&mut self, objects: &mut [Value]) {
        let index = self.rng.index(objects.len());
        let Value::Object(target) = &mut objects[index] else {
            return;
        };
        let keys: Vec<String> = target.keys().cloned().collect();
        let victim = keys[self.rng.index(keys.len())].clone();
        match self.rng.index(7) {
            0 => {
                target.remove(&victim);
            }
            1 => {
                target.insert("extra".to_string(), json!(1));
            }
            2 => {
                target.insert(victim, json!([1]));
            }
            3 => {
                target.insert(victim, json!({}));
            }
            4 => {
                target.insert(victim, json!({"z": 1}));
            }
            5 => target.clear(),
            _ => objects[index] = Value::Null,
        }
    }

    /// Returns an array of uniform objects (often eligible for tabular form), sometimes broken.
    fn uniform_array(&mut self) -> Value {
        let shape = self.shape(2);
        let rows = 1 + self.rng.index(5);
        let mut items: Vec<Value> = (0..rows)
            .map(|_| Value::Object(self.instantiate(&shape)))
            .collect();
        if self.rng.chance(1, 5) {
            self.sabotage(&mut items);
        }
        Value::Array(items)
    }

    /// Returns an object whose values are uniform objects (often eligible for keyed form).
    fn keyed_object(&mut self) -> Value {
        let shape = self.shape(2);
        let entries = 1 + self.rng.index(4);
        let mut values: Vec<Value> = (0..entries)
            .map(|_| Value::Object(self.instantiate(&shape)))
            .collect();
        if self.rng.chance(1, 5) {
            self.sabotage(&mut values);
        }
        let mut map = Map::new();
        for value in values {
            map.insert(self.key(), value);
        }
        Value::Object(map)
    }

    /// Returns a random value with at most `depth` levels of containers.
    pub(crate) fn value(&mut self, depth: usize) -> Value {
        if depth == 0 {
            return self.primitive();
        }
        match self.rng.index(11) {
            0..=2 => self.primitive(),
            3 | 4 => {
                let mut map = Map::new();
                for _ in 0..self.rng.index(6) {
                    let key = self.key();
                    let value = self.value(depth - 1);
                    map.insert(key, value);
                }
                Value::Object(map)
            }
            5 => self.uniform_array(),
            6 => self.keyed_object(),
            7 => Value::Array((0..self.rng.index(6)).map(|_| self.primitive()).collect()),
            8 => Value::Array(
                (0..self.rng.index(6))
                    .map(|_| self.value(depth - 1))
                    .collect(),
            ),
            9 => Value::Array(
                (0..=self.rng.index(3))
                    .map(|_| {
                        if self.rng.chance(1, 2) {
                            self.uniform_array()
                        } else {
                            Value::Array((0..self.rng.index(4)).map(|_| self.primitive()).collect())
                        }
                    })
                    .collect(),
            ),
            _ => {
                let mut map = Map::new();
                map.insert(self.key(), self.uniform_array());
                map.insert(self.key(), self.keyed_object());
                map.insert(self.key(), self.value(depth - 1));
                Value::Object(map)
            }
        }
    }
}
