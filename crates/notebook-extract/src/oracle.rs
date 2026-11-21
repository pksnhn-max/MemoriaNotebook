//! ORACLE (differential testing, testing-manual §2): verbatim copies of
//! the current implementations in `lib.rs`, frozen 2026-11-21 from the
//! tree at repo HEAD 3118d042. NEVER fix or "improve" this module —
//! bugs included. If live and oracle diverge, suspect the live side.
//!
//! The only deviations from the copied text are:
//! - `xhtml_to_text` widened to `pub` so differential tests can call
//!   it (body unchanged).
//! - `ExtractError` is the crate's own type (a data definition, shared
//!   so `Result` values compare directly); all logic is copied.

#![allow(dead_code)]

use std::collections::HashMap;
use std::io::{Cursor, Read};

use crate::ExtractError;

/// Text files: UTF-8 (BOM tolerated) first, Shift_JIS as the fallback
/// (Aozora Bunko's distribution encoding). Markdown goes through the
/// same path — markers survive as text, which is fine for chunking.
pub fn extract_txt(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(s) => normalize_newlines(s),
        Err(_) => {
            let (cow, _, _) = encoding_rs::SHIFT_JIS.decode(bytes);
            normalize_newlines(&cow)
        }
    }
}

/// Lenient HTML -> text for web pages (NOT well-formed XML): a byte
/// scanner that drops script/style bodies, turns block tags into line
/// breaks, strips the rest, and decodes the common entities.
pub fn html_to_text(html: &str) -> String {
    let b = html.as_bytes();
    let mut out = String::with_capacity(html.len() / 3);
    let mut i = 0usize;
    let lower = html.to_ascii_lowercase();
    let lb = lower.as_bytes();
    while i < b.len() {
        if b[i] == b'<' {
            // Find the end of the tag, skipping '>' inside quoted
            // attribute values.
            let mut end = None;
            let mut quote = 0u8;
            let mut j = i + 1;
            while j < b.len() {
                let c = b[j];
                if quote != 0 {
                    if c == quote {
                        quote = 0;
                    }
                } else if c == b'"' || c == b'\'' {
                    quote = c;
                } else if c == b'>' {
                    end = Some(j);
                    break;
                }
                j += 1;
            }
            let Some(end) = end else { break };
            let tag = &lower[i + 1..end];
            let name: String = tag
                .trim_start_matches('/')
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric())
                .collect();
            if (name == "script" || name == "style") && !tag.starts_with('/') {
                // Skip to the closing tag.
                let close = format!("</{name}");
                match find_sub(lb, end, close.as_bytes()) {
                    Some(p) => {
                        i = match find_byte(b, p, b'>') {
                            Some(e2) => e2 + 1,
                            None => break,
                        };
                    }
                    None => break,
                }
                continue;
            }
            if matches!(
                name.as_str(),
                "p" | "div" | "br" | "li" | "tr" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
                    | "section" | "article" | "blockquote" | "header" | "footer"
            ) {
                out.push('\n');
            }
            i = end + 1;
        } else if b[i] == b'&' {
            let Some(end) = find_byte(b, i, b';') else {
                out.push('&');
                i += 1;
                continue;
            };
            let ent = &html[i + 1..end];
            // Numeric entities (&#123; / &#x1F600;) decode to their char.
            let numeric = if let Some(num) = ent.strip_prefix('#') {
                let v = if let Some(hex) = num.strip_prefix('x').or(num.strip_prefix('X')) {
                    u32::from_str_radix(hex, 16).ok()
                } else {
                    num.parse::<u32>().ok()
                };
                v.and_then(char::from_u32)
            } else {
                None
            };
            if let Some(c) = numeric {
                out.push(c);
                i = end + 1;
                continue;
            }
            let decoded = match ent {
                "amp" => Some("&"),
                "lt" => Some("<"),
                "gt" => Some(">"),
                "quot" => Some("\""),
                "apos" => Some("'"),
                "nbsp" => Some(" "),
                _ => None,
            };
            match decoded {
                Some(d) => {
                    out.push_str(d);
                    i = end + 1;
                }
                None => {
                    out.push('&');
                    i += 1;
                }
            }
        } else {
            // Copy one UTF-8 char.
            let ch_len = utf8_len(b[i]);
            let upto = (i + ch_len).min(b.len());
            if let Ok(s) = std::str::from_utf8(&b[i..upto]) {
                out.push_str(s);
            }
            i = upto;
        }
    }
    let mut lines: Vec<&str> = Vec::new();
    for line in out.lines() {
        let line = line.trim();
        if line.is_empty() {
            if lines.last().is_some_and(|l| !l.is_empty()) {
                lines.push("");
            }
        } else {
            lines.push(line);
        }
    }
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}

fn find_byte(b: &[u8], from: usize, byte: u8) -> Option<usize> {
    b[from..].iter().position(|&x| x == byte).map(|p| from + p)
}

fn find_sub(b: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || b.len() < needle.len() {
        return None;
    }
    (from..=b.len() - needle.len()).find(|&i| &b[i..i + needle.len()] == needle)
}

fn utf8_len(first: u8) -> usize {
    if first < 0x80 {
        1
    } else if first >> 5 == 0b110 {
        2
    } else if first >> 4 == 0b1110 {
        3
    } else if first >> 3 == 0b11110 {
        4
    } else {
        1
    }
}

fn normalize_newlines(s: &str) -> String {
    s.replace("\r\n", "\n").replace('\r', "\n")
}

