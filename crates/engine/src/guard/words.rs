// SPDX-License-Identifier: Apache-2.0
//! The vocabulary of the guard: which words matter, and a constant-time way to recognize them.
//!
//! The scanner reads text one byte at a time and never allocates. Each run of letters is hashed
//! while it is read (FNV-1a over the lower-cased letters, so `IGNORE`, `Ignore` and `ignore`
//! agree) and the hash is looked up in a table that is built at compile time. A word that is not
//! in the table is [`W::Other`]. The phrase rules of the scanner then work on the classes of the
//! last few words instead of on strings, which keeps a full scan to a few nanoseconds per byte
//! even in an unoptimized build.

/// The starting value of the word hash.
pub(super) const FNV_START: u64 = 0xcbf2_9ce4_8422_2325;

/// The multiplier of the word hash.
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Number of slots of the lookup table. It must be a power of two and comfortably larger than the
/// number of words, so that a probe always ends at an empty slot.
const TABLE_SIZE: usize = 1024;

/// The class of a word, as far as the phrase rules are concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum W {
    /// Any word the rules do not care about.
    Other,
    /// `ignore`, `forget`, `discard`, ...: discards something, but needs a qualifier.
    Ignore,
    /// `disregard`: discards something, and is suspicious on its own.
    Disregard,
    /// `override`, `bypass`: needs a strong qualifier.
    Bypass,
    /// `previous`, `prior`, `earlier`, ...
    Prev,
    /// `above`.
    Above,
    /// `before`.
    Before,
    /// `system`, `developer`, `hidden`, `secret`.
    Sys,
    /// `all`, `any`, `every`, `whatever`.
    Quant,
    /// `everything`.
    Everything,
    /// `your`, `my`.
    Your,
    /// Articles, adverbs and other words that may sit between a verb and its object.
    Filler,
    /// `and`, `or`.
    And,
    /// `this`.
    This,
    /// `it`.
    It,
    /// `instructions`, `directives`, `guidelines`, `training`, ...
    Instr,
    /// `prompt`, `prompts`.
    Prompt,
    /// `rules`, `restrictions`, `constraints`, ...
    Rules,
    /// `reveal`, `leak`, `disclose`, ...
    RevealHard,
    /// `print`, `show`, `display`, ...
    RevealSoft,
    /// `repeat`, `recite`, `echo`.
    Repeat,
    /// `tell`, `inform`, `notify`.
    Tell,
    /// `mention`.
    Mention,
    /// `words`, `text`, `message`, ...
    TextNoun,
    /// `user`, `users`, `human`, `anyone`, ...
    UserObj,
    /// `you`.
    You,
    /// `youre` (from `you're`).
    Youre,
    /// `are`.
    Are,
    /// `will`.
    Will,
    /// `now`.
    Now,
    /// `from`.
    From,
    /// `on`.
    On,
    /// The word that can follow `you are now` in an attempt to reassign the reader's role.
    RoleNext,
    /// `act`, `behave`, `respond`, ...
    Act,
    /// `pretend`.
    Pretend,
    /// `as`.
    As,
    /// `without`.
    Without,
    /// `anything`.
    Anything,
    /// `do`.
    Do,
    /// `dont` (from `don't`).
    Dont,
    /// `not`.
    Not,
    /// `never`.
    Never,
    /// `must`.
    Must,
    /// `should`.
    Should,
    /// `always`.
    Always,
    /// `new`, `updated`, `revised`, ...
    NewAdj,
    /// `run`, `execute`, `invoke`, ...
    Run,
    /// `paste`.
    Paste,
    /// `following`, `below`, `next`.
    Following,
    /// `command`, `script`, `code`, ...
    Cmd,
    /// `download`.
    Download,
    /// `powershell`, `pwsh`.
    Powershell,
    /// `e`, `enc`, `encodedcommand`.
    EncFlag,
    /// `curl`, `wget`.
    Curl,
    /// `downloadstring`.
    DownloadString,
    /// `method`.
    Method,
    /// `tools`.
    Tools,
    /// `call`.
    Call,
}

