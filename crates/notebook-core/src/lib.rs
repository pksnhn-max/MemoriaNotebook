//! notebook-core: the shared core of the notebook app.
//!
//! Design 1 (core), first cut:
//! - `CoreState` is split out of the host app's LLM state and owns the
//!   record layer under `data_dir`:
//!   `notebooks.jsonl`, `sources.jsonl`, `turns.jsonl` (append-only JSONL).
//! - The generation seat is `Local` by default. `Api` is an opt-in seat the
//!   user plugs in from settings; its endpoint and model are user-chosen
//!   (BYOK, unrestricted). The API key itself never lives in this crate's
//!   config on disk — the app hands it in at call time from its vault.
//! - Citations are record-level for now (one entry per source record).
//!   The chunk sidecar (`chunks.jsonl`, paragraph units of roughly 400
//!   characters) is in place, and `Citation` can already carry a chunk
//!   id; the embedding sidecar (`emb_chunks.bin`) and chunk-level
//!   retrieval arrive with the retrieval binding.
//! - Retrieval and inference are not wired yet; the async retrieval
//!   contract of the shipped Memoria core (`emb_query_begin` /
//!   `retrieval_select` / `RetrWait`) is the shape this crate will bind to.
//!
//! Ownership contract: `CoreState` is a single-owner value. Mutations
//! take `&mut self`; the host app serializes access (one owner thread,
//! or an external lock). It is not internally synchronized.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

/// Which seat generates chat replies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GenSeat {
    /// On-device llama.cpp. The standard seat.
    Local,
    /// A user-plugged API. Opt-in only, never the default.
    Api,
}

impl Default for GenSeat {
    fn default() -> Self {
        GenSeat::Local
    }
}

/// User-configured API seat. Endpoint and model are the user's free
/// choice; no provider is baked in. The key is *not* stored here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiCfg {
    pub base_url: String,
    pub model: String,
}

/// Engine configuration handed to `CoreState::new`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineCfg {
    pub gen_seat: GenSeat,
    /// Present only when the user has plugged in an API seat.
    pub api: Option<ApiCfg>,
}

impl Default for EngineCfg {
    fn default() -> Self {
        EngineCfg {
            gen_seat: GenSeat::Local,
            api: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notebook {
    pub id: u64,
    pub title: String,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Pdf,
    Web,
    Text,
    Audio,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRec {
    pub id: u64,
    /// Owning notebook.
    pub nb: u64,
    pub kind: SourceKind,
    pub title: String,
    /// Absolute path of the stored source on the device.
    pub abs_path: String,
    pub added_at: u64,
}

/// A citation: the reply leaned on this source record. `chunk_id` is
/// set when the grounding chunk is known (chunk-level citation); older
/// records without the field read back as `None`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Citation {
    pub source_id: u64,
    #[serde(default)]
    pub chunk_id: Option<u64>,
}

/// One chunk of a source's text, in the chunk sidecar (`chunks.jsonl`).
/// Chunks are paragraph units of roughly 400 characters. Re-chunking a
/// source appends a new set with a higher `rev`; readers keep only the
/// newest `rev` per source, so the append-only file stays consistent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkRec {
    pub id: u64,
    pub source_id: u64,
    /// Owning notebook (denormalized for scoped reads).
    pub nb: u64,
    /// Order inside the source, 0-based.
    pub ord: usize,
    /// Revision of the source's chunk set.
    pub rev: u64,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatTurn {
    /// Owning notebook (the `nb` field of design 1).
    pub nb: u64,
    pub role: Role,
    pub text: String,
    /// Empty for user turns and for "not in the records" replies.
    pub citations: Vec<Citation>,
    pub at: u64,
}

/// The retrieval scope of one notebook: the set of absolute paths the
/// search is allowed to touch. Design 1 passes the allowed set itself.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchScope {
    allowed_abs: BTreeSet<String>,
}

impl SearchScope {
    pub fn allows(&self, abs_path: &str) -> bool {
        self.allowed_abs.contains(abs_path)
    }

    pub fn len(&self) -> usize {
        self.allowed_abs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.allowed_abs.is_empty()
    }
}

#[derive(Debug)]
pub enum CoreError {
    Io(io::Error),
    Json(serde_json::Error),
    NotebookNotFound(u64),
    SourceNotFound(u64),
}

impl From<io::Error> for CoreError {
    fn from(e: io::Error) -> Self {
        CoreError::Io(e)
    }
}

impl From<serde_json::Error> for CoreError {
    fn from(e: serde_json::Error) -> Self {
        CoreError::Json(e)
    }
}

impl std::fmt::Display for CoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CoreError::Io(e) => write!(f, "io: {e}"),
            CoreError::Json(e) => write!(f, "json: {e}"),
            CoreError::NotebookNotFound(id) => write!(f, "notebook not found: {id}"),
            CoreError::SourceNotFound(id) => write!(f, "source not found: {id}"),
        }
    }
}

