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
//!   Chunk-level citations arrive later via a sidecar index.
//! - Retrieval and inference are not wired yet; the async retrieval
//!   contract of the shipped Memoria core (`emb_query_begin` /
//!   `retrieval_select` / `RetrWait`) is the shape this crate will bind to.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
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

/// A record-level citation: the reply leaned on this source record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Citation {
    pub source_id: u64,
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
    next_notebook_id: u64,
    next_source_id: u64,
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
        let next_notebook_id = notebooks.iter().map(|n| n.id).max().unwrap_or(0) + 1;
        let next_source_id = sources.iter().map(|s| s.id).max().unwrap_or(0) + 1;
        Ok(CoreState {
            data_dir: data_dir.to_path_buf(),
            cfg,
            local_model_display_name: local_model_display_name.to_string(),
            notebooks,
            sources,
            next_notebook_id,
            next_source_id,
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

fn append_jsonl<T: Serialize>(path: &Path, rec: &T) -> CoreResult<()> {
    let mut f = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(f, "{}", serde_json::to_string(rec)?)?;
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
        out.push(serde_json::from_str(&line)?);
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
            citations: vec![Citation { source_id: src.id }],
            at: 3,
        })
        .unwrap();
        let turns = st.turns_of(nb.id).unwrap();
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].citations[0].source_id, src.id);
        assert!(st.turns_of(999).unwrap().is_empty());
    }

    #[test]
    fn add_source_to_missing_notebook_fails() {
        let dir = temp_dir();
        let mut st = CoreState::new(&dir, EngineCfg::default(), "m").unwrap();
        let r = st.add_source(42, SourceKind::Text, "x", "/x", 1);
        assert!(matches!(r, Err(CoreError::NotebookNotFound(42))));
    }
}
