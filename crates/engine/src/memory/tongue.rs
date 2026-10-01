// SPDX-License-Identifier: Apache-2.0
//! Which language a memory is written in, and the function words of each.
//!
//! # Why a memory has a language at all
//! Three of the things this module's callers do are language-specific: dropping the words that
//! carry no meaning, finding the word a negation attaches to, and finding the thing a decision
//! picked. Each needs a list of function words, and those lists differ per language.
//!
//! The obvious shortcut — one list holding every language's words — is wrong, and wrong in the
//! direction that matters. `sin` is "without" in Spanish and a noun in English; `no` negates in
//! both; `usa` is "uses" in Spanish and a country in English. A single list makes an English
//! memory trip over Spanish grammar and the other way round, and the failure lands in contradiction
//! detection, which is the one place this subsystem must not produce a false report.
//!
//! So each memory is read in one language, and only that language's words are applied to it.
//!
//! # How the language is decided
//! By counting, not by guessing: how many of a text's words appear in each language's list of
//! function words. Function words are the most frequent words in any language and they are almost
//! entirely distinct between these two, which makes the count a strong signal over a sentence and
//! a weak one over three words.
//!
//! A tie is English, because that is what the rest of the tool is written in and because the cost
//! of the two mistakes is not equal: reading Spanish as English loses a little recall, while
//! reading English as Spanish could apply `sin` as a negation to a sentence that meant the noun.
//!
//! # What this does not do
//! It knows two languages. A memory in a third is read as English, which drops nothing it should
//! have dropped and finds no negation — so it is stored and retrieved correctly and is simply less
//! likely to be recognised as a duplicate of another memory in that language. That is the safe
//! failure, and it is the same one the tool had for every language before this existed.

/// The language a memory's function words are read in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tongue {
    /// English, and the fallback for anything not recognised.
    #[default]
    English,
    /// Spanish, chosen when a text carries more Spanish function words than English ones.
    Spanish,
}

/// English words carrying too little meaning to count towards similarity.
const EN_STOP: [&str; 42] = [
    "a", "an", "and", "are", "as", "at", "be", "been", "but", "by", "can", "do", "for", "from",
    "has", "have", "if", "in", "into", "is", "it", "its", "of", "on", "or", "our", "should", "so",
    "than", "that", "the", "then", "there", "this", "to", "was", "we", "were", "when", "which",
    "with", "you",
];

/// Spanish words carrying too little meaning to count towards similarity.
///
/// A word that is also a meaningful English word is left out on purpose — `son`, `era`, `mas`, `ha`
/// and `he` all read as content in English, and dropping them from an English memory would lose
/// exactly the word that distinguishes it. The list is shorter than it could be for that reason.
const ES_STOP: [&str; 44] = [
    "al", "algo", "como", "con", "cual", "cuando", "de", "del", "desde", "donde", "el", "ella",
    "ellos", "en", "entre", "esa", "ese", "eso", "esta", "estas", "este", "esto", "estos", "hasta",
    "hay", "la", "las", "lo", "los", "muy", "para", "pero", "por", "porque", "que", "se", "segun",
    "ser", "si", "sobre", "su", "sus", "una", "unos",
];

/// English words that turn a claim into its opposite.
///
/// `instead` and `rather` are here because of what follows them: "instead of X" and "rather than X"
/// both deny X, and `of` and `than` are stop words, so the marker ends up next to the word it
/// denies exactly as `not` does.
const EN_NEGATION: [&str; 11] = [
    "not", "never", "no", "none", "don't", "doesn't", "avoid", "stop", "without", "instead",
    "rather",
];

/// Spanish words that turn a claim into its opposite.
///
/// `lugar` and `vez` are here for the same reason `instead` is in the English list: "en lugar de X"
/// and "en vez de X" both deny X, and `en` and `de` are stop words, so the marker lands next to the
/// word it denies.
const ES_NEGATION: [&str; 14] = [
    "no", "nunca", "ni", "sin", "jamas", "tampoco", "nada", "ninguno", "ninguna", "ningun",
    "evitar", "evita", "lugar", "vez",
];

/// English words that introduce the thing a decision picked.
const EN_CHOICE: [&str; 6] = ["use", "prefer", "choose", "adopt", "switch", "keep"];