impl std::error::Error for CoreError {}

pub type CoreResult<T> = Result<T, CoreError>;

pub struct CoreState {
    data_dir: PathBuf,
    cfg: EngineCfg,
    local_model_display_name: String,
    notebooks: Vec<Notebook>,
    sources: Vec<SourceRec>,
    /// Latest chunk set per source (older `rev`s in the file are dropped
    /// at load).
    chunks: Vec<ChunkRec>,
    chunk_rev: BTreeMap<u64, u64>,
    next_notebook_id: u64,
    next_source_id: u64,
    next_chunk_id: u64,
}

impl CoreState {
    /// Design 1 interface: `new(data_dir, EngineCfg, local model display name)`.
    pub fn new(
        data_dir: &Path,
        cfg: EngineCfg,
        local_model_display_name: &str,
    ) -> CoreResult<Self> {
        fs::create_dir_all(data_dir)?;
        let notebooks: Vec<Notebook> = read_jsonl(&data_dir.join("notebooks.jsonl"))?;
        let sources: Vec<SourceRec> = read_jsonl(&data_dir.join("sources.jsonl"))?;
        let all_chunks: Vec<ChunkRec> = read_jsonl(&data_dir.join("chunks.jsonl"))?;
        let next_notebook_id = notebooks.iter().map(|n| n.id).max().unwrap_or(0) + 1;
        let next_source_id = sources.iter().map(|s| s.id).max().unwrap_or(0) + 1;
        let next_chunk_id = all_chunks.iter().map(|c| c.id).max().unwrap_or(0) + 1;
        // Keep only the newest revision per source.
        let mut chunk_rev: BTreeMap<u64, u64> = BTreeMap::new();
        for c in &all_chunks {
            let e = chunk_rev.entry(c.source_id).or_insert(0);
            *e = (*e).max(c.rev);
        }
        let chunks: Vec<ChunkRec> = all_chunks
            .into_iter()
            .filter(|c| chunk_rev.get(&c.source_id) == Some(&c.rev))
            .collect();
        Ok(CoreState {
            data_dir: data_dir.to_path_buf(),
            cfg,
            local_model_display_name: local_model_display_name.to_string(),
            notebooks,
            sources,
            chunks,
            chunk_rev,
            next_notebook_id,
            next_source_id,
            next_chunk_id,
        })
    }

    pub fn gen_seat(&self) -> GenSeat {
        self.cfg.gen_seat
    }

    pub fn local_model_display_name(&self) -> &str {
        &self.local_model_display_name
    }

    pub fn notebooks(&self) -> &[Notebook] {
        &self.notebooks
    }

    pub fn sources_of(&self, nb: u64) -> Vec<&SourceRec> {
        self.sources.iter().filter(|s| s.nb == nb).collect()
    }

    pub fn create_notebook(&mut self, title: &str, now: u64) -> CoreResult<Notebook> {
        let nb = Notebook {
            id: self.next_notebook_id,
            title: title.to_string(),
            created_at: now,
            updated_at: now,
        };
        append_jsonl(&self.data_dir.join("notebooks.jsonl"), &nb)?;
        self.next_notebook_id += 1;
        self.notebooks.push(nb.clone());
        Ok(nb)
    }

    pub fn add_source(
        &mut self,
        nb: u64,
        kind: SourceKind,
        title: &str,
        abs_path: &str,
        now: u64,
    ) -> CoreResult<SourceRec> {
        if !self.notebooks.iter().any(|n| n.id == nb) {
            return Err(CoreError::NotebookNotFound(nb));
        }
        let rec = SourceRec {
            id: self.next_source_id,
            nb,
            kind,
            title: title.to_string(),
            abs_path: abs_path.to_string(),
            added_at: now,
        };
        append_jsonl(&self.data_dir.join("sources.jsonl"), &rec)?;
        self.next_source_id += 1;
        self.sources.push(rec.clone());
        Ok(rec)
    }

