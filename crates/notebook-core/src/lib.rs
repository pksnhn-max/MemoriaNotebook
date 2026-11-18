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
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};

pub mod voice;

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
    GeneratorError(String),
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
            CoreError::GeneratorError(msg) => write!(f, "generator: {msg}"),
        }
    }
}

impl std::error::Error for CoreError {}

pub type CoreResult<T> = Result<T, CoreError>;

/// Trait for text embedders. The real ONNX embedder lives in the host
/// app; this crate only needs the trait and a deterministic fake for tests.
pub trait Embedder {
    /// Embed a single text into a vector. The dimension is embedder-dependent.
    fn embed(&mut self, text: &str) -> CoreResult<Vec<f32>>;
}

/// 生成器トレイト。プロンプトを受け取り、生成されたテキストを返す。
/// 実装は内部状態（会話履歴など）を持つことができるため `&mut self` を取る。
pub trait Generator {
    /// プロンプトからテキストを生成する。
    /// エラー時は `CoreError` を返す。
    fn generate(&mut self, prompt: &str) -> CoreResult<String>;
}

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
    /// Embedding vectors for chunks that have been indexed.
    /// chunk_id -> vector (f32)
    embeddings: HashMap<u64, Vec<f32>>,
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
        let all_notebooks: Vec<Notebook> = read_jsonl(&data_dir.join("notebooks.jsonl"))?;
        // The id counter runs over ALL notebooks ever created,
        // including deleted ones, so ids are never reused.
        let next_notebook_id = all_notebooks.iter().map(|n| n.id).max().unwrap_or(0) + 1;
        // Tombstones: ids in deleted.jsonl are dropped from the live
        // list (the records themselves stay in their files).
        let deleted: Vec<u64> = read_jsonl(&data_dir.join("deleted.jsonl"))?;
        let notebooks: Vec<Notebook> = all_notebooks
            .into_iter()
            .filter(|n| !deleted.contains(&n.id))
            .collect();
        let sources: Vec<SourceRec> = read_jsonl(&data_dir.join("sources.jsonl"))?;
        let all_chunks: Vec<ChunkRec> = read_jsonl(&data_dir.join("chunks.jsonl"))?;
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

        // Load embedding sidecar.
        let embeddings = load_embeddings(&data_dir.join("emb_chunks.bin"), &chunks)?;

        Ok(CoreState {
            data_dir: data_dir.to_path_buf(),
            cfg,
            local_model_display_name: local_model_display_name.to_string(),
            notebooks,
            sources,
            chunks,
            chunk_rev,
            embeddings,
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

    /// Tombstone a notebook: append its id to `deleted.jsonl` and
    /// drop it from the live list. Its sources/turns stay in their
    /// files but become unreachable. Unknown ids are an error.
    pub fn delete_notebook(&mut self, id: u64) -> CoreResult<()> {
        let pos = self
            .notebooks
            .iter()
            .position(|n| n.id == id)
            .ok_or(CoreError::NotebookNotFound(id))?;
        append_jsonl(&self.data_dir.join("deleted.jsonl"), &id)?;
        self.notebooks.remove(pos);
        Ok(())
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
        // Remove old chunks and their embeddings.
        let old_chunk_ids: Vec<u64> = self
            .chunks
            .iter()
            .filter(|c| c.source_id == source_id)
            .map(|c| c.id)
            .collect();
        for id in old_chunk_ids {
            self.embeddings.remove(&id);
        }
        self.chunks.retain(|c| c.source_id != source_id);
        self.chunks.extend(made.iter().cloned());
        self.chunk_rev.insert(source_id, rev);
        Ok(made)
    }

    /// One current chunk by its id, if it still exists. Only the
    /// newest revision per source is kept in memory, so an id from
    /// a replaced chunk set reads back as `None`.
    pub fn chunk_by_id(&self, chunk_id: u64) -> Option<&ChunkRec> {
        self.chunks.iter().find(|c| c.id == chunk_id)
    }

    /// The notebook's source text for the audio-overview script: the
    /// current chunks of every source of the notebook, in source and
    /// reading order, joined and truncated to `cap` characters.
    pub fn notebook_source_text(&self, nb: u64, cap: usize) -> String {
        let mut out = String::new();
        for src in self.sources.iter().filter(|s| s.nb == nb) {
            for chunk in self.chunks_of(src.id) {
                if out.chars().count() >= cap {
                    return out.chars().take(cap).collect();
                }
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(&chunk.text);
            }
        }
        out.chars().take(cap).collect()
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

    /// Embed all current chunks of `source_id` using `embedder`, append
    /// vectors to the sidecar, and keep them in memory. Returns the
    /// number of chunks embedded.
    pub fn index_source(
        &mut self,
        source_id: u64,
        embedder: &mut dyn Embedder,
    ) -> CoreResult<usize> {
        if !self.sources.iter().any(|s| s.id == source_id) {
            return Err(CoreError::SourceNotFound(source_id));
        }
        let chunks: Vec<(u64, String)> = self
            .chunks_of(source_id)
            .into_iter()
            .map(|c| (c.id, c.text.clone()))
            .collect();
        if chunks.is_empty() {
            return Ok(0);
        }
        let path = self.data_dir.join("emb_chunks.bin");
        let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
        let mut count = 0;
        for (chunk_id, chunk_text) in chunks {
            let vec = embedder.embed(&chunk_text)?;
            // Write record: chunk_id (u64 LE), dim (u32 LE), f32[dim] LE
            file.write_all(&chunk_id.to_le_bytes())?;
            let dim = vec.len() as u32;
            file.write_all(&dim.to_le_bytes())?;
            for v in &vec {
                file.write_all(&v.to_le_bytes())?;
            }
            file.flush()?;
            self.embeddings.insert(chunk_id, vec);
            count += 1;
        }
        file.sync_all()?;
        Ok(count)
    }

    /// Search within notebook `nb` for chunks similar to `query` vector.
    /// Only considers chunks belonging to `nb` that have an embedding
    /// of matching dimension. Returns top_k (chunk, cosine_similarity)
    /// sorted by similarity descending, then chunk_id ascending.
    pub fn search(
        &self,
        nb: u64,
        query: &[f32],
        top_k: usize,
    ) -> Vec<(ChunkRec, f32)> {
        if query.is_empty() || top_k == 0 {
            return Vec::new();
        }
        let query_norm = l2_norm(query);
        if query_norm == 0.0 {
            return Vec::new();
        }
        let mut results = Vec::new();
        for chunk in self.chunks.iter().filter(|c| c.nb == nb) {
            if let Some(vec) = self.embeddings.get(&chunk.id) {
                if vec.len() != query.len() {
                    continue;
                }
                let vec_norm = l2_norm(vec);
                if vec_norm == 0.0 {
                    continue;
                }
                let dot: f32 = query.iter().zip(vec.iter()).map(|(a, b)| a * b).sum();
                let sim = dot / (query_norm * vec_norm);
                results.push((chunk.clone(), sim));
            }
        }
        results.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.id.cmp(&b.0.id))
        });
        results.truncate(top_k);
        results
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

    /// Ask one question against notebook `nb`: embed the question,
    /// retrieve grounding chunks, build the citation-bound prompt,
    /// generate, validate citations, and record both turns. On any
    /// embedder or generator error nothing is recorded.
    ///
    /// Citation gate (ruled 2026-11-18): when grounding chunks exist
    /// but the generated answer carries no valid citation, the prompt
    /// is rebuilt once, sterner, and generation retried; if the retry
    /// also yields no valid citation, the recorded answer is the
    /// fixed refusal 「資料にありません。」. The gate counts validated
    /// citations only — never the model's own claim of grounding.
    pub fn ask(
        &mut self,
        nb: u64,
        question: &str,
        embedder: &mut dyn Embedder,
        generator: &mut dyn Generator,
        top_k: usize,
        at: u64,
    ) -> CoreResult<ChatTurn> {
        if !self.notebooks.iter().any(|n| n.id == nb) {
            return Err(CoreError::NotebookNotFound(nb));
        }
        // 1. 質問を埋め込み
        let qvec = embedder.embed(question)?;

        // 2. 根拠を引く
        let hits = self.search(nb, &qvec, top_k);

        // 3. プロンプトを組む
        let grounding = if hits.is_empty() {
            "（該当する資料なし）".to_string()
        } else {
            format_grounding(&hits)
        };

        let prompt = format!(
            "あなたはノートの資料だけに基づいて答えるアシスタントです。資料に無いことは「資料にありません」と答えてください。資料の各断片には [#番号] の印が付いています。主張の根拠にした断片の印を、文の末尾にそのまま書いてください。\n\n資料:\n{}\n\n質問:\n{}",
            grounding, question
        );

        // 4. 生成
        let mut generated_text = generator.generate(&prompt)?;

        // 5. 検証（parse_citations）
        let mut citations = parse_citations(&generated_text, &hits);

        // 5b. 出典の門：根拠の断片があるのに有効な出典が一つも無い
        // 答えは、そのまま通さない。一度だけプロンプトを組み直して
        // 再生成し、それでも出典が無ければ固定の拒否文を答えとする。
        if citations.is_empty() && !hits.is_empty() {
            let retry_prompt = format!(
                "前回の回答は出典の印が一つも無かったため却下されました。資料の断片だけを使って答え直してください。主張する文の末尾には、根拠にした断片の印 [#番号] を必ずそのまま書いてください。断片の中に答えが無ければ「資料にありません」とだけ答えてください。\n\n資料:\n{}\n\n質問:\n{}",
                grounding, question
            );
            generated_text = generator.generate(&retry_prompt)?;
            citations = parse_citations(&generated_text, &hits);
            if citations.is_empty() {
                generated_text = "資料にありません。".to_string();
            }
        }

        // 6. ターン記録（ユーザー → アシスタント）
        let user_turn = ChatTurn {
            nb,
            role: Role::User,
            text: question.to_string(),
            citations: vec![],
            at,
        };
        self.append_turn(&user_turn)?;

        let assistant_turn = ChatTurn {
            nb,
            role: Role::Assistant,
            text: generated_text,
            citations,
            at,
        };
        self.append_turn(&assistant_turn)?;

        Ok(assistant_turn)
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

/// Load embeddings from the binary sidecar. Returns a map of chunk_id -> vector.
/// Only keeps vectors for chunk_ids that exist in `current_chunks`.
/// Stops reading at the first truncated/corrupted record.
fn load_embeddings(
    path: &Path,
    current_chunks: &[ChunkRec],
) -> CoreResult<HashMap<u64, Vec<f32>>> {
    let mut map = HashMap::new();
    if !path.exists() {
        return Ok(map);
    }
    let mut file = fs::File::open(path)?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)?;
    let mut pos = 0;
    let valid_ids: std::collections::HashSet<u64> = current_chunks.iter().map(|c| c.id).collect();
    while pos + 8 + 4 <= buf.len() {
        let chunk_id = u64::from_le_bytes(buf[pos..pos + 8].try_into().unwrap());
        pos += 8;
        let dim = u32::from_le_bytes(buf[pos..pos + 4].try_into().unwrap()) as usize;
        pos += 4;
        if pos + dim * 4 > buf.len() {
            // Truncated record; stop here.
            break;
        }
        let mut vec = Vec::with_capacity(dim);
        for _ in 0..dim {
            let v = f32::from_le_bytes(buf[pos..pos + 4].try_into().unwrap());
            pos += 4;
            vec.push(v);
        }
        if valid_ids.contains(&chunk_id) {
            map.insert(chunk_id, vec);
        }
    }
    Ok(map)
}

fn l2_norm(v: &[f32]) -> f32 {
    v.iter().map(|x| x * x).sum::<f32>().sqrt()
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

    // Deterministic fake embedder: hash each word, use as seed for a simple PRNG.
    struct FakeEmbedder {
        dim: usize,
    }

    impl FakeEmbedder {
        fn new(dim: usize) -> Self {
            Self { dim }
        }
    }

    impl Embedder for FakeEmbedder {
        fn embed(&mut self, text: &str) -> CoreResult<Vec<f32>> {
            // Bag-of-words feature hashing: each word votes into one
            // dimension, so texts sharing words get similar vectors.
            let mut vec = vec![0.0f32; self.dim];
            for word in text.split_whitespace() {
                let mut h: u64 = 0xcbf29ce484222325;
                for b in word.as_bytes() {
                    h ^= *b as u64;
                    h = h.wrapping_mul(0x100000001b3);
                }
                vec[(h % self.dim as u64) as usize] += 1.0;
            }
            // Normalize
            let norm = l2_norm(&vec);
            if norm > 0.0 {
                for v in &mut vec {
                    *v /= norm;
                }
            }
            Ok(vec)
        }
    }

    #[test]
    fn delete_notebook_tombstones_and_never_reuses_id() {
        let dir = temp_dir();
        let mut st = CoreState::new(&dir, EngineCfg::default(), "m").unwrap();
        let a = st.create_notebook("A", 1).unwrap();
        let b = st.create_notebook("B", 2).unwrap();
        st.delete_notebook(b.id).unwrap();
        assert_eq!(st.notebooks().len(), 1);
        assert!(st.delete_notebook(999).is_err());
        drop(st);
        // After reload the tombstone still applies, and the next id
        // does not reuse the deleted notebook's id.
        let mut st2 = CoreState::new(&dir, EngineCfg::default(), "m").unwrap();
        assert_eq!(st2.notebooks().len(), 1);
        assert_eq!(st2.notebooks()[0].id, a.id);
        let c = st2.create_notebook("C", 3).unwrap();
        assert_eq!(c.id, b.id + 1);
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
    fn chunk_by_id_hits_current_and_misses_replaced() {
        let dir = temp_dir();
        let mut st = CoreState::new(&dir, EngineCfg::default(), "m").unwrap();
        let nb = st.create_notebook("A", 1).unwrap();
        let src = st
            .add_source(nb.id, SourceKind::Text, "s", "/data/s.txt", 2)
            .unwrap();
        let made = st.set_source_text(src.id, "根拠の文。").unwrap();
        let hit = st.chunk_by_id(made[0].id).unwrap();
        assert_eq!(hit.text, "根拠の文。");
        assert_eq!(hit.source_id, src.id);
        assert!(st.chunk_by_id(999_999).is_none());
        // Re-chunking replaces the set: the old id must stop resolving.
        st.set_source_text(src.id, "新しい文。").unwrap();
        assert!(st.chunk_by_id(made[0].id).is_none());
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

    // --- New embedding / search tests ---

    #[test]
    fn index_and_search_returns_target_chunk_first() {
        let dir = temp_dir();
        let (nb_id, src_id, chunk_ids) = {
            let mut st = CoreState::new(&dir, EngineCfg::default(), "m").unwrap();
            let nb = st.create_notebook("NB", 1).unwrap();
            let src = st.add_source(nb.id, SourceKind::Text, "src", "/s.txt", 2).unwrap();
            let made = st.set_source_text(src.id, &format!("{}\n\n{}", "りんご みかん ばなな ".repeat(25), "さくら もみじ いちょう ".repeat(25))).unwrap();
            assert_eq!(made.len(), 2);
            (nb.id, src.id, made.iter().map(|c| c.id).collect::<Vec<_>>())
        };
        {
            let mut st = CoreState::new(&dir, EngineCfg::default(), "m").unwrap();
            let mut embedder = FakeEmbedder::new(32);
            let count = st.index_source(src_id, &mut embedder).unwrap();
            assert_eq!(count, 2);

            // Query with text similar to first chunk
            let query_vec = embedder.embed("りんご みかん").unwrap();
            let results = st.search(nb_id, &query_vec, 2);
            assert_eq!(results.len(), 2);
            // First chunk should be top
            assert_eq!(results[0].0.id, chunk_ids[0]);
            assert!(results[0].1 > results[1].1);
        }
    }

    #[test]
    fn search_scoped_to_notebook_excludes_other_notebook() {
        let dir = temp_dir();
        let (nb1_id, nb2_id, src1_id, src2_id) = {
            let mut st = CoreState::new(&dir, EngineCfg::default(), "m").unwrap();
            let nb1 = st.create_notebook("NB1", 1).unwrap();
            let nb2 = st.create_notebook("NB2", 2).unwrap();
            let src1 = st.add_source(nb1.id, SourceKind::Text, "s1", "/s1.txt", 3).unwrap();
            let src2 = st.add_source(nb2.id, SourceKind::Text, "s2", "/s2.txt", 4).unwrap();
            st.set_source_text(src1.id, "共通 キーワード だけ").unwrap();
            st.set_source_text(src2.id, "共通 キーワード だけ").unwrap();
            (nb1.id, nb2.id, src1.id, src2.id)
        };
        {
            let mut st = CoreState::new(&dir, EngineCfg::default(), "m").unwrap();
            let mut embedder = FakeEmbedder::new(32);
            st.index_source(src1_id, &mut embedder).unwrap();
            st.index_source(src2_id, &mut embedder).unwrap();

            let query_vec = embedder.embed("共通 キーワード").unwrap();
            // Search in NB1 should only return NB1's chunk
            let results1 = st.search(nb1_id, &query_vec, 5);
            assert_eq!(results1.len(), 1);
            assert_eq!(results1[0].0.nb, nb1_id);

            // Search in NB2 should only return NB2's chunk
            let results2 = st.search(nb2_id, &query_vec, 5);
            assert_eq!(results2.len(), 1);
            assert_eq!(results2[0].0.nb, nb2_id);
        }
    }

    #[test]
    fn embeddings_survive_reload() {
        let dir = temp_dir();
        let (nb_id, src_id, chunk_id) = {
            let mut st = CoreState::new(&dir, EngineCfg::default(), "m").unwrap();
            let nb = st.create_notebook("NB", 1).unwrap();
            let src = st.add_source(nb.id, SourceKind::Text, "s", "/s.txt", 2).unwrap();
            let made = st.set_source_text(src.id, "持続 する ベクトル").unwrap();
            let mut embedder = FakeEmbedder::new(16);
            st.index_source(src.id, &mut embedder).unwrap();
            (nb.id, src.id, made[0].id)
        };
        // Reload
        {
            let mut st = CoreState::new(&dir, EngineCfg::default(), "m").unwrap();
            let mut embedder = FakeEmbedder::new(16);
            let query_vec = embedder.embed("持続 する ベクトル").unwrap();
            let results = st.search(nb_id, &query_vec, 1);
            assert_eq!(results.len(), 1);
            assert_eq!(results[0].0.id, chunk_id);
            assert!(results[0].1 > 0.9); // high similarity since same embedder
        }
    }

    #[test]
    fn rechunk_removes_old_chunks_from_search() {
        let dir = temp_dir();
        let (nb_id, src_id) = {
            let mut st = CoreState::new(&dir, EngineCfg::default(), "m").unwrap();
            let nb = st.create_notebook("NB", 1).unwrap();
            let src = st.add_source(nb.id, SourceKind::Text, "s", "/s.txt", 2).unwrap();
            st.set_source_text(src.id, "古い テキスト だけ").unwrap();
            let mut embedder = FakeEmbedder::new(16);
            st.index_source(src.id, &mut embedder).unwrap();
            (nb.id, src.id)
        };
        // Re-chunk with new text
        {
            let mut st = CoreState::new(&dir, EngineCfg::default(), "m").unwrap();
            let new_text = "完全 に 新しい 文章 です";
            st.set_source_text(src_id, new_text).unwrap();
            let mut embedder = FakeEmbedder::new(16);
            st.index_source(src_id, &mut embedder).unwrap();

            // Search with old text should not match well
            let old_query = embedder.embed("古い テキスト").unwrap();
            let results = st.search(nb_id, &old_query, 5);
            // New chunks exist but similarity should be low
            assert!(!results.is_empty());
            // The top result should be from new text (only chunks available)
            // but similarity should be low since content differs
            assert!(results[0].1 < 0.5);
        }
        // Reload and verify old chunks are gone
        {
            let st = CoreState::new(&dir, EngineCfg::default(), "m").unwrap();
            let chunks = st.chunks_of(src_id);
            assert_eq!(chunks.len(), 1);
            assert_eq!(chunks[0].text, "完全 に 新しい 文章 です");
        }
    }

    #[test]
    fn index_source_missing_source_fails() {
        let dir = temp_dir();
        let mut st = CoreState::new(&dir, EngineCfg::default(), "m").unwrap();
        let mut embedder = FakeEmbedder::new(8);
        let r = st.index_source(999, &mut embedder);
        assert!(matches!(r, Err(CoreError::SourceNotFound(999))));
    }

    #[test]
    fn search_handles_dimension_mismatch_gracefully() {
        let dir = temp_dir();
        let (nb_id, src_id) = {
            let mut st = CoreState::new(&dir, EngineCfg::default(), "m").unwrap();
            let nb = st.create_notebook("NB", 1).unwrap();
            let src = st.add_source(nb.id, SourceKind::Text, "s", "/s.txt", 2).unwrap();
            st.set_source_text(src.id, "テスト").unwrap();
            let mut embedder = FakeEmbedder::new(16);
            st.index_source(src.id, &mut embedder).unwrap();
            (nb.id, src.id)
        };
        // Reload with different dimension embedder (should not crash, just skip)
        {
            let st = CoreState::new(&dir, EngineCfg::default(), "m").unwrap();
            // Query with different dimension (32 vs stored 16)
            let query_vec = vec![0.1f32; 32];
            let results = st.search(nb_id, &query_vec, 5);
            // Should return empty because dimension mismatch skips the chunk
            assert!(results.is_empty());
        }
    }

    #[test]
    fn search_empty_query_returns_empty() {
        let dir = temp_dir();
        let mut st = CoreState::new(&dir, EngineCfg::default(), "m").unwrap();
        let nb = st.create_notebook("NB", 1).unwrap();
        let src = st.add_source(nb.id, SourceKind::Text, "s", "/s.txt", 2).unwrap();
        st.set_source_text(src.id, "テスト").unwrap();
        let mut embedder = FakeEmbedder::new(16);
        st.index_source(src.id, &mut embedder).unwrap();

        let results = st.search(nb.id, &[], 5);
        assert!(results.is_empty());

        let results = st.search(nb.id, &[0.1, 0.2], 0);
        assert!(results.is_empty());
    }

    #[test]
    fn search_sorts_by_similarity_then_chunk_id() {
        let dir = temp_dir();
        let (nb_id, chunk_ids) = {
            let mut st = CoreState::new(&dir, EngineCfg::default(), "m").unwrap();
            let nb = st.create_notebook("NB", 1).unwrap();
            let src = st.add_source(nb.id, SourceKind::Text, "s", "/s.txt", 2).unwrap();
            // Create 3 chunks with distinct content
            let made = st.set_source_text(src.id, &format!("{}\n\n{}\n\n{}", "aaa ".repeat(150), "bbb ".repeat(150), "ccc ".repeat(150))).unwrap();
            let mut embedder = FakeEmbedder::new(16);
            st.index_source(src.id, &mut embedder).unwrap();
            (nb.id, made.iter().map(|c| c.id).collect::<Vec<_>>())
        };
        {
            let st = CoreState::new(&dir, EngineCfg::default(), "m").unwrap();
            // Query matches all similarly (fake embedder produces similar vectors for similar words)
            // But we can test tie-breaking by chunk_id with a query that gives equal similarity
            // Since fake embedder is deterministic, we just verify ordering is stable
            let mut embedder = FakeEmbedder::new(16);
            let query = embedder.embed("aaa bbb ccc").unwrap();
            let results = st.search(nb_id, &query, 10);
            assert_eq!(results.len(), 3);
            // Results should be sorted by similarity desc, then chunk_id asc
            for i in 1..results.len() {
                if (results[i-1].1 - results[i].1).abs() < 1e-6 {
                    assert!(results[i-1].0.id < results[i].0.id);
                } else {
                    assert!(results[i-1].1 >= results[i].1);
                }
            }
        }
    }

    // --- Grounding / citation tests ---

    #[test]
    fn format_grounding_includes_chunk_id_and_text() {
        let dir = temp_dir();
        let mut st = CoreState::new(&dir, EngineCfg::default(), "m").unwrap();
        let nb = st.create_notebook("NB", 1).unwrap();
        let src = st.add_source(nb.id, SourceKind::Text, "s", "/s.txt", 2).unwrap();
        let made = st.set_source_text(src.id, &format!("{}\n\n{}", "第一の根拠。".repeat(60), "第二の根拠。".repeat(60))).unwrap();
        assert_eq!(made.len(), 2);

        let hits = vec![
            (made[0].clone(), 0.9),
            (made[1].clone(), 0.7),
        ];
        let grounded = format_grounding(&hits);
        assert!(grounded.contains(&format!("[#{}]", made[0].id)));
        assert!(grounded.contains(&format!("[#{}]", made[1].id)));
        assert!(grounded.contains("第一の根拠"));
        assert!(grounded.contains("第二の根拠"));
        // Blank line between
        assert!(grounded.contains("\n\n"));
        // Scores not included
        assert!(!grounded.contains("0.9"));
        assert!(!grounded.contains("0.7"));
    }

    #[test]
    fn format_grounding_empty_input_returns_empty() {
        let grounded = format_grounding(&[]);
        assert_eq!(grounded, "");
    }

    #[test]
    fn parse_citations_picks_existing_in_order_dedup() {
        let dir = temp_dir();
        let mut st = CoreState::new(&dir, EngineCfg::default(), "m").unwrap();
        let nb = st.create_notebook("NB", 1).unwrap();
        let src = st.add_source(nb.id, SourceKind::Text, "s", "/s.txt", 2).unwrap();
        let made = st.set_source_text(src.id, &format!("{}\n\n{}\n\n{}", "A。".repeat(150), "B。".repeat(150), "C。".repeat(150))).unwrap();
        assert_eq!(made.len(), 3);

        let hits = vec![
            (made[0].clone(), 0.9),
            (made[1].clone(), 0.8),
            (made[2].clone(), 0.7),
        ];

        // Text with citations in order, with duplicate
        let text = "答え[#1]です[#3]ね[#1]";
        let cits = parse_citations(text, &hits);
        assert_eq!(cits.len(), 2);
        assert_eq!(cits[0].chunk_id, Some(made[0].id));
        assert_eq!(cits[1].chunk_id, Some(made[2].id));
        assert_eq!(cits[0].source_id, src.id);
        assert_eq!(cits[1].source_id, src.id);
    }

    #[test]
    fn parse_citations_drops_unknown_chunk_ids() {
        let dir = temp_dir();
        let mut st = CoreState::new(&dir, EngineCfg::default(), "m").unwrap();
        let nb = st.create_notebook("NB", 1).unwrap();
        let src = st.add_source(nb.id, SourceKind::Text, "s", "/s.txt", 2).unwrap();
        let made = st.set_source_text(src.id, "のみ").unwrap();

        let hits = vec![(made[0].clone(), 0.9)];
        // Citation refers to non-existent chunk 999
        let text = "答え[#999]です";
        let cits = parse_citations(text, &hits);
        assert!(cits.is_empty());
    }

    #[test]
    fn parse_citations_ignores_non_numeric_markers() {
        let dir = temp_dir();
        let mut st = CoreState::new(&dir, EngineCfg::default(), "m").unwrap();
        let nb = st.create_notebook("NB", 1).unwrap();
        let src = st.add_source(nb.id, SourceKind::Text, "s", "/s.txt", 2).unwrap();
        let made = st.set_source_text(src.id, "テスト").unwrap();

        let hits = vec![(made[0].clone(), 0.9)];
        // Non-numeric marker should be ignored
        let text = "答え[#abc]です[#1]";
        let cits = parse_citations(text, &hits);
        assert_eq!(cits.len(), 1);
        assert_eq!(cits[0].chunk_id, Some(made[0].id));
    }

    #[test]
    fn citation_roundtrip_append_and_reload() {
        let dir = temp_dir();
        let (nb_id, src_id, chunk_ids) = {
            let mut st = CoreState::new(&dir, EngineCfg::default(), "m").unwrap();
            let nb = st.create_notebook("NB", 1).unwrap();
            let src = st.add_source(nb.id, SourceKind::Text, "s", "/s.txt", 2).unwrap();
            let made = st.set_source_text(src.id, &format!("{}\n\n{}", "根拠A。".repeat(100), "根拠B。".repeat(100))).unwrap();
            (nb.id, src.id, made.iter().map(|c| c.id).collect::<Vec<_>>())
        };
        // Append a turn with parsed citations
        {
            let mut st = CoreState::new(&dir, EngineCfg::default(), "m").unwrap();
            let hits = vec![
                (st.chunks.iter().find(|c| c.id == chunk_ids[0]).unwrap().clone(), 0.9),
                (st.chunks.iter().find(|c| c.id == chunk_ids[1]).unwrap().clone(), 0.8),
            ];
            let text = "回答[#1]です[#2]";
            let cits = parse_citations(text, &hits);
            st.append_turn(&ChatTurn {
                nb: nb_id,
                role: Role::Assistant,
                text: text.into(),
                citations: cits,
                at: 3,
            }).unwrap();
        }
        // Reload and verify
        {
            let st = CoreState::new(&dir, EngineCfg::default(), "m").unwrap();
            let turns = st.turns_of(nb_id).unwrap();
            assert_eq!(turns.len(), 1);
            assert_eq!(turns[0].citations.len(), 2);
            assert_eq!(turns[0].citations[0].chunk_id, Some(chunk_ids[0]));
            assert_eq!(turns[0].citations[1].chunk_id, Some(chunk_ids[1]));
            assert_eq!(turns[0].citations[0].source_id, src_id);
            assert_eq!(turns[0].citations[1].source_id, src_id);
        }
    }
    // --- Stage 5: Generator seat / ask flow ---

    struct RecordingGenerator {
        captured: std::sync::Arc<std::sync::Mutex<Option<String>>>,
    }

    impl Generator for RecordingGenerator {
        fn generate(&mut self, prompt: &str) -> CoreResult<String> {
            *self.captured.lock().unwrap() = Some(prompt.to_string());
            // Cite the first numeric [#n] marker found in the prompt,
            // and mix in a bogus one that validation must drop.
            let bytes = prompt.as_bytes();
            let mut first: Option<u64> = None;
            let mut i = 0;
            while i + 2 < bytes.len() {
                if bytes[i] == b'[' && bytes[i + 1] == b'#' {
                    let mut j = i + 2;
                    let mut num = String::new();
                    while j < bytes.len() && bytes[j].is_ascii_digit() {
                        num.push(bytes[j] as char);
                        j += 1;
                    }
                    if j < bytes.len() && bytes[j] == b']' && !num.is_empty() {
                        first = num.parse().ok();
                        break;
                    }
                    i = j;
                } else {
                    i += 1;
                }
            }
            match first {
                Some(id) => Ok(format!("答えです[#{id}]。無い印[#9999]も混ぜる。")),
                None => Ok("資料にありません。".to_string()),
            }
        }
    }

    struct FixedGenerator(&'static str);

    impl Generator for FixedGenerator {
        fn generate(&mut self, _prompt: &str) -> CoreResult<String> {
            Ok(self.0.to_string())
        }
    }

    struct ErrorGenerator;

    impl Generator for ErrorGenerator {
        fn generate(&mut self, _prompt: &str) -> CoreResult<String> {
            Err(CoreError::GeneratorError("boom".into()))
        }
    }

    /// Returns queued answers in order; counts the calls so gate
    /// tests can prove whether a retry happened.
    struct SeqGenerator {
        answers: std::collections::VecDeque<String>,
        calls: usize,
    }

    impl SeqGenerator {
        fn new(answers: &[&str]) -> Self {
            SeqGenerator {
                answers: answers.iter().map(|s| s.to_string()).collect(),
                calls: 0,
            }
        }
    }

    impl Generator for SeqGenerator {
        fn generate(&mut self, _prompt: &str) -> CoreResult<String> {
            self.calls += 1;
            Ok(self
                .answers
                .pop_front()
                .expect("SeqGenerator ran out of answers"))
        }
    }

    /// Answers once, then errors — for the gate's retry-error path.
    struct FailOnSecondGenerator {
        calls: usize,
    }

    impl Generator for FailOnSecondGenerator {
        fn generate(&mut self, _prompt: &str) -> CoreResult<String> {
            self.calls += 1;
            if self.calls == 1 {
                Ok("出典のない答えです。".to_string())
            } else {
                Err(CoreError::GeneratorError("retry boom".into()))
            }
        }
    }

    fn ask_fixture(dir: &std::path::Path) -> (CoreState, u64, u64) {
        let mut st = CoreState::new(dir, EngineCfg::default(), "m").unwrap();
        let nb = st.create_notebook("NB", 1).unwrap();
        let src = st.add_source(nb.id, SourceKind::Text, "s", "/s.txt", 2).unwrap();
        st.set_source_text(
            src.id,
            &format!(
                "{}\n\n{}",
                "猫 は 魚 が 好き 。 ".repeat(40),
                "犬 は 公園 で 走る 。 ".repeat(40)
            ),
        )
        .unwrap();
        let mut emb = FakeEmbedder::new(32);
        st.index_source(src.id, &mut emb).unwrap();
        (st, nb.id, src.id)
    }

    #[test]
    fn ask_roundtrip_records_cited_turn() {
        let dir = temp_dir();
        let (mut st, nb_id, src_id) = ask_fixture(&dir);
        let captured = std::sync::Arc::new(std::sync::Mutex::new(None));
        let mut gen = RecordingGenerator { captured: captured.clone() };
        let mut emb = FakeEmbedder::new(32);
        let turn = st.ask(nb_id, "猫 魚", &mut emb, &mut gen, 2, 10).unwrap();
        assert_eq!(turn.role, Role::Assistant);
        // The bogus [#9999] marker must have been dropped.
        assert_eq!(turn.citations.len(), 1);
        assert_eq!(turn.citations[0].source_id, src_id);
        let top = st.search(nb_id, &emb.embed("猫 魚").unwrap(), 1);
        assert_eq!(turn.citations[0].chunk_id, Some(top[0].0.id));
        let turns = st.turns_of(nb_id).unwrap();
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].role, Role::User);
        drop(st);
        let st2 = CoreState::new(&dir, EngineCfg::default(), "m").unwrap();
        assert_eq!(st2.turns_of(nb_id).unwrap().len(), 2);
    }

    #[test]
    fn ask_gate_retries_once_and_accepts_cited_retry() {
        let dir = temp_dir();
        let (mut st, nb_id, _src_id) = ask_fixture(&dir);
        let mut emb = FakeEmbedder::new(32);
        // Learn a real chunk id to cite in the retry answer.
        let top = st.search(nb_id, &emb.embed("猫 魚").unwrap(), 1);
        let cited = format!("答えです[#{}]", top[0].0.id);
        let mut gen = SeqGenerator::new(&["出典のない答えです。", &cited]);
        let turn = st.ask(nb_id, "猫 魚", &mut emb, &mut gen, 2, 10).unwrap();
        assert_eq!(gen.calls, 2, "gate must retry exactly once");
        assert_eq!(turn.text, cited);
        assert_eq!(turn.citations.len(), 1);
        // Only the accepted answer is recorded.
        let turns = st.turns_of(nb_id).unwrap();
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[1].text, cited);
    }

    #[test]
    fn ask_gate_refuses_when_retry_also_uncited() {
        let dir = temp_dir();
        let (mut st, nb_id, _src_id) = ask_fixture(&dir);
        let mut emb = FakeEmbedder::new(32);
        let mut gen = SeqGenerator::new(&[
            "自前知識で答えます。",
            "やはり出典なしで答えます。",
        ]);
        let turn = st.ask(nb_id, "猫 魚", &mut emb, &mut gen, 2, 10).unwrap();
        assert_eq!(gen.calls, 2);
        assert_eq!(turn.text, "資料にありません。");
        assert!(turn.citations.is_empty());
        let turns = st.turns_of(nb_id).unwrap();
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[1].text, "資料にありません。");
    }

    #[test]
    fn ask_gate_retry_error_records_nothing() {
        let dir = temp_dir();
        let (mut st, nb_id, _src_id) = ask_fixture(&dir);
        let mut emb = FakeEmbedder::new(32);
        let mut gen = FailOnSecondGenerator { calls: 0 };
        let r = st.ask(nb_id, "猫 魚", &mut emb, &mut gen, 2, 10);
        assert_eq!(gen.calls, 2, "first answer uncited, retry attempted");
        assert!(matches!(r, Err(CoreError::GeneratorError(_))));
        assert_eq!(st.turns_of(nb_id).unwrap().len(), 0);
    }

    #[test]
    fn ask_gate_passes_cited_first_answer_without_retry() {
        let dir = temp_dir();
        let (mut st, nb_id, _src_id) = ask_fixture(&dir);
        let mut emb = FakeEmbedder::new(32);
        let top = st.search(nb_id, &emb.embed("猫 魚").unwrap(), 1);
        let cited = format!("最初から出典付き[#{}]", top[0].0.id);
        let mut gen = SeqGenerator::new(&[&cited]);
        let turn = st.ask(nb_id, "猫 魚", &mut emb, &mut gen, 2, 10).unwrap();
        assert_eq!(gen.calls, 1, "cited answer must not trigger a retry");
        assert_eq!(turn.text, cited);
        assert_eq!(turn.citations.len(), 1);
    }

    #[test]
    fn ask_drops_unknown_markers() {
        let dir = temp_dir();
        let (mut st, nb_id, _) = ask_fixture(&dir);
        let mut gen = FixedGenerator("答え[#9999]だけ。");
        let mut emb = FakeEmbedder::new(32);
        let turn = st.ask(nb_id, "猫 魚", &mut emb, &mut gen, 2, 10).unwrap();
        assert!(turn.citations.is_empty());
    }

    #[test]
    fn ask_without_grounding_still_answers() {
        let dir = temp_dir();
        let mut st = CoreState::new(&dir, EngineCfg::default(), "m").unwrap();
        let nb = st.create_notebook("EMPTY", 1).unwrap();
        let captured = std::sync::Arc::new(std::sync::Mutex::new(None));
        let mut gen = RecordingGenerator { captured: captured.clone() };
        let mut emb = FakeEmbedder::new(32);
        let turn = st.ask(nb.id, "何か", &mut emb, &mut gen, 3, 10).unwrap();
        assert!(turn.citations.is_empty());
        let prompt = captured.lock().unwrap().clone().unwrap();
        assert!(prompt.contains("（該当する資料なし）"));
    }

    #[test]
    fn ask_generator_error_records_nothing() {
        let dir = temp_dir();
        let (mut st, nb_id, _) = ask_fixture(&dir);
        let mut gen = ErrorGenerator;
        let mut emb = FakeEmbedder::new(32);
        let r = st.ask(nb_id, "猫 魚", &mut emb, &mut gen, 2, 10);
        assert!(matches!(r, Err(CoreError::GeneratorError(_))));
        assert_eq!(st.turns_of(nb_id).unwrap().len(), 0);
    }

    #[test]
    fn ask_missing_notebook_fails() {
        let dir = temp_dir();
        let mut st = CoreState::new(&dir, EngineCfg::default(), "m").unwrap();
        let mut gen = FixedGenerator("x");
        let mut emb = FakeEmbedder::new(32);
        let r = st.ask(999, "q", &mut emb, &mut gen, 2, 10);
        assert!(matches!(r, Err(CoreError::NotebookNotFound(999))));
    }

}