/// Spanish words that introduce the thing a decision picked.
const ES_CHOICE: [&str; 10] = [
    "usar",
    "usamos",
    "preferir",
    "preferimos",
    "elegir",
    "elegimos",
    "escoger",
    "adoptar",
    "mantener",
    "cambiar",
];

/// Suffixes stripped to find an English root, longest first so `-es` is not read as `-s`.
const EN_SUFFIX: [&str; 4] = ["ing", "ed", "es", "s"];

/// Suffixes stripped to find a Spanish root.
///
/// Infinitives and participles first, then plurals, so `calibrar` and `calibrado` reach the same
/// root as `calibra`. Spanish inflects far more than English, which is why this list is longer.
const ES_SUFFIX: [&str; 12] = [
    "aciones", "ación", "acion", "ando", "iendo", "ados", "idos", "ada", "ado", "ida", "ido", "es",
];

/// The shortest a stripped word may be and still be treated as a root.
pub(super) const MIN_ROOT_LEN: usize = 4;

impl Tongue {
    /// A short, stable name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::Spanish => "es",
        }
    }

    /// The words this language drops as carrying too little meaning.
    const fn stop(self) -> &'static [&'static str] {
        match self {
            Self::English => &EN_STOP,
            Self::Spanish => &ES_STOP,
        }
    }

    /// The words this language negates with.
    pub(super) const fn negations(self) -> &'static [&'static str] {
        match self {
            Self::English => &EN_NEGATION,
            Self::Spanish => &ES_NEGATION,
        }
    }

    /// The words this language introduces a choice with.
    pub(super) const fn choices(self) -> &'static [&'static str] {
        match self {
            Self::English => &EN_CHOICE,
            Self::Spanish => &ES_CHOICE,
        }
    }

    /// Whether `word` is a stop word in this language.
    pub(crate) fn is_stop(self, word: &str) -> bool {
        self.stop().contains(&word)
    }

    /// A crude root of a word, used only to compare a denial against a plain statement.
    ///
    /// A language inflects the same verb differently in the two places that matter here: a denial
    /// writes "does not **reject**" while the statement it denies writes "the parser **rejects**".
    /// Comparing the words as written would miss the commonest shape of a contradiction.
    ///
    /// Only a few suffixes are stripped, and only when what remains is still long enough to be a
    /// word. An aggressive stem would make unrelated words equal, and every such collision turns
    /// two notes that agree into a false report — the one mistake this must not make.
    #[must_use]
    pub fn root(self, word: &str) -> &str {
        let suffixes: &[&str] = match self {
            Self::English => &EN_SUFFIX,
            Self::Spanish => &ES_SUFFIX,
        };
        for suffix in suffixes {
            if let Some(stem) = word.strip_suffix(suffix) {
                if stem.chars().count() >= MIN_ROOT_LEN {
                    return stem;
                }
            }
        }
        word
    }
}

/// The language a text is written in, decided by counting its function words.
///
/// A tie is [`Tongue::English`]: it is what the rest of the tool is written in, and reading Spanish
/// as English only loses a little recall, while reading English as Spanish could apply `sin` as a
/// negation to a sentence that meant the noun.
///
/// # Examples
/// ```
/// use pn_ultramemory_engine::{Tongue, detect_tongue};
///
/// assert_eq!(detect_tongue("the parser rejects a file that is too large"), Tongue::English);
/// assert_eq!(detect_tongue("el analizador rechaza un fichero que es demasiado grande"), Tongue::Spanish);
/// assert_eq!(detect_tongue(""), Tongue::English);
/// ```
#[must_use]
pub fn detect_tongue(text: &str) -> Tongue {
    let mut english = 0_usize;
    let mut spanish = 0_usize;
    for word in split_words(text) {
        // A word both languages share says nothing about which this is, so it counts for neither.
        let english_word = EN_STOP.contains(&word.as_str());
        let spanish_word = ES_STOP.contains(&word.as_str());
        match (english_word, spanish_word) {
            (true, false) => english += 1,
            (false, true) => spanish += 1,
            _ => {}
        }
    }
    if spanish > english {
        Tongue::Spanish
    } else {
        Tongue::English
    }
}

/// The word in lowercase with its accents folded: *Cómo* becomes *como*, *contraseña* becomes
/// *contrasena*. The lists of this module are written without accents, and a question is typed with
/// or without them.
pub(crate) fn folded(word: &str) -> String {
    word.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'á' | 'à' | 'ä' => 'a',
            'é' | 'è' | 'ë' => 'e',
            'í' | 'ì' | 'ï' => 'i',
            'ó' | 'ò' | 'ö' => 'o',
            'ú' | 'ù' | 'ü' => 'u',
            'ñ' => 'n',
            other => other,
        })
        .collect()
}

