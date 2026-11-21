//! Properties from the annex that currently do NOT hold against the
//! live implementation. Each is kept in its original, unweakened form
//! and marked #[ignore]; run with `cargo test -- --ignored` to see it
//! fail on the recorded minimal counterexample. The implementation is
//! NOT adjusted to make these pass (brief W4); each entry records the
//! minimal counterexample and its abolition condition （廃止条件）:
//! when the behaviour (or the annex formula) is corrected, un-ignore
//! the test and move it back to `prop_tests.rs`.

use proptest::prelude::*;

use super::percent_encode;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    /// Annex 4.3 （冪等） — DOES NOT HOLD.
    /// Minimal counterexample: b = [0]: once = "%00", twice =
    /// "%2500". Mechanism: '%' (0x25) is not an unreserved byte, so
    /// encoding an encoded string escapes its '%' signs to %25 —
    /// percent-encoding is not idempotent by construction (the
    /// annex's own failure column anticipates exactly this).
    /// Abolition: this closes only if the annex claim is withdrawn
    /// (an encoder that left '%' alone would corrupt its own
    /// output); un-ignore if the encoding rule ever changes so
    /// that '%' passes through.
    #[test]
    #[ignore]
    fn p4_3_encode_idempotent(
        b in prop::collection::vec(any::<u8>(), 0..512),
    ) {
        let once = percent_encode(&b);
        let twice = percent_encode(once.as_bytes());
        prop_assert_eq!(twice, once);
    }
}
