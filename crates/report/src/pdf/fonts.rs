// SPDX-License-Identifier: Apache-2.0
//! Fonts of the PDF: encoding, glyph widths and text fitting.
//!
//! The PDF uses three of the 14 standard fonts, Helvetica, Helvetica-Bold and Helvetica-Oblique,
//! which every viewer provides, so nothing is embedded. Text is written in `WinAnsiEncoding`
//! (Windows-1252), which covers English and Spanish completely: accents, `ñ`, `¿`, `¡`, curly
//! quotes, the bullet, the en and em dashes, the ellipsis and the euro sign.
//!
//! To lay text out the writer needs the width of every glyph. The tables below are the `WX`
//! advance widths of the official Adobe AFM files (`Helvetica.afm`, `Helvetica-Bold.afm`;
//! Helvetica-Oblique has exactly the same widths as Helvetica), converted from glyph names to
//! `WinAnsiEncoding` codes 32 to 255 with the encoding table of the PDF reference. Codes that
//! `WinAnsiEncoding` leaves undefined map to the bullet, as the reference prescribes. Tests
//! compare a sample of widths against the AFM values.
//!
//! Characters outside Windows-1252 are replaced by `?` (one per character); combining marks,
//! variation selectors and zero-width characters are dropped; other Unicode spaces become a
//! plain space.
//!
//! Invariants: [`encode`] never fails and only produces bytes that have a width; measuring and
//! writing use the same byte strings, so what is measured is what is drawn.

/// One of the three fonts of the document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Font {
    /// Helvetica.
    Regular,
    /// Helvetica-Bold.
    Bold,
    /// Helvetica-Oblique.
    Oblique,
}

impl Font {
    /// Every font, in the order of their resource names.
    pub(super) const ALL: [Self; 3] = [Self::Regular, Self::Bold, Self::Oblique];

    /// The resource name used in content streams (`F1`, `F2`, `F3`).
    pub(super) const fn resource(self) -> &'static str {
        match self {
            Self::Regular => "F1",
            Self::Bold => "F2",
            Self::Oblique => "F3",
        }
    }

    /// The PostScript name of the font.
    pub(super) const fn base_font(self) -> &'static str {
        match self {
            Self::Regular => "Helvetica",
            Self::Bold => "Helvetica-Bold",
            Self::Oblique => "Helvetica-Oblique",
        }
    }

    /// The width table of the font.
    const fn widths(self) -> &'static [u16; 224] {
        match self {
            Self::Regular | Self::Oblique => &HELVETICA,
            Self::Bold => &HELVETICA_BOLD,
        }
    }
}

/// Advance widths of Helvetica and Helvetica-Oblique for the codes 32 to 255, from the Adobe AFM files, in units of 1/1000 em.
const HELVETICA: [u16; 224] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556,
    556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556, 1015, 667, 667, 722, 722, 667,
    611, 778, 722, 278, 500, 667, 556, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667,
    667, 611, 278, 278, 278, 469, 556, 333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500,
    222, 833, 556, 556, 556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584,
    350, 556, 350, 222, 556, 333, 1000, 556, 556, 333, 1000, 667, 333, 1000, 350, 611, 350, 350,
    222, 222, 333, 333, 350, 556, 1000, 333, 1000, 500, 333, 944, 350, 500, 667, 278, 333, 556,
    556, 556, 556, 260, 556, 333, 737, 370, 556, 584, 333, 737, 333, 400, 584, 333, 333, 333, 556,
    537, 278, 333, 333, 365, 556, 834, 834, 834, 611, 667, 667, 667, 667, 667, 667, 1000, 722, 667,
    667, 667, 667, 278, 278, 278, 278, 722, 722, 778, 778, 778, 778, 778, 584, 778, 722, 722, 722,
    722, 667, 667, 611, 556, 556, 556, 556, 556, 556, 889, 500, 556, 556, 556, 556, 278, 278, 278,
    278, 556, 556, 556, 556, 556, 556, 556, 584, 611, 556, 556, 556, 556, 500, 556, 500,
];