/// EPUB: container.xml -> OPF -> spine order -> XHTML text. Ruby
/// annotations (rt/rp) are dropped; block elements become line breaks.
pub fn extract_epub(bytes: &[u8]) -> Result<String, ExtractError> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes))?;
    let container = read_entry(&mut zip, "META-INF/container.xml")?
        .ok_or(ExtractError::Missing("container.xml"))?;
    let opf_path = find_rootfile(&container)?;
    let opf = read_entry(&mut zip, &opf_path)?.ok_or(ExtractError::Missing("opf"))?;
    let (manifest, spine) = parse_opf(&opf)?;
    let base = match opf_path.rfind('/') {
        Some(i) => opf_path[..=i].to_string(),
        None => String::new(),
    };
    let mut out = String::new();
    for idref in &spine {
        let Some(href) = manifest.get(idref) else {
            continue;
        };
        let path = join_path(&base, href);
        if let Some(xhtml) = read_entry(&mut zip, &path)? {
            let text = xhtml_to_text(&xhtml);
            let text = text.trim();
            if !text.is_empty() {
                if !out.is_empty() {
                    out.push_str("\n\n");
                }
                out.push_str(text);
            }
        }
    }
    if out.is_empty() {
        return Err(ExtractError::Missing("spine text"));
    }
    Ok(out)
}

fn read_entry(
    zip: &mut zip::ZipArchive<Cursor<&[u8]>>,
    name: &str,
) -> Result<Option<Vec<u8>>, ExtractError> {
    match zip.by_name(name) {
        Ok(mut f) => {
            let mut buf = Vec::new();
            f.read_to_end(&mut buf)
                .map_err(|e| ExtractError::Zip(zip::result::ZipError::Io(e)))?;
            Ok(Some(buf))
        }
        Err(zip::result::ZipError::FileNotFound) => Ok(None),
        Err(e) => Err(ExtractError::Zip(e)),
    }
}

fn attr(e: &quick_xml::events::BytesStart<'_>, key: &[u8]) -> Option<String> {
    for a in e.attributes().flatten() {
        if a.key.as_ref() == key {
            return Some(String::from_utf8_lossy(&a.value).into_owned());
        }
    }
    None
}

fn local_name(qname: &[u8]) -> &[u8] {
    match qname.iter().rposition(|&b| b == b':') {
        Some(i) => &qname[i + 1..],
        None => qname,
    }
}

fn find_rootfile(xml: &[u8]) -> Result<String, ExtractError> {
    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_reader(xml);
    loop {
        match reader.read_event()? {
            Event::Start(e) | Event::Empty(e) => {
                if local_name(e.name().as_ref()) == b"rootfile" {
                    if let Some(p) = attr(&e, b"full-path") {
                        return Ok(p);
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Err(ExtractError::Missing("rootfile"))
}

fn parse_opf(xml: &[u8]) -> Result<(HashMap<String, String>, Vec<String>), ExtractError> {
    use quick_xml::events::Event;
    let mut manifest = HashMap::new();
    let mut spine = Vec::new();
    let mut reader = quick_xml::Reader::from_reader(xml);
    loop {
        match reader.read_event()? {
            Event::Start(e) | Event::Empty(e) => match local_name(e.name().as_ref()) {
                b"item" => {
                    if let (Some(id), Some(href)) = (attr(&e, b"id"), attr(&e, b"href")) {
                        manifest.insert(id, href);
                    }
                }
                b"itemref" => {
                    if let Some(idref) = attr(&e, b"idref") {
                        spine.push(idref);
                    }
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    Ok((manifest, spine))
}

fn join_path(base: &str, href: &str) -> String {
    // hrefs are URL-escaped relative paths; EPUBs in the wild keep them
    // simple. Percent-decode the common escapes only.
    let href = href.replace("%20", " ");
    if href.starts_with('/') {
        href.trim_start_matches('/').to_string()
    } else {
        format!("{base}{href}")
    }
}

const BLOCK_TAGS: &[&[u8]] = &[
    b"p",
    b"div",
    b"h1",
    b"h2",
    b"h3",
    b"h4",
    b"h5",
    b"h6",
    b"li",
    b"tr",
    b"section",
    b"article",
    b"blockquote",
    b"td",
    b"th",
];

pub fn xhtml_to_text(xml: &[u8]) -> String {
    use quick_xml::events::Event;
    let mut out = String::new();
    let mut skip_depth = 0usize; // inside rt/rp/script/style
    let mut reader = quick_xml::Reader::from_reader(xml);
    reader.config_mut().trim_text(false);
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_ascii_lowercase();
                if matches!(name.as_slice(), b"rt" | b"rp" | b"script" | b"style") {
                    skip_depth += 1;
                } else if skip_depth == 0 && BLOCK_TAGS.contains(&name.as_slice()) {
                    out.push('\n');
                }
            }
            Ok(Event::End(e)) => {
                let name = local_name(e.name().as_ref()).to_ascii_lowercase();
                if matches!(name.as_slice(), b"rt" | b"rp" | b"script" | b"style") {
                    skip_depth = skip_depth.saturating_sub(1);
                } else if skip_depth == 0 && BLOCK_TAGS.contains(&name.as_slice()) {
                    out.push('\n');
                }
            }
            Ok(Event::Empty(e)) => {
                let name = local_name(e.name().as_ref()).to_ascii_lowercase();
                if skip_depth == 0 && name.as_slice() == b"br" {
                    out.push('\n');
                }
            }
            Ok(Event::Text(t)) => {
                if skip_depth == 0 {
                    if let Ok(s) = t.decode() {
                        out.push_str(&s);
                    }
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    // Collapse blank runs; keep single newlines between blocks.
    let mut lines: Vec<&str> = Vec::new();
    for line in out.lines() {
        let line = line.trim();
        if line.is_empty() {
            if lines.last().is_some_and(|l| !l.is_empty()) {
                lines.push("");
            }
        } else {
            lines.push(line);
        }
    }
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}