/// Whether a question reads as English: it carries English function words, at least as many as
/// Spanish ones.
///
/// Unlike [`detect_tongue`], a text with no function words at all (*validar cupón*, a few nouns)
/// does **not** read as English. That is the right default for a question, where the cost of the
/// two mistakes is reversed: treating a Spanish question as English finds nothing, while treating a
/// few English nouns as Spanish only adds a related word or none.
pub(crate) fn reads_as_english(text: &str) -> bool {
    let mut english = 0_usize;
    let mut spanish = 0_usize;
    for word in split_words(text) {
        let word = folded(&word);
        let english_word = EN_STOP.contains(&word.as_str());
        let spanish_word = ES_STOP.contains(&word.as_str());
        match (english_word, spanish_word) {
            (true, false) => english += 1,
            (false, true) => spanish += 1,
            _ => {}
        }
    }
    english > 0 && english >= spanish
}

/// Splits a text into lowercase words, keeping only what could be a word.
pub(super) fn split_words(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !c.is_alphanumeric() && c != '_' && c != '\'')
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
}

#[cfg(test)]
mod tests {
    use super::{ES_STOP, Tongue, detect_tongue, folded, reads_as_english};

    /// A question reads as English only when it carries English function words.
    #[test]
    fn questions_read_as_english_only_with_english_words() {
        assert!(reads_as_english("how are the tokens estimated"));
        assert!(!reads_as_english("¿cómo se estima el número de tokens?"));
        assert!(!reads_as_english("validar cupón"));
        assert!(!reads_as_english(""));
        assert_eq!(folded("Cómo"), "como");
        assert_eq!(folded("CONTRASEÑA"), "contrasena");
    }

    /// A sentence in either language is recognised, and a short or empty one falls back to English.
    #[test]
    fn a_sentence_is_recognised() {
        let english = [
            "the parser rejects a file that is larger than the configured limit",
            "handlers return an error and never unwrap inside a request path",
        ];
        for text in english {
            assert_eq!(detect_tongue(text), Tongue::English, "{text}");
        }
        let spanish = [
            "el analizador rechaza un fichero que es mayor que el limite configurado",
            "los manejadores devuelven un error y nunca desenvuelven en una peticion",
        ];
        for text in spanish {
            assert_eq!(detect_tongue(text), Tongue::Spanish, "{text}");
        }
        for text in ["", "   ", "parse", "foo bar baz", "日本語"] {
            assert_eq!(detect_tongue(text), Tongue::English, "{text:?}");
        }
    }

    /// No Spanish stop word is a word that carries meaning in English: dropping one from an English
    /// memory would lose exactly what distinguishes it.
    #[test]
    fn the_spanish_list_avoids_meaningful_english_words() {
        // Words that are common in both, which is why they are kept out of the Spanish list.
        for word in [
            "son", "era", "mas", "ha", "he", "no", "sin", "usa", "van", "vote",
        ] {
            assert!(
                !ES_STOP.contains(&word),
                "`{word}` reads as content in English and must not be dropped from it"
            );
        }
    }

    /// Roots are found per language, and a word too short to strip is left alone.
    #[test]
    fn roots_are_found_per_language() {
        assert_eq!(Tongue::English.root("rejects"), "reject");
        assert_eq!(Tongue::English.root("reject"), "reject");
        assert_eq!(Tongue::English.root("calibrated"), "calibrat");
        // Stripping would leave "us", too short to be a root, so the word survives.
        assert_eq!(Tongue::English.root("used"), "used");

        assert_eq!(Tongue::Spanish.root("calibrado"), "calibr");
        assert_eq!(Tongue::Spanish.root("calibrados"), "calibr");
        assert_eq!(Tongue::Spanish.root("configuracion"), "configur");
        assert_eq!(Tongue::Spanish.root("red"), "red");
    }

    /// The two names are stable, because they are printed.
    #[test]
    fn the_names_are_stable() {
        assert_eq!(Tongue::English.as_str(), "en");
        assert_eq!(Tongue::Spanish.as_str(), "es");
        assert_eq!(Tongue::default(), Tongue::English);
    }
}
