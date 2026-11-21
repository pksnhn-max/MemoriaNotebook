//! Differential tests (testing-manual §2): the live implementation
//! vs the frozen oracle in `super::oracle`, same generated inputs.
//! Floor: 1,000 cases per property (brief W4). `hex` is the one
//! exception in form: its domain is a single byte (256 values), so
//! it is compared exhaustively over the whole domain instead of by
//! sampling — strictly stronger than 1,000 generated cases.

use proptest::prelude::*;

use super::oracle;
use super::{hf_resolve_url, percent_decode, percent_encode};

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

    #[test]
    fn diff_percent_encode(b in prop::collection::vec(any::<u8>(), 0..512)) {
        prop_assert_eq!(percent_encode(&b), oracle::percent_encode(&b));
    }

    #[test]
    fn diff_percent_decode(s in decode_string()) {
        prop_assert_eq!(percent_decode(&s), oracle::percent_decode(&s));
    }

    #[test]
    fn diff_percent_decode_arbitrary(s in any::<String>()) {
        prop_assert_eq!(percent_decode(&s), oracle::percent_decode(&s));
    }

    #[test]
    fn diff_hf_resolve_url(repo in any::<String>(), file in any::<String>()) {
        prop_assert_eq!(
            hf_resolve_url(&repo, &file),
            oracle::hf_resolve_url(&repo, &file)
        );
    }
}

/// `hex` has a 256-value domain: exhaustive comparison.
#[test]
fn diff_hex_exhaustive() {
    for v in 0..=255u16 {
        let b = v as u8;
        assert_eq!(super::hex(b), oracle::hex(b), "byte {v:#04x}");
    }
}