    /// Chunk a source's full text into the sidecar. Replaces the
    /// source's previous chunk set (a new `rev` is appended; the old
    /// set stops being visible at once and is dropped at next load).
    pub fn set_source_text(
        &mut self,
        source_id: u64,
        text: &str,
    ) -> CoreResult<Vec<ChunkRec>> {
        let src = self
            .sources
            .iter()
            .find(|s| s.id == source_id)
            .ok_or(CoreError::SourceNotFound(source_id))?;
        let nb = src.nb;
        let rev = self.chunk_rev.get(&source_id).copied().unwrap_or(0) + 1;
        let path = self.data_dir.join("chunks.jsonl");
        let mut made = Vec::new();
        for (ord, piece) in chunk_text(text).into_iter().enumerate() {
            let rec = ChunkRec {
                id: self.next_chunk_id,
                source_id,
                nb,
                ord,
                rev,
                text: piece,
            };
            append_jsonl(&path, &rec)?;
            self.next_chunk_id += 1;
            made.push(rec);
        }
        self.chunks.retain(|c| c.source_id != source_id);
        self.chunks.extend(made.iter().cloned());
        self.chunk_rev.insert(source_id, rev);
        Ok(made)
    }

    /// The current chunks of one source, in reading order.
    pub fn chunks_of(&self, source_id: u64) -> Vec<&ChunkRec> {
        let mut v: Vec<&ChunkRec> = self
            .chunks
            .iter()
            .filter(|c| c.source_id == source_id)
            .collect();
        v.sort_by_key(|c| c.ord);
        v
    }

    pub fn append_turn(&self, turn: &ChatTurn) -> CoreResult<()> {
        append_jsonl(&self.data_dir.join("turns.jsonl"), turn)
    }

    pub fn turns_of(&self, nb: u64) -> CoreResult<Vec<ChatTurn>> {
        let all: Vec<ChatTurn> = read_jsonl(&self.data_dir.join("turns.jsonl"))?;
        Ok(all.into_iter().filter(|t| t.nb == nb).collect())
    }

    /// Retrieval scope for one notebook: the allowed absolute-path set,
    /// built from that notebook's sources and nothing else.
    pub fn scope_for(&self, nb: u64) -> SearchScope {
        let mut scope = SearchScope::default();
        for s in self.sources.iter().filter(|s| s.nb == nb) {
            scope.allowed_abs.insert(s.abs_path.clone());
        }
        scope
    }
}

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

fn append_jsonl<T: Serialize>(path: &Path, rec: &T) -> CoreResult<()> {
    let mut f = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(f, "{}", serde_json::to_string(rec)?)?;
    f.sync_all()?;
    Ok(())
}

