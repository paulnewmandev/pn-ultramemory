// SPDX-License-Identifier: Apache-2.0
//! A structural validator for the PDF files the crate writes, independent of the writer.
//!
//! It follows the file the way a reader does: header, `startxref`, the cross-reference table
//! (every entry 20 bytes, every offset pointing at `N 0 obj`), the trailer, every object up to
//! its `endobj` with stream lengths verified, the page tree (the `/Count` equals the number of
//! `/Page` objects and of `/Kids`), each page's resources (every font it uses exists), the
//! content streams (balanced text objects, valid numbers, decodable strings), the outline and the
//! document information. It returns the text of each page so tests can look for sentences.

use std::collections::BTreeMap;

/// What the validator learned about a PDF.
#[derive(Debug, Default)]
pub(crate) struct Pdf {
    /// The text of each page: every string shown, joined by newlines.
    pub(crate) pages: Vec<String>,
    /// The decoded titles of the outline items, in order.
    pub(crate) outline: Vec<String>,
    /// The decoded document title.
    pub(crate) title: String,
    /// The creation date string, if any.
    pub(crate) creation_date: Option<String>,
    /// The dictionary of the document information object.
    pub(crate) info: String,
    /// The catalog dictionary.
    pub(crate) catalog: String,
    /// The size of the file in bytes.
    pub(crate) size: usize,
}

/// Returns the position of `needle` in `haystack` at or after `from`.
fn find(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    (from..=haystack.len() - needle.len()).find(|&at| haystack[at..].starts_with(needle))
}

/// Returns the last position of `needle` in `haystack`.
fn rfind(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    (0..=haystack.len().checked_sub(needle.len())?)
        .rev()
        .find(|&at| haystack[at..].starts_with(needle))
}

