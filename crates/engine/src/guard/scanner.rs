// SPDX-License-Identifier: Apache-2.0
//! The single pass that reads a text and collects the reasons to distrust it.
//!
//! The scanner walks the bytes once. It never allocates, never lower-cases a copy of the text and
//! never searches the text again from the start, so its cost is linear in the size of the text.
//! What it looks at:
//!
//! * **Bytes**: ANSI escapes, Unicode tag characters, bidirectional controls and runs of
//!   zero-width characters (recognized from their UTF-8 encoding), URLs, base64-looking runs and
//!   the share of upper-case letters.
//! * **Words**: each run of letters is hashed as it is read and classified through
//!   [`super::words`]. Phrase rules such as "ignore all previous instructions" then look at the
//!   classes of the last few words, so spacing, punctuation, line breaks, letter case and
//!   look-alike letters do not matter. Zero-width characters and apostrophes inside a word are
//!   ignored, so `ig\u{200b}nore` and `don't` read as `ignore` and `dont`.
//! * **Markup**: tool-call and chat-protocol tags, and downloads piped into an interpreter,
//!   which are found where their first byte (`<`, `[`, `|`) is read.

use super::reasons::{Reason, Reasons};
use super::words::{FNV_START, W, classify, fold_letter, fold_lookalike};

/// How many recent words the phrase rules can look back on.
const RING: usize = 8;

/// Tags that imitate the messages of a tool protocol, in lower case.
const PROTOCOL_TAGS: [&str; 10] = [
    "tool_call",
    "tool_calls",
    "function_call",
    "function_calls",
    "function_result",
    "function_results",
    "tool_use",
    "tool_result",
    "tool_results",
    "invoke",
];

/// Interpreters that read a program from a pipe with any arguments.
const SHELLS: [&str; 9] = [
    "sh",
    "bash",
    "zsh",
    "dash",
    "ksh",
    "fish",
    "pwsh",
    "powershell",
    "iex",
];

/// Interpreters that are a problem only when they read the program from the pipe.
const INTERPRETERS: [&str; 5] = ["python", "python3", "perl", "ruby", "node"];

/// Commands that fetch something from the network or decode a payload.
const FETCHERS: [&str; 5] = ["curl", "wget", "base64", "invoke-webrequest", "iwr"];

/// Words that turn a fetched payload into running code when they precede `$(curl ...)`.
const EVALUATORS: [&str; 7] = ["sh", "bash", "zsh", "dash", "ksh", "eval", "source"];

/// What a non-ASCII character is, as far as the scanner is concerned.
enum High {
    /// A letter that looks like an ASCII letter, with its lower-case form.
    Letter(u8),
    /// An invisible character that counts towards a run of zero-width characters.
    ZeroWidth,
    /// An invisible character that only hides inside words (soft hyphen).
    Ignorable,
    /// A bidirectional override or isolate.
    Bidi,
    /// A Unicode tag character.
    Tag,
    /// A right single quotation mark, which acts as an apostrophe.
    Apostrophe,
    /// Anything else.
    Other,
}

/// Reads one text and remembers what it found.
pub(super) struct Scanner<'a> {
    /// The text being read.
    bytes: &'a [u8],
    /// The reasons found so far.
    found: Reasons,
    /// The classes of the most recent words.
    ring: [W; RING],
    /// How many words were read.
    words: usize,
    /// The value of `words` when `act as` was last read, or zero.
    act_as_at: usize,
    /// Where the last `powershell` word ended, or zero.
    powershell_end: usize,
    /// Whether a word is being read.
    in_word: bool,
    /// The hash of the word being read.
    hash: u64,
    /// Where the word being read started.
    word_start: usize,
    /// ASCII letters read.
    letters: usize,
    /// Upper-case ASCII letters read.
    upper: usize,
    /// URLs read (occurrences of `://`).
    urls: usize,
    /// Length of the current run of base64 characters.
    b64_run: usize,
    /// Which characters the current base64 run contains.
    b64_seen: u128,
    /// Length of the current run of zero-width characters.
    zero_width_run: usize,
}

/// Scans a text and returns every reason found.
pub(super) fn scan_reasons(text: &str) -> Reasons {
    let mut scanner = Scanner::new(text.as_bytes());
    scanner.run();
    scanner.finish(text)
}