impl W {
    /// Whether the word can sit between the verb and the noun of an override phrase.
    pub(super) const fn is_gap(self) -> bool {
        matches!(
            self,
            Self::Prev
                | Self::Above
                | Self::Sys
                | Self::Quant
                | Self::Everything
                | Self::Your
                | Self::Filler
                | Self::And
                | Self::This
                | Self::It
        )
    }

    /// Whether the word can follow `you are now` when the aim is to reassign the reader.
    pub(super) const fn is_role_next(self) -> bool {
        matches!(
            self,
            Self::RoleNext | Self::Act | Self::Ignore | Self::Disregard
        )
    }

    /// Whether the word starts a command the reader is told to run.
    pub(super) const fn is_run_verb(self) -> bool {
        matches!(self, Self::Run | Self::Paste)
    }
}

/// Every word of interest and its class, in lower case and without apostrophes.
const WORDS: &[(&str, W)] = &[
    ("ignore", W::Ignore),
    ("forget", W::Ignore),
    ("discard", W::Ignore),
    ("overlook", W::Ignore),
    ("neglect", W::Ignore),
    ("disregard", W::Disregard),
    ("override", W::Bypass),
    ("bypass", W::Bypass),
    ("previous", W::Prev),
    ("prior", W::Prev),
    ("earlier", W::Prev),
    ("preceding", W::Prev),
    ("former", W::Prev),
    ("foregoing", W::Prev),
    ("initial", W::Prev),
    ("original", W::Prev),
    ("existing", W::Prev),
    ("above", W::Above),
    ("before", W::Before),
    ("system", W::Sys),
    ("developer", W::Sys),
    ("hidden", W::Sys),
    ("secret", W::Sys),
    ("all", W::Quant),
    ("any", W::Quant),
    ("every", W::Quant),
    ("whatever", W::Quant),
    ("everything", W::Everything),
    ("your", W::Your),
    ("yours", W::Your),
    ("my", W::Your),
    ("the", W::Filler),
    ("these", W::Filler),
    ("those", W::Filler),
    ("that", W::Filler),
    ("of", W::Filler),
    ("other", W::Filler),
    ("safety", W::Filler),
    ("content", W::Filler),
    ("so", W::Filler),
    ("far", W::Filler),
    ("to", W::Filler),
    ("me", W::Filler),
    ("us", W::Filler),
    ("please", W::Filler),
    ("kindly", W::Filler),
    ("just", W::Filler),
    ("also", W::Filler),
    ("completely", W::Filler),
    ("entirely", W::Filler),
    ("totally", W::Filler),
    ("fully", W::Filler),
    ("immediately", W::Filler),
    ("simply", W::Filler),
    ("full", W::Filler),
    ("entire", W::Filler),
    ("exact", W::Filler),
    ("complete", W::Filler),
    ("whole", W::Filler),
    ("verbatim", W::Filler),
    ("its", W::Filler),
    ("our", W::Filler),
    ("given", W::Filler),
    ("current", W::Filler),
    ("and", W::And),
    ("or", W::And),
    ("this", W::This),
    ("it", W::It),
    ("instructions", W::Instr),
    ("instruction", W::Instr),
    ("directives", W::Instr),
    ("directive", W::Instr),
    ("guidelines", W::Instr),
    ("guideline", W::Instr),
    ("training", W::Instr),
    ("programming", W::Instr),
    ("prompt", W::Prompt),
    ("prompts", W::Prompt),
    ("rules", W::Rules),
    ("restrictions", W::Rules),
    ("constraints", W::Rules),
    ("safeguards", W::Rules),
    ("limitations", W::Rules),
    ("limits", W::Rules),
    ("filters", W::Rules),
    ("guardrails", W::Rules),
    ("policies", W::Rules),
    ("censorship", W::Rules),
    ("boundaries", W::Rules),
    ("ethics", W::Rules),
    ("reveal", W::RevealHard),
    ("leak", W::RevealHard),
    ("disclose", W::RevealHard),
    ("expose", W::RevealHard),
    ("dump", W::RevealHard),
    ("exfiltrate", W::RevealHard),
    ("print", W::RevealSoft),
    ("show", W::RevealSoft),
    ("display", W::RevealSoft),
    ("output", W::RevealSoft),
    ("give", W::RevealSoft),
    ("share", W::RevealSoft),
    ("send", W::RevealSoft),
    ("write", W::RevealSoft),
    ("provide", W::RevealSoft),
    ("repeat", W::Repeat),
    ("recite", W::Repeat),
    ("echo", W::Repeat),
    ("tell", W::Tell),
    ("inform", W::Tell),
    ("notify", W::Tell),
    ("mention", W::Mention),
    ("words", W::TextNoun),
    ("text", W::TextNoun),
    ("sentences", W::TextNoun),
    ("message", W::TextNoun),
    ("messages", W::TextNoun),
    ("user", W::UserObj),
    ("users", W::UserObj),
    ("human", W::UserObj),
    ("humans", W::UserObj),
    ("anyone", W::UserObj),
    ("operator", W::UserObj),
    ("person", W::UserObj),
    ("them", W::UserObj),
    ("you", W::You),
    ("youre", W::Youre),
    ("are", W::Are),
    ("will", W::Will),
    ("now", W::Now),
    ("from", W::From),
    ("on", W::On),
    ("a", W::RoleNext),
    ("an", W::RoleNext),
    ("dan", W::RoleNext),
    ("unrestricted", W::RoleNext),
    ("unfiltered", W::RoleNext),
    ("uncensored", W::RoleNext),
    ("jailbroken", W::RoleNext),
    ("evil", W::RoleNext),
    ("acting", W::RoleNext),
    ("playing", W::RoleNext),
    ("roleplaying", W::RoleNext),
    ("pretending", W::RoleNext),
    ("operating", W::RoleNext),
    ("free", W::RoleNext),
    ("going", W::RoleNext),
    ("called", W::RoleNext),
    ("named", W::RoleNext),
    ("no", W::RoleNext),
    ("become", W::RoleNext),
    ("becoming", W::RoleNext),
    ("act", W::Act),
    ("behave", W::Act),
    ("respond", W::Act),
    ("answer", W::Act),
    ("roleplay", W::Act),
    ("play", W::Act),
    ("obey", W::Act),
    ("follow", W::Act),
    ("comply", W::Act),
    ("operate", W::Act),
    ("pretend", W::Pretend),
    ("as", W::As),
    ("without", W::Without),
    ("anything", W::Anything),
    ("do", W::Do),
    ("dont", W::Dont),
    ("not", W::Not),
    ("never", W::Never),
    ("must", W::Must),
    ("should", W::Should),
    ("always", W::Always),
    ("new", W::NewAdj),
    ("updated", W::NewAdj),
    ("revised", W::NewAdj),
    ("latest", W::NewAdj),
    ("additional", W::NewAdj),
    ("actual", W::NewAdj),
    ("real", W::NewAdj),
    ("run", W::Run),
    ("execute", W::Run),
    ("exec", W::Run),
    ("invoke", W::Run),
    ("launch", W::Run),
    ("type", W::Run),
    ("enter", W::Run),
    ("paste", W::Paste),
    ("following", W::Following),
    ("below", W::Following),
    ("next", W::Following),
    ("command", W::Cmd),
    ("commands", W::Cmd),
    ("script", W::Cmd),
    ("scripts", W::Cmd),
    ("code", W::Cmd),
    ("snippet", W::Cmd),
    ("line", W::Cmd),
    ("lines", W::Cmd),
    ("shell", W::Cmd),
    ("oneliner", W::Cmd),
    ("download", W::Download),
    ("powershell", W::Powershell),
    ("pwsh", W::Powershell),
    ("e", W::EncFlag),
    ("ec", W::EncFlag),
    ("enc", W::EncFlag),
    ("encodedcommand", W::EncFlag),
    ("curl", W::Curl),
    ("wget", W::Curl),
    ("downloadstring", W::DownloadString),
    ("method", W::Method),
    ("tools", W::Tools),
    ("call", W::Call),
];

