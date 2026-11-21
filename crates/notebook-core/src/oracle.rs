//! ORACLE — frozen verbatim copies of live functions from this crate's
//! `lib.rs`, for differential testing (testing-manual §2, placement ①:
//! a `#[cfg(test)]` oracle module; this whole file is test-only because
//! the `mod oracle;` declaration in `lib.rs` is `#[cfg(test)]`).
//!
//! DO NOT EDIT the copied bodies. The oracle is frozen with its bugs;
//! if live and oracle disagree, suspect the live side (or the copy's
//! transcription — re-extract, never hand-fix).
//!
//! Provenance (copied 2026-11-21 from the live source at repo HEAD
//! 3118d042 by mechanical line-range extraction; see the W3 record book
//! `~/workspace/notebook-design/records/2026-11-21-w3-oracle-core.md`):
//! - `CHUNK_TARGET_CHARS`, `chunk_text`, `hard_split`,
//!   `format_grounding`, `parse_citations`
//!   ← `crates/notebook-core/src/lib.rs`
//!
//! Differential property tests: `mod diff_tests` at the bottom.
//! Fixed-corpus fixture tests (tests/fixtures/ledger, tests/fixtures/emb):
//! `mod fixture_tests` at the bottom.

use std::collections::HashMap;

use crate::{ChunkRec, Citation};

/// Target chunk size in characters (paragraph units, roughly 400).
pub const CHUNK_TARGET_CHARS: usize = 400;

/// Split text into chunks: paragraphs (blank-line separated) are packed
/// in order into chunks of at most roughly `CHUNK_TARGET_CHARS`
/// characters; a paragraph longer than twice the target is hard-split
/// near the target, preferring a sentence end (`。` / `、`).
pub fn chunk_text(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    for para in text.split("\n\n") {
        let para = para.trim();
        if para.is_empty() {
            continue;
        }
        if para.chars().count() > CHUNK_TARGET_CHARS * 2 {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            out.extend(hard_split(para));
            continue;
        }
        let need = para.chars().count()
            + if cur.is_empty() {
                0
            } else {
                cur.chars().count() + 1
            };
        if !cur.is_empty() && need > CHUNK_TARGET_CHARS {
            out.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push('\n');
        }
        cur.push_str(para);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn hard_split(para: &str) -> Vec<String> {
    let chars: Vec<char> = para.chars().collect();
    let mut out = Vec::new();
    let mut start = 0;
    while chars.len() - start > CHUNK_TARGET_CHARS {
        let limit = start + CHUNK_TARGET_CHARS;
        // Look back a little for a sentence break.
        let mut cut = limit;
        for i in (start + CHUNK_TARGET_CHARS / 2..limit).rev() {
            if chars[i] == '。' || chars[i] == '、' {
                cut = i + 1;
                break;
            }
        }
        out.push(chars[start..cut].iter().collect());
        start = cut;
    }
    if start < chars.len() {
        out.push(chars[start..].iter().collect());
    }
    out
}

/// Format retrieved chunks as a grounding block for the prompt.
/// Each hit becomes `[#<chunk_id>] <text>` on its own line, with a blank line between hits.
/// Scores are not included. Empty input yields empty string.
pub fn format_grounding(hits: &[(ChunkRec, f32)]) -> String {
    if hits.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for (i, (chunk, _score)) in hits.iter().enumerate() {
        if i > 0 {
            out.push_str("\n\n");
        }
        out.push_str(&format!("[#{}] {}", chunk.id, chunk.text));
    }
    out
}

/// Parse citations from generated text.
/// Scans `text` for markers of the form `[#<number>]`.
/// Only markers whose number matches a `chunk_id` in `hits` are kept.
/// Returns citations in order of first appearance, deduplicated by chunk_id.
/// Markers with non-numeric content (e.g., `[#abc]`) are ignored.
/// Markers referencing unknown chunk_ids are silently dropped.
pub fn parse_citations(text: &str, hits: &[(ChunkRec, f32)]) -> Vec<Citation> {
    // Build a quick lookup: chunk_id -> source_id
    let mut chunk_to_source = HashMap::new();
    for (chunk, _) in hits {
        chunk_to_source.insert(chunk.id, chunk.source_id);
    }

    let mut citations = Vec::new();
    let mut seen = std::collections::HashSet::new();

    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Look for '[#'
        if i + 2 < bytes.len() && bytes[i] == b'[' && bytes[i + 1] == b'#' {
            let mut j = i + 2;
            // Parse digits
            let mut num = 0u64;
            let mut has_digit = false;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                has_digit = true;
                num = num.saturating_mul(10).saturating_add((bytes[j] - b'0') as u64);
                j += 1;
            }
            // Expect closing ']'
            if has_digit && j < bytes.len() && bytes[j] == b']' {
                // Valid marker found
                if let Some(&source_id) = chunk_to_source.get(&num) {
                    if seen.insert(num) {
                        citations.push(Citation {
                            source_id,
                            chunk_id: Some(num),
                        });
                    }
                }
                i = j + 1;
                continue;
            }
        }
        i += 1;
    }
    citations
}

