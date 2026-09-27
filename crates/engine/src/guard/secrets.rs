// SPDX-License-Identifier: Apache-2.0
//! Secret redaction with hand-written scanners (no regular expressions).
//!
//! [`redact`] looks for five families of secrets and replaces each with `[REDACTED]`:
//!
//! 1. **Prefixed keys**: cloud access keys, repository-host tokens, chat-platform tokens,
//!    service keys of the `sk-` family, search-service keys and JSON Web Tokens. Each is
//!    recognised by its documented prefix and length, never by the name of whoever issues it.
//! 2. **PEM private key blocks**, from the `BEGIN` line to the `END` line (or the end of the text).
//! 3. **Passwords inside URLs** (`scheme://user:password@host`): only the password is replaced.
//! 4. **Assignments** whose name mentions a password, secret, token, API key, authorization or
//!    bearer credential and whose value looks like a credential: only the value is replaced.
//!
//! Every detector reports byte ranges; the ranges are merged (so overlapping findings count once)
//! and replaced in one pass. The function is conservative on purpose: prose, commit hashes, UUIDs
//! and file paths are left alone unless a credential-like name points at them, and text that has
//! already been redacted is a fixed point (`redact(redact(t).0)` finds nothing).

use core::ops::Range;

/// What a redacted secret is replaced with.
const REDACTED: &str = "[REDACTED]";

/// Words that mark a name as credential-like, in lower case.
const NAME_KEYWORDS: [&str; 9] = [
    "password",
    "passwd",
    "secret",
    "token",
    "api_key",
    "apikey",
    "api-key",
    "authorization",
    "bearer",
];

/// Values that describe a credential instead of being one, in lower case.
const NON_SECRET_WORDS: [&str; 33] = [
    "password",
    "changeme",
    "example",
    "placeholder",
    "required",
    "optional",
    "mandatory",
    "expired",
    "invalid",
    "missing",
    "undefined",
    "nullable",
    "redacted",
    "disabled",
    "enabled",
    "unknown",
    "deprecated",
    "sensitive",
    "hashed",
    "encrypted",
    "generated",
    "injected",
    "provided",
    "supplied",
    "hardcoded",
    "yourpassword",
    "yourtoken",
    "yoursecret",
    "yourapikey",
    "notset",
    "unset",
    "external",
    "internal",
];

/// Authorization schemes that precede the credential in a header value, in lower case.
const AUTH_SCHEMES: [&str; 4] = ["bearer", "basic", "token", "digest"];

/// Replaces the secrets in `text` with `[REDACTED]` and returns the new text and how many secrets
/// were replaced.
///
/// # Examples
/// ```
/// use pn_ultramemory_engine::redact;
///
/// let (clean, count) = redact("deploy with AKIAIOSFODNN7EXAMPLE and password=hunter2hunter2");
/// assert_eq!(clean, "deploy with [REDACTED] and password=[REDACTED]");
/// assert_eq!(count, 2);
///
/// // Ordinary text, commit hashes and file paths are left alone.
/// let note = "fixed in 3f5a9c1d2e4b6a8c0d1e3f5a7b9c1d3e5f7a9b1c, see src/config.rs";
/// assert_eq!(redact(note), (note.to_owned(), 0));
/// ```
#[must_use]
pub fn redact(text: &str) -> (String, u32) {
    let bytes = text.as_bytes();
    let mut spans: Vec<Range<usize>> = Vec::new();
    prefixed_keys(bytes, &mut spans);
    pem_blocks(text, &mut spans);
    url_passwords(bytes, &mut spans);
    assignments(bytes, &mut spans);
    if spans.is_empty() {
        return (text.to_owned(), 0);
    }
    spans.sort_by_key(|span| (span.start, span.end));
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(spans.len());
    for span in spans {
        match merged.last_mut() {
            Some(last) if span.start <= last.end => last.end = last.end.max(span.end),
            _ => merged.push(span),
        }
    }
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    for span in &merged {
        out.push_str(text.get(cursor..span.start).unwrap_or(""));
        out.push_str(REDACTED);
        cursor = span.end;
    }
    out.push_str(text.get(cursor..).unwrap_or(""));
    (out, u32::try_from(merged.len()).unwrap_or(u32::MAX))
}