/// Advance widths of Helvetica-Bold for the codes 32 to 255, from the Adobe AFM file, in units of 1/1000 em.
const HELVETICA_BOLD: [u16; 224] = [
    278, 333, 474, 556, 556, 889, 722, 238, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556,
    556, 556, 556, 556, 556, 556, 556, 333, 333, 584, 584, 584, 611, 975, 722, 722, 722, 722, 667,
    611, 778, 722, 278, 556, 722, 611, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667,
    667, 611, 333, 278, 333, 584, 556, 333, 556, 611, 556, 611, 556, 333, 611, 611, 278, 278, 556,
    278, 889, 611, 611, 611, 611, 389, 556, 333, 611, 556, 778, 556, 556, 500, 389, 280, 389, 584,
    350, 556, 350, 278, 556, 500, 1000, 556, 556, 333, 1000, 667, 333, 1000, 350, 611, 350, 350,
    278, 278, 500, 500, 350, 556, 1000, 333, 1000, 556, 333, 944, 350, 500, 667, 278, 333, 556,
    556, 556, 556, 280, 556, 333, 737, 370, 556, 584, 333, 737, 333, 400, 584, 333, 333, 333, 611,
    556, 278, 333, 333, 365, 556, 834, 834, 834, 611, 722, 722, 722, 722, 722, 722, 1000, 722, 667,
    667, 667, 667, 278, 278, 278, 278, 722, 722, 778, 778, 778, 778, 778, 584, 778, 722, 722, 722,
    722, 667, 667, 611, 556, 556, 556, 556, 556, 556, 889, 556, 556, 556, 556, 556, 278, 278, 278,
    278, 611, 611, 611, 611, 611, 611, 611, 584, 611, 611, 611, 611, 611, 556, 611, 556,
];

/// The bullet code, which `WinAnsiEncoding` uses for every undefined code.
const BULLET: u8 = 0x95;
/// Tolerance, in points, when comparing a measured width with the room available, so rounding
/// noise never turns a text that fits exactly into a truncated one.
const FIT_SLACK: f64 = 1e-6;
/// The replacement for characters that have no glyph.
const REPLACEMENT: u8 = b'?';
/// The encoded ellipsis.
const ELLIPSIS_BYTE: u8 = 0x85;

/// Unicode characters that live in the `0x80..=0x9F` block of Windows-1252.
const SPECIALS: [(char, u8); 27] = [
    ('\u{20ac}', 0x80),
    ('\u{201a}', 0x82),
    ('\u{0192}', 0x83),
    ('\u{201e}', 0x84),
    ('\u{2026}', 0x85),
    ('\u{2020}', 0x86),
    ('\u{2021}', 0x87),
    ('\u{02c6}', 0x88),
    ('\u{2030}', 0x89),
    ('\u{0160}', 0x8a),
    ('\u{2039}', 0x8b),
    ('\u{0152}', 0x8c),
    ('\u{017d}', 0x8e),
    ('\u{2018}', 0x91),
    ('\u{2019}', 0x92),
    ('\u{201c}', 0x93),
    ('\u{201d}', 0x94),
    ('\u{2022}', 0x95),
    ('\u{2013}', 0x96),
    ('\u{2014}', 0x97),
    ('\u{02dc}', 0x98),
    ('\u{2122}', 0x99),
    ('\u{0161}', 0x9a),
    ('\u{203a}', 0x9b),
    ('\u{0153}', 0x9c),
    ('\u{017e}', 0x9e),
    ('\u{0178}', 0x9f),
];

/// Returns `true` for characters that draw nothing and are dropped: combining marks, variation
/// selectors, zero-width characters and the byte order mark.
fn is_ignorable(c: char) -> bool {
    matches!(
        c,
        '\u{0300}'..='\u{036f}'
            | '\u{200b}'..='\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{2069}'
            | '\u{fe00}'..='\u{fe0f}'
            | '\u{feff}'
    )
}

/// Returns `true` for Unicode spaces that are written as an ordinary space.
fn is_wide_space(c: char) -> bool {
    matches!(
        c,
        '\u{2000}'..='\u{200a}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\t' | '\n' | '\r'
    )
}

