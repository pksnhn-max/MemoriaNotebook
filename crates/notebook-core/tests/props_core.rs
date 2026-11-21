//! Property suite W1 (integration half): chunking, notebook ledger
//! behaviour, the embedding sidecar format, rev replacement, and
//! index/search — candidates 1-1..1-5, 2-3, 2-4, 2-6, 3-1..3-5 of the
//! annex `nb-t3-props-core` (props-core.md).
//!
//! Candidates 2-1, 2-2, 2-5 target the private helpers `read_jsonl`,
//! `append_jsonl`, `load_embeddings` and live in
//! `src/props_internal.rs` instead (brief placement rule).
//!
//! Every generated property runs at 1,000 cases (brief contract; the
//! annex's 100-case row for 1-4 is raised to 1,000 as well).
//!
//! History: two tests here were the annex's claims taken literally
//! and kept ignored against the old implementation (candidate 1-2
//! chunk cap, candidate 3-5 no-reembedding; see the W1 record book
//! ~/workspace/notebook-design/records/2026-11-21-w1-core-props.md).
//! Brief FB (2026-11-21) settled both: 3-5 by changing the
//! implementation (hash gate — regression flip, the test now runs
//! as `index_source_no_reembedding`), and 1-2 by correcting the
//! documented contract to the implementation's real one
//! (`chunk_length_within_double_target` — a yardstick item,
//! proposed to the human, see the FB record book
//! ~/workspace/notebook-design/records/2026-11-21-fb-core-fix.md).

use notebook_core::{
    chunk_text, embed_chunks, CoreError, CoreResult, CoreState, Embedder, EngineCfg,
    SourceKind, CHUNK_TARGET_CHARS,
};
use proptest::prelude::*;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

// ---------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------

static SEQ: AtomicU64 = AtomicU64::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let n = SEQ.fetch_add(1, Ordering::SeqCst);
        let d = std::env::temp_dir().join(format!("nbprops-{tag}-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        TempDir(d)
    }
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Deterministic embedder: byte-bucket counts. Same text -> same
/// vector, across instances and processes.
#[derive(Clone)]
struct DetEmbedder {
    dim: usize,
}

impl Embedder for DetEmbedder {
    fn embed(&mut self, text: &str) -> CoreResult<Vec<f32>> {
        let mut v = vec![0.0f32; self.dim];
        for (i, b) in text.bytes().enumerate() {
            let h = (b as usize).wrapping_mul(31).wrapping_add(i);
            v[h % self.dim] += 1.0;
        }
        Ok(v)
    }
}

/// DetEmbedder that counts how many texts it was asked to embed.
struct CountingEmbedder {
    inner: DetEmbedder,
    calls: usize,
}

impl Embedder for CountingEmbedder {
    fn embed(&mut self, text: &str) -> CoreResult<Vec<f32>> {
        self.calls += 1;
        self.inner.embed(text)
    }
}

/// Embedder returning pre-set vectors; the chunk text is the decimal
/// index into the table ("0", "1", ...).
struct TableEmbedder {
    table: Vec<Vec<f32>>,
}

impl Embedder for TableEmbedder {
    fn embed(&mut self, text: &str) -> CoreResult<Vec<f32>> {
        let i: usize = text
            .parse()
            .map_err(|_| CoreError::GeneratorError("bad table key".into()))?;
        Ok(self.table[i].clone())
    }
}

// ---------------------------------------------------------------------
// strategies
// ---------------------------------------------------------------------

/// Character pool without newline: sentence punctuation, spaces.
fn char_pool() -> impl Strategy<Value = char> {
    prop_oneof![
        6 => Just('あ'),
        2 => Just('a'),
        1 => Just('。'),
        1 => Just('、'),
        1 => Just(' '),
        1 => Just('x'),
    ]
}

/// Character pool with single newlines mixed in.
fn char_pool_nl() -> impl Strategy<Value = char> {
    prop_oneof![
        6 => Just('あ'),
        2 => Just('a'),
        1 => Just('。'),
        1 => Just('、'),
        1 => Just(' '),
        1 => Just('\n'),
        1 => Just('x'),
    ]
}

fn para_of<S: Strategy<Value = char>>(pool: S, max: usize) -> impl Strategy<Value = String> {
    prop::collection::vec(pool, 0..=max).prop_map(|cs| cs.into_iter().collect())
}

/// A paragraph that survives `chunk_text` unchanged when alone in a
/// source: tight characters only (no whitespace, no newline),
/// 1..=max chars, so it is exactly one chunk and its chunk text is
/// the paragraph itself.
fn single_chunk_para(max: usize) -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop_oneof![Just('あ'), Just('a'), Just('。'), Just('、'), Just('x')],
        1..=max,
    )
    .prop_map(|cs| cs.into_iter().collect())
}