// =====================================================================
// Differential property tests: live vs the frozen oracle copies above.
// Every property runs at the manual's floor of 1,000 cases (§1).
// The oracle copies are the ones defined in this module (super::*);
// the live functions are the crate-root ones (crate::*).
// =====================================================================
mod diff_tests {
    use super::{chunk_text as oracle_chunk_text, format_grounding as oracle_format_grounding,
                hard_split as oracle_hard_split, parse_citations as oracle_parse_citations};
    use crate::{chunk_text as live_chunk_text, format_grounding as live_format_grounding,
                hard_split as live_hard_split, parse_citations as live_parse_citations,
                ChunkRec};
    use proptest::prelude::*;

    /// Short mixed-script segments: kana, sentence punctuation, ASCII,
    /// spaces, single newlines, wide chars — the shapes chunking cares
    /// about (paragraph packing, sentence-end preference).
    fn seg_strategy() -> impl Strategy<Value = String> {
        prop::collection::vec(
            prop_oneof![
                Just("あ"), Just("い"), Just("う"), Just("。"), Just("、"),
                Just("a"), Just("b"), Just(" "), Just("\n"), Just("世"),
                Just("🙂"), Just("x"),
            ],
            0..40,
        )
        .prop_map(|v| v.concat())
    }

    /// Long single paragraph (650–950 chars): crosses the 2× target
    /// threshold (800) both ways, so `hard_split` engages on a share.
    fn long_para_strategy() -> impl Strategy<Value = String> {
        prop::collection::vec(
            prop_oneof![
                Just('あ'), Just('。'), Just('、'), Just('a'), Just(' '),
            ],
            650..950,
        )
        .prop_map(|v| v.into_iter().collect::<String>())
    }

    /// Whole texts: paragraphs joined by blank lines, with empty and
    /// whitespace-only paragraphs mixed in.
    fn text_strategy() -> impl Strategy<Value = String> {
        let para = prop_oneof![
            seg_strategy(),
            long_para_strategy(),
            Just(String::new()),
            Just("   ".to_string()),
        ];
        prop::collection::vec(para, 0..10).prop_map(|ps| ps.join("\n\n"))
    }

    fn chunk_rec_strategy() -> impl Strategy<Value = ChunkRec> {
        (0u64..8, 0u64..4, 0u64..4, 0usize..6, 0u64..3, seg_strategy()).prop_map(
            |(id, source_id, nb, ord, rev, text)| ChunkRec {
                id,
                source_id,
                nb,
                ord,
                rev,
                text,
            },
        )
    }

    fn hits_strategy() -> impl Strategy<Value = Vec<(ChunkRec, f32)>> {
        prop::collection::vec((chunk_rec_strategy(), any::<f32>()), 0..8)
    }

