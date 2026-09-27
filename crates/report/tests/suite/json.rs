// SPDX-License-Identifier: Apache-2.0
//! A minimal, strict JSON parser for the tests, used to validate the graph export.
//!
//! It follows RFC 8259: no trailing commas, no comments, strings with only the legal escapes and
//! no raw control characters, numbers in the standard grammar, and exactly one top-level value.

use std::collections::BTreeMap;

/// A parsed JSON value.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Json {
    /// `null`.
    Null,
    /// `true` or `false`.
    Bool(bool),
    /// A number.
    Number(f64),
    /// A string.
    Str(String),
    /// An array.
    Array(Vec<Json>),
    /// An object; duplicate keys are rejected.
    Object(BTreeMap<String, Json>),
}

impl Json {
    /// Returns the member `key` of an object.
    pub(crate) fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Self::Object(map) => map.get(key),
            _ => None,
        }
    }

    /// Returns the elements of an array.
    pub(crate) fn items(&self) -> &[Json] {
        match self {
            Self::Array(items) => items,
            _ => &[],
        }
    }

    /// Returns the text of a string.
    pub(crate) fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(text) => Some(text),
            _ => None,
        }
    }

    /// Returns the value of a number.
    pub(crate) fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Number(number) => Some(*number),
            _ => None,
        }
    }
}

/// The parser state.
struct Parser<'a> {
    /// The input characters.
    chars: Vec<char>,
    /// Position of the next character.
    at: usize,
    /// Keeps the lifetime of the source for error messages.
    source: &'a str,
}

impl Parser<'_> {
    /// Skips whitespace.
    fn skip(&mut self) {
        while matches!(self.chars.get(self.at), Some(' ' | '\n' | '\r' | '\t')) {
            self.at += 1;
        }
    }

    /// Consumes `expected` or fails.
    fn expect(&mut self, expected: char) -> Result<(), String> {
        if self.chars.get(self.at) == Some(&expected) {
            self.at += 1;
            Ok(())
        } else {
            Err(format!(
                "expected {expected:?} at {} in {:?}",
                self.at,
                &self.source[..self.source.len().min(60)]
            ))
        }
    }

    /// Parses any value.
    fn value(&mut self) -> Result<Json, String> {
        self.skip();
        match self.chars.get(self.at).copied() {
            Some('{') => self.object(),
            Some('[') => self.array(),
            Some('"') => self.string().map(Json::Str),
            Some('t') => self.literal("true", Json::Bool(true)),
            Some('f') => self.literal("false", Json::Bool(false)),
            Some('n') => self.literal("null", Json::Null),
            Some('-' | '0'..='9') => self.number(),
            other => Err(format!("unexpected {other:?} at {}", self.at)),
        }
    }

    /// Parses a keyword.
    fn literal(&mut self, word: &str, value: Json) -> Result<Json, String> {
        for expected in word.chars() {
            self.expect(expected)?;
        }
        Ok(value)
    }

    /// Parses a number following the JSON grammar.
    fn number(&mut self) -> Result<Json, String> {
        let start = self.at;
        if self.chars.get(self.at) == Some(&'-') {
            self.at += 1;
        }
        let digits = |parser: &mut Self| {
            let begin = parser.at;
            while matches!(parser.chars.get(parser.at), Some('0'..='9')) {
                parser.at += 1;
            }
            parser.at - begin
        };
        if self.chars.get(self.at) == Some(&'0') {
            self.at += 1;
        } else if digits(self) == 0 {
            return Err("number without digits".into());
        }
        if self.chars.get(self.at) == Some(&'.') {
            self.at += 1;
            if digits(self) == 0 {
                return Err("fraction without digits".into());
            }
        }
        if matches!(self.chars.get(self.at), Some('e' | 'E')) {
            self.at += 1;
            if matches!(self.chars.get(self.at), Some('+' | '-')) {
                self.at += 1;
            }
            if digits(self) == 0 {
                return Err("exponent without digits".into());
            }
        }
        let text: String = self.chars[start..self.at].iter().collect();
        text.parse()
            .map(Json::Number)
            .map_err(|_| format!("bad number {text}"))
    }

    /// Parses a string.
    fn string(&mut self) -> Result<String, String> {
        self.expect('"')?;
        let mut out = String::new();
        loop {
            let c = *self.chars.get(self.at).ok_or("unterminated string")?;
            self.at += 1;
            match c {
                '"' => return Ok(out),
                '\\' => {
                    let escape = *self.chars.get(self.at).ok_or("unterminated escape")?;
                    self.at += 1;
                    match escape {
                        '"' | '\\' | '/' => out.push(escape),
                        'b' => out.push('\u{8}'),
                        'f' => out.push('\u{c}'),
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        'u' => {
                            let hex: String = self
                                .chars
                                .get(self.at..self.at + 4)
                                .ok_or("short \\u escape")?
                                .iter()
                                .collect();
                            self.at += 4;
                            let unit = u32::from_str_radix(&hex, 16)
                                .map_err(|_| format!("bad \\u{hex}"))?;
                            out.push(char::from_u32(unit).ok_or("lone surrogate")?);
                        }
                        other => return Err(format!("illegal escape \\{other}")),
                    }
                }
                c if u32::from(c) < 0x20 => return Err(format!("raw control character {c:?}")),
                c => out.push(c),
            }
        }
    }

    /// Parses an array.
    fn array(&mut self) -> Result<Json, String> {
        self.expect('[')?;
        let mut items = Vec::new();
        self.skip();
        if self.chars.get(self.at) == Some(&']') {
            self.at += 1;
            return Ok(Json::Array(items));
        }
        loop {
            items.push(self.value()?);
            self.skip();
            match self.chars.get(self.at) {
                Some(',') => self.at += 1,
                Some(']') => {
                    self.at += 1;
                    return Ok(Json::Array(items));
                }
                other => return Err(format!("expected , or ] but found {other:?}")),
            }
        }
    }

    /// Parses an object.
    fn object(&mut self) -> Result<Json, String> {
        self.expect('{')?;
        let mut map = BTreeMap::new();
        self.skip();
        if self.chars.get(self.at) == Some(&'}') {
            self.at += 1;
            return Ok(Json::Object(map));
        }
        loop {
            self.skip();
            let key = self.string()?;
            self.skip();
            self.expect(':')?;
            let value = self.value()?;
            if map.insert(key.clone(), value).is_some() {
                return Err(format!("duplicate key {key}"));
            }
            self.skip();
            match self.chars.get(self.at) {
                Some(',') => self.at += 1,
                Some('}') => {
                    self.at += 1;
                    return Ok(Json::Object(map));
                }
                other => return Err(format!("expected , or }} but found {other:?}")),
            }
        }
    }
}

