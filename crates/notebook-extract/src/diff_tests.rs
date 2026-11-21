//! Differential tests (testing-manual §2): the live implementation vs
//! the frozen oracle in `super::oracle`, fed the same generated inputs.
//! Floor: 1,000 cases per property (brief W4). The oracle is never
//! edited; a divergence here means the live side changed behaviour.

use proptest::prelude::*;

use super::oracle;
use super::{extract_epub, extract_txt, html_to_text, xhtml_to_text, ExtractError};

fn outcome(r: Result<String, ExtractError>) -> (bool, String) {
    match r {
        Ok(s) => (true, s),
        Err(e) => (false, format!("{e}")),
    }
}

/// Random raw bytes.
fn arb_bytes(max: usize) -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(any::<u8>(), 0..max)
}

/// Payload with 0..=3 UTF-8 BOM prefixes (BOM variants matter: the
/// live strip_prefix removes exactly one).
fn bom_variant_bytes() -> impl Strategy<Value = Vec<u8>> {
    (0..=3usize, arb_bytes(512)).prop_map(|(n, mut payload)| {
        let mut out = Vec::new();
        for _ in 0..n {
            out.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
        }
        out.append(&mut payload);
        out
    })
}

/// Bytes of a valid UTF-8 string (arbitrary Unicode, may contain
/// \r\n, control chars, U+FEFF in the middle, ...).
fn utf8_bytes() -> impl Strategy<Value = Vec<u8>> {
    any::<String>().prop_map(|s| s.into_bytes())
}

const SJIS_POOL: &str = "あいうえおかきくけこさしすせそたちつてとなにぬねのはひふへほ\
    アイウエオカキクケコサシスセソタチツテト0123456789 \
    漢字日本語盲獣天満川水位記録、。ーｱｲｳｴｵ";

/// Bytes of a Shift_JIS encoding of a string drawn from a
/// SJIS-representable pool.
fn sjis_bytes() -> impl Strategy<Value = Vec<u8>> {
    let pool: Vec<char> = SJIS_POOL.chars().collect();
    prop::collection::vec(prop::sample::select(pool), 0..200).prop_map(|chars| {
        let s: String = chars.into_iter().collect();
        encoding_rs::SHIFT_JIS.encode(&s).0.into_owned()
    })
}

const HTML_FRAGMENTS: &[&str] = &[
    "<p>", "</p>", "<div>", "</div>", "<br>", "<br/>", "<span>", "</span>",
    "<script>", "</script>", "<style>", "</style>", "<td>", "</td>",
    "<tr>", "</tr>", "<h1>", "</h1>", "<a href=\"x>y\">", "</a>",
    "<p title=\"a>b\">", "&amp;", "&lt;", "&gt;", "&quot;", "&apos;",
    "&nbsp;", "&#65;", "&#x1F600;", "&unknown;", "&#xD800;", "&", ";",
    "<", ">", "text", "あ", " ", "\n", "\r", "a\rb", "<!--", "-->",
    "<SCRIPT>", "</SCRIPT>", "<script src=\"a>b\">", "'quoted'", "\"dq\"",
];

fn html_string() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(HTML_FRAGMENTS), 0..40)
        .prop_map(|parts| parts.concat())
}

const XHTML_FRAGMENTS: &[&str] = &[
    "<p>", "</p>", "<div>", "</div>", "<br/>", "<br>", "<ruby>", "</ruby>",
    "<rt>", "</rt>", "<rp>", "</rp>", "<script>", "</script>",
    "<style>", "</style>", "<td>", "</td>", "<h2>", "</h2>",
    "text", "盲獣", "じゅう", " ", "\n", "&amp;", "&#12398;",
];

fn xhtml_bytes() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(prop::sample::select(XHTML_FRAGMENTS), 0..40)
        .prop_map(|parts| parts.concat().into_bytes())
}

/// Build a stored (uncompressed) zip from (name, data) entries.
fn build_zip(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
    use std::io::Write;
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut w = zip::write::ZipWriter::new(&mut buf);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, data) in entries {
            w.start_file(name.as_str(), opts).unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap();
    }
    buf.into_inner()
}

