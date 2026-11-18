//! File import: pull plain text out of the file formats the app accepts
//! as sources (TXT / Markdown / EPUB). Pure bytes in, text out — the
//! caller (core) owns chunking and indexing.

use std::collections::HashMap;
use std::io::{Cursor, Read};

#[derive(Debug)]
pub enum ExtractError {
    Zip(zip::result::ZipError),
    Xml(quick_xml::Error),
    Missing(&'static str),
}

impl std::fmt::Display for ExtractError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExtractError::Zip(e) => write!(f, "zip: {e}"),
            ExtractError::Xml(e) => write!(f, "xml: {e}"),
            ExtractError::Missing(what) => write!(f, "missing {what}"),
        }
    }
}
impl std::error::Error for ExtractError {}
impl From<zip::result::ZipError> for ExtractError {
    fn from(e: zip::result::ZipError) -> Self {
        ExtractError::Zip(e)
    }
}
impl From<quick_xml::Error> for ExtractError {
    fn from(e: quick_xml::Error) -> Self {
        ExtractError::Xml(e)
    }
}

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
            // Find the end of the tag.
            let Some(end) = find_byte(b, i, b'>') else { break };
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
            let decoded = match ent {
                "amp" => Some("&"),
                "lt" => Some("<"),
                "gt" => Some(">"),
                "quot" => Some("\""),
                "apos" | "#39" => Some("'"),
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
];

fn xhtml_to_text(xml: &[u8]) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn utf8_txt_passthrough() {
        assert_eq!(extract_txt("天満川の水位".as_bytes()), "天満川の水位");
    }

    #[test]
    fn shift_jis_txt_decoded() {
        let (bytes, _, _) = encoding_rs::SHIFT_JIS.encode("盲獣は地底に潜む");
        assert_eq!(extract_txt(&bytes), "盲獣は地底に潜む");
    }

    fn minimal_epub() -> Vec<u8> {
        let mut buf = Cursor::new(Vec::new());
        {
            let mut w = zip::write::ZipWriter::new(&mut buf);
            let stored = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            let deflated = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            w.start_file("mimetype", stored).unwrap();
            w.write_all(b"application/epub+zip").unwrap();
            w.start_file("META-INF/container.xml", deflated).unwrap();
            w.write_all(
                br#"<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#,
            )
            .unwrap();
            w.start_file("OEBPS/content.opf", deflated).unwrap();
            w.write_all(
                br#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf"><manifest><item id="c1" href="c1.xhtml" media-type="application/xhtml+xml"/><item id="c2" href="c2.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="c1"/><itemref idref="c2"/></spine></package>"#,
            )
            .unwrap();
            w.start_file("OEBPS/c1.xhtml", deflated).unwrap();
            w.write_all(
                "<?xml version=\"1.0\" encoding=\"utf-8\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p>第一章。盲<ruby>獣<rt>じゅう</rt></ruby>は潜む。</p></body></html>".as_bytes(),
            )
            .unwrap();
            w.start_file("OEBPS/c2.xhtml", deflated).unwrap();
            w.write_all(
                "<?xml version=\"1.0\" encoding=\"utf-8\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p>第二章。天満川の水位。</p></body></html>".as_bytes(),
            )
            .unwrap();
            w.finish().unwrap();
        }
        buf.into_inner()
    }

    #[test]
    fn epub_spine_order_and_ruby_dropped() {
        let text = extract_epub(&minimal_epub()).unwrap();
        assert!(text.contains("第一章。盲獣は潜む。"), "{text}");
        assert!(!text.contains("じゅう"), "ruby leaked: {text}");
        assert!(text.find("第一章").unwrap() < text.find("第二章").unwrap());
    }

    /// Real-file check, runs only when MOJU_EPUB / MOJU_TXT are set.
    #[test]
    fn real_moju_when_env_set() {
        if let Ok(path) = std::env::var("MOJU_EPUB") {
            let bytes = std::fs::read(path).unwrap();
            let text = extract_epub(&bytes).unwrap();
            let n = text.chars().count();
            assert!(n > 50_000, "epub chars {n}");
            if let Ok(txt_path) = std::env::var("MOJU_TXT") {
                let t = extract_txt(&std::fs::read(txt_path).unwrap());
                // Same work, different editions: order-of-magnitude match.
                let m = t.chars().count();
                assert!(m > 50_000, "txt chars {m}");
            }
        }
    }
}