/// Arbitrary source text: a few paragraphs joined by blank lines,
/// some short, some long enough to reach the hard-split path.
fn arb_text() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop_oneof![
            3 => para_of(char_pool_nl(), 120),
            1 => para_of(char_pool_nl(), 1000),
        ],
        0..=5,
    )
    .prop_map(|paras| paras.join("\n\n"))
}

/// The character content `chunk_text` must preserve, in order: the
/// trimmed, non-empty paragraphs of `text` (split on blank lines),
/// with every '\n' removed (packing inserts '\n' separators; single
/// newlines inside a paragraph are content, but both sides of the
/// comparison drop '\n' uniformly).
fn normalized_char_content(text: &str) -> String {
    let mut out = String::new();
    for para in text.split("\n\n") {
        let t = para.trim();
        if t.is_empty() {
            continue;
        }
        out.extend(t.chars().filter(|c| *c != '\n'));
    }
    out
}

fn content_of(chunks: &[String]) -> String {
    chunks
        .concat()
        .chars()
        .filter(|c| *c != '\n')
        .collect()
}

// ---------------------------------------------------------------------
// 1. chunk_text
// ---------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    /// Candidate 1-1, round trip (no hard split): joining the chunks
    /// with '\n' reproduces exactly the trimmed, non-empty paragraphs
    /// joined with '\n'. Chunking may only insert '\n' separators.
    /// A "paragraph" here is a blank-line-separated segment, so the
    /// generated paragraphs are built from non-empty segments joined
    /// by single '\n' — they can contain single newlines but never
    /// the "\n\n" separator itself.
    #[test]
    fn chunk_roundtrip_no_hard_split(
        paras in prop::collection::vec(
            prop::collection::vec(
                prop::collection::vec(char_pool(), 1..=100)
                    .prop_map(|cs| cs.into_iter().collect::<String>()),
                0..=6
            )
            .prop_map(|segs| segs.join("\n")),
            0..=8
        )
    ) {
        let text = paras.join("\n\n");
        let expected: Vec<String> = paras
            .iter()
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect();
        let chunks = chunk_text(&text);
        prop_assert!(chunks.iter().all(|c| !c.is_empty()));
        prop_assert_eq!(chunks.join("\n"), expected.join("\n"));
    }

    /// Candidate 1-1, hard-split leg: one over-long paragraph is cut
    /// into pieces of at most CHUNK_TARGET_CHARS whose concatenation
    /// is exactly the trimmed paragraph (no character lost or added).
    #[test]
    fn chunk_hard_split_concat(
        para in prop::collection::vec(char_pool(), (CHUNK_TARGET_CHARS * 2 + 1)..=2000)
            .prop_map(|cs| cs.into_iter().collect::<String>())
    ) {
        prop_assume!(para.trim().chars().count() > CHUNK_TARGET_CHARS * 2);
        let chunks = chunk_text(&para);
        prop_assert!(chunks.len() >= 2, "long paragraph must be split, got {} chunk(s)", chunks.len());
        prop_assert_eq!(chunks.concat(), para.trim().to_string());
        prop_assert!(chunks
            .iter()
            .all(|c| !c.is_empty() && c.chars().count() <= CHUNK_TARGET_CHARS));
    }

    /// Candidate 1-1, general leg: for arbitrary text the character
    /// content (modulo '\n') is preserved exactly, chunks are
    /// non-empty, and no chunk contains a blank line.
    #[test]
    fn chunk_char_content_preserved(text in arb_text()) {
        let chunks = chunk_text(&text);
        prop_assert_eq!(content_of(&chunks), normalized_char_content(&text));
        prop_assert!(chunks.iter().all(|c| !c.is_empty() && !c.contains("\n\n")));
    }

    /// Candidate 1-2, real-contract form (brief FB decision 3;
    /// yardstick item — the expectation side changed, see the FB
    /// record book): `CHUNK_TARGET_CHARS` is a target, not a cap.
    /// Paragraphs are packed up to the target, a single paragraph
    /// stays whole up to twice the target, and longer paragraphs
    /// are hard-split — so every chunk is at most
    /// `2 * CHUNK_TARGET_CHARS`. (The hard-split pieces' own bound,
    /// at most `CHUNK_TARGET_CHARS`, is asserted by
    /// `chunk_hard_split_concat`.)
    #[test]
    fn chunk_length_within_double_target(text in arb_text()) {
        let chunks = chunk_text(&text);
        for c in &chunks {
            prop_assert!(
                c.chars().count() <= 2 * CHUNK_TARGET_CHARS,
                "chunk of {} chars exceeds twice the target {}",
                c.chars().count(),
                2 * CHUNK_TARGET_CHARS
            );
        }
    }

    /// Candidate 1-3: paragraph order is preserved. Recovering the
    /// paragraphs from the chunks yields the input paragraph list
    /// (trimmed, empties dropped), in order, with no loss or dup.
    #[test]
    fn chunk_paragraph_order(
        paras in prop::collection::vec(para_of(char_pool(), 600), 1..=10)
    ) {
        let text = paras.join("\n\n");
        let expected: Vec<String> = paras
            .iter()
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect();
        let chunks = chunk_text(&text);
        if expected.is_empty() {
            prop_assert!(chunks.is_empty());
        } else {
            let recovered: Vec<String> =
                chunks.join("\n").split('\n').map(|s| s.to_string()).collect();
            prop_assert_eq!(recovered, expected);
        }
    }

    /// Candidate 1-4 (floor raised 100 -> 1,000 by the brief):
    /// whitespace-only input yields no chunks at all.
    #[test]
    fn chunk_blank_inputs_yield_no_chunks(
        s in prop::collection::vec(
            prop_oneof![Just(' '), Just('\n'), Just('\t'), Just('\r'), Just('\u{3000}')],
            0..=64
        )
        .prop_map(|cs| cs.into_iter().collect::<String>())
    ) {
        prop_assert!(chunk_text(&s).is_empty());
    }

    /// Candidate 1-5 (resume equivalence): chunks persisted through
    /// `set_source_text` and reloaded by a fresh `CoreState` are
    /// exactly the chunks `chunk_text` produces for the same text,
    /// and their content still reconstructs the source text.
    #[test]
    fn chunk_reload_resume_equivalence(text in arb_text()) {
        let dir = TempDir::new("c15");
        let (src_id, made_texts) = {
            let mut st = CoreState::new(dir.path(), EngineCfg::default(), "props").unwrap();
            let nb = st.create_notebook("t", 1).unwrap();
            let src = st
                .add_source(nb.id, SourceKind::Text, "s", "/s.txt", 2)
                .unwrap();
            let made = st.set_source_text(src.id, &text).unwrap();
            (
                src.id,
                made.iter().map(|c| c.text.clone()).collect::<Vec<_>>(),
            )
        };
        let st2 = CoreState::new(dir.path(), EngineCfg::default(), "props").unwrap();
        let reloaded: Vec<String> =
            st2.chunks_of(src_id).iter().map(|c| c.text.clone()).collect();
        prop_assert_eq!(&reloaded, &made_texts);
        prop_assert_eq!(&reloaded, &chunk_text(&text));
        prop_assert_eq!(content_of(&reloaded), normalized_char_content(&text));
    }
}

