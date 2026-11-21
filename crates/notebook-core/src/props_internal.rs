//! Property suite W1 (internal half): candidates 2-1, 2-2, 2-5 of
//! the annex `nb-t3-props-core` (props-core.md). These target the
//! private ledger helpers `read_jsonl`, `append_jsonl` and
//! `load_embeddings`, which integration tests cannot reach; the
//! brief places exactly these inside the crate.
//!
//! Every generated property runs at 1,000 cases (brief contract).
//!
//! History: `read_jsonl_skips_invalid_utf8_lines` (candidate 2-1)
//! was the annex's claim kept failing against the old
//! `BufReader::lines` implementation; brief FB (2026-11-21) changed
//! the implementation to per-line isolation with a skip count, and
//! the test now asserts the standing contract (regression flip —
//! see the FB record book
//! ~/workspace/notebook-design/records/2026-11-21-fb-core-fix.md).
//! Continuation FB2 (same day): the merged run found the test's
//! own generator leaking its contract — byte strings that decode
//! to a whitespace-only string (e.g. [0xC2, 0xA0] = U+00A0) are
//! blank lines under the contract, not damaged lines. The
//! generator now states and enforces that contract (see
//! `bad_line`); the implementation was not changed.

use super::*;
use proptest::prelude::*;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(100_000);

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let n = SEQ.fetch_add(1, Ordering::SeqCst);
        let d = std::env::temp_dir().join(format!("nbprops-{tag}-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        TempDir(d)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// ---------------------------------------------------------------------
// record strategies
// ---------------------------------------------------------------------

fn arb_string() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop_oneof![
            Just('a'),
            Just('あ'),
            Just('"'),
            Just('\\'),
            Just('\n'),
            Just(' '),
            Just('/'),
        ],
        0..=12,
    )
    .prop_map(|cs| cs.into_iter().collect())
}

fn arb_notebook() -> impl Strategy<Value = Notebook> {
    (0..=1000u64, arb_string(), 0..=1000u64, 0..=1000u64).prop_map(
        |(id, title, created_at, updated_at)| Notebook {
            id,
            title,
            created_at,
            updated_at,
        },
    )
}

fn arb_source() -> impl Strategy<Value = SourceRec> {
    (
        0..=100u64,
        0..=100u64,
        prop_oneof![
            Just(SourceKind::Pdf),
            Just(SourceKind::Web),
            Just(SourceKind::Text),
            Just(SourceKind::Audio)
        ],
        arb_string(),
        arb_string(),
        0..=1000u64,
    )
        .prop_map(|(id, nb, kind, title, abs_path, added_at)| SourceRec {
            id,
            nb,
            kind,
            title,
            abs_path,
            added_at,
        })
}

fn arb_chunk() -> impl Strategy<Value = ChunkRec> {
    (
        0..=100u64,
        0..=100u64,
        0..=100u64,
        0..=20usize,
        0..=5u64,
        arb_string(),
    )
        .prop_map(|(id, source_id, nb, ord, rev, text)| ChunkRec {
            id,
            source_id,
            nb,
            ord,
            rev,
            text,
        })
}

fn arb_turn() -> impl Strategy<Value = ChatTurn> {
    (
        0..=100u64,
        prop_oneof![Just(Role::User), Just(Role::Assistant)],
        arb_string(),
        prop::collection::vec((0..=100u64, prop::option::of(0..=100u64)), 0..=3),
        0..=1000u64,
    )
        .prop_map(|(nb, role, text, cits, at)| ChatTurn {
            nb,
            role,
            text,
            citations: cits
                .into_iter()
                .map(|(source_id, chunk_id)| Citation {
                    source_id,
                    chunk_id,
                })
                .collect(),
            at,
        })
}

// ---------------------------------------------------------------------
// candidate 2-2: append_jsonl -> read_jsonl round trip, per record type
// ---------------------------------------------------------------------

fn append_read_roundtrip<T>(recs: &[T], path: &Path) -> Vec<T>
where
    T: Serialize + for<'de> Deserialize<'de>,
{
    for r in recs {
        append_jsonl(path, r).unwrap();
    }
    let out = read_jsonl(path).unwrap();
    assert_eq!(out.skipped, 0, "a clean round trip skips nothing");
    out.records
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    /// Candidate 2-2 (Notebook): appends persist in order — no loss,
    /// no duplication, no reordering — across a fresh read.
    #[test]
    fn append_read_roundtrip_notebooks(
        recs in prop::collection::vec(arb_notebook(), 0..=10)
    ) {
        let dir = TempDir::new("i22nb");
        let got = append_read_roundtrip(&recs, &dir.path().join("notebooks.jsonl"));
        prop_assert_eq!(got, recs);
    }

    /// Candidate 2-2 (SourceRec): same round trip.
    #[test]
    fn append_read_roundtrip_sources(
        recs in prop::collection::vec(arb_source(), 0..=10)
    ) {
        let dir = TempDir::new("i22src");
        let got = append_read_roundtrip(&recs, &dir.path().join("sources.jsonl"));
        prop_assert_eq!(got, recs);
    }

    /// Candidate 2-2 (ChunkRec): same round trip.
    #[test]
    fn append_read_roundtrip_chunks(
        recs in prop::collection::vec(arb_chunk(), 0..=10)
    ) {
        let dir = TempDir::new("i22chk");
        let got = append_read_roundtrip(&recs, &dir.path().join("chunks.jsonl"));
        prop_assert_eq!(got, recs);
    }

    /// Candidate 2-2 (ChatTurn): same round trip, citations included.
    #[test]
    fn append_read_roundtrip_turns(
        recs in prop::collection::vec(arb_turn(), 0..=10)
    ) {
        let dir = TempDir::new("i22trn");
        let got = append_read_roundtrip(&recs, &dir.path().join("turns.jsonl"));
        prop_assert_eq!(got, recs);
    }
}

