// SPDX-License-Identifier: Apache-2.0
//! Canonical number formatting (encoder) and number-token parsing (decoder).
//!
//! # Role in the architecture
//! Section 2 of the TOON 4.1 specification fixes how numbers are written, and section 4 fixes
//! which tokens a decoder reads as numbers. This module is the only place that knows either rule,
//! so the encoder and the decoder cannot drift apart.
//!
//! # Encoding rules
//! * Integers (`u64`, `i64`) are written in decimal.
//! * Zero, including `-0.0`, is written `0`.
//! * A finite float with `1e-6 <= |n| < 1e21` is written in plain decimal (no exponent) with the
//!   shortest digits that read back to the same `f64`; integer-valued floats have no fraction.
//!   Integer-valued floats from `2^53` up are written with their exact decimal expansion, so the
//!   text denotes the very number that was encoded rather than a rounded neighbour.
//! * Any other finite float is written as `<digits>e<sign><exponent>` (lowercase `e`, explicit
//!   sign, shortest digits), for example `1e-7` and `1.5e+300`.
//!
//! # Decoding policy (documented per section 4)
//! * The grammar is exactly `-?[0-9]+(\.[0-9]+)?([eE][+-]?[0-9]+)?`; a leading zero in a
//!   multi-digit integer part makes the token a string. No host parser is used to decide.
//! * A token without fraction or exponent that fits `i64` or `u64` decodes exactly.
//! * Any other numeric token decodes to the nearest `f64`; if that value is integer-valued and
//!   fits `i64` or `u64` it becomes an integer (so `1e3` and `5.0` decode to `1000` and `5`).
//! * A numeric token that overflows `f64` (for example `1e999`) is not representable, so it
//!   decodes as a string rather than being rejected or silently changed.
//! * `-0` and `-0.0` decode to the integer `0`.

use serde_json::Number;
use std::fmt::Write as _;
use std::num::FpCategory;

/// Smallest magnitude (inclusive) written without an exponent.
const PLAIN_MIN: f64 = 1e-6;
/// Magnitude (exclusive) from which an exponent is used.
const PLAIN_MAX: f64 = 1e21;
/// `2^53`: from here on every finite `f64` is an integer, and shortest digits stop being exact.
const EXACT_INT_LIMIT: f64 = 9_007_199_254_740_992.0;
/// `2^63`, the exclusive upper bound of `i64` as a float.
const I64_LIMIT: f64 = 9_223_372_036_854_775_808.0;
/// `2^64`, the exclusive upper bound of `u64` as a float.
const U64_LIMIT: f64 = 18_446_744_073_709_551_616.0;

/// Appends the canonical text of `n` to `out`.
///
/// Every `serde_json::Number` is finite, so this never has to write `null` for `NaN` or infinity
/// (section 3 of the specification maps those to `null` when a `Value` is built).
pub(crate) fn push_number(out: &mut String, n: &Number) {
    if let Some(u) = n.as_u64() {
        let _ = write!(out, "{u}");
    } else if let Some(i) = n.as_i64() {
        let _ = write!(out, "{i}");
    } else if let Some(f) = n.as_f64() {
        push_f64(out, f);
    } else {
        out.push('0');
    }
}

/// Appends the canonical text of the float `x` to `out`.
///
/// Non-finite input, which a `serde_json::Number` cannot hold, is written as `null` so that the
/// function is total.
pub(crate) fn push_f64(out: &mut String, x: f64) {
    if !x.is_finite() {
        out.push_str("null");
        return;
    }
    if matches!(x.classify(), FpCategory::Zero) {
        out.push('0');
        return;
    }
    let magnitude = x.abs();
    if !(PLAIN_MIN..PLAIN_MAX).contains(&magnitude) {
        let text = format!("{x:e}");
        match text.split_once('e') {
            Some((mantissa, exponent)) if exponent.starts_with('-') => {
                let _ = write!(out, "{mantissa}e{exponent}");
            }
            Some((mantissa, exponent)) => {
                let _ = write!(out, "{mantissa}e+{exponent}");
            }
            None => out.push_str(&text),
        }
    } else if magnitude >= EXACT_INT_LIMIT {
        let _ = write!(out, "{x:.0}");
    } else {
        let _ = write!(out, "{x}");
    }
}