/// Maps one character to its `WinAnsiEncoding` code, if it has one.
fn winansi_code(c: char) -> Option<u8> {
    let code = u32::from(c);
    match code {
        0x20..=0x7e | 0xa0..=0xff => u8::try_from(code).ok(),
        _ => SPECIALS
            .iter()
            .find(|(special, _)| *special == c)
            .map(|&(_, byte)| byte),
    }
}

/// Encodes text for the PDF: one byte per character in `WinAnsiEncoding`.
///
/// Unsupported characters become `?`, ignorable ones are dropped, and other Unicode spaces
/// become a plain space.
pub(super) fn encode(text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len());
    for c in text.chars() {
        if is_ignorable(c) {
            continue;
        }
        if is_wide_space(c) {
            out.push(b' ');
        } else if let Some(byte) = winansi_code(c) {
            out.push(byte);
        } else {
            out.push(REPLACEMENT);
        }
    }
    out
}

/// Returns the advance width of one code in units of 1/1000 em.
fn glyph_width(font: Font, code: u8) -> u32 {
    let widths = font.widths();
    match code {
        32..=255 => u32::from(widths[usize::from(code) - 32]),
        _ => u32::from(widths[usize::from(BULLET) - 32]),
    }
}

/// Returns the width of encoded text in points at `size`.
pub(super) fn text_width(font: Font, text: &[u8], size: f64) -> f64 {
    let units: u32 = text.iter().map(|&code| glyph_width(font, code)).sum();
    f64::from(units) * size / 1000.0
}

/// Returns the longest prefix of `text` that, followed by an ellipsis, is at most `max_width`
/// points wide, with the ellipsis appended. When not even the ellipsis fits, the result is empty.
fn ellipsize(font: Font, text: &[u8], size: f64, max_width: f64) -> Vec<u8> {
    let mut used = text_width(font, &[ELLIPSIS_BYTE], size);
    let mut out = Vec::with_capacity(text.len().min(64) + 1);
    if used > max_width {
        return out;
    }
    for &code in text {
        let width = f64::from(glyph_width(font, code)) * size / 1000.0;
        if used + width > max_width {
            break;
        }
        used += width;
        out.push(code);
    }
    while out.last() == Some(&b' ') {
        out.pop();
    }
    out.push(ELLIPSIS_BYTE);
    out
}

/// Shortens `text` with an ellipsis so it is at most `max_width` points wide.
///
/// Text that already fits is returned unchanged. When even the ellipsis does not fit, the result
/// is empty.
pub(super) fn fit(font: Font, text: &[u8], size: f64, max_width: f64) -> Vec<u8> {
    if text_width(font, text, size) <= max_width + FIT_SLACK {
        text.to_vec()
    } else {
        ellipsize(font, text, size, max_width)
    }
}

/// Splits a word wider than `max_width` into pieces that each fit, breaking between characters.
fn break_word(font: Font, word: &[u8], size: f64, max_width: f64) -> Vec<Vec<u8>> {
    let mut pieces = Vec::new();
    let mut start = 0usize;
    let mut used = 0.0;
    for (index, &code) in word.iter().enumerate() {
        let width = f64::from(glyph_width(font, code)) * size / 1000.0;
        if used + width > max_width && index > start {
            pieces.push(word[start..index].to_vec());
            start = index;
            used = 0.0;
        }
        used += width;
    }
    pieces.push(word[start..].to_vec());
    pieces
}