/// One step of the word hash: folds an already lower-cased letter into `hash`.
pub(super) const fn fold_letter(hash: u64, lower: u8) -> u64 {
    (hash ^ lower as u64).wrapping_mul(FNV_PRIME)
}

/// The hash of a whole word made of lower-case letters.
const fn hash_word(word: &[u8]) -> u64 {
    let mut hash = FNV_START;
    let mut i = 0;
    while i < word.len() {
        hash = fold_letter(hash, word[i]);
        i += 1;
    }
    hash
}

/// The slot where a probe for `hash` starts.
#[allow(
    clippy::cast_possible_truncation,
    reason = "the value is masked to the table size"
)]
const fn first_slot(hash: u64) -> usize {
    (hash & (TABLE_SIZE as u64 - 1)) as usize
}

/// Builds the open-addressing table of [`WORDS`] at compile time.
const fn build_table() -> [(u64, W); TABLE_SIZE] {
    let mut table = [(0_u64, W::Other); TABLE_SIZE];
    let mut i = 0;
    while i < WORDS.len() {
        let (word, class) = WORDS[i];
        let hash = hash_word(word.as_bytes());
        let mut slot = first_slot(hash);
        while table[slot].0 != 0 {
            slot = (slot + 1) & (TABLE_SIZE - 1);
        }
        table[slot] = (hash, class);
        i += 1;
    }
    table
}