    /// Answer texts full of citation-marker shapes: valid markers for
    /// small ids, unknown ids, non-numeric markers, unclosed markers,
    /// a saturating-overflow number, and plain prose.
    fn answer_strategy() -> impl Strategy<Value = String> {
        let piece = prop_oneof![
            (0u64..10).prop_map(|n| format!("[#{}]", n)),
            Just("[#abc]".to_string()),
            Just("[#".to_string()),
            Just("[#99999999999999999999999999999999]".to_string()),
            Just("[##]".to_string()),
            Just("]".to_string()),
            seg_strategy(),
        ];
        prop::collection::vec(piece, 0..14).prop_map(|ps| ps.concat())
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn chunk_text_matches_oracle(text in text_strategy()) {
            prop_assert_eq!(live_chunk_text(&text), oracle_chunk_text(&text));
        }

        #[test]
        fn hard_split_matches_oracle(para in long_para_strategy()) {
            prop_assert_eq!(live_hard_split(&para), oracle_hard_split(&para));
        }

        #[test]
        fn format_grounding_matches_oracle(hits in hits_strategy()) {
            prop_assert_eq!(live_format_grounding(&hits), oracle_format_grounding(&hits));
        }

        #[test]
        fn parse_citations_matches_oracle(text in answer_strategy(), hits in hits_strategy()) {
            prop_assert_eq!(
                live_parse_citations(&text, &hits),
                oracle_parse_citations(&text, &hits)
            );
        }
    }
}

// =====================================================================
// Fixed-corpus fixture tests for the ledger (JSONL) and embedding
// sidecar (emb_chunks.bin) surfaces.
//
// There is no oracle copy of the readers (`crate::read_jsonl`,
// `crate::load_embeddings`) — the brief's oracle list does not include
// them, and the design annex classes CoreState loading as
// non-oracle-able. These tests therefore pin the LIVE readers against
// independent minimal readers written here (manual §1, 形式整合）:
// the expectations are computed by separate code from the documented
// semantics, not by the implementation under test.
//
// Fixture shapes come from the corpus-b1 annex, re-expressed in the
// real record schemas (ChunkRec JSON lines; bare u64 lines for
// deleted.jsonl). See tests/fixtures/README.md for the mapping.
// Inputs the annex puts in the undefined bucket get no fixed
// expectation: the test only asserts the live reader does not panic.
// =====================================================================
mod fixture_tests {
    use crate::{load_embeddings, read_jsonl, ChunkRec};
    use std::collections::{HashMap, HashSet};
    use std::path::PathBuf;

    fn fixture_path(rel: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join(rel)
    }

    fn chunk_rec(id: u64) -> ChunkRec {
        ChunkRec {
            id,
            source_id: 1,
            nb: 1,
            ord: 0,
            rev: 1,
            text: String::new(),
        }
    }

