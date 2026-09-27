// SPDX-License-Identifier: Apache-2.0
//! TOON (Token-Oriented Object Notation) encoder and decoder for compact, token-efficient
//! structured text.
//!
//! # Role in the architecture
//! TOON is the default output format of pn-ultramemory (see `docs/formats.md`): a line-oriented,
//! indentation-based notation of the JSON data model that declares the shape of uniform arrays
//! once instead of repeating keys on every row. This crate is a leaf: it converts between
//! [`serde_json::Value`] and TOON text, performs no I/O, and depends on no other workspace crate.
//!
//! It implements version **4.1** of the [TOON specification](https://github.com/toon-format/spec)
//! (MIT licensed) completely: primitives and quoting, inline arrays, tabular arrays with nested
//! field groups, list form, keyed tabular objects, objects as list items, the three delimiters,
//! comments, blank-line rules, CRLF and byte-order-mark handling, and every strict-mode error of
//! section 14. The official conformance fixtures are vendored under `tests/fixtures` and all of
//! them run in the test suite.
//!
//! # Usage
//!
//! ```
//! use pn_ultramemory_toon::{from_str, to_string};
//! use serde_json::json;
//!
//! let value = json!({"users": [{"id": 1, "name": "Ada"}, {"id": 2, "name": "Bob"}]});
//! let text = to_string(&value);
//! assert_eq!(text, "users[2]{id,name}:\n  1,Ada\n  2,Bob");
//! assert_eq!(from_str(&text).unwrap(), value);
//! ```
//!
//! # Choices the specification leaves to the implementation
//! * **Data model**: [`serde_json::Value`] with insertion-ordered objects (the workspace enables
//!   `preserve_order`). Non-finite floats cannot exist in a `serde_json::Number`; they are `null`
//!   from the moment a `Value` is built (section 3).
//! * **Numbers on decode**: integers that fit `i64`/`u64` are exact; other numeric tokens become the
//!   nearest `f64`, and integer-valued results that fit an integer type become integers; a token
//!   that overflows `f64` decodes as a string (section 4, out-of-range policy).
//! * **Numbers on encode**: canonical decimal without exponent for `1e-6 <= |n| < 1e21`, integral
//!   floats from `2^53` up are written exactly, everything else as `1.5e+300` / `1e-7`.
//! * **Nesting limit**: at most 256 nested containers on decode (section 15); deeper input is an
//!   error.
//! * **Tabs in non-strict mode**: a leading tab counts as one full indentation level.
//! * **Byte input**: the API takes `&str`, so UTF-8 validation is the caller's; the rule about
//!   ill-formed UTF-8 in byte input (section 4) does not apply.
//! * **Prototype keys**: every key, including `__proto__`, is an ordinary entry.

mod decode;
mod encode;
mod error;
mod header;
mod lines;
mod number;
mod options;
mod quote;
mod token;

pub use decode::decode;
pub use encode::encode;
pub use error::DecodeError;
pub use options::{DecodeOptions, Delimiter, EncodeOptions};

use serde_json::Value;

/// Encodes `value` with the default options (2-space indentation, comma delimiter).
///
/// # Examples
///
/// ```
/// use pn_ultramemory_toon::to_string;
/// use serde_json::json;
///
/// assert_eq!(to_string(&json!({"id": 1, "tags": ["a", "b"]})), "id: 1\ntags[2]: a,b");
/// ```
#[must_use]
pub fn to_string(value: &Value) -> String {
    encode(value, &EncodeOptions::default())
}

/// Decodes `input` in strict mode with 2-space indentation.
///
/// # Errors
///
/// Returns a [`DecodeError`] for any syntax or strict-mode violation; see [`decode`].
///
/// # Examples
///
/// ```
/// use pn_ultramemory_toon::from_str;
/// use serde_json::json;
///
/// assert_eq!(from_str("id: 1\ntags[2]: a,b").unwrap(), json!({"id": 1, "tags": ["a", "b"]}));
/// assert!(from_str("tags[3]: a,b").is_err());
/// ```
pub fn from_str(input: &str) -> Result<Value, DecodeError> {
    decode(input, &DecodeOptions::default())
}