/// Number of leading bytes of `bytes` that satisfy `accept`.
fn run_len(bytes: &[u8], accept: impl Fn(u8) -> bool) -> usize {
    bytes.iter().take_while(|&&b| accept(b)).count()
}

/// Whether `haystack` contains `needle`, ignoring ASCII case.
fn contains_ci(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack.len() >= needle.len()
        && haystack
            .windows(needle.len())
            .any(|window| window.eq_ignore_ascii_case(needle))
}

/// Position of the first `needle` in `haystack` at or after `from`.
fn find_from(haystack: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    haystack
        .get(from..)?
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|offset| from + offset)
}

// ---- prefixed keys ----------------------------------------------------------------------------

/// Finds keys and tokens that announce themselves with a fixed prefix.
fn prefixed_keys(bytes: &[u8], out: &mut Vec<Range<usize>>) {
    let mut i = 0;
    while i < bytes.len() {
        let at_word_start = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
        if at_word_start {
            if let Some(len) = key_at(&bytes[i..]) {
                out.push(i..i + len);
                i += len;
                continue;
            }
        }
        i += 1;
    }
}

/// The length of the key that starts `rest`, if one does.
fn key_at(rest: &[u8]) -> Option<usize> {
    match rest.first()? {
        b'A' => aws_key(rest).or_else(|| google_key(rest)),
        b'g' => github_token(rest),
        b'x' => slack_token(rest),
        b's' => secret_key(rest),
        b'e' => json_web_token(rest),
        _ => None,
    }
}

/// `AKIA` or `ASIA` followed by exactly 16 upper-case letters or digits.
fn aws_key(rest: &[u8]) -> Option<usize> {
    if !(rest.starts_with(b"AKIA") || rest.starts_with(b"ASIA")) {
        return None;
    }
    let body = run_len(&rest[4..], |b| b.is_ascii_uppercase() || b.is_ascii_digit());
    (body == 16).then_some(20)
}

/// `AIza` followed by at least 35 characters of the URL-safe alphabet.
fn google_key(rest: &[u8]) -> Option<usize> {
    if !rest.starts_with(b"AIza") {
        return None;
    }
    let body = run_len(&rest[4..], |b| {
        b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
    });
    (body >= 35).then_some(4 + body)
}

/// `ghp_`, `gho_`, `ghu_`, `ghs_`, `ghr_` with 30 or more alphanumerics, or `github_pat_`.
fn github_token(rest: &[u8]) -> Option<usize> {
    if rest.starts_with(b"github_pat_") {
        let body = run_len(&rest[11..], |b| b.is_ascii_alphanumeric() || b == b'_');
        return (body >= 22).then_some(11 + body);
    }
    let known = rest.len() > 4
        && rest.starts_with(b"gh")
        && matches!(rest[2], b'p' | b'o' | b'u' | b's' | b'r')
        && rest[3] == b'_';
    if !known {
        return None;
    }
    let body = run_len(&rest[4..], |b| b.is_ascii_alphanumeric());
    (body >= 30).then_some(4 + body)
}

/// `xoxb-`, `xoxa-`, `xoxp-`, `xoxr-` or `xoxs-` followed by at least ten token characters.
fn slack_token(rest: &[u8]) -> Option<usize> {
    let known = rest.len() > 5
        && rest.starts_with(b"xox")
        && matches!(rest[3], b'b' | b'a' | b'p' | b'r' | b's')
        && rest[4] == b'-';
    if !known {
        return None;
    }
    let body = run_len(&rest[5..], |b| b.is_ascii_alphanumeric() || b == b'-');
    (body >= 10).then_some(5 + body)
}

