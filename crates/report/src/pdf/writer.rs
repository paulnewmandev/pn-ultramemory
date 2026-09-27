// SPDX-License-Identifier: Apache-2.0
//! The low-level PDF 1.4 file writer: objects, streams, the cross-reference table and the trailer.
//!
//! Objects are collected in memory with numbers assigned in order of creation; [`Pdf::finish`]
//! serializes them one after the other, records the byte offset of each and appends the
//! cross-reference table (`xref`), the trailer with a deterministic file identifier, and the
//! `startxref` pointer. Streams are stored uncompressed and carry an exact direct `/Length`.
//!
//! Invariants: the output is pure ASCII except for the four high bytes of the binary-marker
//! comment; every `xref` entry is exactly 20 bytes; every offset points at `N 0 obj`; the file
//! ends with `%%EOF` and a newline; the same objects always produce the same bytes.

use std::fmt::Write as _;

/// A PDF file under construction.
#[derive(Debug, Default)]
pub(super) struct Pdf {
    /// Object bodies; the object with number `n` is at index `n - 1`.
    objects: Vec<Vec<u8>>,
}

impl Pdf {
    /// Creates an empty file.
    pub(super) fn new() -> Self {
        Self::default()
    }

    /// Reserves the next object number, to be filled with [`Pdf::set`].
    pub(super) fn reserve(&mut self) -> usize {
        self.objects.push(Vec::new());
        self.objects.len()
    }

    /// Sets the body of a reserved object.
    ///
    /// A number that was never reserved is ignored, so a wiring mistake cannot panic.
    pub(super) fn set(&mut self, id: usize, body: Vec<u8>) {
        if let Some(slot) = id
            .checked_sub(1)
            .and_then(|index| self.objects.get_mut(index))
        {
            *slot = body;
        }
    }

    /// Serializes the file. `root` and `info` are the object numbers of the catalog and the
    /// document information dictionary.
    pub(super) fn finish(self, root: usize, info: usize) -> Vec<u8> {
        let mut out: Vec<u8> =
            Vec::with_capacity(self.objects.iter().map(|o| o.len() + 24).sum::<usize>() + 512);
        out.extend_from_slice(b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n");
        let mut offsets = Vec::with_capacity(self.objects.len());
        for (index, body) in self.objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
            out.extend_from_slice(body);
            out.extend_from_slice(b"\nendobj\n");
        }
        let id = file_id(&out);
        let xref_at = out.len();
        let mut table = String::new();
        let _ = writeln!(table, "xref\n0 {}", offsets.len() + 1);
        table.push_str("0000000000 65535 f \n");
        for offset in &offsets {
            let _ = writeln!(table, "{offset:010} 00000 n ");
        }
        let _ = write!(
            table,
            "trailer\n<< /Size {} /Root {root} 0 R /Info {info} 0 R /ID [<{id}> <{id}>] >>\nstartxref\n{xref_at}\n%%EOF\n",
            offsets.len() + 1
        );
        out.extend_from_slice(table.as_bytes());
        out
    }
}

/// Builds the body of a stream object: the dictionary entries, the exact length and the data.
pub(super) fn stream_body(extra: &str, data: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(data.len() + 64);
    body.extend_from_slice(format!("<< {extra}/Length {} >>\nstream\n", data.len()).as_bytes());
    body.extend_from_slice(data);
    body.extend_from_slice(b"\nendstream");
    body
}

/// Computes a 128-bit file identifier as 32 hexadecimal digits: two FNV-1a passes over the
/// content with different offset bases. It is deterministic, which is all the identifier needs
/// here, and it is not a security feature.
fn file_id(content: &[u8]) -> String {
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut first = 0xcbf2_9ce4_8422_2325_u64;
    let mut second = 0x8422_2325_cbf2_9ce4_u64;
    for &byte in content {
        first = (first ^ u64::from(byte)).wrapping_mul(PRIME);
        second = (second ^ u64::from(byte))
            .wrapping_mul(PRIME)
            .rotate_left(5);
    }
    format!("{first:016x}{second:016x}")
}

/// Formats bytes as a PDF literal string: parentheses, backslashes and every byte outside
/// printable ASCII are escaped, so the result is 7-bit clean.
pub(super) fn literal(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() + 2);
    out.push('(');
    for &byte in bytes {
        match byte {
            b'(' | b')' | b'\\' => {
                out.push('\\');
                out.push(char::from(byte));
            }
            0x20..=0x7e => out.push(char::from(byte)),
            _ => {
                let _ = write!(out, "\\{byte:03o}");
            }
        }
    }
    out.push(')');
    out
}

/// Formats text as a PDF text string in UTF-16BE with a byte order mark, written in hex, which
/// can carry any Unicode character (used for the title and the bookmarks).
pub(super) fn text_string(text: &str) -> String {
    let mut out = String::from("<FEFF");
    for unit in text.encode_utf16() {
        let _ = write!(out, "{unit:04X}");
    }
    out.push('>');
    out
}

