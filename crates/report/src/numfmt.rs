// SPDX-License-Identifier: Apache-2.0
//! Number formatting that follows the language of the report.
//!
//! English groups thousands with a comma and marks decimals with a point (`1,234.5`); Spanish
//! groups with a point and marks decimals with a comma (`1.234,5`). Percentages carry at most one
//! decimal and drop a trailing zero, and Spanish separates the sign with a non-breaking space
//! (`87,5 %`) as the Royal Spanish Academy recommends.
//!
//! Invariants: the functions are pure, never panic, never depend on the platform locale, and map
//! non-finite floats to `0` so a corrupt value cannot poison a whole report.

use crate::lang::Lang;

/// The non-breaking space Spanish puts between a number and `%`.
const NBSP: char = '\u{a0}';

/// Converts a count to `f64`; counts beyond 2^53 lose their lowest bits, which is irrelevant for
/// the percentages and bar lengths this crate computes.
#[allow(clippy::cast_precision_loss)] // reason: counters far beyond 2^53 do not occur in practice
pub(crate) const fn to_f64(n: u64) -> f64 {
    n as f64
}

/// Converts a length or index to `f64` (see [`to_f64`]).
pub(crate) fn usize_to_f64(n: usize) -> f64 {
    to_f64(u64::try_from(n).unwrap_or(u64::MAX))
}

/// Returns the thousands separator of `lang`.
const fn group_separator(lang: Lang) -> char {
    match lang {
        Lang::En => ',',
        Lang::Es => '.',
    }
}

/// Returns the decimal separator of `lang`.
const fn decimal_separator(lang: Lang) -> char {
    match lang {
        Lang::En => '.',
        Lang::Es => ',',
    }
}

/// Inserts the thousands separator of `lang` into a string of ASCII digits.
fn group_digits(digits: &str, lang: Lang) -> String {
    let sep = group_separator(lang);
    let len = digits.len();
    let mut out = String::with_capacity(len + len / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (len - index) % 3 == 0 {
            out.push(sep);
        }
        out.push(digit);
    }
    out
}

/// Formats an unsigned integer with thousands separators.
///
/// # Examples
///
/// ```text
/// int(1234567, Lang::En) == "1,234,567"
/// int(1234567, Lang::Es) == "1.234.567"
/// ```
pub(crate) fn int(n: u64, lang: Lang) -> String {
    group_digits(&n.to_string(), lang)
}

/// Formats a float with at most `max_decimals` decimals, dropping trailing zeros.
///
/// Non-finite values format as `0`. A value that rounds to zero never carries a minus sign.
pub(crate) fn decimal(value: f64, max_decimals: usize, lang: Lang) -> String {
    let value = if value.is_finite() { value } else { 0.0 };
    let fixed = format!("{:.*}", max_decimals, value.abs());
    let (whole, fraction) = fixed.split_once('.').unwrap_or((fixed.as_str(), ""));
    let fraction = fraction.trim_end_matches('0');
    let mut out = String::new();
    if value < 0.0 && (whole.trim_start_matches('0').len() + fraction.len()) > 0 {
        out.push('-');
    }
    out.push_str(&group_digits(whole, lang));
    if !fraction.is_empty() {
        out.push(decimal_separator(lang));
        out.push_str(fraction);
    }
    out
}

/// Formats a percentage with at most one decimal (`87.5%`, or `87,5 %` in Spanish).
pub(crate) fn percent(value: f64, lang: Lang) -> String {
    let number = decimal(value, 1, lang);
    match lang {
        Lang::En => format!("{number}%"),
        Lang::Es => format!("{number}{NBSP}%"),
    }
}

/// Formats a whole-number percentage such as the average budget share.
pub(crate) fn percent_int(value: u32, lang: Lang) -> String {
    let number = int(u64::from(value), lang);
    match lang {
        Lang::En => format!("{number}%"),
        Lang::Es => format!("{number}{NBSP}%"),
    }
}

/// Returns `part / whole * 100`, clamped to `0..=100`, and `0` when `whole` is zero.
pub(crate) fn ratio_percent(part: u64, whole: u64) -> f64 {
    if whole == 0 {
        return 0.0;
    }
    (to_f64(part) / to_f64(whole) * 100.0).clamp(0.0, 100.0)
}

