//! Property tests (testing-manual §1) for notebook-net.
//! Targets and shapes follow annex `props-extract-net.md` §4–§6;
//! the case floor is 1,000 for every generated property (brief W4,
//! which raises the annex's 66/200/500 floors). Properties that turn
//! out NOT to hold are moved, unmodified, to `props_not_holding.rs`.
//!
//! The percent round-trip range (annex 4.2 / 5.3): percent_decode
//! maps a bare '+' to a space, so — per the brief, which fixes this
//! caveat as spec — the round-trip properties below are stated over
//! inputs whose bytes contain no 0x2B ('+'). (Observed in the code:
//! percent_encode escapes '+' as %2B and never emits a bare '+', so
//! this range is the conservative one; full-range behaviour is still
//! pinned by the differential tests against the oracle.)

use proptest::prelude::*;

use super::{hf_resolve_url, percent_decode, percent_encode};

fn is_unreserved(b: u8) -> bool {
    matches!(b, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~')
}

/// All byte values except 0x2B ('+') — the round-trip range.
fn bytes_without_plus() -> impl Strategy<Value = Vec<u8>> {
    let pool: Vec<u8> = (0..=255u16)
        .map(|v| v as u8)
        .filter(|&b| b != 0x2B)
        .collect();
    prop::collection::vec(prop::sample::select(pool), 0..512)
}

const DECODE_FRAGMENTS: &[&str] = &[
    "%", "%4", "%41", "%4G", "%GG", "%e3%81%82", "%2B", "%25", "%00",
    "+", "a", "Z", "0", " ", "あ", "~", "/", "?", "&", "=", "http",
    "%x", "%1G", "%%", "%aF", "%Af",
];

fn decode_string() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(DECODE_FRAGMENTS), 0..40)
        .prop_map(|parts| parts.concat())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    /// Annex 4.1 （不発）: percent_encode never panics and its output
    /// is always ASCII.
    #[test]
    fn p4_1_encode_never_panics_ascii_out(
        b in prop::collection::vec(any::<u8>(), 0..1024),
    ) {
        let out = percent_encode(&b);
        prop_assert!(out.is_ascii());
    }

    /// Annex 4.2 （往復， brief-fixed range: no '+' bytes — see the
    /// module comment): decode(encode(b)) == from_utf8_lossy(b).
    #[test]
    fn p4_2_encode_decode_roundtrip(b in bytes_without_plus()) {
        let expected = String::from_utf8_lossy(&b).into_owned();
        prop_assert_eq!(percent_decode(&percent_encode(&b)), expected);
    }

    /// Annex 5.3: the same round trip stated from the decode side,
    /// over the same '+'-free range.
    #[test]
    fn p5_3_encode_decode_roundtrip(b in bytes_without_plus()) {
        let expected = String::from_utf8_lossy(&b).into_owned();
        prop_assert_eq!(percent_decode(&percent_encode(&b)), expected);
    }

    /// Annex 4.4 （予約文字不変）: unreserved bytes pass through
    /// unchanged (the annex's fixed 66-byte enumeration, generated
    /// here at the brief's 1,000-case floor).
    #[test]
    fn p4_4_unreserved_pass_through(
        b in prop::collection::vec(
            prop::sample::select(
                (0..=255u16).map(|v| v as u8).filter(|&b| is_unreserved(b)).collect::<Vec<u8>>(),
            ),
            0..256,
        ),
    ) {
        let expected = String::from_utf8(b.clone()).unwrap();
        prop_assert_eq!(percent_encode(&b), expected);
    }

    /// Annex 4.5 （出力形式）: the output consists solely of
    /// unreserved characters and "%XX" with uppercase hex digits.
    #[test]
    fn p4_5_encode_output_form(b in prop::collection::vec(any::<u8>(), 0..512)) {
        let out = percent_encode(&b);
        let bytes = out.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'%' {
                prop_assert!(i + 2 < bytes.len(), "dangling % in {out:?}");
                let h = bytes[i + 1];
                let l = bytes[i + 2];
                prop_assert!(
                    h.is_ascii_digit() || (b'A'..=b'F').contains(&h),
                    "non-uppercase-hex {h} in {out:?}"
                );
                prop_assert!(
                    l.is_ascii_digit() || (b'A'..=b'F').contains(&l),
                    "non-uppercase-hex {l} in {out:?}"
                );
                i += 3;
            } else {
                prop_assert!(is_unreserved(bytes[i]), "byte {} in {out:?}", bytes[i]);
                i += 1;
            }
        }
    }

    /// Annex 5.1 （不発）: percent_decode never panics on any string.
    #[test]
    fn p5_1_decode_never_panics(s in prop_oneof![decode_string(), any::<String>()]) {
        let _ = percent_decode(&s);
    }

    /// Annex 5.2 （往復）: decode is a fixed point of
    /// decode ∘ encode — decode(encode(decode(s))) == decode(s).
    #[test]
    fn p5_2_decode_encode_decode_stable(s in decode_string()) {
        let d = percent_decode(&s);
        let again = percent_decode(&percent_encode(d.as_bytes()));
        prop_assert_eq!(again, d);
    }

    /// Annex 5.6 (UTF-8 損失許容）: decoding the %XX spelling of
    /// arbitrary bytes (either hex case) yields from_utf8_lossy of
    /// those bytes — replacement chars and all, never a panic.
    #[test]
    fn p5_6_decode_percent_spelled_bytes_lossy(
        b in prop::collection::vec(any::<u8>(), 0..256),
    ) {
        let upper: String = b.iter().map(|x| format!("%{x:02X}")).collect();
        prop_assert_eq!(
            percent_decode(&upper),
            String::from_utf8_lossy(&b).into_owned()
        );
        let lower: String = b.iter().map(|x| format!("%{x:02x}")).collect();
        prop_assert_eq!(
            percent_decode(&lower),
            String::from_utf8_lossy(&b).into_owned()
        );
    }
}