/// Checks `token` against the number grammar of section 4.
///
/// Returns `Some(true)` for a plain integer (no fraction, no exponent), `Some(false)` for any other
/// valid number, and `None` when the token is not a number, including forbidden leading zeros.
fn scan_number(token: &str) -> Option<bool> {
    let bytes = token.as_bytes();
    let mut i = usize::from(bytes.first() == Some(&b'-'));
    let int_start = i;
    while bytes.get(i).is_some_and(u8::is_ascii_digit) {
        i += 1;
    }
    let int_len = i - int_start;
    if int_len == 0 || (int_len > 1 && bytes[int_start] == b'0') {
        return None;
    }
    let mut plain = true;
    if bytes.get(i) == Some(&b'.') {
        i += 1;
        let frac_start = i;
        while bytes.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        if i == frac_start {
            return None;
        }
        plain = false;
    }
    if matches!(bytes.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(bytes.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        let exp_start = i;
        while bytes.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        if i == exp_start {
            return None;
        }
        plain = false;
    }
    (i == bytes.len()).then_some(plain)
}

/// Converts a finite float to a `Number`, using an integer when the value is integer-valued and
/// fits `i64` or `u64`.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the value is integer-valued and range-checked against the target type just above"
)]
fn number_from_f64(x: f64) -> Option<Number> {
    if !x.is_finite() {
        return None;
    }
    if matches!(x.classify(), FpCategory::Zero) {
        return Some(Number::from(0_u64));
    }
    if matches!(x.fract().classify(), FpCategory::Zero) {
        if (-I64_LIMIT..0.0).contains(&x) {
            return Some(Number::from(x as i64));
        }
        if (0.0..U64_LIMIT).contains(&x) {
            return Some(Number::from(x as u64));
        }
    }
    Number::from_f64(x)
}