/// Converts a `generated_on` value such as `2026-09-25` or `2026-09-25T14:30:05` to a PDF date
/// (`D:20260925` or `D:20260925143005`). Returns `None` when the text does not start with a
/// plausible date, so the document simply has no creation date.
pub(super) fn creation_date(generated_on: &str) -> Option<String> {
    let bytes = generated_on.trim().as_bytes();
    let digits = |range: std::ops::Range<usize>| -> Option<u32> {
        let slice = bytes.get(range)?;
        if slice.iter().all(u8::is_ascii_digit) {
            std::str::from_utf8(slice).ok()?.parse().ok()
        } else {
            None
        }
    };
    let (year, month, day) = (digits(0..4)?, digits(5..7)?, digits(8..10)?);
    if !matches!(bytes.get(4), Some(b'-' | b'/')) || !matches!(bytes.get(7), Some(b'-' | b'/')) {
        return None;
    }
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || year == 0 {
        return None;
    }
    let mut date = format!("D:{year:04}{month:02}{day:02}");
    if matches!(bytes.get(10), Some(b'T' | b' ')) && bytes.get(13) == Some(&b':') {
        if let (Some(hour), Some(minute)) = (digits(11..13), digits(14..16)) {
            if hour < 24 && minute < 60 {
                let second = if bytes.get(16) == Some(&b':') {
                    digits(17..19).filter(|s| *s < 60)
                } else {
                    None
                };
                let _ = write!(date, "{hour:02}{minute:02}{:02}", second.unwrap_or(0));
            }
        }
    }
    Some(date)
}

#[cfg(test)]
mod tests {
    use super::{Pdf, creation_date, file_id, literal, stream_body, text_string};

    /// Literal strings escape delimiters and high bytes.
    #[test]
    fn literal_strings_are_escaped() {
        assert_eq!(literal(b"plain"), "(plain)");
        assert_eq!(literal(b"a(b)c\\d"), "(a\\(b\\)c\\\\d)");
        assert_eq!(literal(&[0xe1, 0x00, 0x0a, 0x7f]), "(\\341\\000\\012\\177)");
        assert_eq!(literal(b""), "()");
    }

    /// Text strings are UTF-16BE hex with a byte order mark and handle astral characters.
    #[test]
    fn text_strings_are_utf16() {
        assert_eq!(text_string("Aé"), "<FEFF004100E9>");
        assert_eq!(text_string("🙂"), "<FEFFD83DDE42>");
        assert_eq!(text_string(""), "<FEFF>");
    }

    /// Creation dates come only from well-formed dates, with an optional time.
    #[test]
    fn creation_dates() {
        assert_eq!(creation_date("2026-09-25").as_deref(), Some("D:20260925"));
        assert_eq!(creation_date(" 2026-09-25 ").as_deref(), Some("D:20260925"));
        assert_eq!(
            creation_date("2026-09-25T14:30:05").as_deref(),
            Some("D:20260925143005")
        );
        assert_eq!(
            creation_date("2026-09-25 14:30").as_deref(),
            Some("D:20260925143000")
        );
        assert_eq!(creation_date("2026-13-25"), None);
        assert_eq!(creation_date("2026-09-32"), None);
        assert_eq!(creation_date("0000-01-01"), None);
        assert_eq!(creation_date("25/09/2026"), None);
        assert_eq!(creation_date("yesterday"), None);
        assert_eq!(creation_date(""), None);
        assert_eq!(creation_date("2026-09-25T99:99"), Some("D:20260925".into()));
    }

    /// Streams declare their exact length.
    #[test]
    fn stream_length_is_exact() {
        let body = stream_body("/Type /X ", b"0 0 m");
        assert_eq!(
            body,
            b"<< /Type /X /Length 5 >>\nstream\n0 0 m\nendstream".to_vec()
        );
    }

    /// The file identifier is deterministic and depends on the content.
    #[test]
    fn file_id_is_stable() {
        assert_eq!(file_id(b"abc"), file_id(b"abc"));
        assert_ne!(file_id(b"abc"), file_id(b"abd"));
        assert_eq!(file_id(b"").len(), 32);
    }

    /// A finished file has the frame, a consistent cross-reference table and a trailer.
    #[test]
    fn finished_file_is_consistent() {
        let mut pdf = Pdf::new();
        let root = pdf.reserve();
        let info = pdf.reserve();
        pdf.set(info, b"<< /Producer (t) >>".to_vec());
        pdf.set(root, b"<< /Type /Catalog >>".to_vec());
        pdf.set(99, b"ignored".to_vec());
        pdf.set(0, b"ignored".to_vec());
        let bytes = pdf.finish(root, info);
        let find = |needle: &[u8], from: usize| -> usize {
            (from..bytes.len())
                .find(|&at| bytes[at..].starts_with(needle))
                .expect("marker present")
        };
        assert!(bytes.starts_with(b"%PDF-1.4\n%"));
        assert!(bytes.ends_with(b"%%EOF\n"));
        let xref = find(b"xref\n0 3\n0000000000 65535 f \n", 0);
        let trailer = String::from_utf8_lossy(&bytes[xref..]).into_owned();
        assert!(trailer.contains("/Size 3 /Root 1 0 R /Info 2 0 R"));
        let start_at = find(b"startxref\n", xref) + "startxref\n".len();
        let start: usize = String::from_utf8_lossy(&bytes[start_at..])
            .lines()
            .next()
            .and_then(|line| line.parse().ok())
            .expect("startxref offset");
        assert_eq!(start, xref);
        let first_entry = xref + "xref\n0 3\n0000000000 65535 f \n".len();
        let first_offset: usize = String::from_utf8_lossy(&bytes[first_entry..first_entry + 10])
            .parse()
            .expect("offset");
        assert!(bytes[first_offset..].starts_with(b"1 0 obj"));
    }
}
