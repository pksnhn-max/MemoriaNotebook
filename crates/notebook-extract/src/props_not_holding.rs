//! Properties from the annex that currently do NOT hold against the
//! live implementation. Each is kept in its original, unweakened form
//! and marked #[ignore]; run with `cargo test -- --ignored` to see it
//! fail on the recorded minimal counterexample. The implementation is
//! NOT adjusted to make these pass (brief W4); each entry records the
//! minimal counterexample found by proptest (2026-11-21, 1,000 cases)
//! and its abolition condition （廃止条件）: when the behaviour — or
//! the annex formula — is corrected, un-ignore the test and move it
//! back to `prop_tests.rs`.

use proptest::prelude::*;

use super::{extract_txt, html_to_text};

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

fn normalize(s: &str) -> String {
    s.replace("\r\n", "\n").replace('\r', "\n")
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

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    /// Annex 1.2 （再抽出等価） — DOES NOT HOLD.
    /// Minimal counterexample: b = [239, 187, 191] × 3 (three BOMs):
    /// once = "\u{feff}\u{feff}", twice = "\u{feff}".
    /// Mechanism: strip_prefix removes exactly one BOM per call, and
    /// a leading U+FEFF in the output encodes to a BOM again, so each
    /// pass strips one more. Abolition: when extract_txt strips
    /// repeated BOMs (or the annex claim is withdrawn), un-ignore.
    #[test]
    #[ignore]
    fn p1_2_extract_txt_idempotent(b in mixed_bytes()) {
        let once = extract_txt(&b);
        let twice = extract_txt(once.as_bytes());
        prop_assert_eq!(twice, once);
    }

    /// Annex 1.3 (UTF-8 往復） — DOES NOT HOLD as formulated.
    /// Minimal counterexample: s = "", bom = true (input is a bare
    /// BOM): extract_txt = "", the annex formula yields "\u{feff}".
    /// Mechanism: the implementation strips the BOM (its doc comment
    /// says "BOM tolerated"); the annex formula
    /// normalize_newlines(from_utf8(b)) does not model the strip.
    /// Abolition: when the annex formula is corrected to include BOM
    /// stripping (or the implementation stops stripping), un-ignore.
    #[test]
    #[ignore]
    fn p1_3_extract_txt_utf8_formula(
        s in any::<String>(),
        bom in any::<bool>(),
    ) {
        let mut bytes = Vec::new();
        if bom {
            bytes.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
        }
        bytes.extend_from_slice(s.as_bytes());
        let decoded = String::from_utf8(bytes.clone()).unwrap();
        prop_assert_eq!(extract_txt(&bytes), normalize(&decoded));
    }

    /// Annex 1.5 （形式整合・改行） — DOES NOT HOLD.
    /// Minimal counterexample: b = [10] ("\n"): the output is "\n",
    /// which starts with a blank line.
    /// Mechanism: extract_txt performs newline normalization only —
    /// no blank-line collapsing and no trimming (that post-processing
    /// exists in the html/xhtml path, not the txt path).
    /// Abolition: when extract_txt gains blank-line collapsing /
    /// trimming (or the annex claim is withdrawn), un-ignore.
    #[test]
    #[ignore]
    fn p1_5_extract_txt_newline_form(b in mixed_bytes()) {
        let out = extract_txt(&b);
        prop_assert!(!out.contains('\r'), "CR in output: {out:?}");
        prop_assert!(!out.contains("\n\n"), "blank line in output: {out:?}");
        prop_assert!(!out.starts_with('\n'), "leading blank: {out:?}");
        prop_assert!(!out.ends_with('\n'), "trailing blank: {out:?}");
    }

    /// Annex 2.2 （再実行等価） — DOES NOT HOLD.
    /// Minimal counterexample: s = "&lt;": once = "<", twice = "".
    /// Mechanism: entity decoding emits a literal '<' into the
    /// output; a second pass parses it as a tag start and, finding
    /// no '>', drops the rest.
    /// Abolition: when html_to_text's output can no longer be
    /// re-parsed as markup (e.g. decoded '<' is escaped), un-ignore.
    #[test]
    #[ignore]
    fn p2_2_html_idempotent(s in html_string()) {
        let once = html_to_text(&s);
        let twice = html_to_text(&once);
        prop_assert_eq!(twice, once);
    }

    /// Annex 2.6 （形式整合・出力） — DOES NOT HOLD (the '\r' clause).
    /// Minimal counterexample: s = "a\rb": the output "a\rb" still
    /// contains '\r'.
    /// Mechanism: the post-processing splits on '\n' and strips only
    /// a line-final '\r'; a lone '\r' inside a line survives, and
    /// lines are trimmed at the edges only. (The other two clauses —
    /// no "\n\n\n", trimmed fringe — were not the failing part.)
    /// Abolition: when the post-processing also normalizes lone
    /// in-line '\r', un-ignore.
    #[test]
    #[ignore]
    fn p2_6_html_output_form(s in html_string()) {
        let out = html_to_text(&s);
        prop_assert!(!out.contains('\r'), "CR in output: {out:?}");
        prop_assert!(!out.contains("\n\n\n"), "double blank in output: {out:?}");
        prop_assert!(out.trim() == out.as_str(), "untrimmed fringe: {out:?}");
    }
}