// ---------------------------------------------------------------------
// 2. ledger behaviour through the public surface (2-3, 2-4, 2-6)
// ---------------------------------------------------------------------

#[derive(Debug, Clone)]
enum NbOp {
    Create,
    DeleteLive(usize),
    DeleteUnknown(u64),
}

fn nb_op() -> impl Strategy<Value = NbOp> {
    prop_oneof![
        3 => Just(NbOp::Create),
        2 => (0..16usize).prop_map(NbOp::DeleteLive),
        1 => (0..4u64).prop_map(NbOp::DeleteUnknown),
    ]
}

/// FNV-1a (64-bit), written from the algorithm's definition for
/// the sidecar format check — deliberately not shared with the
/// implementation under test.
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    /// Candidate 2-3: notebook ids are issued 1,2,3,... with no
    /// reuse ever; tombstones in deleted.jsonl are exactly the
    /// deleted ids in deletion order; after a reload the live set is
    /// unchanged and the id counter still does not reuse.
    #[test]
    fn notebook_ids_monotonic_tombstones_persist(
        ops in prop::collection::vec(nb_op(), 0..=16)
    ) {
        let dir = TempDir::new("c23");
        let mut st = CoreState::new(dir.path(), EngineCfg::default(), "props").unwrap();
        let mut live: Vec<u64> = Vec::new();
        let mut deleted: Vec<u64> = Vec::new();
        let mut next_id = 1u64;
        for op in &ops {
            match op {
                NbOp::Create => {
                    let nb = st.create_notebook("t", next_id).unwrap();
                    prop_assert_eq!(nb.id, next_id);
                    live.push(nb.id);
                    next_id += 1;
                }
                NbOp::DeleteLive(i) => {
                    if live.is_empty() {
                        continue;
                    }
                    let id = live.remove(i % live.len());
                    st.delete_notebook(id).unwrap();
                    deleted.push(id);
                }
                NbOp::DeleteUnknown(k) => {
                    let id = next_id + k;
                    let r = st.delete_notebook(id);
                    prop_assert!(
                        matches!(r, Err(CoreError::NotebookNotFound(_))),
                        "expected NotebookNotFound for unknown id {id}"
                    );
                }
            }
        }
        let raw = std::fs::read_to_string(dir.path().join("deleted.jsonl"))
            .unwrap_or_default();
        let file_deleted: Vec<u64> = raw
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| l.trim().parse().unwrap())
            .collect();
        prop_assert_eq!(&file_deleted, &deleted);
        drop(st);
        let mut st2 = CoreState::new(dir.path(), EngineCfg::default(), "props").unwrap();
        let live_after: Vec<u64> = st2.notebooks().iter().map(|n| n.id).collect();
        prop_assert_eq!(live_after, live);
        let nb = st2.create_notebook("after", 999).unwrap();
        prop_assert_eq!(nb.id, next_id);
    }

    /// Candidate 2-4 (format integrity): the records `embed_chunks`
    /// writes to emb_chunks.bin parse back, with an independent
    /// minimal parser written here, to exactly the input pairs —
    /// format v2 (brief FB decision 2): chunk_id u64 LE, dim u32 LE
    /// with the top flag bit set, text hash u64 LE (FNV-1a of the
    /// chunk text, reimplemented below from the algorithm's
    /// definition so the check shares no code with the writer),
    /// f32[dim] LE, no trailing bytes.
    #[test]
    fn emb_sidecar_format_independent_parse(
        table in prop::collection::vec(
            prop::collection::vec(any::<f32>(), 0..=8),
            0..=10
        ),
        ids in prop::collection::vec(any::<u64>(), 0..=10),
    ) {
        let n = table.len().min(ids.len());
        let table: Vec<Vec<f32>> = table.into_iter().take(n).collect();
        let ids: Vec<u64> = ids.into_iter().take(n).collect();
        let dir = TempDir::new("c24");
        let chunks: Vec<(u64, String)> = ids
            .iter()
            .enumerate()
            .map(|(i, id)| (*id, i.to_string()))
            .collect();
        let mut emb = TableEmbedder { table: table.clone() };
        let (pairs, res) = embed_chunks(chunks, &mut emb, dir.path());
        prop_assert_eq!(res.unwrap(), n);
        prop_assert_eq!(pairs.len(), n);
        for (i, (id, v)) in pairs.iter().enumerate() {
            prop_assert_eq!(*id, ids[i]);
            prop_assert_eq!(v.len(), table[i].len());
            for (a, b) in v.iter().zip(table[i].iter()) {
                prop_assert_eq!(a.to_bits(), b.to_bits());
            }
        }
        let bytes = std::fs::read(dir.path().join("emb_chunks.bin")).unwrap_or_default();
        let mut pos = 0usize;
        let mut recs: Vec<(u64, Vec<u32>)> = Vec::new();
        while pos < bytes.len() {
            prop_assert!(pos + 20 <= bytes.len(), "trailing partial header at {pos}");
            let id = u64::from_le_bytes(bytes[pos..pos + 8].try_into().unwrap());
            let dim_word =
                u32::from_le_bytes(bytes[pos + 8..pos + 12].try_into().unwrap());
            prop_assert!(
                dim_word & 0x8000_0000 != 0,
                "v2 record must carry the hash flag (dim word {dim_word:#x})"
            );
            let dim = (dim_word & !0x8000_0000) as usize;
            let hash = u64::from_le_bytes(bytes[pos + 12..pos + 20].try_into().unwrap());
            // The chunk texts in this test are the enumeration
            // index as a string, in record order.
            let expect_hash = fnv1a64(recs.len().to_string().as_bytes());
            prop_assert_eq!(hash, expect_hash, "record {} text hash", recs.len());
            pos += 20;
            prop_assert!(pos + dim * 4 <= bytes.len(), "record overruns file");
            let mut bits = Vec::new();
            for _ in 0..dim {
                bits.push(u32::from_le_bytes(bytes[pos..pos + 4].try_into().unwrap()));
                pos += 4;
            }
            recs.push((id, bits));
        }
        prop_assert_eq!(recs.len(), n);
        for (i, (id, bits)) in recs.iter().enumerate() {
            prop_assert_eq!(*id, ids[i]);
            let expect: Vec<u32> = table[i].iter().map(|f| f.to_bits()).collect();
            prop_assert_eq!(bits, &expect);
        }
    }

    /// Candidate 2-6 (resume equivalence): re-setting a source's text
    /// replaces its chunk set (rev + 1), drops the old chunks and
    /// their embeddings at once, and the picture survives a reload.
    #[test]
    fn set_source_text_rev_replacement(
        text_a in prop::collection::vec(single_chunk_para(300), 1..=4)
            .prop_map(|ps| ps.join("\n\n")),
        text_b in prop::collection::vec(single_chunk_para(300), 1..=4)
            .prop_map(|ps| ps.join("\n\n")),
    ) {
        let dir = TempDir::new("c26");
        let mut st = CoreState::new(dir.path(), EngineCfg::default(), "props").unwrap();
        let nb = st.create_notebook("t", 1).unwrap();
        let src = st
            .add_source(nb.id, SourceKind::Text, "s", "/s.txt", 2)
            .unwrap();
        let mut emb = DetEmbedder { dim: 16 };
        let chunks_a = st.set_source_text(src.id, &text_a).unwrap();
        prop_assert!(!chunks_a.is_empty());
        let rev_a = chunks_a[0].rev;
        st.index_source(src.id, &mut emb).unwrap();
        let probe = emb.embed(&chunks_a[0].text).unwrap();
        prop_assert!(!st.search(nb.id, &probe, 8).is_empty());
        let chunks_b = st.set_source_text(src.id, &text_b).unwrap();
        prop_assert!(!chunks_b.is_empty());
        prop_assert!(chunks_b.iter().all(|c| c.rev == rev_a + 1));
        for c in &chunks_a {
            prop_assert!(st.chunk_by_id(c.id).is_none());
        }
        // Old embeddings are gone and the new set is not indexed yet.
        prop_assert!(st.search(nb.id, &probe, 8).is_empty());
        let texts_b: Vec<String> =
            st.chunks_of(src.id).iter().map(|c| c.text.clone()).collect();
        prop_assert_eq!(&texts_b, &chunk_text(&text_b));
        drop(st);
        let st2 = CoreState::new(dir.path(), EngineCfg::default(), "props").unwrap();
        let texts_b2: Vec<String> =
            st2.chunks_of(src.id).iter().map(|c| c.text.clone()).collect();
        prop_assert_eq!(texts_b2, chunk_text(&text_b));
        for c in &chunks_a {
            prop_assert!(st2.chunk_by_id(c.id).is_none());
        }
        prop_assert!(st2.search(nb.id, &probe, 8).is_empty());
    }
}

