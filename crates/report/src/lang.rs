// SPDX-License-Identifier: Apache-2.0
//! The languages a report can be written in.
//!
//! Every fixed string of every report format is looked up through [`Lang`] (see the `i18n`
//! module), and numbers are formatted according to it (see `numfmt`). The type is deliberately
//! tiny and `Copy`, so it can be threaded through every renderer by value.
//!
//! Invariant: adding a variant here forces the compiler to flag every `match` that must learn the
//! new language, and the completeness test in `i18n` fails until every string has a translation.

/// A language a report can be written in.
///
/// # Examples
///
/// ```
/// use pn_ultramemory_report::Lang;
///
/// assert_eq!(Lang::from_code("Español"), Some(Lang::Es));
/// assert_eq!(Lang::Es.code(), "es");
/// assert_eq!(Lang::default(), Lang::En);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Lang {
    /// English, the default language.
    #[default]
    En,
    /// Spanish.
    Es,
}

impl Lang {
    /// Parses a language code or name, ignoring case and surrounding whitespace.
    ///
    /// Accepted values are `en`, `english`, `es`, `spanish`, `espanol` and `español`. A locale
    /// tag such as `en-US` or `es_MX` is accepted too: only the part before the first `-` or `_`
    /// is looked at. Anything else returns `None`.
    ///
    /// # Examples
    ///
    /// ```
    /// use pn_ultramemory_report::Lang;
    ///
    /// assert_eq!(Lang::from_code("EN"), Some(Lang::En));
    /// assert_eq!(Lang::from_code(" spanish "), Some(Lang::Es));
    /// assert_eq!(Lang::from_code("es_MX"), Some(Lang::Es));
    /// assert_eq!(Lang::from_code("fr"), None);
    /// ```
    #[must_use]
    pub fn from_code(code: &str) -> Option<Self> {
        let lowered = code.trim().to_lowercase();
        let primary = lowered.split(['-', '_']).next().unwrap_or("");
        match primary {
            "en" | "english" => Some(Self::En),
            "es" | "spanish" | "espanol" | "español" => Some(Self::Es),
            _ => None,
        }
    }

    /// Returns the two-letter code of the language (`en` or `es`), also a valid BCP 47 tag.
    ///
    /// # Examples
    ///
    /// ```
    /// use pn_ultramemory_report::Lang;
    ///
    /// assert_eq!(Lang::En.code(), "en");
    /// ```
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Es => "es",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Lang;

    /// Every documented spelling parses, in any case and with surrounding spaces.
    #[test]
    fn parses_all_documented_names() {
        for name in ["en", "EN", "english", "English", " en "] {
            assert_eq!(Lang::from_code(name), Some(Lang::En), "{name}");
        }
        for name in [
            "es", "ES", "spanish", "Spanish", "espanol", "español", "ESPAÑOL", "\tes\n",
        ] {
            assert_eq!(Lang::from_code(name), Some(Lang::Es), "{name}");
        }
    }

    /// Locale tags are reduced to their primary language.
    #[test]
    fn accepts_locale_tags() {
        assert_eq!(Lang::from_code("en-US"), Some(Lang::En));
        assert_eq!(Lang::from_code("es_AR"), Some(Lang::Es));
    }

    /// Unknown, empty and hostile values return `None` instead of panicking.
    #[test]
    fn rejects_unknown_values() {
        for name in [
            "", " ", "fr", "e", "esp", "english!", "\u{0}", "🇪🇸", "en us",
        ] {
            assert_eq!(Lang::from_code(name), None, "{name:?}");
        }
    }

    /// The code round-trips through the parser.
    #[test]
    fn code_round_trips() {
        for lang in [Lang::En, Lang::Es] {
            assert_eq!(Lang::from_code(lang.code()), Some(lang));
        }
    }
}