// ---------------------------------------------------------------------
// candidate 2-1: read_jsonl skips corrupted lines
// ---------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Line {
    Valid(Notebook),
    Corrupt(String),
}

/// Lines that are valid UTF-8 but must not parse as a Notebook:
/// empty/whitespace, broken JSON, valid JSON of the wrong shape.
fn corrupt_line() -> impl Strategy<Value = String> {
    prop_oneof![
        Just(String::new()),
        Just("   ".to_string()),
        Just("\t".to_string()),
        Just("{".to_string()),
        Just("}".to_string()),
        Just("not json".to_string()),
        Just("123".to_string()),
        Just("\"a string\"".to_string()),
        Just("null".to_string()),
        Just("[1,2]".to_string()),
        Just("{}".to_string()),
        Just("{\"id\":1}".to_string()),
        Just("{\"id\":\"x\",\"title\":\"t\",\"created_at\":0,\"updated_at\":0}".to_string()),
    ]
}

fn line_strategy() -> impl Strategy<Value = Line> {
    prop_oneof![
        3 => arb_notebook().prop_map(Line::Valid),
        2 => corrupt_line().prop_map(Line::Corrupt),
        // A proper char-boundary prefix of a serialized record:
        // unbalanced, so it can never parse as a complete Notebook.
        2 => (arb_notebook(), 0..=40usize).prop_map(|(nb, cut)| {
            let s = serde_json::to_string(&nb).unwrap();
            let chars: Vec<char> = s.chars().collect();
            let k = cut % chars.len();
            Line::Corrupt(chars[..k].iter().collect())
        }),
    ]
}

/// One damaged line for the invalid-UTF-8 leg, as raw bytes over
/// 0x80..=0xFF. Contract, stated at face value: a generated line is
/// a line the reader counts as damaged exactly once — either it is
/// not valid UTF-8 at all (counted at the UTF-8 gate), or it
/// decodes to text with non-whitespace content, which contains no
/// ASCII structure and so fails JSON parsing (counted there).
/// Byte strings that decode to a whitespace-only string (e.g.
/// [0xC2, 0xA0] = U+00A0, a NO-BREAK SPACE) are NOT damaged lines:
/// under the standing contract (brief FB decision 1) a
/// whitespace-only line is a blank line, skipped silently and
/// never counted. The filter excludes exactly that class, by the
/// contract's own criterion (`str::trim`).
fn bad_line() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(0x80u8..=0xFF, 1..=6).prop_filter(
        "whitespace-only decodings are blank lines, not damaged lines",
        |b| match std::str::from_utf8(b) {
            Ok(s) => !s.trim().is_empty(),
            Err(_) => true,
        },
    )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    /// Candidate 2-1, UTF-8 leg: corrupt lines that are still valid
    /// UTF-8 (broken JSON, wrong shape, blank) are skipped, and
    /// every valid record around them is read, in order, with no
    /// panic and no early stop. The skip signal counts exactly the
    /// non-blank corrupt lines (blank lines are not damage).
    #[test]
    fn read_jsonl_skips_utf8_valid_corrupt_lines(
        lines in prop::collection::vec(line_strategy(), 0..=16)
    ) {
        let dir = TempDir::new("i21a");
        let path = dir.path().join("mixed.jsonl");
        let mut content = String::new();
        let mut expected: Vec<Notebook> = Vec::new();
        let mut expected_skipped = 0usize;
        for l in &lines {
            match l {
                Line::Valid(nb) => {
                    content.push_str(&serde_json::to_string(nb).unwrap());
                    expected.push(nb.clone());
                }
                Line::Corrupt(s) => {
                    if !s.trim().is_empty() {
                        expected_skipped += 1;
                    }
                    content.push_str(s)
                }
            }
            content.push('\n');
        }
        std::fs::write(&path, content).unwrap();
        let got = read_jsonl::<Notebook>(&path).unwrap();
        prop_assert_eq!(&got.records, &expected);
        prop_assert_eq!(got.skipped, expected_skipped);
    }

    /// Candidate 2-1, invalid-UTF-8 leg: lines of raw high bytes
    /// mixed into the file are isolated — every valid record around
    /// them is read, the read does not fail, and the skip signal
    /// counts exactly the injected bad lines. (Each bad line is
    /// counted exactly once: as invalid UTF-8, or — for the byte
    /// patterns that happen to be valid UTF-8 — as unparseable
    /// JSON, since it contains no ASCII structure. Byte patterns
    /// that decode to whitespace only are blank lines under the
    /// contract, not bad lines; `bad_line` states and enforces
    /// that boundary.)
    #[test]
    fn read_jsonl_skips_invalid_utf8_lines(
        valids in prop::collection::vec(arb_notebook(), 1..=6),
        bad in prop::collection::vec(bad_line(), 1..=3),
        pos in 0..=6usize,
    ) {
        let dir = TempDir::new("i21b");
        let path = dir.path().join("mixed.jsonl");
        let mut lines: Vec<Vec<u8>> = valids
            .iter()
            .map(|nb| serde_json::to_string(nb).unwrap().into_bytes())
            .collect();
        let at = pos % (lines.len() + 1);
        for (j, b) in bad.iter().enumerate() {
            lines.insert((at + j).min(lines.len()), b.clone());
        }
        let mut bytes: Vec<u8> = Vec::new();
        for l in &lines {
            bytes.extend_from_slice(l);
            bytes.push(b'\n');
        }
        std::fs::write(&path, bytes).unwrap();
        let got = read_jsonl::<Notebook>(&path).unwrap();
        prop_assert_eq!(&got.records, &valids);
        prop_assert_eq!(got.skipped, bad.len());
    }
}