// ---------------------------------------------------------------------
// 3. index / search (3-1..3-5)
// ---------------------------------------------------------------------

fn vec_pool() -> impl Strategy<Value = Vec<f32>> {
    prop_oneof![
        Just(vec![1.0, 0.0, 0.0]),
        Just(vec![0.0, 1.0, 0.0]),
        Just(vec![0.0, 0.0, 1.0]),
        Just(vec![1.0, 1.0, 0.0]),
        Just(vec![2.0, 0.0, 0.0]), // same direction as (1,0,0): cosine tie
        Just(vec![-1.0, 0.0, 0.0]),
        Just(vec![0.0, 0.0, 0.0]), // zero norm: ineligible
        Just(vec![1.0, 0.0]),      // dim mismatch for dim-3 queries
        Just(vec![0.5, 0.5, 0.5]),
    ]
}

fn query_pool() -> impl Strategy<Value = Vec<f32>> {
    prop_oneof![
        Just(vec![1.0, 0.0, 0.0]),
        Just(vec![1.0, 1.0, 1.0]),
        Just(vec![2.0, 2.0, 0.0]),
        Just(vec![0.0, 0.0, 0.0]), // zero query: no hits
        Just(vec![1.0, 0.0]),      // dim-2 query
        Just(vec![-1.0, 1.0, 0.0]),
    ]
}

