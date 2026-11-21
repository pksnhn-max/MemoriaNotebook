//! Property tests (testing-manual §1) for notebook-extract.
//! Targets and shapes follow annex `props-extract-net.md` §1–§3;
//! the case floor is 1,000 for every generated property (brief W4).
//! Properties that turn out NOT to hold are moved, unmodified, to
//! `props_not_holding.rs` with their minimal counterexamples — the
//! implementation is never adjusted to make a property pass.

use proptest::prelude::*;

use super::{extract_epub, extract_txt, html_to_text, ExtractError};

// ---------- shared strategies ----------

fn arb_bytes(max: usize) -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(any::<u8>(), 0..max)
}

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

fn mixed_bytes() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        arb_bytes(2048),
        bom_variant_bytes(),
        any::<String>().prop_map(|s| s.into_bytes()),
    ]
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
    prop_oneof![
        valid,
        arb_bytes(1024),
        prop::collection::vec(any::<u8>(), 0..256)
            .prop_map(|b| build_zip(&[("dummy.txt".to_string(), b)])),
    ]
}

// ---------- §1 extract_txt ----------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    /// Annex 1.1 (不発): extract_txt never panics on any byte input.
    #[test]
    fn p1_1_extract_txt_never_panics(b in mixed_bytes()) {
        let _ = extract_txt(&b);
    }

}

// ---------- §2 html_to_text ----------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    /// Annex 2.1 （不発）: html_to_text never panics on any string.
    #[test]
    fn p2_1_html_never_panics(s in prop_oneof![html_string(), any::<String>()]) {
        let _ = html_to_text(&s);
    }

}