    /// Independent JSONL reader for ChunkRec lines: a line yields a
    /// record iff it is a JSON object carrying all six fields with the
    /// right types (unknown fields ignored). Blank lines and anything
    /// else are skipped; order is preserved. Written from the schema,
    /// not from `read_jsonl`'s derive-based code.
    fn expected_chunk_records(content: &str) -> Vec<ChunkRec> {
        let mut out = Vec::new();
        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let v: serde_json::Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let obj = match v.as_object() {
                Some(o) => o,
                None => continue,
            };
            let get = |k: &str| obj.get(k).and_then(|x| x.as_u64());
            let rec = (|| -> Option<ChunkRec> {
                Some(ChunkRec {
                    id: get("id")?,
                    source_id: get("source_id")?,
                    nb: get("nb")?,
                    ord: get("ord")? as usize,
                    rev: get("rev")?,
                    text: obj.get("text")?.as_str()?.to_string(),
                })
            })();
            if let Some(r) = rec {
                out.push(r);
            }
        }
        out
    }

    /// Independent reader for deleted.jsonl (bare u64 per line).
    fn expected_deleted_ids(content: &str) -> Vec<u64> {
        content
            .lines()
            .filter_map(|l| l.trim().parse::<u64>().ok())
            .collect()
    }

    /// Independent sidecar walker, written from the wire format and
    /// the documented semantics of `load_embeddings`: sequential
    /// records of (u64 LE id, u32 LE dim, f32[dim] LE); stop at the
    /// first incomplete record; keep only ids in `current`; a later
    /// record for the same id overwrites the earlier one.
    fn expected_embeddings(bytes: &[u8], current: &HashSet<u64>) -> HashMap<u64, Vec<f32>> {
        let mut map = HashMap::new();
        let mut pos = 0usize;
        while pos + 12 <= bytes.len() {
            let id = u64::from_le_bytes(bytes[pos..pos + 8].try_into().unwrap());
            let dim = u32::from_le_bytes(bytes[pos + 8..pos + 12].try_into().unwrap()) as usize;
            pos += 12;
            if pos + dim * 4 > bytes.len() {
                break;
            }
            let mut v = Vec::with_capacity(dim);
            for k in 0..dim {
                v.push(f32::from_le_bytes(
                    bytes[pos + 4 * k..pos + 4 * k + 4].try_into().unwrap(),
                ));
            }
            pos += dim * 4;
            if current.contains(&id) {
                map.insert(id, v);
            }
        }
        map
    }

    fn assert_ledger_fixture(rel: &str) {
        let path = fixture_path(rel);
        let content = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("fixture {rel} unreadable: {e}"));
        let live: Vec<ChunkRec> = read_jsonl(&path).expect("live read_jsonl failed");
        assert_eq!(live, expected_chunk_records(&content), "fixture {rel}");
    }

    fn assert_deleted_fixture(rel: &str) {
        let path = fixture_path(rel);
        let content = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("fixture {rel} unreadable: {e}"));
        let live: Vec<u64> = read_jsonl(&path).expect("live read_jsonl failed");
        assert_eq!(live, expected_deleted_ids(&content), "fixture {rel}");
    }

    fn assert_emb_fixture(rel: &str, current_ids: &[u64]) {
        let path = fixture_path(rel);
        let bytes = std::fs::read(&path)
            .unwrap_or_else(|e| panic!("fixture {rel} unreadable: {e}"));
        let current: Vec<ChunkRec> = current_ids.iter().map(|&id| chunk_rec(id)).collect();
        let live = load_embeddings(&path, &current).expect("live load_embeddings failed");
        let set: HashSet<u64> = current_ids.iter().copied().collect();
        assert_eq!(live, expected_embeddings(&bytes, &set), "fixture {rel}");
    }

    #[test]
    fn ledger_fixtures_match_independent_reader() {
        for rel in [
            "ledger/00_empty.jsonl",
            "ledger/01_one_valid.jsonl",
            "ledger/02_all_blank.jsonl",
            "ledger/10_blank_head.jsonl",
            "ledger/11_blank_middle.jsonl",
            "ledger/12_blank_tail.jsonl",
            "ledger/13_whitespace_only.jsonl",
            "ledger/14_broken_json.jsonl",
            "ledger/15_wrong_type.jsonl",
            "ledger/16_mixed_garbage.jsonl",
            "ledger/20_very_long_line.jsonl",
            "ledger/21_long_middle.jsonl",
            "ledger/22_long_tail.jsonl",
            "ledger/30_dup_id.jsonl",
            "ledger/31_dup_nonadjacent.jsonl",
            "ledger/32_chunks.jsonl",
            "ledger/40_fail_head.jsonl",
            "ledger/41_fail_middle.jsonl",
            "ledger/42_fail_tail.jsonl",
            "ledger/43_all_fail.jsonl",
            "ledger/50_partial_tail_line.jsonl",
            "ledger/51_partial_mid_line.jsonl",
            "ledger/52_chunks.jsonl",
            "ledger/60_resume_twice.jsonl",
            "ledger/61_resume_with_garbage.jsonl",
            "ledger/70_chunks.jsonl",
            "ledger/71_chunks.jsonl",
            "ledger/80_skip_empty.jsonl",
            "ledger/81_skip_all.jsonl",
            "ledger/82_skip_overlap_fail.jsonl",
        ] {
            assert_ledger_fixture(rel);
        }
    }

    #[test]
    fn deleted_fixtures_match_independent_reader() {
        assert_deleted_fixture("ledger/32_deleted.jsonl");
        assert_deleted_fixture("ledger/33_deleted.jsonl");
        assert_deleted_fixture("ledger/71_deleted.jsonl");
    }

    #[test]
    fn emb_fixtures_match_independent_walker() {
        // (fixture, ids treated as current chunks)
        let cases: &[(&str, &[u64])] = &[
            ("emb/00_empty.bin", &[100]),
            ("emb/01_one_vec.bin", &[100]),
            // Vector whose chunk is not current: dropped by both.
            ("emb/02_no_corresponding_ledger.bin", &[100]),
            // …and kept when it is current.
            ("emb/02_no_corresponding_ledger.bin", &[999]),
            ("emb/20_dim_mismatch_short.bin", &[10]),
            ("emb/21_dim_mismatch_long.bin", &[11, 12]),
            ("emb/22_truncated_tail.bin", &[20]),
            ("emb/23_truncated_middle.bin", &[30, 31, 32]),
            ("emb/30_dup_chunk_id.bin", &[10]),
            ("emb/31_dup_nonadjacent.bin", &[10, 11]),
            ("emb/40_fail_head.bin", &[10, 11, 12]),
            ("emb/41_fail_middle.bin", &[10, 11, 12]),
            ("emb/42_fail_tail.bin", &[10, 11, 12]),
            ("emb/43_all_fail.bin", &[10, 11, 12]),
            ("emb/50_truncated_tail_record.bin", &[10, 11]),
            ("emb/51_truncated_mid_record.bin", &[10, 11, 12]),
            // Ledger claims 100,101,102 but the sidecar holds only two.
            ("emb/52_partial.bin", &[100, 101, 102]),
            // Sidecar holds an orphan 102 the ledger does not claim.
            ("emb/52_full.bin", &[100, 101]),
            ("emb/60_resume_twice.bin", &[10, 11, 12, 13, 14]),
            ("emb/61_resume_with_truncation.bin", &[10, 11, 12, 13]),
            ("emb/70_extra_vectors.bin", &[1, 2]),
            ("emb/71_vector_for_deleted.bin", &[1, 2]),
            // Skip-set dimension, exercised via the current-chunk
            // selection (read_jsonl has no skip-set parameter):
            // everything current / nothing current / selection that
            // overlaps a corrupt record (emb/82: the corrupt record
            // still stops the walk before 12 is reached).
            ("emb/80_skip_empty.bin", &[10, 11]),
            ("emb/81_skip_all.bin", &[]),
            ("emb/82_skip_overlap_corrupt.bin", &[12]),
            ("emb/82_skip_overlap_corrupt.bin", &[10, 11, 12]),
        ];
        for (rel, current) in cases {
            assert_emb_fixture(rel, current);
        }
    }

    /// Undefined bucket (annex category 8): no expectation is fixed.
    /// Current face value, recorded from the live code's semantics:
    /// ledger/83 — the line missing a required field is skipped, the
    /// line with an unknown extra field parses as a normal record;
    /// emb/83 — a dim=0 record loads as an empty vector. The only
    /// assertion here is that the live readers do not panic.
    #[test]
    fn undefined_bucket_inputs_do_not_panic() {
        let ledger = fixture_path("ledger/83_undefined_bucket.jsonl");
        let _: Vec<ChunkRec> = read_jsonl(&ledger).expect("live read_jsonl failed");
        let emb = fixture_path("emb/83_undefined_bucket.bin");
        let current = vec![chunk_rec(10), chunk_rec(11), chunk_rec(12)];
        let _ = load_embeddings(&emb, &current).expect("live load_embeddings failed");
    }
}