fn path_pool() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("/data/a.txt".to_string()),
        Just("/data/b.txt".to_string()),
        Just("/data/A.txt".to_string()),
        Just("".to_string()),
        Just("rel/path.txt".to_string()),
        Just("/data/a.txt ".to_string()),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    /// Candidate 3-1 (equivalence): indexing via the split path
    /// (index_prepare -> embed_chunks -> index_commit) lands in
    /// exactly the same place as the bulk `index_source`: identical
    /// sidecar bytes and identical search behaviour.
    #[test]
    fn index_split_equals_bulk(
        text in prop::collection::vec(single_chunk_para(300), 1..=5)
            .prop_map(|ps| ps.join("\n\n")),
        probes in prop::collection::vec(para_of(char_pool(), 40), 1..=4),
    ) {
        let dir_a = TempDir::new("c31a");
        let dir_b = TempDir::new("c31b");
        let build = |dir: &std::path::Path| {
            let mut st = CoreState::new(dir, EngineCfg::default(), "props").unwrap();
            let nb = st.create_notebook("t", 1).unwrap();
            let src = st
                .add_source(nb.id, SourceKind::Text, "s", "/s.txt", 2)
                .unwrap();
            st.set_source_text(src.id, &text).unwrap();
            (st, nb.id, src.id)
        };
        let (mut sa, nb_a, src_a) = build(dir_a.path());
        let (mut sb, nb_b, src_b) = build(dir_b.path());
        prop_assert_eq!(nb_a, nb_b);
        prop_assert_eq!(src_a, src_b);
        let mut ea = DetEmbedder { dim: 16 };
        let na = sa.index_source(src_a, &mut ea).unwrap();
        let prepared = sb.index_prepare(src_b).unwrap();
        let mut eb = DetEmbedder { dim: 16 };
        let (pairs, res) = embed_chunks(prepared, &mut eb, dir_b.path());
        sb.index_commit(pairs);
        prop_assert_eq!(na, res.unwrap());
        let bytes_a = std::fs::read(dir_a.path().join("emb_chunks.bin")).unwrap_or_default();
        let bytes_b = std::fs::read(dir_b.path().join("emb_chunks.bin")).unwrap_or_default();
        prop_assert_eq!(bytes_a, bytes_b);
        let mut eq = DetEmbedder { dim: 16 };
        for p in &probes {
            let q = eq.embed(p).unwrap();
            let ha = sa.search(nb_a, &q, 8);
            let hb = sb.search(nb_b, &q, 8);
            prop_assert_eq!(ha.len(), hb.len());
            for ((ca, fa), (cb, fb)) in ha.iter().zip(hb.iter()) {
                prop_assert_eq!(ca.id, cb.id);
                prop_assert_eq!(fa.to_bits(), fb.to_bits());
            }
        }
    }

    /// Candidate 3-2 (format integrity): `search` returns exactly
    /// the eligible chunks (same notebook, embedding of matching
    /// dimension, non-zero norm), ordered by cosine similarity
    /// descending with chunk_id ascending on ties, truncated to
    /// top_k. The expected order is recomputed here from the spec
    /// formula; pool values are small dyadic numbers so f32
    /// arithmetic is exact and ties are genuine.
    #[test]
    fn search_order_and_eligibility(
        vecs in prop::collection::vec(vec_pool(), 1..=8),
        query in query_pool(),
        top_k in 0..=10usize,
    ) {
        let dir = TempDir::new("c32");
        let mut st = CoreState::new(dir.path(), EngineCfg::default(), "props").unwrap();
        let nb = st.create_notebook("t", 1).unwrap();
        let mut chunk_ids = Vec::new();
        for i in 0..vecs.len() {
            let src = st
                .add_source(nb.id, SourceKind::Text, "s", &format!("/s{i}.txt"), 2)
                .unwrap();
            let made = st
                .set_source_text(src.id, &format!("chunk number {i} marker"))
                .unwrap();
            prop_assert_eq!(made.len(), 1);
            chunk_ids.push(made[0].id);
        }
        let pairs: Vec<(u64, Vec<f32>)> =
            chunk_ids.iter().cloned().zip(vecs.iter().cloned()).collect();
        st.index_commit(pairs);
        let hits = st.search(nb.id, &query, top_k);
        let mut expected: Vec<(u64, f32)> = Vec::new();
        if !query.is_empty() && top_k > 0 {
            let qn = query.iter().map(|x| x * x).sum::<f32>().sqrt();
            if qn != 0.0 {
                for (cid, v) in chunk_ids.iter().zip(vecs.iter()) {
                    if v.len() != query.len() {
                        continue;
                    }
                    let vn = v.iter().map(|x| x * x).sum::<f32>().sqrt();
                    if vn == 0.0 {
                        continue;
                    }
                    let dot: f32 =
                        query.iter().zip(v.iter()).map(|(a, b)| a * b).sum();
                    expected.push((*cid, dot / (qn * vn)));
                }
                expected.sort_by(|a, b| {
                    b.1.partial_cmp(&a.1)
                        .unwrap()
                        .then_with(|| a.0.cmp(&b.0))
                });
                expected.truncate(top_k);
            }
        }
        prop_assert_eq!(hits.len(), expected.len());
        for ((chunk, sim), (eid, esim)) in hits.iter().zip(expected.iter()) {
            prop_assert_eq!(chunk.id, *eid);
            prop_assert_eq!(sim.to_bits(), esim.to_bits());
        }
    }

    /// Candidate 3-3 (equivalence / no-fire): a search scoped to one
    /// notebook returns exactly that notebook's indexed chunks —
    /// all of them (no similarity threshold) and none of the other
    /// notebook's.
    #[test]
    fn search_scope_isolation(
        n_a in 1..=4usize,
        n_b in 1..=4usize,
        probe in para_of(char_pool(), 40),
    ) {
        prop_assume!(!probe.is_empty());
        let dir = TempDir::new("c33");
        let mut st = CoreState::new(dir.path(), EngineCfg::default(), "props").unwrap();
        let mut emb = DetEmbedder { dim: 16 };
        let setup = |st: &mut CoreState,
                     emb: &mut DetEmbedder,
                     tag: &str,
                     n: usize|
         -> (u64, Vec<u64>) {
            let nb = st.create_notebook(tag, 1).unwrap();
            let mut ids = Vec::new();
            for i in 0..n {
                let src = st
                    .add_source(nb.id, SourceKind::Text, "s", &format!("/{tag}{i}.txt"), 2)
                    .unwrap();
                let made = st
                    .set_source_text(src.id, &format!("{tag} source {i} content here"))
                    .unwrap();
                st.index_source(src.id, emb).unwrap();
                ids.extend(made.iter().map(|c| c.id));
            }
            (nb.id, ids)
        };
        let (nb_a, ids_a) = setup(&mut st, &mut emb, "alpha", n_a);
        let (nb_b, ids_b) = setup(&mut st, &mut emb, "beta", n_b);
        let q = emb.embed(&probe).unwrap();
        for (nb, ids) in [(nb_a, &ids_a), (nb_b, &ids_b)] {
            let hits = st.search(nb, &q, 1000);
            let got: BTreeSet<u64> = hits.iter().map(|(c, _)| c.id).collect();
            let want: BTreeSet<u64> = ids.iter().cloned().collect();
            prop_assert_eq!(got, want);
            prop_assert!(hits.iter().all(|(c, _)| c.nb == nb));
        }
        prop_assert!(st.search(9_999_999, &q, 10).is_empty());
    }

    /// Candidate 3-4 (equivalence / format integrity): for every
    /// probe path, `scope_for(nb).allows(p)` holds exactly when one
    /// of the notebook's sources has that absolute path, and the
    /// scope's size is the notebook's distinct-path count.
    #[test]
    fn scope_for_matches_source_paths(
        layout in prop::collection::vec(
            prop::collection::vec(path_pool(), 0..=4),
            1..=3
        ),
        probes in prop::collection::vec(path_pool(), 0..=6),
    ) {
        let dir = TempDir::new("c34");
        let mut st = CoreState::new(dir.path(), EngineCfg::default(), "props").unwrap();
        let mut nb_ids = Vec::new();
        for paths in &layout {
            let nb = st.create_notebook("t", 1).unwrap();
            for p in paths {
                st.add_source(nb.id, SourceKind::Text, "s", p, 2).unwrap();
            }
            nb_ids.push(nb.id);
        }
        let mut all_probes: Vec<String> = probes.clone();
        all_probes.push("/nonexistent".to_string());
        for (nb, paths) in nb_ids.iter().zip(layout.iter()) {
            let scope = st.scope_for(*nb);
            let distinct: BTreeSet<&String> = paths.iter().collect();
            prop_assert_eq!(scope.len(), distinct.len());
            for p in &all_probes {
                let expected = paths.iter().any(|x| x == p);
                prop_assert_eq!(scope.allows(p), expected, "path {:?}", p);
            }
            for s in st.sources_of(*nb) {
                prop_assert!(scope.allows(&s.abs_path));
            }
        }
    }

    /// Candidate 3-5, state-equivalence leg: a second `index_source`
    /// call leaves the observable state — search results in memory
    /// and after a reload — unchanged. Its return count follows the
    /// hash-gate contract (brief FB decision 2): the first call
    /// embeds every chunk, the second embeds nothing, where the
    /// pre-gate implementation re-embedded and returned the same
    /// count; the equivalence assertions below are unchanged.
    #[test]
    fn index_source_second_run_state_equivalent(
        text in prop::collection::vec(single_chunk_para(300), 1..=5)
            .prop_map(|ps| ps.join("\n\n")),
    ) {
        let dir = TempDir::new("c35a");
        let mut st = CoreState::new(dir.path(), EngineCfg::default(), "props").unwrap();
        let nb = st.create_notebook("t", 1).unwrap();
        let src = st
            .add_source(nb.id, SourceKind::Text, "s", "/s.txt", 2)
            .unwrap();
        let made = st.set_source_text(src.id, &text).unwrap();
        prop_assert!(!made.is_empty());
        let mut emb = CountingEmbedder {
            inner: DetEmbedder { dim: 16 },
            calls: 0,
        };
        let n1 = st.index_source(src.id, &mut emb).unwrap();
        let probe = DetEmbedder { dim: 16 }.embed(&made[0].text).unwrap();
        let snap = |st: &CoreState| -> Vec<(u64, u32)> {
            st.search(nb.id, &probe, 100)
                .iter()
                .map(|(c, s)| (c.id, s.to_bits()))
                .collect()
        };
        let hits1 = snap(&st);
        prop_assert!(!hits1.is_empty());
        prop_assert_eq!(n1, made.len());
        let n2 = st.index_source(src.id, &mut emb).unwrap();
        prop_assert_eq!(n2, 0, "second run must embed nothing (hash gate)");
        prop_assert_eq!(&hits1, &snap(&st));
        drop(st);
        let st2 = CoreState::new(dir.path(), EngineCfg::default(), "props").unwrap();
        prop_assert_eq!(hits1, snap(&st2));
    }

    /// Candidate 3-5 (no-reembedding form, now the standing
    /// contract): a second `index_source` call on unchanged text
    /// embeds nothing again and the sidecar does not grow — the
    /// hash gate in `index_prepare` withholds chunks whose stored
    /// embedding was computed from the current text. This was the
    /// annex's claim, kept ignored until brief FB (2026-11-21)
    /// changed the implementation to meet it (regression flip).
    #[test]
    fn index_source_no_reembedding(
        text in prop::collection::vec(single_chunk_para(300), 1..=5)
            .prop_map(|ps| ps.join("\n\n")),
    ) {
        let dir = TempDir::new("c35b");
        let mut st = CoreState::new(dir.path(), EngineCfg::default(), "props").unwrap();
        let nb = st.create_notebook("t", 1).unwrap();
        let src = st
            .add_source(nb.id, SourceKind::Text, "s", "/s.txt", 2)
            .unwrap();
        let made = st.set_source_text(src.id, &text).unwrap();
        prop_assert!(!made.is_empty());
        let mut emb = CountingEmbedder {
            inner: DetEmbedder { dim: 16 },
            calls: 0,
        };
        st.index_source(src.id, &mut emb).unwrap();
        let calls_after_first = emb.calls;
        let size_after_first =
            std::fs::metadata(dir.path().join("emb_chunks.bin")).unwrap().len();
        st.index_source(src.id, &mut emb).unwrap();
        prop_assert_eq!(
            emb.calls, calls_after_first,
            "second index_source re-embedded chunks"
        );
        let size_after_second =
            std::fs::metadata(dir.path().join("emb_chunks.bin")).unwrap().len();
        prop_assert_eq!(
            size_after_second, size_after_first,
            "sidecar grew on second index_source"
        );
        let _ = nb;
    }
}