impl<'a> Scanner<'a> {
    /// Creates a scanner for a text.
    fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            found: Reasons::default(),
            ring: [W::Other; RING],
            words: 0,
            act_as_at: 0,
            powershell_end: 0,
            in_word: false,
            hash: FNV_START,
            word_start: 0,
            letters: 0,
            upper: 0,
            urls: 0,
            b64_run: 0,
            b64_seen: 0,
            zero_width_run: 0,
        }
    }

    /// Reads the whole text.
    fn run(&mut self) {
        let len = self.bytes.len();
        let mut i = 0;
        while i < len {
            let c = self.bytes[i];
            if c.is_ascii_alphabetic() {
                self.letter(c | 0x20, i);
                self.letters += 1;
                self.upper += usize::from(c < b'a');
                self.b64_step(c);
                self.zero_width_run = 0;
                i += 1;
            } else if c < 0x80 {
                self.zero_width_run = 0;
                let apostrophe = c == b'\''
                    && self.in_word
                    && self.bytes.get(i + 1).is_some_and(u8::is_ascii_alphabetic);
                if !apostrophe {
                    self.end_word(i);
                    self.b64_step(c);
                    self.symbol(c, i);
                }
                i += 1;
            } else {
                i += self.high(i);
            }
        }
        self.end_word(len);
    }

    /// Adds a letter (already lower-cased) to the word being read, starting one if needed.
    fn letter(&mut self, lower: u8, at: usize) {
        if !self.in_word {
            self.in_word = true;
            self.word_start = at;
            self.hash = FNV_START;
        }
        self.hash = fold_letter(self.hash, lower);
    }

    /// Tracks the run of base64 characters that ends with `c`.
    fn b64_step(&mut self, c: u8) {
        if c.is_ascii_alphanumeric() || matches!(c, b'+' | b'/' | b'=') {
            self.b64_run += 1;
            self.b64_seen |= 1_u128 << (c & 0x7f);
            if self.b64_run >= 200 && self.b64_seen.count_ones() >= 12 {
                self.found.add(Reason::Base64Blob);
            }
        } else {
            self.b64_run = 0;
            self.b64_seen = 0;
        }
    }

    /// Reads the ASCII byte `c` that is not a letter, at position `i`.
    fn symbol(&mut self, c: u8, i: usize) {
        match c {
            0x1B => self.found.add(Reason::AnsiEscape),
            b'<' => self.on_less_than(i),
            b'[' => self.on_bracket(i),
            b'|' => self.on_pipe(i),
            b':' if self.bytes[i + 1..].starts_with(b"//") => self.urls += 1,
            _ => {}
        }
    }

    /// Reads the non-ASCII character at `i` and returns how many bytes it takes.
    fn high(&mut self, i: usize) -> usize {
        let (kind, len) = classify_high(self.bytes, i);
        match kind {
            High::Letter(lower) => {
                self.letter(lower, i);
                self.zero_width_run = 0;
                self.b64_step(0);
            }
            High::ZeroWidth => {
                self.zero_width_run += 1;
                if self.zero_width_run >= 3 {
                    self.found.add(Reason::ZeroWidthRun);
                }
            }
            High::Ignorable => {}
            High::Apostrophe => {
                self.zero_width_run = 0;
                let inside =
                    self.in_word && self.bytes.get(i + len).is_some_and(u8::is_ascii_alphabetic);
                if !inside {
                    self.end_word(i);
                }
                self.b64_step(0);
            }
            High::Bidi | High::Tag | High::Other => {
                self.zero_width_run = 0;
                self.end_word(i);
                self.b64_step(0);
                match kind {
                    High::Bidi => self.found.add(Reason::BidiControl),
                    High::Tag => self.found.add(Reason::TagCharacters),
                    _ => {}
                }
            }
        }
        len
    }

    /// Ends the word being read, if any, and applies the phrase rules to it.
    fn end_word(&mut self, end: usize) {
        if !self.in_word {
            return;
        }
        self.in_word = false;
        let class = classify(self.hash);
        self.on_word(class, self.word_start, end);
    }

    /// Adds the flags that depend on the whole text and returns the reasons.
    fn finish(mut self, text: &str) -> Reasons {
        if self.letters >= 40 && self.upper * 100 >= self.letters * 60 {
            self.found.add(Reason::Shouting);
        }
        if self.urls >= 4 {
            self.found.add(Reason::ManyUrls);
        }
        if text.len() > 2000 && text.chars().nth(2000).is_some() {
            self.found.add(Reason::TooLong);
        }
        self.found
    }

    // ---- the last few words -----------------------------------------------------------------

    /// Records a word in the ring.
    fn push(&mut self, class: W) {
        self.ring[self.words % RING] = class;
        self.words += 1;
    }

    /// The class of the word `k` positions before the current one (`k = 0` is the current one).
    fn back(&self, k: usize) -> W {
        if k >= self.words.min(RING) {
            W::Other
        } else {
            self.ring[(self.words - 1 - k) % RING]
        }
    }

    /// Whether the word `k` positions back negates what follows: `never`, `don't` or `not`.
    fn negated(&self, k: usize) -> bool {
        matches!(self.back(k), W::Never | W::Dont | W::Not)
    }

    /// Whether the words before the current one read `you are` or `you will`, followed by `now`.
    fn after_you_now(&self) -> bool {
        (matches!(self.back(2), W::Are | W::Will) && self.back(3) == W::You)
            || self.back(2) == W::Youre
    }

    // ---- phrase rules -----------------------------------------------------------------------

    /// Applies the phrase rules to a word that just ended.
    fn on_word(&mut self, class: W, start: usize, end: usize) {
        self.push(class);
        if self.back(1) == W::Now && self.after_you_now() && class.is_role_next() {
            self.found.add(Reason::RoleHijack);
        }
        match class {
            W::Instr | W::Prompt | W::Rules => self.on_noun(class, end),
            W::Prev | W::Before => self.on_previous(),
            W::Above => {
                self.on_previous();
                self.on_above();
            }
            W::Now => {
                if self.back(1) == W::Anything && self.back(2) == W::Do {
                    self.found.add(Reason::UnrestrictedPersona);
                }
            }
            W::You => self.on_you(),
            W::Must => {
                if self.back(1) == W::You {
                    self.found.add(Reason::DirectiveLanguage);
                }
            }
            W::Always | W::Never => {
                if matches!(self.back(1), W::Should | W::Will) && self.back(2) == W::You {
                    self.found.add(Reason::DirectiveLanguage);
                }
            }
            W::Mention => {
                if self.negated(1) {
                    self.found.add(Reason::DirectiveLanguage);
                }
            }
            W::UserObj => self.on_user_object(),
            W::As => {
                if self.back(1) == W::Act {
                    self.act_as_at = self.words;
                }
            }
            W::Following => {
                self.on_following();
                self.on_download_target();
            }
            W::Cmd => {
                self.on_command();
                self.on_download_target();
            }
            W::It | W::This => self.on_download_target(),
            W::Powershell => self.powershell_end = end,
            W::EncFlag => self.on_encoded_flag(start),
            W::Call => self.on_call(start),
            W::Curl => self.on_fetch_substitution(start),
            W::DownloadString => self.on_download_string(start),
            _ => {}
        }
    }

    /// A noun that names instructions, a prompt or rules.
    fn on_noun(&mut self, class: W, end: usize) {
        if class == W::Rules {
            if self.override_phrase(true) {
                self.found.add(Reason::OverrideInstructions);
            }
        } else {
            if self.override_phrase(false) {
                self.found.add(Reason::OverrideInstructions);
            }
            if self.reveal_phrase() {
                self.found.add(Reason::RevealPrompt);
            }
            if class == W::Instr && self.back(1) == W::NewAdj && self.colon_after(end) {
                self.found.add(Reason::NewInstructions);
            }
        }
        if self.persona_phrase() {
            self.found.add(Reason::UnrestrictedPersona);
        }
    }

    /// Whether a colon follows `end`, after blanks and emphasis marks.
    fn colon_after(&self, end: usize) -> bool {
        self.bytes
            .get(end..)
            .into_iter()
            .flatten()
            .find(|c| !matches!(c, b' ' | b'\t' | b'*' | b'_' | b'`'))
            == Some(&b':')
    }

    /// A verb that discards earlier instructions, then only filler and qualifiers, then the noun.
    ///
    /// `rules` is true when the noun is `rules`, `restrictions` and the like, which need a
    /// stronger qualifier than `instructions` do.
    fn override_phrase(&self, rules: bool) -> bool {
        let (mut strong, mut system, mut universal, mut your) = (false, false, false, false);
        for k in 1..=6 {
            match self.back(k) {
                W::Disregard => return !rules || strong || system || your,
                W::Ignore => return strong || system || your || (universal && !rules),
                W::Bypass => return strong || your,
                W::Prev | W::Above => strong = true,
                W::Sys => system = true,
                W::Quant | W::Everything => universal = true,
                W::Your => your = true,
                class if class.is_gap() => {}
                _ => return false,
            }
        }
        false
    }

    /// A verb that shows something, then the reader's own instructions or the system prompt.
    fn reveal_phrase(&self) -> bool {
        let (mut your, mut system) = (false, false);
        for k in 1..=6 {
            match self.back(k) {
                W::RevealHard => return your || system,
                W::RevealSoft | W::Tell | W::Repeat => return your,
                W::Your => your = true,
                W::Sys => system = true,
                class if class.is_gap() => {}
                _ => return false,
            }
        }
        false
    }

    /// `act as ... without ... restrictions`: the `without` is close to the noun and the `act as`
    /// is not far before it.
    fn persona_phrase(&self) -> bool {
        self.act_as_at > 0
            && self.words - self.act_as_at <= 14
            && (1..=4).any(|k| self.back(k) == W::Without)
    }

    /// `ignore everything above`, `forget everything before`.
    fn on_previous(&mut self) {
        if self.back(1) == W::Everything && matches!(self.back(2), W::Ignore | W::Disregard) {
            self.found.add(Reason::OverrideInstructions);
        }
    }

    /// `repeat the words above`, `print everything above`.
    fn on_above(&mut self) {
        let mut text = false;
        let mut everything = false;
        for k in 1..=4 {
            match self.back(k) {
                W::Repeat => {
                    if text || everything {
                        self.found.add(Reason::RevealPrompt);
                    }
                    return;
                }
                W::RevealSoft => {
                    if everything {
                        self.found.add(Reason::RevealPrompt);
                    }
                    return;
                }
                W::TextNoun => text = true,
                W::Everything => everything = true,
                W::Filler | W::Quant => {}
                _ => return,
            }
        }
    }

    /// Second-person openings that are directives: `from now on you`, `pretend you`.
    fn on_you(&mut self) {
        let from_now_on =
            self.back(1) == W::On && self.back(2) == W::Now && self.back(3) == W::From;
        let pretend =
            self.back(1) == W::Pretend || (self.back(1) == W::Filler && self.back(2) == W::Pretend);
        if from_now_on || pretend {
            self.found.add(Reason::DirectiveLanguage);
        }
    }

    /// `never tell the user`, `do not reveal to anyone`.
    fn on_user_object(&mut self) {
        for k in 1..=3 {
            match self.back(k) {
                W::Filler | W::Quant => {}
                W::Tell | W::RevealHard | W::Mention => {
                    if self.negated(k + 1) {
                        self.found.add(Reason::DirectiveLanguage);
                    }
                    return;
                }
                _ => return,
            }
        }
    }

    /// `run the command below`, `paste the following`.
    fn on_following(&mut self) {
        let mut command = false;
        for k in 1..=5 {
            match self.back(k) {
                W::Cmd => command = true,
                W::Filler | W::Following => {}
                class if class.is_run_verb() => {
                    if command || class == W::Paste {
                        self.found.add(Reason::RunCommand);
                    }
                    return;
                }
                _ => return,
            }
        }
    }

    /// `run the following command`, `execute the following shell script`.
    fn on_command(&mut self) {
        let mut following = false;
        for k in 1..=5 {
            match self.back(k) {
                W::Following => following = true,
                W::Filler | W::Cmd => {}
                class if class.is_run_verb() => {
                    if following {
                        self.found.add(Reason::RunCommand);
                    }
                    return;
                }
                _ => return,
            }
        }
    }

    /// `download and run it`, `download and execute the following script`.
    fn on_download_target(&mut self) {
        for k in 1..=5 {
            match self.back(k) {
                W::Filler | W::Following | W::Cmd | W::This | W::It => {}
                class if class.is_run_verb() => {
                    if self.back(k + 1) == W::And && self.back(k + 2) == W::Download {
                        self.found.add(Reason::RunCommand);
                    }
                    return;
                }
                _ => return,
            }
        }
    }

    /// `"method": "tools/call"`.
    fn on_call(&mut self, start: usize) {
        if self.back(1) == W::Tools
            && self.back(2) == W::Method
            && start > 0
            && self.bytes[start - 1] == b'/'
        {
            self.found.add(Reason::ProtocolMarkup);
        }
    }

    /// `powershell ... -enc <payload>`.
    fn on_encoded_flag(&mut self, start: usize) {
        let near = self.powershell_end > 0
            && start > self.powershell_end
            && start - self.powershell_end <= 120;
        if near && start > 0 && matches!(self.bytes[start - 1], b'-' | b'/') {
            self.found.add(Reason::EncodedPowershell);
        }
    }

    /// `sh -c "$(curl ...)"`, `bash <(curl ...)`, `eval "$(wget ...)"`.
    fn on_fetch_substitution(&mut self, start: usize) {
        let before = &self.bytes[..start];
        let opener = if before.ends_with(b"$(") || before.ends_with(b"<(") {
            2
        } else if before.ends_with(b"`") {
            1
        } else {
            return;
        };
        let prefix = line_prefix(self.bytes, start - opener);
        if has_word(prefix, &EVALUATORS) {
            self.found.add(Reason::PipeToShell);
        }
    }

    /// `IEX (New-Object Net.WebClient).DownloadString(...)`.
    fn on_download_string(&mut self, start: usize) {
        let prefix = line_prefix(self.bytes, start);
        if has_word(prefix, &["iex"]) || contains_ci(prefix, b"invoke-expression") {
            self.found.add(Reason::PipeToShell);
        }
    }

    // ---- markup -----------------------------------------------------------------------------

    /// A `<` that may open a protocol tag: `<|im_start|>`, `<<SYS>>`, `<tool_call>`.
    fn on_less_than(&mut self, i: usize) {
        let rest = &self.bytes[i + 1..];
        let rest = &rest[..rest.len().min(40)];
        let flagged = match rest.first() {
            Some(b'|') => {
                let name = rest[1..]
                    .iter()
                    .take_while(|b| b.is_ascii_alphanumeric() || **b == b'_')
                    .count();
                (2..=32).contains(&name) && rest[1 + name..].starts_with(b"|>")
            }
            Some(b'<') => {
                rest[1..].len() >= 5
                    && (rest[1..5].eq_ignore_ascii_case(b"sys>")
                        || rest[1..]
                            .get(..6)
                            .is_some_and(|s| s.eq_ignore_ascii_case(b"/sys>>")))
            }
            _ => {
                let rest = rest.strip_prefix(b"/").unwrap_or(rest);
                let name = rest
                    .iter()
                    .take_while(|b| b.is_ascii_alphabetic() || **b == b'_')
                    .count();
                let closes = matches!(rest.get(name), Some(b'>' | b' ' | b'/' | b'\n' | b'\t'));
                closes
                    && PROTOCOL_TAGS
                        .iter()
                        .any(|tag| rest[..name].eq_ignore_ascii_case(tag.as_bytes()))
            }
        };
        if flagged {
            self.found.add(Reason::ProtocolMarkup);
        }
    }

    /// A `[` that may open `[INST]` or `[/INST]`.
    fn on_bracket(&mut self, i: usize) {
        let rest = &self.bytes[i + 1..];
        if rest.len() >= 5 && rest[..5].eq_ignore_ascii_case(b"inst]")
            || rest.len() >= 6 && rest[..6].eq_ignore_ascii_case(b"/inst]")
        {
            self.found.add(Reason::ProtocolMarkup);
        }
    }

    /// A `|` that may pipe a download into an interpreter.
    fn on_pipe(&mut self, i: usize) {
        let mut j = skip_blanks(self.bytes, i + 1);
        let (mut word, mut after) = word_at(self.bytes, j);
        if ["sudo", "env", "exec", "command", "nohup", "time"]
            .iter()
            .any(|w| word.eq_ignore_ascii_case(w.as_bytes()))
        {
            j = skip_blanks(self.bytes, after);
            while self.bytes.get(j) == Some(&b'-') {
                while self.bytes.get(j).is_some_and(|b| !b.is_ascii_whitespace()) {
                    j += 1;
                }
                j = skip_blanks(self.bytes, j);
            }
            (word, after) = word_at(self.bytes, j);
        }
        let is_shell = SHELLS
            .iter()
            .any(|s| word.eq_ignore_ascii_case(s.as_bytes()));
        let reads_stdin = INTERPRETERS
            .iter()
            .any(|s| word.eq_ignore_ascii_case(s.as_bytes()))
            && reads_program_from_pipe(self.bytes, after);
        if (is_shell || reads_stdin)
            && FETCHERS
                .iter()
                .any(|f| contains_ci(line_prefix(self.bytes, i), f.as_bytes()))
        {
            self.found.add(Reason::PipeToShell);
        }
    }
}