/// Reads the integer that follows `key` in a dictionary text, like `/Count 5`.
fn number_after(dict: &str, key: &str) -> Option<usize> {
    let at = dict.find(key)? + key.len();
    let digits: String = dict[at..]
        .trim_start()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

/// Reads the object number of `key N 0 R` in a dictionary text.
fn reference_after(dict: &str, key: &str) -> Option<usize> {
    let at = dict.find(key)? + key.len();
    let mut parts = dict[at..].split_whitespace();
    let number = parts.next()?.parse().ok()?;
    (parts.next()? == "0" && parts.next()?.starts_with('R')).then_some(number)
}

/// Reads every `N 0 R` reference in `text`, in order.
fn references(text: &str) -> Vec<usize> {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let mut found = Vec::new();
    for window in tokens.windows(3) {
        if window[1] == "0" && window[2].trim_end_matches([']', '>']) == "R" {
            if let Ok(number) = window[0].trim_start_matches('[').parse() {
                found.push(number);
            }
        }
    }
    found
}

/// Decodes a `WinAnsiEncoding` byte to a character, independently of the crate under test.
fn winansi(byte: u8) -> char {
    const HIGH: [char; 32] = [
        '\u{20ac}', '?', '\u{201a}', '\u{192}', '\u{201e}', '\u{2026}', '\u{2020}', '\u{2021}',
        '\u{2c6}', '\u{2030}', '\u{160}', '\u{2039}', '\u{152}', '?', '\u{17d}', '?', '?',
        '\u{2018}', '\u{2019}', '\u{201c}', '\u{201d}', '\u{2022}', '\u{2013}', '\u{2014}',
        '\u{2dc}', '\u{2122}', '\u{161}', '\u{203a}', '\u{153}', '?', '\u{17e}', '\u{178}',
    ];
    match byte {
        0x80..=0x9f => HIGH[usize::from(byte) - 0x80],
        _ => char::from(byte),
    }
}

/// Decodes a hexadecimal UTF-16BE text string such as `<FEFF0041>`.
fn utf16_hex(text: &str) -> Result<String, String> {
    let hex = text
        .trim()
        .strip_prefix("<FEFF")
        .and_then(|rest| rest.strip_suffix('>'))
        .ok_or("not a UTF-16BE hex string")?;
    if hex.len() % 4 != 0 {
        return Err("odd UTF-16 length".into());
    }
    let units: Result<Vec<u16>, _> = (0..hex.len())
        .step_by(4)
        .map(|at| u16::from_str_radix(&hex[at..at + 4], 16))
        .collect();
    String::from_utf16(&units.map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

/// Extracts the strings shown by a content stream and checks its syntax.
fn read_content(stream: &[u8], fonts: &[String]) -> Result<String, String> {
    let mut text = String::new();
    let mut at = 0usize;
    let mut in_text = false;
    let mut line_tokens: Vec<String> = Vec::new();
    while at < stream.len() {
        match stream[at] {
            b'(' => {
                let mut bytes = Vec::new();
                at += 1;
                loop {
                    match stream.get(at).copied() {
                        None => return Err("unterminated string".into()),
                        Some(b')') => break,
                        Some(b'\\') => {
                            at += 1;
                            match stream.get(at).copied() {
                                Some(c @ (b'(' | b')' | b'\\')) => bytes.push(c),
                                Some(d) if d.is_ascii_digit() => {
                                    let octal = std::str::from_utf8(
                                        &stream[at..(at + 3).min(stream.len())],
                                    )
                                    .map_err(|e| e.to_string())?;
                                    bytes.push(
                                        u8::from_str_radix(octal, 8)
                                            .map_err(|e| format!("{octal}: {e}"))?,
                                    );
                                    at += 2;
                                }
                                other => return Err(format!("bad escape {other:?}")),
                            }
                        }
                        Some(c) => bytes.push(c),
                    }
                    at += 1;
                }
                if !in_text {
                    return Err("string outside BT/ET".into());
                }
                text.extend(bytes.iter().map(|&b| winansi(b)));
                text.push('\n');
                at += 1;
            }
            b'\n' => {
                line_tokens.clear();
                at += 1;
            }
            b' ' => at += 1,
            _ => {
                let end = stream[at..]
                    .iter()
                    .position(|b| matches!(b, b' ' | b'\n' | b'('))
                    .map_or(stream.len(), |p| p + at);
                let token = std::str::from_utf8(&stream[at..end])
                    .map_err(|e| e.to_string())?
                    .to_owned();
                at = end;
                if token == "BT" {
                    if in_text {
                        return Err("nested BT".into());
                    }
                    in_text = true;
                } else if token == "ET" {
                    if !in_text {
                        return Err("ET without BT".into());
                    }
                    in_text = false;
                } else if token == "Tf" {
                    let font = line_tokens
                        .iter()
                        .rev()
                        .nth(1)
                        .ok_or("Tf without operands")?;
                    let name = font.strip_prefix('/').ok_or("Tf font is not a name")?;
                    if !fonts.iter().any(|known| known == name) {
                        return Err(format!("font {name} used but not in the page resources"));
                    }
                } else if !token.starts_with('/') {
                    let core = token.trim_matches(['[', ']']);
                    let is_number = core.parse::<f64>().is_ok()
                        && core
                            .chars()
                            .all(|c| c.is_ascii_digit() || matches!(c, '.' | '-'));
                    let is_operator = core.chars().all(|c| c.is_ascii_alphabetic() || c == '*');
                    if !(core.is_empty() || is_number || is_operator) {
                        return Err(format!("invalid token {token:?}"));
                    }
                }
                line_tokens.push(token);
            }
        }
    }
    if in_text {
        return Err("unterminated BT".into());
    }
    Ok(text)
}

/// Validates the PDF and returns what it found.
#[allow(clippy::too_many_lines)] // reason: one linear walk over the file reads best as one function
pub(crate) fn check_pdf(bytes: &[u8]) -> Result<Pdf, String> {
    if !bytes.starts_with(b"%PDF-1.4\n%") {
        return Err("bad header".into());
    }
    if !bytes.ends_with(b"\n%%EOF\n") {
        return Err("bad trailer end".into());
    }
    let start_key = rfind(bytes, b"startxref\n").ok_or("no startxref")?;
    let number_start = start_key + "startxref\n".len();
    let number_end = find(bytes, b"\n", number_start).ok_or("startxref without offset")?;
    let xref_at: usize = std::str::from_utf8(&bytes[number_start..number_end])
        .map_err(|e| e.to_string())?
        .parse()
        .map_err(|_| "bad startxref offset")?;
    if !bytes[xref_at..].starts_with(b"xref\n") {
        return Err("startxref does not point at xref".into());
    }
    let header_end = find(bytes, b"\n", xref_at + 5).ok_or("no xref header")?;
    let header = std::str::from_utf8(&bytes[xref_at + 5..header_end]).map_err(|e| e.to_string())?;
    let (first, count) = header.split_once(' ').ok_or("bad xref header")?;
    if first != "0" {
        return Err("xref does not start at 0".into());
    }
    let count: usize = count.parse().map_err(|_| "bad xref count")?;

    let mut offsets = Vec::new();
    let mut pos = header_end + 1;
    for index in 0..count {
        let entry = bytes.get(pos..pos + 20).ok_or("xref truncated")?;
        let text = std::str::from_utf8(entry).map_err(|_| "xref entry not ASCII")?;
        let well_formed = text.as_bytes()[..10].iter().all(u8::is_ascii_digit)
            && text.as_bytes()[10] == b' '
            && text.as_bytes()[11..16].iter().all(u8::is_ascii_digit)
            && text.as_bytes()[16] == b' '
            && matches!(text.as_bytes()[17], b'n' | b'f')
            && &text[18..] == " \n";
        if !well_formed {
            return Err(format!("xref entry {index} malformed: {text:?}"));
        }
        if index == 0 {
            if text != "0000000000 65535 f \n" {
                return Err("first xref entry must be the free head".into());
            }
        } else {
            if text.as_bytes()[17] != b'n' {
                return Err(format!("xref entry {index} is free"));
            }
            offsets.push(text[..10].parse::<usize>().map_err(|e| e.to_string())?);
        }
        pos += 20;
    }
    let trailer_start = pos;
    if !bytes[trailer_start..].starts_with(b"trailer\n<< ") {
        return Err("trailer missing right after xref".into());
    }
    let trailer = String::from_utf8_lossy(&bytes[trailer_start..start_key]).into_owned();
    if number_after(&trailer, "/Size") != Some(count) {
        return Err("trailer /Size disagrees with xref".into());
    }
    let root = reference_after(&trailer, "/Root").ok_or("no /Root")?;
    let info_id = reference_after(&trailer, "/Info").ok_or("no /Info")?;
    if !trailer.contains("/ID [<") {
        return Err("no /ID".into());
    }

    // Objects.
    let mut bodies: BTreeMap<usize, Vec<u8>> = BTreeMap::new();
    let mut streams: BTreeMap<usize, Vec<u8>> = BTreeMap::new();
    let mut expected_next = find(bytes, b"\n", 9).ok_or("no binary comment")? + 1;
    for (index, &offset) in offsets.iter().enumerate() {
        let number = index + 1;
        if offset != expected_next {
            return Err(format!(
                "object {number} at {offset}, expected {expected_next}"
            ));
        }
        let prefix = format!("{number} 0 obj\n");
        if !bytes[offset..].starts_with(prefix.as_bytes()) {
            return Err(format!(
                "xref offset of object {number} does not point at its header"
            ));
        }
        let body_start = offset + prefix.len();
        let dict_end = find(bytes, b"\n", body_start).unwrap_or(bytes.len());
        let first_line = String::from_utf8_lossy(&bytes[body_start..dict_end]).into_owned();
        let end = if first_line.contains("/Length")
            && bytes[dict_end + 1..].starts_with(b"stream\n")
        {
            let length = number_after(&first_line, "/Length").ok_or("stream without /Length")?;
            let data_start = dict_end + 1 + "stream\n".len();
            let data = bytes
                .get(data_start..data_start + length)
                .ok_or("stream longer than the file")?;
            let tail = b"\nendstream\nendobj\n";
            if !bytes[data_start + length..].starts_with(tail) {
                return Err(format!(
                    "object {number}: /Length {length} does not match the stream"
                ));
            }
            streams.insert(number, data.to_vec());
            bodies.insert(number, first_line.into_bytes());
            data_start + length + tail.len()
        } else {
            let stop = find(bytes, b"\nendobj\n", body_start)
                .ok_or_else(|| format!("object {number} has no endobj"))?;
            bodies.insert(number, bytes[body_start..stop].to_vec());
            stop + "\nendobj\n".len()
        };
        expected_next = end;
    }
    if expected_next != xref_at {
        return Err("xref does not directly follow the last object".into());
    }
    let text = |number: usize| -> Result<String, String> {
        bodies
            .get(&number)
            .map(|b| String::from_utf8_lossy(b).into_owned())
            .ok_or_else(|| format!("missing object {number}"))
    };

    // Catalog and page tree.
    let catalog = text(root)?;
    if !catalog.contains("/Type /Catalog") {
        return Err("root is not a catalog".into());
    }
    let tree_id = reference_after(&catalog, "/Pages").ok_or("catalog without /Pages")?;
    let tree = text(tree_id)?;
    let declared = number_after(&tree, "/Count").ok_or("no /Count")?;
    let kids = references(
        tree.split("/Kids")
            .nth(1)
            .ok_or("no /Kids")?
            .split(']')
            .next()
            .unwrap_or(""),
    );
    let page_objects: Vec<usize> = bodies
        .iter()
        .filter(|(_, b)| String::from_utf8_lossy(b).contains("/Type /Page "))
        .map(|(n, _)| *n)
        .collect();
    if declared != kids.len() || declared != page_objects.len() {
        return Err(format!(
            "/Count {declared}, kids {}, page objects {}",
            kids.len(),
            page_objects.len()
        ));
    }
    if kids != page_objects {
        return Err("kids are not the page objects in order".into());
    }
    let mut pages = Vec::new();
    for &page_id in &kids {
        let page = text(page_id)?;
        if reference_after(&page, "/Parent") != Some(tree_id) {
            return Err(format!("page {page_id} has the wrong parent"));
        }
        if !page.contains("/MediaBox [0 0 595.28 841.89]") {
            return Err(format!("page {page_id} is not A4"));
        }
        let content_id = reference_after(&page, "/Contents").ok_or("page without contents")?;
        let content = streams
            .get(&content_id)
            .ok_or("page contents is not a stream")?;
        let font_dict = page
            .split("/Font <<")
            .nth(1)
            .ok_or("page without fonts")?
            .split(">>")
            .next()
            .unwrap_or("");
        let mut font_names = Vec::new();
        for token_pair in font_dict.split('/').skip(1) {
            let name = token_pair
                .split_whitespace()
                .next()
                .ok_or("empty font name")?
                .to_owned();
            let font_id = reference_after(&format!("/{token_pair}"), &format!("/{name}"))
                .ok_or("font without reference")?;
            let font = text(font_id)?;
            for required in [
                "/Type /Font",
                "/Subtype /Type1",
                "/Encoding /WinAnsiEncoding",
                "/BaseFont /Helvetica",
            ] {
                if !font.contains(required) {
                    return Err(format!("font {name} lacks {required}"));
                }
            }
            font_names.push(name);
        }
        pages.push(read_content(content, &font_names)?);
    }

    // Outline.
    let mut outline = Vec::new();
    if let Some(outline_id) = reference_after(&catalog, "/Outlines") {
        let root_dict = text(outline_id)?;
        let declared_items = number_after(&root_dict, "/Count").ok_or("outline without /Count")?;
        let mut next = reference_after(&root_dict, "/First");
        let mut last = None;
        while let Some(item_id) = next {
            let item = text(item_id)?;
            let title = item
                .split("/Title ")
                .nth(1)
                .ok_or("outline item without title")?
                .split('>')
                .next()
                .unwrap_or("")
                .to_owned()
                + ">";
            outline.push(utf16_hex(&title)?);
            let destination = item
                .split("/Dest [")
                .nth(1)
                .ok_or("outline item without destination")?;
            let target = references(destination)
                .first()
                .copied()
                .ok_or("destination without page")?;
            if !kids.contains(&target) {
                return Err("outline destination is not a page".into());
            }
            last = Some(item_id);
            next = reference_after(&item, "/Next");
        }
        if outline.len() != declared_items || reference_after(&root_dict, "/Last") != last {
            return Err("outline count or last item mismatch".into());
        }
    }

    // Document information.
    let info = text(info_id)?;
    if !info.contains("/Author (pn-ultramemory)") {
        return Err("missing author".into());
    }
    let title = info
        .split("/Title ")
        .nth(1)
        .map(|t| t.split('>').next().unwrap_or("").to_owned() + ">")
        .ok_or("missing title")?;
    let creation_date = info
        .split("/CreationDate (")
        .nth(1)
        .and_then(|d| d.split(')').next())
        .map(str::to_owned);
    Ok(Pdf {
        pages,
        outline,
        title: utf16_hex(&title)?,
        creation_date,
        info,
        catalog,
        size: bytes.len(),
    })
}
