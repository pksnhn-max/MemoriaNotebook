//! Properties from the annex that currently do NOT hold against the
//! live implementation. Each is kept in its original, unweakened form
//! and marked #[ignore]; run with `cargo test -- --ignored` to see it
//! fail on the recorded minimal counterexample. The implementation is
//! NOT adjusted to make these pass (brief W4); each entry records the
//! minimal counterexample found by proptest (2026-11-21, 1,000 cases)
//! and its abolition condition （廃止条件）: when the behaviour — or
//! the annex formula — is corrected, un-ignore the test and move it
//! back to `prop_tests.rs`.
//!
//! FC wave (2026-11-21): annex 2.6 and 2.4 met their abolition
//! conditions (the implementation was corrected) and moved back to
//! `prop_tests.rs`; annex 1.5 was rewritten there as the
//! pass-through contract under FC decision 4. What remains here:
//! 1.2, 1.3, 1.4, 2.2.

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