/// Classifies the non-ASCII character that starts at `i` and returns it with its length in bytes.
fn classify_high(bytes: &[u8], i: usize) -> (High, usize) {
    let at = |k: usize| bytes.get(i + k).copied().unwrap_or(0);
    match bytes[i] {
        0xC2 if at(1) == 0xAD => (High::Ignorable, 2),
        0xE2 => {
            let kind = match (at(1), at(2)) {
                (0x80, 0x8B..=0x8D) | (0x81, 0xA0) => High::ZeroWidth,
                (0x80, 0xAA..=0xAE) | (0x81, 0xA6..=0xA9) => High::Bidi,
                (0x80, 0x99) => High::Apostrophe,
                _ => High::Other,
            };
            (kind, 3)
        }
        0xEF if at(1) == 0xBB && at(2) == 0xBF => (High::ZeroWidth, 3),
        0xF3 if at(1) == 0xA0 && matches!(at(2), 0x80 | 0x81) && (0x80..=0xBF).contains(&at(3)) => {
            (High::Tag, 4)
        }
        lead => match fold_lookalike(bytes, i) {
            Some((lower, len)) => (High::Letter(lower), len),
            None => (High::Other, utf8_len(lead)),
        },
    }
}

/// The length of the UTF-8 sequence that starts with `lead`, at least one.
const fn utf8_len(lead: u8) -> usize {
    match lead {
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF7 => 4,
        _ => 1,
    }
}