/// `sk_live_` or `sk_test_` with 20 or more alphanumerics, or `sk-` with 20 or more token
/// characters that look random (a digit, or both letter cases), so that a kebab-case word such as
/// `sk-learn-pipeline-preprocessing` is not mistaken for a key.
fn secret_key(rest: &[u8]) -> Option<usize> {
    for prefix in [&b"sk_live_"[..], &b"sk_test_"[..]] {
        if rest.starts_with(prefix) {
            let body = run_len(&rest[prefix.len()..], |b| b.is_ascii_alphanumeric());
            return (body >= 20).then_some(prefix.len() + body);
        }
    }
    if !rest.starts_with(b"sk-") {
        return None;
    }
    let body = &rest[3..];
    let len = run_len(body, |b| {
        b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
    });
    let token = &body[..len];
    let random = token.iter().any(u8::is_ascii_digit)
        || (token.iter().any(u8::is_ascii_uppercase) && token.iter().any(u8::is_ascii_lowercase));
    (len >= 20 && random).then_some(3 + len)
}

/// Three base64url segments, the first starting with `eyJ`.
fn json_web_token(rest: &[u8]) -> Option<usize> {
    if !rest.starts_with(b"eyJ") {
        return None;
    }
    let segment = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'-';
    let first = run_len(rest, segment);
    if first < 10 || rest.get(first) != Some(&b'.') {
        return None;
    }
    let second = run_len(&rest[first + 1..], segment);
    let after_second = first + 1 + second;
    if second < 4 || rest.get(after_second) != Some(&b'.') {
        return None;
    }
    let third = run_len(&rest[after_second + 1..], segment);
    (third >= 8).then_some(after_second + 1 + third)
}

// ---- PEM private keys -------------------------------------------------------------------------

/// Finds `-----BEGIN ... PRIVATE KEY-----` blocks, up to their `END` line or the end of the text.
fn pem_blocks(text: &str, out: &mut Vec<Range<usize>>) {
    const BEGIN: &str = "-----BEGIN ";
    const DASHES: &str = "-----";
    let mut from = 0;
    while let Some(offset) = text.get(from..).and_then(|rest| rest.find(BEGIN)) {
        let begin = from + offset;
        let label_start = begin + BEGIN.len();
        from = label_start;
        let Some(line) = text.get(label_start..) else {
            break;
        };
        let line = line.split('\n').next().unwrap_or("");
        let Some(close) = line.find(DASHES) else {
            continue;
        };
        if !line[..close].contains("PRIVATE KEY") {
            continue;
        }
        let body_start = label_start + close + DASHES.len();
        let end = text.get(body_start..).and_then(|body| {
            let marker = body.find("-----END ")?;
            let after = marker + "-----END ".len();
            let closing = body.get(after..)?.find(DASHES)?;
            Some(body_start + after + closing + DASHES.len())
        });
        let end = end.or_else(|| {
            let has_body = text
                .get(body_start..)?
                .lines()
                .any(|l| l.trim().len() >= 16 && l.trim().bytes().all(is_base64_byte));
            has_body.then_some(text.len())
        });
        if let Some(end) = end {
            out.push(begin..end);
            from = end;
        }
    }
}

/// Whether a byte belongs to the standard base64 alphabet.
const fn is_base64_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=')
}

// ---- credentials in URLs ----------------------------------------------------------------------

/// Finds `scheme://user:password@host` and marks the password.
fn url_passwords(bytes: &[u8], out: &mut Vec<Range<usize>>) {
    let mut from = 0;
    while let Some(marker) = find_from(bytes, from, b"://") {
        from = marker + 3;
        let scheme_len = bytes[..marker]
            .iter()
            .rev()
            .take_while(|&&b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'.' | b'-'))
            .count();
        if scheme_len == 0 {
            continue;
        }
        let start = marker + 3;
        let authority_len = run_len(&bytes[start..], |b| {
            !(b.is_ascii_whitespace()
                || matches!(
                    b,
                    b'/' | b'?' | b'#' | b'"' | b'\'' | b'<' | b'>' | b'`' | b'\\'
                ))
        });
        let authority = &bytes[start..start + authority_len];
        let Some(at) = authority.iter().rposition(|&b| b == b'@') else {
            continue;
        };
        let userinfo = &authority[..at];
        let Some(colon) = userinfo.iter().position(|&b| b == b':') else {
            continue;
        };
        let password = &userinfo[colon + 1..];
        if !password.is_empty() && !is_reference(password) {
            out.push(start + colon + 1..start + at);
        }
    }
}