/// Parses a complete JSON document.
pub(crate) fn parse(source: &str) -> Result<Json, String> {
    let mut parser = Parser {
        chars: source.chars().collect(),
        at: 0,
        source,
    };
    let value = parser.value()?;
    parser.skip();
    if parser.at != parser.chars.len() {
        return Err(format!("trailing characters at {}", parser.at));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::{Json, parse};

    /// Valid documents parse to the expected values.
    #[test]
    fn parses_valid_documents() {
        let value =
            parse("{\"a\": [1, -2.5e3, true, null, \"x\\u00e9\\n\"], \"b\": {}}").expect("valid");
        assert_eq!(value.get("a").expect("a").items().len(), 5);
        assert_eq!(
            value.get("a").expect("a").items()[4].as_str(),
            Some("x\u{e9}\n")
        );
        assert_eq!(
            value.get("a").expect("a").items()[1].as_f64(),
            Some(-2500.0)
        );
        assert_eq!(
            value.get("b"),
            Some(&Json::Object(std::collections::BTreeMap::new()))
        );
    }

    /// Invalid documents are rejected.
    #[test]
    fn rejects_invalid_documents() {
        for bad in [
            "",
            "{",
            "[1,]",
            "{\"a\":1,}",
            "{\"a\" 1}",
            "\"\u{1}\"",
            "\"\\x\"",
            "01",
            "1.",
            "{\"a\":1,\"a\":2}",
            "[] []",
            "'a'",
            "nul",
            "\"\\ud800\"",
        ] {
            assert!(parse(bad).is_err(), "should reject {bad:?}");
        }
    }
}