fn read_jsonl<T: for<'de> Deserialize<'de>>(path: &Path) -> CoreResult<Vec<T>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let f = fs::File::open(path)?;
    let mut out = Vec::new();
    for line in BufReader::new(f).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str(&line) {
            Ok(record) => out.push(record),
            Err(_) => {
                // Skip corrupted line, continue with next
                continue;
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    fn temp_dir() -> PathBuf {
        let n = SEQ.fetch_add(1, Ordering::SeqCst);
        let d = std::env::temp_dir().join(format!("nbcore-{}-{n}", std::process::id()));
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn default_seat_is_local() {
        let dir = temp_dir();
        let st = CoreState::new(&dir, EngineCfg::default(), "local-model").unwrap();
        assert_eq!(st.gen_seat(), GenSeat::Local);
        assert_eq!(st.local_model_display_name(), "local-model");
    }

    #[test]
    fn notebook_and_source_roundtrip_through_disk() {
        let dir = temp_dir();
        {
            let mut st = CoreState::new(&dir, EngineCfg::default(), "m").unwrap();
            let nb = st.create_notebook("川辺の記録", 100).unwrap();
            st.add_source(nb.id, SourceKind::Pdf, "年報", "/data/a.pdf", 101)
                .unwrap();
            st.add_source(nb.id, SourceKind::Web, "百科事典", "/data/b.html", 102)
                .unwrap();
        }
        let st = CoreState::new(&dir, EngineCfg::default(), "m").unwrap();
        assert_eq!(st.notebooks().len(), 1);
        assert_eq!(st.notebooks()[0].title, "川辺の記録");
        assert_eq!(st.sources_of(1).len(), 2);
    }

    #[test]
    fn scope_only_allows_own_notebook_paths() {
        let dir = temp_dir();
        let mut st = CoreState::new(&dir, EngineCfg::default(), "m").unwrap();
        let a = st.create_notebook("A", 1).unwrap();
        let b = st.create_notebook("B", 2).unwrap();
        st.add_source(a.id, SourceKind::Text, "a", "/data/a.txt", 3)
            .unwrap();
        st.add_source(b.id, SourceKind::Text, "b", "/data/b.txt", 4)
            .unwrap();
        let scope = st.scope_for(a.id);
        assert!(scope.allows("/data/a.txt"));
        assert!(!scope.allows("/data/b.txt"));
        assert_eq!(scope.len(), 1);
    }

    #[test]
    fn turns_keep_notebook_and_citations() {
        let dir = temp_dir();
        let mut st = CoreState::new(&dir, EngineCfg::default(), "m").unwrap();
        let nb = st.create_notebook("A", 1).unwrap();
        let src = st
            .add_source(nb.id, SourceKind::Pdf, "s", "/data/s.pdf", 2)
            .unwrap();
        st.append_turn(&ChatTurn {
            nb: nb.id,
            role: Role::Assistant,
            text: "答え".into(),
            citations: vec![Citation {
                source_id: src.id,
                chunk_id: None,
            }],
            at: 3,
        })
        .unwrap();
        let turns = st.turns_of(nb.id).unwrap();
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].citations[0].source_id, src.id);
        assert!(st.turns_of(999).unwrap().is_empty());
    }

    #[test]
    fn chunk_text_packs_paragraphs_near_target() {
        let p = "あ".repeat(150);
        let text = format!("{p}\n\n{p}\n\n{p}\n\n{p}");
        let chunks = chunk_text(&text);
        // 150x2=300 fits, +150 would pass 400 -> 2 chunks of 2 paragraphs.
        assert_eq!(chunks.len(), 2);
        assert!(chunks.iter().all(|c| c.chars().count() <= 301));
        assert_eq!(chunks.join("").matches('あ').count(), 150 * 4);
    }

    #[test]
    fn chunk_text_hard_splits_long_paragraph() {
        let text = "い".repeat(1000);
        let chunks = chunk_text(&text);
        assert!(chunks.len() >= 3);
        assert!(chunks.iter().all(|c| c.chars().count() <= CHUNK_TARGET_CHARS));
        assert_eq!(chunks.join(""), text);
    }

    #[test]
    fn source_text_roundtrip_and_rechunk_replaces() {
        let dir = temp_dir();
        let (nb_id, src_id, first_ids) = {
            let mut st = CoreState::new(&dir, EngineCfg::default(), "m").unwrap();
            let nb = st.create_notebook("A", 1).unwrap();
            let src = st
                .add_source(nb.id, SourceKind::Text, "s", "/data/s.txt", 2)
                .unwrap();
            let made = st
                .set_source_text(src.id, "第一段落。\n\n第二段落。")
                .unwrap();
            assert_eq!(made.len(), 1); // short paragraphs pack into one chunk
            (nb.id, src.id, made.iter().map(|c| c.id).collect::<Vec<_>>())
        };
        {
            let mut st = CoreState::new(&dir, EngineCfg::default(), "m").unwrap();
            let got = st.chunks_of(src_id);
            assert_eq!(got.len(), 1);
            assert_eq!(got[0].nb, nb_id);
            // Re-chunk with different text: old set must vanish at once.
            let p = "う".repeat(300);
            let made = st
                .set_source_text(src_id, &format!("{p}\n\n{p}"))
                .unwrap();
            assert_eq!(made.len(), 2);
            assert!(made.iter().all(|c| !first_ids.contains(&c.id)));
            assert_eq!(st.chunks_of(src_id).len(), 2);
        }
        // After reload, only the newest revision survives.
        let st = CoreState::new(&dir, EngineCfg::default(), "m").unwrap();
        let got = st.chunks_of(src_id);
        assert_eq!(got.len(), 2);
        assert!(got.iter().all(|c| c.rev == 2));
        assert_eq!(got[0].ord, 0);
        assert_eq!(got[1].ord, 1);
    }

    #[test]
    fn set_source_text_missing_source_fails() {
        let dir = temp_dir();
        let mut st = CoreState::new(&dir, EngineCfg::default(), "m").unwrap();
        let r = st.set_source_text(99, "x");
        assert!(matches!(r, Err(CoreError::SourceNotFound(99))));
    }

    #[test]
    fn citation_with_chunk_id_roundtrips() {
        let dir = temp_dir();
        let mut st = CoreState::new(&dir, EngineCfg::default(), "m").unwrap();
        let nb = st.create_notebook("A", 1).unwrap();
        let src = st
            .add_source(nb.id, SourceKind::Text, "s", "/data/s.txt", 2)
            .unwrap();
        let made = st.set_source_text(src.id, "根拠の文。").unwrap();
        st.append_turn(&ChatTurn {
            nb: nb.id,
            role: Role::Assistant,
            text: "答え".into(),
            citations: vec![Citation {
                source_id: src.id,
                chunk_id: Some(made[0].id),
            }],
            at: 3,
        })
        .unwrap();
        let turns = st.turns_of(nb.id).unwrap();
        assert_eq!(turns[0].citations[0].chunk_id, Some(made[0].id));
    }

    #[test]
    fn add_source_to_missing_notebook_fails() {
        let dir = temp_dir();
        let mut st = CoreState::new(&dir, EngineCfg::default(), "m").unwrap();
        let r = st.add_source(42, SourceKind::Text, "x", "/x", 1);
        assert!(matches!(r, Err(CoreError::NotebookNotFound(42))));
    }

    #[test]
    fn read_jsonl_skips_corrupted_lines() {
        let dir = temp_dir();
        let file = dir.join("corrupt.jsonl");
        // Write: valid, corrupted, valid, empty line, corrupted, valid
        fs::write(&file, "{\"id\":1,\"val\":\"a\"}\nnot json\n{\"id\":2,\"val\":\"b\"}\n\n{broken\n{\"id\":3,\"val\":\"c\"}\n").unwrap();

        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
        struct Rec {
            id: u64,
            val: String,
        }

        let recs: Vec<Rec> = read_jsonl(&file).unwrap();
        assert_eq!(recs.len(), 3);
        assert_eq!(recs[0].id, 1);
        assert_eq!(recs[1].id, 2);
        assert_eq!(recs[2].id, 3);
    }

    #[test]
    fn append_then_reload_ids_monotonic_no_duplicates() {
        let dir = temp_dir();
        // First session: create notebook and source
        {
            let mut st = CoreState::new(&dir, EngineCfg::default(), "m").unwrap();
            let nb = st.create_notebook("NB", 10).unwrap();
            assert_eq!(nb.id, 1);
            let src = st.add_source(nb.id, SourceKind::Pdf, "S", "/s.pdf", 11).unwrap();
            assert_eq!(src.id, 1);
            st.append_turn(&ChatTurn {
                nb: nb.id,
                role: Role::User,
                text: "hello".into(),
                citations: vec![],
                at: 12,
            }).unwrap();
        }
        // Second session: reload and continue
        {
            let mut st = CoreState::new(&dir, EngineCfg::default(), "m").unwrap();
            assert_eq!(st.notebooks().len(), 1);
            assert_eq!(st.sources_of(1).len(), 1);
            let nb = st.create_notebook("NB2", 20).unwrap();
            assert_eq!(nb.id, 2); // continues from 1
            let src = st.add_source(nb.id, SourceKind::Text, "S2", "/s2.txt", 21).unwrap();
            assert_eq!(src.id, 2); // continues from 1
            st.append_turn(&ChatTurn {
                nb: nb.id,
                role: Role::Assistant,
                text: "world".into(),
                citations: vec![],
                at: 22,
            }).unwrap();
        }
        // Third session: verify all present and IDs unique
        {
            let st = CoreState::new(&dir, EngineCfg::default(), "m").unwrap();
            let nbs = st.notebooks();
            assert_eq!(nbs.len(), 2);
            let ids: Vec<u64> = nbs.iter().map(|n| n.id).collect();
            assert_eq!(ids, vec![1, 2]);
            let srcs: Vec<_> = st.sources.iter().map(|s| s.id).collect();
            assert_eq!(srcs, vec![1, 2]);
            let turns: Vec<ChatTurn> = read_jsonl(&dir.join("turns.jsonl")).unwrap();
            assert_eq!(turns.len(), 2);
        }
    }
}