/// Annex 5.4 （不正 % 透過）, generated form: over an alphabet with
/// no hex digits and no '+', every '%' is necessarily invalid, so
/// decoding is the identity.
fn no_hex_alphabet_string() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop::sample::select(vec!['%', 'G', 'z', ' ', '-', '_', 'あ', '\n', 'J', 'Q']),
        0..40,
    )
    .prop_map(|chars| chars.into_iter().collect::<String>())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    #[test]
    fn p5_4_invalid_percent_passes_through(s in no_hex_alphabet_string()) {
        prop_assert_eq!(percent_decode(&s), s.clone());
    }

    /// Annex 5.5: without '%' in play, decoding is exactly
    /// '+' -> ' ' and identity elsewhere.
    #[test]
    fn p5_5_plus_becomes_space(
        parts in prop::collection::vec(
            prop::sample::select(vec!["+", "a", "B", " ", "あ", "++", "x+y", "~"]),
            0..30,
        ),
    ) {
        let s = parts.concat();
        prop_assert_eq!(percent_decode(&s), s.replace('+', " "));
    }
}

/// Annex 5.4, the annex's named fixed cases.
#[test]
fn p5_4_named_cases() {
    assert_eq!(percent_decode("%"), "%");
    assert_eq!(percent_decode("%A"), "%A");
    assert_eq!(percent_decode("%GG"), "%GG");
    assert_eq!(percent_decode("%1G"), "%1G");
}

// ---------- §6 hf_resolve_url ----------

/// Test-local, independent segment encoder (the RFC 3986 unreserved
/// rule restated here so §6's yardstick does not borrow the live
/// percent_encode).
fn enc_seg(seg: &str) -> String {
    let mut out = String::new();
    for &b in seg.as_bytes() {
        if is_unreserved(b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn segments() -> impl Strategy<Value = Vec<String>> {
    prop::collection::vec("[a-zA-Z0-9 ._%~-]{0,12}", 0..4)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    /// Annex 6.1 （不発）: hf_resolve_url never panics.
    #[test]
    fn p6_1_hf_never_panics(repo in any::<String>(), file in any::<String>()) {
        let _ = hf_resolve_url(&repo, &file);
    }

    /// Annex 6.2 （セグメント単位エンコード）: '/' separators stay
    /// literal; each segment is encoded by the unreserved rule.
    #[test]
    fn p6_2_hf_segmentwise_encoding(
        repo_segs in segments(),
        file_segs in segments(),
    ) {
        let repo = repo_segs.join("/");
        let file = file_segs.join("/");
        let enc = |segs: &[String]| {
            segs.iter().map(|s| enc_seg(s)).collect::<Vec<_>>().join("/")
        };
        let expected = format!(
            "https://huggingface.co/{}/resolve/main/{}",
            enc(&repo_segs),
            enc(&file_segs)
        );
        prop_assert_eq!(hf_resolve_url(&repo, &file), expected);
    }

    /// Annex 6.3 （往復的整合）: percent-decoding each path segment
    /// of the produced URL recovers the original repo/file segments
    /// (segments here are '+'-free, matching the brief's round-trip
    /// range — a decoded '+' would come back as a space).
    #[test]
    fn p6_3_hf_segments_recover_by_decode(
        repo_segs in segments(),
        file_segs in segments(),
    ) {
        let repo = repo_segs.join("/");
        let file = file_segs.join("/");
        let url = hf_resolve_url(&repo, &file);
        let rest = url
            .strip_prefix("https://huggingface.co/")
            .expect("prefix");
        let decoded: Vec<String> = rest
            .split('/')
            .map(|seg| percent_decode(seg))
            .collect();
        // "".split('/') yields one empty segment, so an empty
        // segment list still contributes a single "" to the path.
        let mut expected: Vec<String> = if repo_segs.is_empty() {
            vec![String::new()]
        } else {
            repo_segs.clone()
        };
        expected.push("resolve".to_string());
        expected.push("main".to_string());
        if file_segs.is_empty() {
            expected.push(String::new());
        } else {
            expected.extend(file_segs.iter().cloned());
        }
        prop_assert_eq!(decoded, expected);
    }

    /// Annex 6.4 (URL 形式）: the output carries the fixed prefix
    /// and the /resolve/main/ infix (annex floor 200, raised to the
    /// brief's uniform 1,000).
    #[test]
    fn p6_4_hf_url_form(repo in any::<String>(), file in any::<String>()) {
        let url = hf_resolve_url(&repo, &file);
        prop_assert!(url.starts_with("https://huggingface.co/"), "{url}");
        prop_assert!(url.contains("/resolve/main/"), "{url}");
    }
}