// ---------------------------------------------------------------------
// candidate 2-5: load_embeddings filtering and truncation stop
// ---------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Tail {
    /// 1..=11 stray bytes: too few for another record header.
    Partial(Vec<u8>),
    /// A full record header (dim >= 1) whose vector bytes are cut
    /// short: `present` < dim * 4 bytes follow.
    DimCut(u32, usize),
}

fn tail_strategy() -> impl Strategy<Value = Tail> {
    prop_oneof![
        prop::collection::vec(any::<u8>(), 1..=11).prop_map(Tail::Partial),
        (1..=6u32, 0..=24usize)
            .prop_map(|(dim, present)| Tail::DimCut(dim, present % (dim as usize * 4))),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    /// Candidate 2-5 (no-fire / format integrity): `load_embeddings`
    /// keeps exactly the complete records whose chunk_id is a
    /// current chunk (later duplicates win), and a truncated tail —
    /// partial header or cut vector — stops the read without
    /// panicking and without losing the records before it.
    #[test]
    fn load_embeddings_filters_and_stops_at_truncation(
        chunk_id_pool in prop::collection::vec(0..=12u64, 1..=6),
        body in prop::collection::vec(
            (0..=17usize, prop::collection::vec(any::<f32>(), 0..=6)),
            0..=10
        ),
        tail in prop::option::of(tail_strategy()),
    ) {
        let mut chunk_ids: Vec<u64> = Vec::new();
        for id in &chunk_id_pool {
            if !chunk_ids.contains(id) {
                chunk_ids.push(*id);
            }
        }
        let chunks: Vec<ChunkRec> = chunk_ids
            .iter()
            .map(|&id| ChunkRec {
                id,
                source_id: 1,
                nb: 1,
                ord: 0,
                rev: 1,
                text: String::new(),
            })
            .collect();
        let valid: HashSet<u64> = chunk_ids.iter().cloned().collect();
        let mut pool: Vec<u64> = chunk_ids.clone();
        pool.extend([100, 101, 102, 103, 104]);
        let dir = TempDir::new("i25");
        let path = dir.path().join("emb_chunks.bin");
        let mut bytes: Vec<u8> = Vec::new();
        let mut expected: HashMap<u64, Vec<u32>> = HashMap::new();
        for (pick, v) in &body {
            let id = pool[pick % pool.len()];
            bytes.extend_from_slice(&id.to_le_bytes());
            bytes.extend_from_slice(&(v.len() as u32).to_le_bytes());
            for f in v {
                bytes.extend_from_slice(&f.to_le_bytes());
            }
            if valid.contains(&id) {
                expected.insert(id, v.iter().map(|f| f.to_bits()).collect());
            }
        }
        match &tail {
            None => {}
            Some(Tail::Partial(bs)) => bytes.extend_from_slice(bs),
            Some(Tail::DimCut(dim, present)) => {
                bytes.extend_from_slice(&777u64.to_le_bytes());
                bytes.extend_from_slice(&dim.to_le_bytes());
                bytes.extend(std::iter::repeat(0xABu8).take(*present));
            }
        }
        std::fs::write(&path, &bytes).unwrap();
        let got = load_embeddings(&path, &chunks).unwrap();
        let got_bits: HashMap<u64, Vec<u32>> = got
            .iter()
            .map(|(k, v)| (*k, v.iter().map(|f| f.to_bits()).collect()))
            .collect();
        prop_assert_eq!(got_bits, expected);
    }
}