/// The lookup table: a hash and its class, or a zero hash for an empty slot.
static TABLE: [(u64, W); TABLE_SIZE] = build_table();

/// The class of the word whose hash is `hash`.
pub(super) fn classify(hash: u64) -> W {
    let mut slot = first_slot(hash);
    for _ in 0..TABLE_SIZE {
        let (stored, class) = TABLE[slot];
        if stored == hash {
            return class;
        }
        if stored == 0 {
            break;
        }
        slot = (slot + 1) & (TABLE_SIZE - 1);
    }
    W::Other
}

/// Folds a letter-like character outside ASCII into the ASCII letter it imitates, so that a
/// phrase written with look-alike letters is still read as the phrase. Returns the lower-case
/// letter and the number of bytes the character takes.
pub(super) fn fold_lookalike(bytes: &[u8], i: usize) -> Option<(u8, usize)> {
    let lead = *bytes.get(i)?;
    let second = *bytes.get(i + 1)?;
    // A lookup table reads better with one entry per line, even where two code points fold to the
    // same letter, so the arms are deliberately not merged.
    #[allow(
        clippy::match_same_arms,
        reason = "a lookup table is clearer one entry per line"
    )]
    let folded = match (lead, second) {
        // Cyrillic small letters.
        (0xD0, 0xB0) => b'a',
        (0xD0, 0xB5) => b'e',
        (0xD0, 0xBE) => b'o',
        (0xD1, 0x80) => b'p',
        (0xD1, 0x81) => b'c',
        (0xD1, 0x83) => b'y',
        (0xD1, 0x85) => b'x',
        (0xD1, 0x95) => b's',
        (0xD1, 0x96) => b'i',
        (0xD1, 0x98) => b'j',
        // Cyrillic capital letters.
        (0xD0, 0x86) => b'i',
        (0xD0, 0x90) => b'a',
        (0xD0, 0x92) => b'b',
        (0xD0, 0x95) => b'e',
        (0xD0, 0x9A) => b'k',
        (0xD0, 0x9C) => b'm',
        (0xD0, 0x9D) => b'h',
        (0xD0, 0x9E) => b'o',
        (0xD0, 0xA0) => b'p',
        (0xD0, 0xA1) => b'c',
        (0xD0, 0xA2) => b't',
        (0xD0, 0xA5) => b'x',
        // Greek letters.
        (0xCE, 0xBF | 0x9F) => b'o',
        (0xCE, 0xB1 | 0x91) => b'a',
        (0xCE, 0xBD) => b'v',
        (0xCE, 0xB9) => b'i',
        (0xCE, 0xBA) => b'k',
        (0xCF, 0x85) => b'u',
        (0xCF, 0x81) | (0xCE, 0xA1) => b'p',
        (0xCE, 0x92) => b'b',
        (0xCE, 0x95) => b'e',
        (0xCE, 0x97) => b'h',
        (0xCE, 0x99) => b'i',
        (0xCE, 0x9A) => b'k',
        (0xCE, 0x9C) => b'm',
        (0xCE, 0x9D) => b'n',
        (0xCE, 0xA4) => b't',
        (0xCE, 0xA5) => b'y',
        (0xCE, 0xA7) => b'x',
        // Full-width Latin letters (U+FF21..U+FF3A and U+FF41..U+FF5A).
        (0xEF, 0xBC | 0xBD) => {
            let third = *bytes.get(i + 2)?;
            return match (second, third) {
                (0xBC, 0xA1..=0xBA) => Some((b'a' + (third - 0xA1), 3)),
                (0xBD, 0x81..=0x9A) => Some((b'a' + (third - 0x81), 3)),
                _ => None,
            };
        }
        _ => return None,
    };
    Some((folded, 2))
}