/// The position of the first byte at or after `from` that is not a space or a tab.
fn skip_blanks(bytes: &[u8], from: usize) -> usize {
    from + bytes
        .get(from..)
        .into_iter()
        .flatten()
        .take_while(|b| **b == b' ' || **b == b'\t')
        .count()
}

/// The alphanumeric word that starts at `from` and the position just after it.
fn word_at(bytes: &[u8], from: usize) -> (&[u8], usize) {
    let rest = bytes.get(from..).unwrap_or(&[]);
    let len = rest
        .iter()
        .take_while(|b| b.is_ascii_alphanumeric())
        .count();
    (&rest[..len], from + len)
}

/// Whether an interpreter that ends at `after` is asked to run the program on its input: nothing
/// follows on the line, or only a lone `-`.
fn reads_program_from_pipe(bytes: &[u8], after: usize) -> bool {
    let next = skip_blanks(bytes, after);
    match bytes.get(next) {
        None | Some(b'\n' | b'\r') => true,
        Some(b'-') => matches!(
            bytes.get(next + 1),
            None | Some(b' ' | b'\t' | b'\n' | b'\r')
        ),
        Some(_) => false,
    }
}

/// The part of the line that precedes position `i`, looking back at most 512 bytes.
fn line_prefix(bytes: &[u8], i: usize) -> &[u8] {
    let window = &bytes[i.saturating_sub(512)..i];
    let start = window
        .iter()
        .rposition(|&b| b == b'\n')
        .map_or(0, |p| p + 1);
    &window[start..]
}

/// Whether `haystack` contains `needle`, ignoring ASCII case.
fn contains_ci(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.len() >= needle.len()
        && haystack
            .windows(needle.len())
            .any(|window| window.eq_ignore_ascii_case(needle))
}

/// Whether one of `words` occurs in `text` as a whole word, ignoring ASCII case.
fn has_word(text: &[u8], words: &[&str]) -> bool {
    text.split(|b| !b.is_ascii_alphanumeric()).any(|part| {
        words
            .iter()
            .any(|w| part.eq_ignore_ascii_case(w.as_bytes()))
    })
}