/// A structurally valid EPUB whose chapters carry the given body
/// markup, with the spine in the given idref order (ids may repeat or
/// point nowhere).
fn build_epub(chapters: &[(String, String)], spine: &[String]) -> Vec<u8> {
    let mut items = String::new();
    for (id, _) in chapters {
        items.push_str(&format!(
            "<item id=\"{id}\" href=\"{id}.xhtml\" media-type=\"application/xhtml+xml\"/>"
        ));
    }
    let mut refs = String::new();
    for r in spine {
        refs.push_str(&format!("<itemref idref=\"{r}\"/>"));
    }
    let container = "<?xml version=\"1.0\"?><container xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\"><rootfiles><rootfile full-path=\"OEBPS/content.opf\" media-type=\"application/oebps-package+xml\"/></rootfiles></container>";
    let opf = format!(
        "<?xml version=\"1.0\"?><package xmlns=\"http://www.idpf.org/2007/opf\"><manifest>{items}</manifest><spine>{refs}</spine></package>"
    );
    let mut entries: Vec<(String, Vec<u8>)> = vec![
        ("META-INF/container.xml".to_string(), container.as_bytes().to_vec()),
        ("OEBPS/content.opf".to_string(), opf.into_bytes()),
    ];
    for (id, body) in chapters {
        let xhtml = format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><body>{body}</body></html>"
        );
        entries.push((format!("OEBPS/{id}.xhtml"), xhtml.into_bytes()));
    }
    build_zip(&entries)
}

/// EPUB-shaped inputs: valid generated EPUBs, byte-level mutations of
/// them, zips without the EPUB scaffolding, and plain random bytes.
fn epub_like_bytes() -> impl Strategy<Value = Vec<u8>> {
    let valid = prop::collection::vec(
        "[a-zA-Z0-9 ]{0,60}".prop_map(|t| format!("<p>{t}</p>")),
        0..4,
    )
    .prop_map(|bodies| {
        let chapters: Vec<(String, String)> = bodies
            .into_iter()
            .enumerate()
            .map(|(i, b)| (format!("c{i}"), b))
            .collect();
        let spine: Vec<String> = chapters.iter().map(|(id, _)| id.clone()).collect();
        build_epub(&chapters, &spine)
    });
    let mutated = valid.clone().prop_flat_map(|bytes| {
        (Just(bytes), prop::collection::vec(0..64usize, 0..8), any::<u8>())
            .prop_map(|(mut b, positions, val)| {
                for p in positions {
                    if p < b.len() {
                        b[p] = val;
                    }
                }
                b
            })
    });
    let truncated = valid.clone().prop_flat_map(|bytes| {
        (0..=bytes.len()).prop_map(move |n| bytes[..n].to_vec())
    });
    prop_oneof![
        valid,
        mutated,
        truncated,
        arb_bytes(1024),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    #[test]
    fn diff_extract_txt_random_bytes(b in arb_bytes(2048)) {
        prop_assert_eq!(extract_txt(&b), oracle::extract_txt(&b));
    }

    #[test]
    fn diff_extract_txt_bom_variants(b in bom_variant_bytes()) {
        prop_assert_eq!(extract_txt(&b), oracle::extract_txt(&b));
    }

    #[test]
    fn diff_extract_txt_utf8(b in utf8_bytes()) {
        prop_assert_eq!(extract_txt(&b), oracle::extract_txt(&b));
    }

    #[test]
    fn diff_extract_txt_sjis(b in sjis_bytes()) {
        prop_assert_eq!(extract_txt(&b), oracle::extract_txt(&b));
    }

    #[test]
    fn diff_html_to_text_fragments(s in html_string()) {
        prop_assert_eq!(html_to_text(&s), oracle::html_to_text(&s));
    }

    #[test]
    fn diff_html_to_text_arbitrary(s in any::<String>()) {
        prop_assert_eq!(html_to_text(&s), oracle::html_to_text(&s));
    }

    #[test]
    fn diff_xhtml_to_text_fragments(b in xhtml_bytes()) {
        prop_assert_eq!(xhtml_to_text(&b), oracle::xhtml_to_text(&b));
    }

    #[test]
    fn diff_xhtml_to_text_random_bytes(b in arb_bytes(1024)) {
        prop_assert_eq!(xhtml_to_text(&b), oracle::xhtml_to_text(&b));
    }

    #[test]
    fn diff_extract_epub(b in epub_like_bytes()) {
        prop_assert_eq!(
            outcome(extract_epub(&b)),
            outcome(oracle::extract_epub(&b))
        );
    }
}