/// Formats a coordinate or length for SVG and PDF output: at most two decimals, no exponent, no
/// trailing zeros, and `0` for non-finite input.
pub(crate) fn coord(value: f64) -> String {
    let value = if value.is_finite() {
        value.clamp(-1.0e7, 1.0e7)
    } else {
        0.0
    };
    let mut text = format!("{value:.2}");
    if text.contains('.') {
        while text.ends_with('0') {
            text.pop();
        }
        if text.ends_with('.') {
            text.pop();
        }
    }
    if text == "-0" { "0".to_owned() } else { text }
}

#[cfg(test)]
mod tests {
    use super::{coord, decimal, int, percent, percent_int, ratio_percent};
    use crate::lang::Lang;

    /// Integers group by thousands with the separator of each language.
    #[test]
    fn integers_are_grouped_per_language() {
        assert_eq!(int(0, Lang::En), "0");
        assert_eq!(int(999, Lang::En), "999");
        assert_eq!(int(1000, Lang::En), "1,000");
        assert_eq!(int(1_234_567, Lang::En), "1,234,567");
        assert_eq!(int(1_234_567, Lang::Es), "1.234.567");
        assert_eq!(int(u64::MAX, Lang::En), "18,446,744,073,709,551,615");
        assert_eq!(int(u64::MAX, Lang::Es), "18.446.744.073.709.551.615");
    }

    /// Decimals use the decimal mark of each language and drop trailing zeros.
    #[test]
    fn decimals_follow_the_language() {
        assert_eq!(decimal(1234.5, 1, Lang::En), "1,234.5");
        assert_eq!(decimal(1234.5, 1, Lang::Es), "1.234,5");
        assert_eq!(decimal(2.0, 1, Lang::En), "2");
        assert_eq!(decimal(2.04, 1, Lang::Es), "2");
        assert_eq!(decimal(0.05, 1, Lang::En), "0.1");
        assert_eq!(decimal(1_000_000.25, 2, Lang::Es), "1.000.000,25");
    }

    /// Negative, zero-rounding and non-finite values are handled without a stray sign or panic.
    #[test]
    fn decimals_handle_odd_values() {
        assert_eq!(decimal(-1234.56, 1, Lang::En), "-1,234.6");
        assert_eq!(decimal(-0.04, 1, Lang::En), "0");
        assert_eq!(decimal(f64::NAN, 1, Lang::En), "0");
        assert_eq!(decimal(f64::INFINITY, 1, Lang::Es), "0");
        assert_eq!(decimal(f64::NEG_INFINITY, 1, Lang::En), "0");
    }

    /// Percentages have at most one decimal, and Spanish uses a non-breaking space.
    #[test]
    fn percentages_have_one_decimal_at_most() {
        assert_eq!(percent(87.5, Lang::En), "87.5%");
        assert_eq!(percent(87.5, Lang::Es), "87,5\u{a0}%");
        assert_eq!(percent(100.0, Lang::En), "100%");
        assert_eq!(percent(33.333_333, Lang::En), "33.3%");
        assert_eq!(percent(0.0, Lang::Es), "0\u{a0}%");
        assert_eq!(percent_int(42, Lang::En), "42%");
        assert_eq!(percent_int(1042, Lang::Es), "1.042\u{a0}%");
    }

    /// The ratio is clamped, and an empty whole gives zero.
    #[test]
    fn ratio_is_clamped() {
        assert!((ratio_percent(1, 4) - 25.0).abs() < 1e-9);
        assert!((ratio_percent(5, 4) - 100.0).abs() < 1e-9);
        assert!(ratio_percent(3, 0).abs() < 1e-9);
        assert!(ratio_percent(0, 0).abs() < 1e-9);
    }

    /// Coordinates never use an exponent or a negative zero and trim trailing zeros.
    #[test]
    fn coordinates_are_compact() {
        assert_eq!(coord(12.0), "12");
        assert_eq!(coord(12.5), "12.5");
        assert_eq!(coord(12.345), "12.35");
        assert_eq!(coord(-0.001), "0");
        assert_eq!(coord(-3.25), "-3.25");
        assert_eq!(coord(f64::NAN), "0");
        assert_eq!(coord(1.0e30), "10000000");
        assert_eq!(coord(100.0), "100");
    }
}