fn script_case() -> impl Strategy<Value = (String, bool)> {
    (
        prop::sample::select(vec!["script", "style"]),
        prop::sample::select(vec!["", " type=\"text/javascript\"", " data-x=\"a>b\""]),
        prop::sample::select(vec!["lower", "upper", "mixed"]),
        any::<bool>(),
    )
        .prop_map(|(tag, attrs, case, closed)| {
            let (open, close) = match case {
                "upper" => (tag.to_uppercase(), tag.to_uppercase()),
                "mixed" => (
                    tag.chars()
                        .enumerate()
                        .map(|(i, c)| if i % 2 == 0 { c.to_ascii_uppercase() } else { c })
                        .collect::<String>(),
                    tag.to_string(),
                ),
                _ => (tag.to_string(), tag.to_string()),
            };
            let body = "ZXQWSECRET body <b>inner</b> tail";
            let html = if closed {
                format!("PRETOK<{open}{attrs}>{body}</{close}>POSTTOK")
            } else {
                format!("PRETOK<{open}{attrs}>{body}")
            };
            (html, closed)
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    /// Annex 2.3: script/style bodies never reach the output; for a
    /// well-formed element the surrounding text survives.
    #[test]
    fn p2_3_script_style_bodies_removed((html, closed) in script_case()) {
        let out = html_to_text(&html);
        prop_assert!(!out.contains("ZXQWSECRET"), "body leaked: {out:?}");
        if closed {
            prop_assert!(out.contains("PRETOK"), "prefix lost: {out:?}");
            prop_assert!(out.contains("POSTTOK"), "suffix lost: {out:?}");
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    /// Annex 2.4, <br> form: a lone <br> between two texts is a
    /// single newline (no blank line).
    #[test]
    fn p2_4b_br_single_newline(x in "[a-z]{1,12}", y in "[a-z]{1,12}") {
        let html = format!("{x}<br>{y}");
        prop_assert_eq!(html_to_text(&html), format!("{x}\n{y}"));
    }
}

const NAMED_ENTITIES: &[(&str, char)] = &[
    ("amp", '&'),
    ("lt", '<'),
    ("gt", '>'),
    ("quot", '"'),
    ("apos", '\''),
    ("nbsp", ' '),
];

const UNKNOWN_ENTITIES: &[&str] = &[
    "foo", "copy", "hellip", "mdash", "reg", "trade", "alpha", "Omega",
];

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    /// Annex 2.5: the named entities of the implementation's table
    /// decode to their character.
    #[test]
    fn p2_5_named_entities_decode(
        (name, ch) in prop::sample::select(NAMED_ENTITIES),
    ) {
        let html = format!("X&{name};Y");
        prop_assert_eq!(html_to_text(&html), format!("X{ch}Y"));
    }

    /// Annex 2.5: numeric entities (decimal and hex) decode to the
    /// char for every Unicode scalar value.
    #[test]
    fn p2_5_numeric_entities_decode(c in any::<char>()) {
        let dec = format!("X&#{};Y", c as u32);
        prop_assert_eq!(html_to_text(&dec), format!("X{c}Y"));
        let hex = format!("X&#x{:X};Y", c as u32);
        prop_assert_eq!(html_to_text(&hex), format!("X{c}Y"));
    }

    /// Annex 2.5: numeric entities outside the scalar range
    /// (surrogates, > U+10FFFF) are passed through literally, as are
    /// unknown entity names.
    #[test]
    fn p2_5_invalid_entities_pass_through(
        v in prop_oneof![
            (0xD800u32..=0xDFFF).boxed(),
            (0x110000u32..=0xFFFFFF).boxed(),
        ],
        name in prop::sample::select(UNKNOWN_ENTITIES),
    ) {
        let numeric = format!("X&#x{v:X};Y");
        prop_assert_eq!(html_to_text(&numeric), numeric.clone());
        let named = format!("X&{name};Y");
        prop_assert_eq!(html_to_text(&named), named.clone());
    }
}

// ---------- §3 extract_epub ----------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    /// Annex 3.1 （不発）: extract_epub never panics on any bytes.
    #[test]
    fn p3_1_epub_never_panics(b in epub_like_bytes()) {
        let _ = extract_epub(&b);
    }

    /// Annex 3.2 （再抽出等価・テキスト層）: text obtained from a
    /// valid EPUB is a fixed point of extract_txt.
    #[test]
    fn p3_2_epub_text_is_txt_fixed_point(
        texts in prop::collection::vec("[a-zA-Z0-9 ]{0,60}", 1..4),
    ) {
        let chapters: Vec<(String, String)> = texts
            .iter()
            .enumerate()
            .map(|(i, t)| (format!("c{i}"), format!("<p>{t}</p>")))
            .collect();
        let spine: Vec<String> = chapters.iter().map(|(id, _)| id.clone()).collect();
        let bytes = build_epub(&chapters, &spine);
        if let Ok(t) = extract_epub(&bytes) {
            prop_assert_eq!(extract_txt(t.as_bytes()), t);
        }
    }

    /// Annex 3.3 (spine 順序保存）: chapter texts appear in the
    /// output in spine order, with multiplicity (a duplicated idref
    /// duplicates the text; an unknown idref contributes nothing).
    /// If no idref resolves, the result is an error.
    #[test]
    fn p3_3_spine_order_preserved(
        n in 1..4usize,
        spine_idx in prop::collection::vec(0..6usize, 0..8),
    ) {
        let chapters: Vec<(String, String)> = (0..n)
            .map(|i| (format!("c{i}"), format!("<p>CHTOKEN{i}</p>")))
            .collect();
        let spine: Vec<String> = spine_idx
            .iter()
            .map(|&i| {
                if i < n {
                    format!("c{i}")
                } else {
                    "missing".to_string()
                }
            })
            .collect();
        let bytes = build_epub(&chapters, &spine);
        let expected: Vec<String> = spine_idx
            .iter()
            .filter(|&&i| i < n)
            .map(|&i| format!("CHTOKEN{i}"))
            .collect();
        match extract_epub(&bytes) {
            Ok(t) => {
                let mut from = 0usize;
                for tok in &expected {
                    let rel = t[from..].find(tok.as_str());
                    prop_assert!(rel.is_some(), "token {tok} missing after {from} in {t:?}");
                    from += rel.unwrap() + tok.len();
                }
            }
            Err(_) => {
                prop_assert!(expected.is_empty(), "error but resolvable idrefs exist");
            }
        }
    }

    /// Annex 3.4 (ruby 除去）: rt/rp readings never reach the output;
    /// the base text does.
    #[test]
    fn p3_4_ruby_readings_dropped(
        suffix in "[a-z]{1,6}",
        with_rp in any::<bool>(),
    ) {
        let base = format!("BASE{suffix}");
        let reading = format!("READ{suffix}");
        let ruby = if with_rp {
            format!("<ruby>{base}<rp>(</rp><rt>{reading}</rt><rp>)</rp></ruby>")
        } else {
            format!("<ruby>{base}<rt>{reading}</rt></ruby>")
        };
        let chapters = vec![("c0".to_string(), format!("<p>pre{ruby}post</p>"))];
        let spine = vec!["c0".to_string()];
        let bytes = build_epub(&chapters, &spine);
        let t = extract_epub(&bytes).expect("valid epub");
        prop_assert!(t.contains(&base), "base lost: {t:?}");
        prop_assert!(!t.contains(&reading), "reading leaked: {t:?}");
    }

    /// Annex 3.5 （ブロック改行・結合）: chapters join with exactly
    /// "\n\n"; empty chapters are skipped; blocks inside a chapter
    /// are separated by one blank line as well (the collapse pass
    /// leaves a single empty line — see record book for how the
    /// annex's "単一 \n" was rendered).
    #[test]
    fn p3_5_chapter_and_block_joins(
        chapters_spec in prop::collection::vec(
            prop::collection::vec("[a-z]{1,10}", 0..3),
            0..4,
        ),
    ) {
        let chapters: Vec<(String, String)> = chapters_spec
            .iter()
            .enumerate()
            .map(|(i, toks)| {
                let body = toks
                    .iter()
                    .map(|t| format!("<p>{t}</p>"))
                    .collect::<Vec<_>>()
                    .join("");
                (format!("c{i}"), body)
            })
            .collect();
        let spine: Vec<String> = chapters.iter().map(|(id, _)| id.clone()).collect();
        let bytes = build_epub(&chapters, &spine);
        let chapter_texts: Vec<String> = chapters_spec
            .iter()
            .filter(|toks| !toks.is_empty())
            .map(|toks| toks.join("\n\n"))
            .collect();
        match extract_epub(&bytes) {
            Ok(t) => {
                prop_assert_eq!(t, chapter_texts.join("\n\n"));
            }
            Err(_) => {
                prop_assert!(chapter_texts.is_empty(), "error but chapters have text");
            }
        }
    }
}

/// Annex 3.6 （エラー型整合）: each structural breakage stage yields
/// its documented ExtractError variant (and payload for Missing).
fn error_stage() -> impl Strategy<Value = (u8, Vec<u8>)> {
    (0..6u8, arb_bytes(256))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    #[test]
    fn p3_6_error_variants_by_stage((stage, filler) in error_stage()) {
        let container_ok = "<?xml version=\"1.0\"?><container xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\"><rootfiles><rootfile full-path=\"OEBPS/content.opf\" media-type=\"application/oebps-package+xml\"/></rootfiles></container>";
        let result = match stage {
            0 => {
                let mut b = vec![0xDE, 0xAD, 0xBE, 0xEF];
                b.extend_from_slice(&filler);
                extract_epub(&b)
            }
            1 => {
                let bytes = build_zip(&[("dummy.txt".to_string(), filler)]);
                extract_epub(&bytes)
            }
            2 => {
                let container = "<?xml version=\"1.0\"?><container xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\"><rootfiles></rootfiles></container>";
                let bytes = build_zip(&[(
                    "META-INF/container.xml".to_string(),
                    container.as_bytes().to_vec(),
                )]);
                extract_epub(&bytes)
            }
            3 => {
                let bytes = build_zip(&[(
                    "META-INF/container.xml".to_string(),
                    container_ok.as_bytes().to_vec(),
                )]);
                extract_epub(&bytes)
            }
            4 => {
                let chapters: Vec<(String, String)> = Vec::new();
                let bytes = build_epub(&chapters, &[]);
                extract_epub(&bytes)
            }
            _ => {
                let chapters = vec![("c0".to_string(), "<p></p>".to_string())];
                let spine = vec!["c0".to_string()];
                let bytes = build_epub(&chapters, &spine);
                extract_epub(&bytes)
            }
        };
        match stage {
            0 => prop_assert!(
                matches!(result, Err(ExtractError::Zip(_))),
                "stage 0: {result:?}"
            ),
            1 => prop_assert!(
                matches!(result, Err(ExtractError::Missing("container.xml"))),
                "stage 1: {result:?}"
            ),
            2 => prop_assert!(
                matches!(result, Err(ExtractError::Missing("rootfile"))),
                "stage 2: {result:?}"
            ),
            3 => prop_assert!(
                matches!(result, Err(ExtractError::Missing("opf"))),
                "stage 3: {result:?}"
            ),
            _ => prop_assert!(
                matches!(result, Err(ExtractError::Missing("spine text"))),
                "stage {stage}: {result:?}"
            ),
        }
    }
}