/// Parses an unquoted token as a number, or returns `None` if it is not one.
///
/// `None` covers both "not a number" (the token is then a string) and "a number the host cannot
/// hold" (for example `1e999`), following the policy in the module documentation.
pub(crate) fn parse_number(token: &str) -> Option<Number> {
    let plain = scan_number(token)?;
    if plain {
        if let Ok(u) = token.parse::<u64>() {
            return Some(Number::from(u));
        }
        if let Ok(i) = token.parse::<i64>() {
            return Some(Number::from(i));
        }
    }
    number_from_f64(token.parse::<f64>().ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Formats a float through the canonical writer.
    fn fmt_f(x: f64) -> String {
        let mut s = String::new();
        push_f64(&mut s, x);
        s
    }

    /// Formats a JSON number literal through the canonical writer.
    fn fmt_json(text: &str) -> String {
        let value: serde_json::Value = serde_json::from_str(text).unwrap();
        let mut s = String::new();
        push_number(&mut s, value.as_number().unwrap());
        s
    }

    /// Zero and negative zero are both written `0`.
    #[test]
    fn zero_and_negative_zero() {
        assert_eq!(fmt_f(0.0), "0");
        assert_eq!(fmt_f(-0.0), "0");
        assert_eq!(fmt_json("-0.0"), "0");
        assert_eq!(fmt_json("0"), "0");
    }

    /// Integer-valued floats have no fraction and no exponent inside the plain range.
    #[test]
    fn integer_valued_floats_are_integers() {
        assert_eq!(fmt_f(1.0), "1");
        assert_eq!(fmt_f(-3.0), "-3");
        assert_eq!(fmt_f(1e6), "1000000");
        assert_eq!(fmt_f(1e20), "100000000000000000000");
        assert_eq!(fmt_json("1.0"), "1");
    }

    /// Fractions use the shortest digits that read back to the same float.
    #[test]
    fn shortest_round_trip_digits() {
        assert_eq!(fmt_f(0.1), "0.1");
        assert_eq!(fmt_f(1.5), "1.5");
        assert_eq!(fmt_f(0.333_333_333_333_333_3), "0.3333333333333333");
        assert_eq!(fmt_f(0.1 + 0.2), "0.30000000000000004");
        assert_eq!(fmt_f(-7.5), "-7.5");
        assert_eq!(fmt_f(1e-6), "0.000001");
        assert_eq!(fmt_f(1.5e-6), "0.0000015");
    }

    /// Outside `[1e-6, 1e21)` a lowercase exponent with an explicit sign is used.
    #[test]
    fn exponent_form_outside_plain_range() {
        assert_eq!(fmt_f(1e-7), "1e-7");
        assert_eq!(fmt_f(-1.5e-7), "-1.5e-7");
        assert_eq!(fmt_f(1e21), "1e+21");
        assert_eq!(fmt_f(1.5e300), "1.5e+300");
        assert_eq!(fmt_f(f64::MIN_POSITIVE), "2.2250738585072014e-308");
        assert_eq!(fmt_f(f64::MAX), "1.7976931348623157e+308");
        assert_eq!(fmt_f(5e-324), "5e-324");
    }

    /// Large integer-valued floats are written exactly, not rounded to shortest digits.
    #[test]
    fn large_floats_are_exact_integers() {
        assert_eq!(fmt_f(2f64.powi(60)), "1152921504606846976");
        assert_eq!(fmt_f(9_007_199_254_740_992.0), "9007199254740992");
        assert_eq!(fmt_f(9_007_199_254_740_991.0), "9007199254740991");
    }

    /// Integers of every `serde_json` representation are written in decimal.
    #[test]
    fn integers_in_decimal() {
        assert_eq!(fmt_json("18446744073709551615"), "18446744073709551615");
        assert_eq!(fmt_json("-9223372036854775808"), "-9223372036854775808");
        assert_eq!(fmt_json("42"), "42");
    }

    /// Non-finite floats (not representable in a `Number`) never produce invalid text.
    #[test]
    fn non_finite_is_null() {
        assert_eq!(fmt_f(f64::NAN), "null");
        assert_eq!(fmt_f(f64::INFINITY), "null");
    }

    /// Parses a token and renders the result as JSON text for easy comparison.
    fn parsed(token: &str) -> Option<String> {
        parse_number(token).map(|n| serde_json::Value::Number(n).to_string())
    }

    /// Valid number tokens follow the grammar of section 4.
    #[test]
    fn accepts_the_number_grammar() {
        assert_eq!(parsed("42").as_deref(), Some("42"));
        assert_eq!(parsed("-3.14").as_deref(), Some("-3.14"));
        assert_eq!(parsed("1.5000").as_deref(), Some("1.5"));
        assert_eq!(parsed("-1E+03").as_deref(), Some("-1000"));
        assert_eq!(parsed("2.5e2").as_deref(), Some("250"));
        assert_eq!(parsed("3E-02").as_deref(), Some("0.03"));
        assert_eq!(parsed("0e1").as_deref(), Some("0"));
        assert_eq!(parsed("-0").as_deref(), Some("0"));
        assert_eq!(parsed("-0.0").as_deref(), Some("0"));
        assert_eq!(parsed("0.5").as_deref(), Some("0.5"));
        assert_eq!(parsed("1e-10").as_deref(), Some("1e-10"));
    }

    /// Tokens outside the grammar are not numbers.
    #[test]
    fn rejects_tokens_outside_the_grammar() {
        for token in [
            "", "-", "+5", ".5", "1.", "05", "-05", "007", "0x10", "1_000", "Infinity", "NaN",
            "1e", "1e+", "--1", "1.2.3", "1 2", "١٢٣", "1,5",
        ] {
            assert!(
                parse_number(token).is_none(),
                "{token:?} must not be a number"
            );
        }
    }

    /// Integers beyond `i64` and `u64` fall back to a float; overflow beyond `f64` is a string.
    #[test]
    fn domain_edges() {
        assert_eq!(
            parsed("18446744073709551615").as_deref(),
            Some("18446744073709551615")
        );
        assert_eq!(
            parsed("-9223372036854775808").as_deref(),
            Some("-9223372036854775808")
        );
        let big = parse_number("100000000000000000000").unwrap();
        assert_eq!(big.as_f64().map(f64::to_bits), Some(1e20_f64.to_bits()));
        assert!(parse_number("1e999").is_none());
        assert!(parse_number(&"9".repeat(400)).is_none());
    }

    /// Every finite float survives a write and read cycle unchanged.
    #[test]
    fn floats_round_trip() {
        let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
        for _ in 0..20_000 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let x = f64::from_bits(state);
            if !x.is_finite() || matches!(x.classify(), FpCategory::Zero) {
                continue;
            }
            let text = fmt_f(x);
            let back = parse_number(&text).and_then(|n| n.as_f64()).unwrap();
            assert_eq!(back.to_bits(), x.to_bits(), "{x:e} -> {text} -> {back:e}");
        }
    }
}