/// Splits `text` into lines no wider than `max_width` points, at most `max_lines` of them.
///
/// Lines break at spaces; a word wider than a line is broken between characters. When the text
/// needs more than `max_lines` lines, the last kept line ends with an ellipsis.
pub(super) fn wrap(
    font: Font,
    text: &[u8],
    size: f64,
    max_width: f64,
    max_lines: usize,
) -> Vec<Vec<u8>> {
    let space = text_width(font, b" ", size);
    let mut lines: Vec<Vec<u8>> = Vec::new();
    let mut current: Vec<u8> = Vec::new();
    let mut current_width = 0.0;
    for word in text
        .split(|&code| code == b' ')
        .filter(|word| !word.is_empty())
    {
        let pieces = if text_width(font, word, size) <= max_width + FIT_SLACK {
            vec![word.to_vec()]
        } else {
            break_word(font, word, size, max_width)
        };
        for piece in pieces {
            let width = text_width(font, &piece, size);
            if current.is_empty() {
                current = piece;
                current_width = width;
            } else if current_width + space + width <= max_width + FIT_SLACK {
                current.push(b' ');
                current.extend_from_slice(&piece);
                current_width += space + width;
            } else {
                lines.push(std::mem::replace(&mut current, piece));
                current_width = width;
            }
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.len() > max_lines {
        lines.truncate(max_lines);
        if let Some(last) = lines.last_mut() {
            let mut open = last.clone();
            open.push(ELLIPSIS_BYTE);
            *last = if text_width(font, &open, size) <= max_width + FIT_SLACK {
                open
            } else {
                ellipsize(font, last, size, max_width)
            };
        }
    }
    lines
}

/// Converts encoded bytes back to text; only tests use it to read what was written.
#[cfg(test)]
pub(super) fn decode(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&code| match code {
            0x20..=0x7e | 0xa0..=0xff => char::from(code),
            _ => SPECIALS
                .iter()
                .find(|(_, byte)| *byte == code)
                .map_or(char::REPLACEMENT_CHARACTER, |&(c, _)| c),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        Font, HELVETICA, HELVETICA_BOLD, decode, encode, fit, glyph_width, text_width, wrap,
    };

    /// Widths of a sample of glyphs, copied from the official AFM files, match the tables.
    #[test]
    fn widths_match_the_afm_files() {
        // (character, Helvetica, Helvetica-Bold)
        let afm = [
            (' ', 278, 278),
            ('A', 667, 722),
            ('a', 556, 556),
            ('i', 222, 278),
            ('W', 944, 944),
            ('m', 833, 889),
            ('\u{e9}', 556, 556),
            ('\u{20ac}', 556, 556),
            ('\u{2013}', 556, 556),
            ('\u{2014}', 1000, 1000),
            ('\u{2022}', 350, 350),
            ('\u{f1}', 556, 611),
            ('\u{bf}', 611, 611),
            ('\u{a1}', 333, 333),
            ('\u{2026}', 1000, 1000),
            ('\u{2018}', 222, 278),
            ('\u{2019}', 222, 278),
            ('\u{201c}', 333, 500),
            ('\u{201d}', 333, 500),
            ('0', 556, 556),
            ('.', 278, 278),
            ('\u{d1}', 722, 722),
            ('\u{f3}', 556, 611),
            ('\u{ba}', 365, 365),
            ('l', 222, 278),
            ('w', 722, 778),
        ];
        for (c, regular, bold) in afm {
            let code = encode(&c.to_string());
            assert_eq!(code.len(), 1, "{c:?}");
            assert_eq!(
                glyph_width(Font::Regular, code[0]),
                regular,
                "regular {c:?}"
            );
            assert_eq!(
                glyph_width(Font::Oblique, code[0]),
                regular,
                "oblique {c:?}"
            );
            assert_eq!(glyph_width(Font::Bold, code[0]), bold, "bold {c:?}");
        }
    }

    /// The tables have plausible contents: every width is positive and monospaced digits agree.
    #[test]
    fn tables_are_sane() {
        for table in [&HELVETICA, &HELVETICA_BOLD] {
            assert!(table.iter().all(|&w| (150..=1100).contains(&w)));
            assert!(
                table[usize::from(b'0') - 32..=usize::from(b'9') - 32]
                    .iter()
                    .all(|&w| w == 556)
            );
        }
    }

    /// Spanish text, curly quotes, dashes, bullet and euro encode to single Windows-1252 bytes.
    #[test]
    fn spanish_and_typography_encode() {
        let text = "¿Cómo está? ¡Ñandú! “hola” \u{2018}x\u{2019} \u{2013} \u{2014} \u{2022} \u{20ac} \u{2026} ü";
        let bytes = encode(text);
        assert_eq!(bytes.len(), text.chars().count());
        assert_eq!(bytes[0], 0xbf);
        assert_eq!(decode(&bytes), text);
    }

    /// Unsupported characters become `?`, ignorable ones vanish and odd spaces become spaces.
    #[test]
    fn unsupported_text_is_replaced() {
        assert_eq!(decode(&encode("日本語")), "???");
        assert_eq!(decode(&encode("a🙂b")), "a?b");
        assert_eq!(decode(&encode("e\u{301}")), "e");
        assert_eq!(decode(&encode("a\u{200b}b\u{fe0f}")), "ab");
        assert_eq!(decode(&encode("a\u{2003}b\tc\nd")), "a b c d");
        assert_eq!(decode(&encode("שלום")), "????");
        assert_eq!(encode(""), Vec::<u8>::new());
    }

    /// Widths add up per glyph and scale with the size.
    #[test]
    fn text_width_scales() {
        let bytes = encode("Aa ");
        assert!(
            (text_width(Font::Regular, &bytes, 10.0) - (667.0 + 556.0 + 278.0) / 100.0).abs()
                < 1e-9
        );
        assert!(
            (text_width(Font::Bold, &bytes, 20.0) - (722.0 + 556.0 + 278.0) / 50.0).abs() < 1e-9
        );
        assert!(text_width(Font::Regular, &[], 10.0).abs() < 1e-9);
    }

    /// Fitting keeps text that fits, ellipsizes text that does not, and never overflows.
    #[test]
    fn fit_truncates_with_an_ellipsis() {
        let text = encode("The quick brown fox jumps over the lazy dog");
        assert_eq!(fit(Font::Regular, &text, 10.0, 1000.0), text);
        for width in [5.0, 20.0, 60.0, 120.0, 200.0] {
            let out = fit(Font::Regular, &text, 10.0, width);
            assert!(
                text_width(Font::Regular, &out, 10.0) <= width + 1e-9,
                "{width}"
            );
            if width >= 20.0 && text_width(Font::Regular, &text, 10.0) > width {
                assert_eq!(out.last(), Some(&0x85));
            }
        }
        assert!(fit(Font::Regular, &text, 10.0, 2.0).is_empty());
    }

    /// Wrapping respects the width, keeps every word, and breaks very long words.
    #[test]
    fn wrap_respects_width() {
        let text = encode("alpha beta gamma delta epsilon zeta eta theta iota kappa");
        let lines = wrap(Font::Regular, &text, 10.0, 90.0, usize::MAX);
        assert!(lines.len() > 2);
        for line in &lines {
            assert!(text_width(Font::Regular, line, 10.0) <= 90.0 + 1e-9);
        }
        let joined = lines
            .iter()
            .map(|l| decode(l))
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(joined, decode(&text));
        let long = encode(&"x".repeat(200));
        let pieces = wrap(Font::Regular, &long, 10.0, 60.0, usize::MAX);
        assert!(pieces.len() > 5);
        assert!(
            pieces
                .iter()
                .all(|p| text_width(Font::Regular, p, 10.0) <= 60.0 + 1e-9)
        );
        assert_eq!(pieces.iter().map(Vec::len).sum::<usize>(), 200);
    }

    /// A line limit truncates with an ellipsis on the last line.
    #[test]
    fn wrap_limits_lines() {
        let text = encode(&"word ".repeat(100));
        let lines = wrap(Font::Regular, &text, 10.0, 80.0, 3);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[2].last(), Some(&0x85));
        assert!(
            lines
                .iter()
                .all(|l| text_width(Font::Regular, l, 10.0) <= 80.0 + 1e-9)
        );
        assert!(wrap(Font::Regular, &text, 10.0, 80.0, 0).is_empty());
        assert!(wrap(Font::Regular, b"", 10.0, 80.0, 3).is_empty());
        let short = wrap(Font::Regular, &encode("two words"), 10.0, 200.0, 3);
        assert_eq!(short.len(), 1);
    }
}