#[cfg(test)]
mod tests {
    use super::{
        FNV_START, TABLE_SIZE, W, WORDS, classify, fold_letter, fold_lookalike, hash_word,
    };

    /// Hashes a word the way the scanner does, letter by letter.
    fn hash_of(word: &str) -> u64 {
        word.bytes().fold(FNV_START, |hash, b| {
            fold_letter(hash, b.to_ascii_lowercase())
        })
    }

    /// Every listed word is found with its class, and hashes are unique.
    #[test]
    fn every_word_is_classified_and_unique() {
        assert!(WORDS.len() * 3 < TABLE_SIZE, "the table must stay sparse");
        let mut seen = std::collections::HashSet::new();
        for (word, class) in WORDS {
            assert!(word.bytes().all(|b| b.is_ascii_lowercase()), "{word}");
            assert!(seen.insert(*word), "{word} is listed twice");
            let hash = hash_word(word.as_bytes());
            assert_ne!(hash, 0);
            assert_eq!(classify(hash), *class, "{word}");
            assert_eq!(hash_of(&word.to_uppercase()), hash, "{word}");
        }
        let hashes: std::collections::HashSet<u64> =
            WORDS.iter().map(|(w, _)| hash_word(w.as_bytes())).collect();
        assert_eq!(hashes.len(), WORDS.len(), "two words share a hash");
    }

    /// Words that are not in the table are `Other`.
    #[test]
    fn unknown_words_are_other() {
        for word in [
            "banana",
            "compiler",
            "tokenizer",
            "ignored",
            "instructionsx",
            "",
        ] {
            assert_eq!(classify(hash_of(word)), W::Other, "{word}");
        }
    }

    /// Look-alike letters fold to the letter they imitate.
    #[test]
    fn lookalikes_fold() {
        assert_eq!(fold_lookalike("о".as_bytes(), 0), Some((b'o', 2)));
        assert_eq!(fold_lookalike("ο".as_bytes(), 0), Some((b'o', 2)));
        assert_eq!(fold_lookalike("ｉ".as_bytes(), 0), Some((b'i', 3)));
        assert_eq!(fold_lookalike("Ｉ".as_bytes(), 0), Some((b'i', 3)));
        assert_eq!(fold_lookalike("é".as_bytes(), 0), None);
        assert_eq!(fold_lookalike("字".as_bytes(), 0), None);
    }
}