/// Whether a value is a placeholder, a mask or a reference to a secret held elsewhere, such as
/// `<password>`, `${DB_PASSWORD}`, `$TOKEN`, `[REDACTED]` or `********`.
fn is_reference(value: &[u8]) -> bool {
    let Some(&first) = value.first() else {
        return true;
    };
    matches!(first, b'$' | b'%' | b'<' | b'{' | b'[' | b'@' | b'&' | b'(')
        || value
            .iter()
            .all(|b| matches!(b, b'*' | b'x' | b'X' | b'#' | b'.' | b'_' | b'-'))
}

// ---- assignments ------------------------------------------------------------------------------

/// Whether a byte can be part of a name such as `db_password` or `x-api-key`.
const fn is_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.')
}

/// Whether a lower-cased name mentions a credential.
fn mentions_credential(name: &[u8]) -> bool {
    NAME_KEYWORDS
        .iter()
        .any(|keyword| contains_ci(name, keyword.as_bytes()))
}

/// Finds `name: value` and `name=value` where the name mentions a credential.
fn assignments(bytes: &[u8], out: &mut Vec<Range<usize>>) {
    let mut i = 0;
    while i < bytes.len() {
        if !is_name_byte(bytes[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && is_name_byte(bytes[i]) {
            i += 1;
        }
        let name = &bytes[start..i];
        if !mentions_credential(name) {
            continue;
        }
        if let Some(range) = assigned_value(bytes, start, i) {
            let value = &bytes[range.clone()];
            let bare_bearer = name.eq_ignore_ascii_case(b"bearer") && !has_separator(bytes, i);
            let credential = if bare_bearer {
                value.len() >= 20
                    && value.iter().any(u8::is_ascii_digit)
                    && looks_like_secret(value)
            } else {
                looks_like_secret(value)
            };
            if credential {
                out.push(range);
            }
        }
    }
}

/// Whether the name that ends at `end` is followed by `:` or `=` (after an optional closing quote
/// and blanks).
fn has_separator(bytes: &[u8], end: usize) -> bool {
    separator_end(bytes, end).is_some()
}

/// Skips blanks (spaces and tabs) from `from`.
fn skip_blanks(bytes: &[u8], from: usize) -> usize {
    from + run_len(bytes.get(from..).unwrap_or(&[]), |b| {
        b == b' ' || b == b'\t'
    })
}

/// The position just after the separator that follows the name ending at `end`, if any. The
/// separators are `:`, `=`, `:=` and `=>`; `::` (a path) and `==` (a comparison) are refused.
fn separator_end(bytes: &[u8], end: usize) -> Option<usize> {
    let mut p = end;
    if matches!(bytes.get(p), Some(b'"' | b'\'' | b'`')) {
        p += 1;
    }
    p = skip_blanks(bytes, p);
    match (bytes.get(p)?, bytes.get(p + 1)) {
        (b':', Some(b':')) | (b'=', Some(b'=')) => None,
        (b':', Some(b'=')) | (b'=', Some(b'>')) => Some(p + 2),
        (b':' | b'=', _) => Some(p + 1),
        _ => None,
    }
}

/// The byte range of the value assigned to the name that spans `start..end`.
fn assigned_value(bytes: &[u8], start: usize, end: usize) -> Option<Range<usize>> {
    let name = &bytes[start..end];
    let in_query = start > 0 && matches!(bytes[start - 1], b'?' | b'&');
    let mut p = if let Some(after) = separator_end(bytes, end) {
        skip_blanks(bytes, after)
    } else if name.starts_with(b"--") || name.eq_ignore_ascii_case(b"bearer") {
        // `--password value` and `Bearer value`: the value follows after blanks.
        let after = skip_blanks(bytes, end);
        if after == end {
            return None;
        }
        after
    } else {
        return None;
    };
    // A scheme word before the credential: `Authorization: Bearer <credential>`.
    let word_len = run_len(bytes.get(p..)?, |b| b.is_ascii_alphabetic());
    let word = bytes.get(p..p + word_len)?;
    if AUTH_SCHEMES
        .iter()
        .any(|scheme| word.eq_ignore_ascii_case(scheme.as_bytes()))
        && matches!(bytes.get(p + word_len), Some(b' ' | b'\t'))
    {
        p = skip_blanks(bytes, p + word_len);
    }
    let first = *bytes.get(p)?;
    let (from, to) = if matches!(first, b'"' | b'\'' | b'`') {
        let inner = p + 1;
        let closing = bytes[inner..]
            .iter()
            .position(|&b| b == first || b == b'\n')
            .filter(|&offset| bytes[inner + offset] == first);
        match closing {
            Some(offset) => (inner, inner + offset),
            None => (inner, inner + unquoted_len(&bytes[inner..], in_query)),
        }
    } else {
        (p, p + unquoted_len(&bytes[p..], in_query))
    };
    let mut to = to;
    while to > from
        && matches!(
            bytes[to - 1],
            b'"' | b'\'' | b'`' | b',' | b';' | b')' | b']' | b'}' | b'>'
        )
    {
        to -= 1;
    }
    (to > from).then_some(from..to)
}

/// Length of an unquoted value: up to a blank, and up to `&` inside a URL query.
fn unquoted_len(rest: &[u8], in_query: bool) -> usize {
    run_len(rest, |b| {
        !(b.is_ascii_whitespace() || (in_query && b == b'&'))
    })
}

/// Decides whether an assigned value looks like a credential and not like a description of one.
///
/// A value is rejected when it is shorter than eight bytes; a reference, mask or placeholder; code
/// (parentheses, brackets, braces or angle brackets); a number or a version; a path or a URL; a
/// well-known non-secret word; or an identifier (letters with underscores, dots, hyphens or mixed
/// case, and no digit), which is how variables and types are written.
fn looks_like_secret(value: &[u8]) -> bool {
    if value.len() < 8 || is_reference(value) {
        return false;
    }
    if value
        .iter()
        .any(|b| matches!(b, b'(' | b')' | b'[' | b']' | b'{' | b'}' | b'<' | b'>'))
    {
        return false;
    }
    if is_path_like(value) || contains_ci(value, b"://") {
        return false;
    }
    if value
        .iter()
        .all(|b| b.is_ascii_digit() || matches!(b, b'.' | b',' | b'-' | b'+' | b':' | b'/'))
    {
        return false;
    }
    let lower = value.to_ascii_lowercase();
    if NON_SECRET_WORDS
        .iter()
        .any(|word| lower.as_slice() == word.as_bytes())
    {
        return false;
    }
    !is_identifier_like(value)
}

/// Whether a value starts like a file path.
fn is_path_like(value: &[u8]) -> bool {
    value.starts_with(b"/")
        || value.starts_with(b"./")
        || value.starts_with(b"../")
        || value.starts_with(b"~/")
        || value.starts_with(b"\\\\")
        || (value.len() > 2
            && value[0].is_ascii_alphabetic()
            && value[1] == b':'
            && matches!(value[2], b'\\' | b'/'))
}

/// Whether a value is written like a variable or a type name: letters, underscores, dots and
/// hyphens only, with a separator or a change of case.
fn is_identifier_like(value: &[u8]) -> bool {
    if !value
        .iter()
        .all(|b| b.is_ascii_alphabetic() || matches!(b, b'_' | b'.' | b'-'))
    {
        return false;
    }
    let separated = value.iter().any(|b| matches!(b, b'_' | b'.' | b'-'));
    let mixed =
        value.iter().any(u8::is_ascii_uppercase) && value.iter().any(u8::is_ascii_lowercase);
    separated || mixed
}

#[cfg(test)]
mod tests {
    use super::{looks_like_secret, redact};

    /// Redacts and returns only the new text.
    fn clean(text: &str) -> String {
        redact(text).0
    }

    /// Each provider key format is found, in the middle of a sentence.
    #[test]
    fn provider_keys_are_redacted() {
        // Every key here is a published example value, not a credential. They are still written
        // as a prefix joined to a body rather than as one literal, because a secret scanner reads
        // source text and cannot tell an example from the real thing: written whole, this file
        // would trip the scanner of everyone who clones the repository, and it blocked a push to
        // GitHub before it was split. `concat!` joins them at compile time, so what the redactor
        // is tested against is byte for byte what a real key looks like.
        let keys = [
            concat!("AKIA", "IOSFODNN7EXAMPLE"),
            concat!("ASIA", "IOSFODNN7EXAMPLE"),
            concat!("ghp_", "16C7e42F292c6912E7710c838347Ae178B4a"),
            concat!("gho_", "16C7e42F292c6912E7710c838347Ae178B4a"),
            concat!(
                "github_pat_",
                "11ABCDEFG0abcdefghijkl_mnopqrstuvwxyz0123456789ABCDEFGHIJKLMNOPQRSTUVW"
            ),
            concat!(
                "xoxb-",
                "123456789012-1234567890123-AbCdEfGhIjKlMnOpQrStUvWx"
            ),
            concat!("sk_live_", "4eC39HqLyjWDarjtT1zdp7dc"),
            concat!("sk_test_", "4eC39HqLyjWDarjtT1zdp7dc"),
            concat!("sk-proj-", "Abc123Def456Ghi789Jkl012Mno345"),
            concat!("AIzaSy", "A-1234567890abcdefghijklmnopqrstuv"),
            concat!(
                "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.",
                "eyJzdWIiOiIxMjM0NTY3ODkwIn0.",
                "dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U"
            ),
        ];
        for key in keys {
            let text = format!("the value {key} was pasted here");
            assert_eq!(
                redact(&text),
                ("the value [REDACTED] was pasted here".to_owned(), 1),
                "{key}"
            );
        }
    }

    /// Keys are not found inside longer words, and short lookalikes are ignored.
    #[test]
    fn near_misses_are_left_alone() {
        for text in [
            "risk-assessment-for-the-billing-service-rollout",
            "sk-learn-pipeline-with-preprocessing-steps",
            "AKIA1234",
            "AKIAIOSFODNN7EXAMPLEXTRA",
            "prefixAKIAIOSFODNN7EXAMPLE",
            "ghp_short",
            "eyJhbGciOiJIUzI1NiJ9.short",
            "xoxb-1",
        ] {
            assert_eq!(redact(text), (text.to_owned(), 0), "{text}");
        }
    }

    /// A PEM block is removed up to its END line, or to the end of the text when it is cut off.
    #[test]
    fn pem_blocks_are_removed() {
        let text = "key:\n-----BEGIN RSA PRIVATE KEY-----\nMIIEpAIBAAKCAQEA1234567890abcdef\n\
                    -----END RSA PRIVATE KEY-----\nafter";
        assert_eq!(redact(text), ("key:\n[REDACTED]\nafter".to_owned(), 1));
        let cut = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAABG5vbmU=\nmore";
        assert_eq!(redact(cut), ("[REDACTED]".to_owned(), 1));
        let mention = "the header -----BEGIN PRIVATE KEY----- marks the file";
        assert_eq!(redact(mention), (mention.to_owned(), 0));
        let public = "-----BEGIN PUBLIC KEY-----\nMIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8A\n-----END PUBLIC KEY-----";
        assert_eq!(redact(public), (public.to_owned(), 0));
    }

    /// Only the password of a URL is replaced.
    #[test]
    fn url_passwords_are_replaced() {
        assert_eq!(
            clean("postgres://admin:s3cr3t@db.internal:5432/app"),
            "postgres://admin:[REDACTED]@db.internal:5432/app"
        );
        let harmless = "http://localhost:3000/path@here and ssh://git@github.com:22/x.git";
        assert_eq!(redact(harmless), (harmless.to_owned(), 0));
        let reference = "postgres://user:${DB_PASSWORD}@host/db and redis://:<password>@cache";
        assert_eq!(redact(reference), (reference.to_owned(), 0));
    }

    /// Assignments redact the value only, in several syntaxes.
    #[test]
    fn assignments_redact_the_value_only() {
        assert_eq!(clean("password=hunter2hunter2"), "password=[REDACTED]");
        assert_eq!(
            clean("DB_PASSWORD: 's3cr3t-value'"),
            "DB_PASSWORD: '[REDACTED]'"
        );
        assert_eq!(
            clean(r#"{"api_key": "abcd1234efgh5678"}"#),
            r#"{"api_key": "[REDACTED]"}"#
        );
        assert_eq!(
            clean("Authorization: Bearer abc123DEF456ghi789"),
            "Authorization: Bearer [REDACTED]"
        );
        assert_eq!(
            clean("curl --password hunter2hunter2 x"),
            "curl --password [REDACTED] x"
        );
        assert_eq!(
            clean("GET /cb?token=abc123def456ghi&state=ok"),
            "GET /cb?token=[REDACTED]&state=ok"
        );
        assert_eq!(
            clean("secret := \"correcthorsebattery\""),
            "secret := \"[REDACTED]\""
        );
    }

    /// Descriptions, references, numbers, paths and code are not credentials.
    #[test]
    fn non_credentials_are_kept() {
        for text in [
            "the password must be at least 8 characters",
            "token: required",
            "password: $DB_PASSWORD_FROM_VAULT",
            "secret = self.client_secret_value",
            "token = generate_token_v2(user)",
            "password = os.environ[\"DB_PASSWORD\"]",
            "max_tokens: 4096",
            "token_budget = 100000000",
            "token_url=https://auth.example.com/oauth/token",
            "secrets_dir=/var/lib/secrets/current",
            "authorization: SessionHandlerType",
            "commit: 3f5a9c1d2e4b6a8c0d1e3f5a7b9c1d3e5f7a9b1c",
            "id 550e8400-e29b-41d4-a716-446655440000 in src/auth/token.rs",
            "if password == expected_password_value {",
            "use crate::auth::token::TokenValidator",
        ] {
            assert_eq!(redact(text), (text.to_owned(), 0), "{text}");
        }
    }

    /// A credential-like name points at a hash or a UUID, so the value goes.
    #[test]
    fn credential_names_redact_hashes_and_uuids() {
        assert_eq!(
            clean("token: 550e8400-e29b-41d4-a716-446655440000"),
            "token: [REDACTED]"
        );
        assert_eq!(
            clean("secret=3f5a9c1d2e4b6a8c0d1e3f5a7b9c1d3e5f7a9b1c"),
            "secret=[REDACTED]"
        );
    }

    /// Overlapping findings count once and redaction is a fixed point.
    #[test]
    fn overlaps_merge_and_redaction_is_idempotent() {
        let text = "Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U";
        let (once, count) = redact(text);
        assert_eq!(once, "Authorization: Bearer [REDACTED]");
        assert_eq!(count, 1);
        assert_eq!(redact(&once), (once.clone(), 0));
        let many = "password=hunter2hunter2 AKIAIOSFODNN7EXAMPLE postgres://a:b1c2d3@h/x";
        let (cleaned, count) = redact(many);
        assert_eq!(count, 3);
        assert_eq!(redact(&cleaned), (cleaned.clone(), 0));
    }

    /// Non-ASCII text around a secret survives intact.
    #[test]
    fn unicode_is_preserved() {
        let (cleaned, count) = redact("clave 🔑 AKIAIOSFODNN7EXAMPLE ñandú");
        assert_eq!(cleaned, "clave 🔑 [REDACTED] ñandú");
        assert_eq!(count, 1);
    }

    /// The value classifier rejects short values and accepts mixed ones.
    #[test]
    fn classifier_boundaries() {
        assert!(!looks_like_secret(b"short1"));
        assert!(looks_like_secret(b"abcd1234"));
        assert!(looks_like_secret(b"supersecret"));
        assert!(!looks_like_secret(b"snake_case_name"));
        assert!(!looks_like_secret(b"CamelCaseValue"));
    }
}