const SJIS_POOL: &str = "あいうえおかきくけこさしすせそたちつてとなにぬねのはひふへほ\
    アイウエオカキクケコサシスセソタチツテト0123456789 \
    漢字日本語盲獣天満川水位記録、。ーｱｲｳｴｵ";

fn sjis_candidate_string() -> impl Strategy<Value = String> {
    let mut pool: Vec<char> = SJIS_POOL.chars().collect();
    // Annex 1.4 strategy deliberately includes characters Shift_JIS
    // cannot represent, plus known ambiguous mappings.
    pool.extend(['\u{1F600}', '\u{1F389}', '\u{1F680}', '\u{FF5E}', '\u{301C}', '\u{00A5}', '\\', '\n', '\r']);
    prop::collection::vec(prop::sample::select(pool), 0..120)
        .prop_map(|chars| chars.into_iter().collect::<String>())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    /// Annex 1.4 (Shift_JIS 往復） — DOES NOT HOLD as formulated.
    /// Minimal counterexample: s = "😀" (U+1F600): SHIFT_JIS.encode
    /// emits the numeric character reference "&#128512;", and
    /// extract_txt returns that literally: "&#128512;" ≠ "😀".
    /// Mechanism: encoding_rs's encoder replaces unrepresentable
    /// characters with NCRs (WHATWG encoding behaviour); the annex's
    /// "becomes U+FFFD" note describes the *decoder's* replacement
    /// of invalid bytes, not this encode-side path.
    /// Abolition: when the annex expectation is corrected for the
    /// NCR behaviour, un-ignore.
    #[test]
    #[ignore]
    fn p1_4_extract_txt_sjis_roundtrip(s in sjis_candidate_string()) {
        let (bytes, _, _) = encoding_rs::SHIFT_JIS.encode(&s);
        prop_assert_eq!(extract_txt(&bytes), normalize(&s));
    }
}

const BLOCK_TAGS_ANNEX: &[&str] = &[
    "p", "div", "h1", "h2", "h3", "h4", "h5", "h6", "li", "tr",
    "section", "article", "blockquote", "header", "footer", "td", "th",
];

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    /// Annex 2.4 （ブロック改行整合） — DOES NOT HOLD for td/th.
    /// Minimal counterexample: tags = ["td", "p"]: the live output
    /// is "TK0\nTK1", the annex expectation is "TK0\n\nTK1".
    /// Mechanism: td/th are not in html_to_text's block-tag list
    /// (they are block-level only in xhtml_to_text's BLOCK_TAGS), so
    /// a td boundary contributes no newline at all.
    /// Abolition: when td/th join html_to_text's block list (or the
    /// annex tag list is corrected), un-ignore.
    #[test]
    #[ignore]
    fn p2_4_block_boundaries(
        tags in prop::collection::vec(prop::sample::select(BLOCK_TAGS_ANNEX), 1..8),
    ) {
        let mut html = String::new();
        let mut tokens = Vec::new();
        for (i, tag) in tags.iter().enumerate() {
            let tok = format!("TK{i}");
            html.push_str(&format!("<{tag}>{tok}</{tag}>"));
            tokens.push(tok);
        }
        let expected = tokens.join("\n\n");
        prop_assert_eq!(html_to_text(&html), expected);
    }
}
