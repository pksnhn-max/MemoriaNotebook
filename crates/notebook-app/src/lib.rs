//! The Android vessel's Rust side (stages 6a-6d).
//!
//! A thin JNI bridge over `notebook-core`. The Kotlin host owns one
//! Activity and calls these functions; all state lives in one
//! `CoreState` behind a mutex (the core's single-owner contract is
//! honoured by serializing here).
//!
//! The embedder seat is real since 6c (ruri via notebook-embed,
//! with the bag-of-words placeholder as the visible fallback);
//! the generator seat is real since 6d (TinySwallow via
//! notebook-infer/llama.cpp, loaded lazily from the models dir;
//! with no model on device, ask() refuses with guidance instead
//! of faking an answer). File import (notebook-extract) and the
//! network seat (notebook-net: model download, web search, page
//! fetch) also land in 6d.
//!
//! The JNI class is `jp.memorianotebook.Bridge` — provisional until
//! the package name is ruled.

use jni::objects::{JClass, JString};
use jni::sys::{jboolean, jlong, jstring};
use jni::JNIEnv;
use notebook_core::{ChatTurn, CoreResult, CoreState, Embedder, EngineCfg, SourceKind};
use notebook_core::Generator;
use std::sync::Mutex;
use notebook_core::CoreError;
use std::fs;
use std::io::Write;
use serde::{Deserialize, Serialize};
use notebook_net::{get_with_headers, post_json};

/// Everything the bridge owns: the core plus the embedder seat.
/// The embedder is chosen once at init (ruri when its files are
/// present, the BoW placeholder otherwise) and its display name is
/// kept so the settings screen can show which seat is live — a
/// fallback is visible, never silent.
struct Host {
    core: CoreState,
    emb: Box<dyn Embedder + Send>,
    emb_name: String,
    data_dir: std::path::PathBuf,
    /// The local generator seat: loaded lazily on the first ask
    /// once a GGUF sits in the models dir (a 1 GB model is not
    /// something to load on every cold start before it is needed).
    gen: Option<notebook_infer::LlamaGenerator>,
    gen_name: String,
    /// Persisted API configuration and seat selection.
    api_config: ApiConfig,
    /// Current generator seat: "local" or "api".
    gen_seat: GenSeat,
}

/// Generator seat selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum GenSeat {
    Local,
    Api,
}

impl Default for GenSeat {
    fn default() -> Self {
        GenSeat::Local
    }
}

/// API generator configuration, persisted as settings.json.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ApiConfig {
    base_url: String,
    api_key: String,
    model: String,
}

impl ApiConfig {
    fn is_complete(&self) -> bool {
        !self.base_url.is_empty() && !self.api_key.is_empty() && !self.model.is_empty()
    }
}

/// Settings file name.
const SETTINGS_FILE: &str = "settings.json";


/// The GGUF currently designated as the local model: the first
/// `.gguf` file (sorted) in `data_dir/models`, if any.
fn designated_model(data_dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let dir = data_dir.join("models");
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .map(|e| e.eq_ignore_ascii_case("gguf"))
                .unwrap_or(false)
        })
        .collect();
    files.sort();
    files.into_iter().next()
}

fn gen_name_for(data_dir: &std::path::Path) -> String {
    match designated_model(data_dir) {
        Some(p) => p
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "ローカルモデル".to_string()),
        None => "未導入".to_string(),
    }
}

static STATE: Mutex<Option<Host>> = Mutex::new(None);

/// Load settings from data_dir/settings.json. Returns defaults on any error.
fn load_settings(data_dir: &std::path::Path) -> (GenSeat, ApiConfig) {
    let path = data_dir.join(SETTINGS_FILE);
    match fs::read_to_string(&path) {
        Ok(content) => {
            match serde_json::from_str::<Settings>(&content) {
                Ok(s) => (s.gen_seat, s.api_config),
                Err(_) => (GenSeat::default(), ApiConfig::default()),
            }
        }
        Err(_) => (GenSeat::default(), ApiConfig::default()),
    }
}

/// Persisted settings structure.
#[derive(Serialize, Deserialize)]
struct Settings {
    gen_seat: GenSeat,
    api_config: ApiConfig,
}

/// Save settings atomically (write temp + rename).
fn save_settings(data_dir: &std::path::Path, seat: GenSeat, config: &ApiConfig) -> Result<(), String> {
    let settings = Settings {
        gen_seat: seat,
        api_config: config.clone(),
    };
    let content = serde_json::to_string(&settings).map_err(|e| e.to_string())?;
    let path = data_dir.join(SETTINGS_FILE);
    let tmp_path = path.with_extension("json.tmp");
    {
        let mut f = fs::File::create(&tmp_path).map_err(|e| e.to_string())?;
        f.write_all(content.as_bytes()).map_err(|e| e.to_string())?;
        f.flush().map_err(|e| e.to_string())?;
    }
    fs::rename(&tmp_path, &path).map_err(|e| e.to_string())?;
    Ok(())
}

/// Build the chat completion request body.
/// Pure function for unit testing.
fn build_chat_body(model: &str, prompt: &str) -> serde_json::Value {
    serde_json::json!({
        "model": model,
        "messages": [
            {"role": "user", "content": prompt}
        ],
        "stream": false
    })
}

/// Parse chat completion response to extract the assistant's message content.
/// Pure function for unit testing.
fn parse_chat_content(resp: &str) -> Result<String, String> {
    let v: serde_json::Value = serde_json::from_str(resp).map_err(|e| format!("invalid json: {e}"))?;
    let choices = v.get("choices").and_then(|c| c.as_array()).ok_or("missing choices")?;
    let first = choices.first().ok_or("empty choices")?;
    let msg = first.get("message").ok_or("missing message")?;
    let content = msg.get("content").and_then(|c| c.as_str()).ok_or("missing content")?;
    Ok(content.to_string())
}

/// Parse OpenAI-compatible /models response to extract model ids.
/// Pure function for unit testing.
fn parse_model_ids(resp: &str) -> Result<Vec<String>, String> {
    let v: serde_json::Value = serde_json::from_str(resp).map_err(|e| format!("invalid json: {e}"))?;
    let data = v.get("data").and_then(|d| d.as_array()).ok_or("missing data array")?;
    let ids: Vec<String> = data
        .iter()
        .filter_map(|m| m.get("id").and_then(|id| id.as_str()).map(|s| s.to_string()))
        .collect();
    Ok(ids)
}

/// API generator seat implementation.
struct ApiGenerator {
    base_url: String,
    api_key: String,
    model: String,
}

impl Generator for ApiGenerator {
    fn generate(&mut self, prompt: &str) -> CoreResult<String> {
        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let body = build_chat_body(&self.model, prompt);
        let headers = [
            ("Authorization", &format!("Bearer {}", self.api_key) as &str),
            ("Content-Type", "application/json"),
        ];
        // Use a generous cap (1 MiB) for the response.
        let resp = post_json(&url, &headers, &body, 1_048_576)
            .map_err(|e| CoreError::GeneratorError(format!("network: {e}")))?;
        parse_chat_content(&resp)
            .map_err(|e| CoreError::GeneratorError(format!("parse: {e}")))
    }
}


/// Placeholder embedder (bag-of-words feature hashing), same family
/// as the core's test fake. Replaced by ruri in stage 6c.
struct BowEmbedder {
    dim: usize,
}

impl Embedder for BowEmbedder {
    fn embed(&mut self, text: &str) -> CoreResult<Vec<f32>> {
        let mut vec = vec![0.0f32; self.dim];
        for word in text.split_whitespace() {
            let mut h: u64 = 0xcbf29ce484222325;
            for b in word.as_bytes() {
                h ^= *b as u64;
                h = h.wrapping_mul(0x100000001b3);
            }
            vec[(h % self.dim as u64) as usize] += 1.0;
        }
        Ok(vec)
    }
}

/// Test-only generator: cites the first grounding marker it finds
/// in the prompt, so the citation path is exercised end to end.
/// (The device path never fakes an answer — see ask().)
#[cfg(test)]
struct CannedGenerator;

#[cfg(test)]
impl Generator for CannedGenerator {
    fn generate(&mut self, prompt: &str) -> CoreResult<String> {
        let bytes = prompt.as_bytes();
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
                    return Ok(format!(
                        "（仮の生成）資料の先頭の断片に基づく答えです[#{num}]。"
                    ));
                }
                i = j;
            } else {
                i += 1;
            }
        }
        Ok("資料にありません。".to_string())
    }
}

/// Escape a field for the bridge's line format: backslash first,
/// then tab and newline, so one record always stays on one line.
fn esc(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\t', "\\t")
        .replace('\n', "\\n")
}

/// One line per notebook: `id \t title \t created_at \t source count`.
fn fmt_notebooks(st: &CoreState) -> String {
    st.notebooks()
        .iter()
        .map(|n| {
            format!(
                "{}\t{}\t{}\t{}",
                n.id,
                esc(&n.title),
                n.created_at,
                st.sources_of(n.id).len()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// One line per source of `nb`: `id \t title`.
fn fmt_sources(st: &CoreState, nb: u64) -> String {
    st.sources_of(nb)
        .iter()
        .map(|s| format!("{}\t{}", s.id, esc(&s.title)))
        .collect::<Vec<_>>()
        .join("\n")
}

/// One line per turn of `nb`: `role \t text \t cited chunk ids
/// (comma-separated, empty when none)`.
fn fmt_turns(st: &CoreState, nb: u64) -> CoreResult<String> {
    let turns = st.turns_of(nb)?;
    Ok(turns
        .iter()
        .map(|t| {
            let role = match t.role {
                notebook_core::Role::User => "user",
                notebook_core::Role::Assistant => "assistant",
            };
            let cites = t
                .citations
                .iter()
                .filter_map(|c| c.chunk_id.map(|id| id.to_string()))
                .collect::<Vec<_>>()
                .join(",");
            format!("{role}\t{}\t{cites}", esc(&t.text))
        })
        .collect::<Vec<_>>()
        .join("\n"))
}

/// One chunk for the citation viewer: `source title \t chunk text`,
/// both escaped. Empty when the chunk id no longer resolves (the
/// source was re-chunked after the turn was recorded).
fn fmt_chunk(st: &CoreState, chunk_id: u64) -> String {
    match st.chunk_by_id(chunk_id) {
        Some(c) => {
            let title = st
                .sources_of(c.nb)
                .into_iter()
                .find(|s| s.id == c.source_id)
                .map(|s| s.title.clone())
                .unwrap_or_default();
            format!("{}\t{}", esc(&title), esc(&c.text))
        }
        None => String::new(),
    }
}

fn with_state<T>(f: impl FnOnce(&mut Host) -> CoreResult<T>) -> Result<T, String> {
    let mut guard = STATE.lock().map_err(|_| "state poisoned".to_string())?;
    let host = guard.as_mut().ok_or("not initialized")?;
    f(host).map_err(|e| e.to_string())
}

fn jstring_to_string(env: &mut JNIEnv, s: &JString) -> Result<String, String> {
    Ok(env.get_string(s).map_err(|e| e.to_string())?.into())
}

/// Initialize the core at `data_dir` and choose the embedder
/// seat: ruri when the host has extracted its files into
/// `data_dir/ruri/` and passes a usable ONNX Runtime path,
/// otherwise the BoW placeholder. When the seat changes (tag
/// mismatch), every existing source is re-indexed once, because
/// stored vectors are only valid for their own embedder.
#[no_mangle]
pub extern "C" fn Java_jp_memorianotebook_Bridge_init(
    mut env: JNIEnv,
    _class: JClass,
    data_dir: JString,
    ort_lib_path: JString,
) -> jboolean {
    let dir = match jstring_to_string(&mut env, &data_dir) {
        Ok(d) => d,
        Err(_) => return 0,
    };
    let ort_path = jstring_to_string(&mut env, &ort_lib_path).unwrap_or_default();
    let root = std::path::Path::new(&dir);
    let mut core = match CoreState::new(root, EngineCfg::default(), "TinySwallow") {
        Ok(st) => st,
        Err(_) => return 0,
    };
    let model = root.join("ruri").join("model_int8.onnx");
    let tokenizer = root.join("ruri").join("tokenizer.json");
    let (emb, emb_name): (Box<dyn Embedder + Send>, String) =
        if !ort_path.is_empty() && model.exists() && tokenizer.exists() {
            match notebook_embed::RuriEmbedder::new(
                &ort_path,
                model.to_str().unwrap_or(""),
                tokenizer.to_str().unwrap_or(""),
            ) {
                Ok(mut ruri) => {
                    let tag_path = root.join("embedder.tag");
                    let tag_ok = std::fs::read_to_string(&tag_path)
                        .map(|t| t.trim() == notebook_embed::TAG)
                        .unwrap_or(false);
                    if !tag_ok {
                        let ids: Vec<u64> = core
                            .notebooks()
                            .iter()
                            .flat_map(|n| core.sources_of(n.id))
                            .map(|s| s.id)
                            .collect();
                        let mut reindex: CoreResult<()> = Ok(());
                        for id in ids {
                            if let Err(e) = core.index_source(id, &mut ruri) {
                                reindex = Err(e);
                                break;
                            }
                        }
                        match reindex {
                            Ok(()) => {
                                let _ = std::fs::write(
                                    &tag_path,
                                    notebook_embed::TAG,
                                );
                                (
                                    Box::new(ruri),
                                    "ruri-v3-30m（256次元）".to_string(),
                                )
                            }
                            Err(e) => (
                                Box::new(ruri),
                                format!(
                                    "ruri-v3-30m（256次元）／再索引で失敗: {e}"
                                ),
                            ),
                        }
                    } else {
                        (
                            Box::new(ruri),
                            "ruri-v3-30m（256次元）".to_string(),
                        )
                    }
                }
                Err(e) => (
                    Box::new(BowEmbedder { dim: 64 }),
                    format!("仮の埋め込み（BoW 64次元）／ruri 読込失敗: {e}"),
                ),
            }
        } else {
            (
                Box::new(BowEmbedder { dim: 64 }),
                "仮の埋め込み（BoW 64次元）".to_string(),
            )
        };
    let gen_name = gen_name_for(root);
    let (gen_seat, api_config) = load_settings(root);
    let mut guard = match STATE.lock() {
        Ok(g) => g,
        Err(_) => return 0,
    };
    *guard = Some(Host {
        core,
        emb,
        emb_name,
        data_dir: root.to_path_buf(),
        gen: None,
        gen_name,
        api_config,
        gen_seat,
    });
    1
}

/// Display name of the live embedder seat, for the settings screen.
#[no_mangle]
pub extern "C" fn Java_jp_memorianotebook_Bridge_embedderInfo(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    let name = STATE
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|h| h.emb_name.clone()))
        .unwrap_or_else(|| "未初期化".to_string());
    env.new_string(name)
        .map(|s| s.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

/// Create a notebook at `now` (unix seconds, supplied by the host
/// clock); returns its id, or -1 on failure.
#[no_mangle]
pub extern "C" fn Java_jp_memorianotebook_Bridge_createNotebook(
    mut env: JNIEnv,
    _class: JClass,
    title: JString,
    now: jlong,
) -> jlong {
    let title = match jstring_to_string(&mut env, &title) {
        Ok(t) => t,
        Err(_) => return -1,
    };
    with_state(|h| {
        let nb = h.core.create_notebook(&title, now.max(0) as u64)?;
        Ok(nb.id as jlong)
    })
    .unwrap_or(-1)
}

/// Add a plain-text source to notebook `nb`, set its text and index
/// it in one call (6a convenience). Returns the source id, or -1.
#[no_mangle]
pub extern "C" fn Java_jp_memorianotebook_Bridge_addTextSource(
    mut env: JNIEnv,
    _class: JClass,
    nb: jlong,
    title: JString,
    text: JString,
) -> jlong {
    let title = match jstring_to_string(&mut env, &title) {
        Ok(t) => t,
        Err(_) => return -1,
    };
    let text = match jstring_to_string(&mut env, &text) {
        Ok(t) => t,
        Err(_) => return -1,
    };
    with_state(|h| {
        let src = h
            .core
            .add_source(nb as u64, SourceKind::Text, &title, "", 0)?;
        h.core.set_source_text(src.id, &text)?;
        h.core.index_source(src.id, &mut *h.emb)?;
        Ok(src.id as jlong)
    })
    .unwrap_or(-1)
}

/// Delete (tombstone) a notebook; false on failure.
#[no_mangle]
pub extern "C" fn Java_jp_memorianotebook_Bridge_deleteNotebook(
    _env: JNIEnv,
    _class: JClass,
    nb: jlong,
) -> jboolean {
    match with_state(|h| h.core.delete_notebook(nb as u64)) {
        Ok(()) => 1,
        Err(_) => 0,
    }
}

/// Set API configuration and seat, persist to settings.json.
/// Returns a short status string.
#[no_mangle]
pub extern "C" fn Java_jp_memorianotebook_Bridge_setApiConfig(
    mut env: JNIEnv,
    _class: JClass,
    seat: JString,
    base_url: JString,
    api_key: JString,
    model: JString,
) -> jstring {
    let result = (|| -> Result<String, String> {
        let seat_str = jstring_to_string(&mut env, &seat)?;
        let base_url = jstring_to_string(&mut env, &base_url)?;
        let api_key = jstring_to_string(&mut env, &api_key)?;
        let model = jstring_to_string(&mut env, &model)?;

        let seat = match seat_str.as_str() {
            "api" => GenSeat::Api,
            _ => GenSeat::Local,
        };
        let config = ApiConfig {
            base_url,
            api_key,
            model,
        };

        let mut guard = STATE.lock().map_err(|_| "state poisoned".to_string())?;
        let host = guard.as_mut().ok_or("not initialized")?;
        save_settings(&host.data_dir, seat, &config)?;
        host.gen_seat = seat;
        host.api_config = config;
        Ok("OK".to_string())
    })();
    string_result(&mut env, result)
}

/// Get current API configuration and seat as JSON for settings prefill.
#[no_mangle]
pub extern "C" fn Java_jp_memorianotebook_Bridge_getApiConfig(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    let result = with_state(|h| {
        let seat_str = match h.gen_seat {
            GenSeat::Local => "local",
            GenSeat::Api => "api",
        };
        Ok(serde_json::json!({
            "seat": seat_str,
            "base_url": h.api_config.base_url,
            "api_key": h.api_config.api_key,
            "model": h.api_config.model,
        })
        .to_string())
    });
    string_result(&mut env, result)
}

/// Fetch available models from an OpenAI-compatible /models endpoint.
/// Returns JSON array of model ids, or {"error":"..."} on failure.
#[no_mangle]
pub extern "C" fn Java_jp_memorianotebook_Bridge_fetchApiModels(
    mut env: JNIEnv,
    _class: JClass,
    base_url: JString,
    api_key: JString,
) -> jstring {
    let result = (|| -> Result<String, String> {
        let base_url = jstring_to_string(&mut env, &base_url)?;
        let api_key = jstring_to_string(&mut env, &api_key)?;
        let url = format!("{}/models", base_url.trim_end_matches('/'));
        let headers = [("Authorization", &format!("Bearer {}", api_key) as &str)];
        let resp = get_with_headers(&url, &headers, 1_048_576)
            .map_err(|e| format!("network: {e}"))?;
        let ids = parse_model_ids(&resp).map_err(|e| format!("parse: {e}"))?;
        Ok(serde_json::to_string(&ids).map_err(|e| e.to_string())?)
    })();
    let text = match result {
        Ok(t) => t,
        Err(e) => {
            // Sanitize: never include key material.
            let err_json = serde_json::json!({"error": e}).to_string();
            err_json
        }
    };
    env.new_string(text)
        .map(|s| s.into_raw())
        .unwrap_or(std::ptr::null_mut())
}


/// Ask one question; returns the assistant's answer text, or a
/// string starting with `ERR:` on failure.
#[no_mangle]
pub extern "C" fn Java_jp_memorianotebook_Bridge_ask(
    mut env: JNIEnv,
    _class: JClass,
    nb: jlong,
    question: JString,
) -> jstring {
    let question = match jstring_to_string(&mut env, &question) {
        Ok(q) => q,
        Err(e) => {
            return env
                .new_string(format!("ERR: {e}"))
                .map(|s| s.into_raw())
                .unwrap_or(std::ptr::null_mut())
        }
    };
    let result = with_state(|h| {
        let mut api_generator;
        let gen: &mut dyn notebook_core::Generator = if h.gen_seat == GenSeat::Api && h.api_config.is_complete() {
            api_generator = ApiGenerator {
                base_url: h.api_config.base_url.clone(),
                api_key: h.api_config.api_key.clone(),
                model: h.api_config.model.clone(),
            };
            &mut api_generator
        } else {
            // Generator seat: the local model, loaded lazily. With no
            // model on device the ask refuses with guidance — an
            // unrecorded error, never a faked answer.
            if h.gen.is_none() {
                match designated_model(&h.data_dir) {
                    Some(path) => {
                        let gen = notebook_infer::LlamaGenerator::load(path)?;
                        h.gen_name = gen_name_for(&h.data_dir);
                        h.gen = Some(gen);
                    }
                    None => {
                        return Err(notebook_core::CoreError::GeneratorError(
                            "モデルが未導入です。設定のモデル管理からダウンロードしてください"
                                .to_string(),
                        ));
                    }
                }
            }
            let Some(gen) = h.gen.as_mut() else {
                return Err(notebook_core::CoreError::GeneratorError(
                    "generator seat empty after load".to_string(),
                ));
            };
            gen
        };
        let turn: ChatTurn =
            h.core
                .ask(nb as u64, &question, &mut *h.emb, gen, 5, 0)?;
        Ok(turn.text)
    });
    let text = match result {
        Ok(t) => t,
        Err(e) => format!("ERR: {e}"),
    };
    env.new_string(text)
        .map(|s| s.into_raw())
        .unwrap_or(std::ptr::null_mut())
}
/// Display name of the local generator seat, for the settings
/// screen ("未導入" until a GGUF sits in the models dir).
#[no_mangle]
pub extern "C" fn Java_jp_memorianotebook_Bridge_modelInfo(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    let name = STATE
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|h| h.gen_name.clone()))
        .unwrap_or_else(|| "未初期化".to_string());
    env.new_string(name)
        .map(|s| s.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

/// Search Hugging Face for GGUF repos; one repo id per line,
/// or `ERR: ...`.
#[no_mangle]
pub extern "C" fn Java_jp_memorianotebook_Bridge_searchModels(
    mut env: JNIEnv,
    _class: JClass,
    query: JString,
) -> jstring {
    let result = (|| -> Result<String, String> {
        let q = jstring_to_string(&mut env, &query)?;
        let repos = notebook_net::hf_search_models(&q).map_err(|e| e.to_string())?;
        Ok(repos.join("\n"))
    })();
    string_result(&mut env, result)
}

/// List a repo's GGUF files: `name \t size` per line, or `ERR: ...`.
#[no_mangle]
pub extern "C" fn Java_jp_memorianotebook_Bridge_listModelFiles(
    mut env: JNIEnv,
    _class: JClass,
    repo: JString,
) -> jstring {
    let result = (|| -> Result<String, String> {
        let repo = jstring_to_string(&mut env, &repo)?;
        let files = notebook_net::hf_model_files(&repo).map_err(|e| e.to_string())?;
        Ok(files
            .iter()
            .map(|(n, s)| format!("{n}\t{s}"))
            .collect::<Vec<_>>()
            .join("\n"))
    })();
    string_result(&mut env, result)
}

/// Download one GGUF from a Hugging Face repo into the models dir
/// (blocking; the vessel calls this off the UI thread). Returns
/// `OK <bytes>` or `ERR: ...`. The downloaded model becomes the
/// designated local model; any loaded generator is dropped so the
/// next ask reloads.
#[no_mangle]
pub extern "C" fn Java_jp_memorianotebook_Bridge_downloadModel(
    mut env: JNIEnv,
    _class: JClass,
    repo: JString,
    file: JString,
) -> jstring {
    let result = (|| -> Result<String, String> {
        let repo = jstring_to_string(&mut env, &repo)?;
        let file = jstring_to_string(&mut env, &file)?;
        // The file name lands on disk under the models dir: it must
        // be a plain file name, never a path (no traversal).
        if file.is_empty()
            || file.contains('/')
            || file.contains('\\')
            || file.contains("..")
        {
            return Err("invalid model file name".to_string());
        }
        // One download at a time (the .part staging file is shared).
        static DOWNLOADING: std::sync::atomic::AtomicBool =
            std::sync::atomic::AtomicBool::new(false);
        if DOWNLOADING
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::SeqCst,
            )
            .is_err()
        {
            return Err("download already in progress".to_string());
        }
        struct Guard;
        impl Drop for Guard {
            fn drop(&mut self) {
                DOWNLOADING.store(false, std::sync::atomic::Ordering::SeqCst);
            }
        }
        let _guard = Guard;
        let data_dir = {
            let guard = STATE.lock().map_err(|_| "state poisoned".to_string())?;
            let host = guard.as_ref().ok_or("not initialized")?;
            host.data_dir.clone()
        };
        let dir = data_dir.join("models");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let dest = dir.join(&file);
        let url = notebook_net::hf_resolve_url(&repo, &file);
        let n = notebook_net::download_to_file(&url, &dest, |_, _| {})
            .map_err(|e| e.to_string())?;
        {
            let mut guard = STATE.lock().map_err(|_| "state poisoned".to_string())?;
            if let Some(host) = guard.as_mut() {
                host.gen = None;
                host.gen_name = gen_name_for(&host.data_dir);
            }
        }
        Ok(format!("OK {n}"))
    })();
    string_result(&mut env, result)
}

/// Add a file as a source: `kind` is "epub" or anything else for
/// plain text (txt/md, UTF-8 or Shift_JIS). The bytes are extracted
/// to text by notebook-extract, then set and indexed like a pasted
/// source. Returns the source id, or -1.
#[no_mangle]
pub extern "C" fn Java_jp_memorianotebook_Bridge_addFileSource(
    mut env: JNIEnv,
    _class: JClass,
    nb: jlong,
    title: JString,
    kind: JString,
    bytes: jni::objects::JByteArray,
) -> jlong {
    let title = match jstring_to_string(&mut env, &title) {
        Ok(t) => t,
        Err(_) => return -1,
    };
    let kind = jstring_to_string(&mut env, &kind).unwrap_or_default();
    let raw = match env.convert_byte_array(&bytes) {
        Ok(b) => b,
        Err(_) => return -1,
    };
    let text = if kind.eq_ignore_ascii_case("epub") {
        match notebook_extract::extract_epub(&raw) {
            Ok(t) => t,
            Err(_) => return -1,
        }
    } else {
        notebook_extract::extract_txt(&raw)
    };
    with_state(|h| {
        let src = h
            .core
            .add_source(nb as u64, SourceKind::Text, &title, "", 0)?;
        h.core.set_source_text(src.id, &text)?;
        h.core.index_source(src.id, &mut *h.emb)?;
        Ok(src.id as jlong)
    })
    .unwrap_or(-1)
}

/// Web search; one hit per line: `title \t url \t snippet`,
/// or `ERR: ...`.
#[no_mangle]
pub extern "C" fn Java_jp_memorianotebook_Bridge_webSearch(
    mut env: JNIEnv,
    _class: JClass,
    query: JString,
) -> jstring {
    let result = (|| -> Result<String, String> {
        let q = jstring_to_string(&mut env, &query)?;
        let hits = notebook_net::search_web(&q, 10).map_err(|e| e.to_string())?;
        Ok(hits
            .iter()
            .map(|h| format!("{}\t{}\t{}", esc(&h.title), esc(&h.url), esc(&h.snippet)))
            .collect::<Vec<_>>()
            .join("\n"))
    })();
    string_result(&mut env, result)
}

/// Fetch a web page and add its text as a source. Returns
/// `OK <source id>` or `ERR: ...`.
#[no_mangle]
pub extern "C" fn Java_jp_memorianotebook_Bridge_addWebSource(
    mut env: JNIEnv,
    _class: JClass,
    nb: jlong,
    title: JString,
    url: JString,
) -> jstring {
    let result = (|| -> Result<String, String> {
        let title = jstring_to_string(&mut env, &title)?;
        let url = jstring_to_string(&mut env, &url)?;
        let text = notebook_net::fetch_page_text(&url).map_err(|e| e.to_string())?;
        let id = with_state(|h| {
            let src = h
                .core
                .add_source(nb as u64, SourceKind::Text, &title, &url, 0)?;
            h.core.set_source_text(src.id, &text)?;
            h.core.index_source(src.id, &mut *h.emb)?;
            Ok(src.id as jlong)
        })?;
        Ok(format!("OK {id}"))
    })();
    string_result(&mut env, result)
}

fn string_result(env: &mut JNIEnv, result: Result<String, String>) -> jstring {
    let text = match result {
        Ok(t) => t,
        Err(e) => format!("ERR: {e}"),
    };
    env.new_string(text)
        .map(|s| s.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

/// List notebooks, one per line: `id \t title \t created_at \t source count`.
#[no_mangle]
pub extern "C" fn Java_jp_memorianotebook_Bridge_listNotebooks(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    let result = with_state(|h| Ok(fmt_notebooks(&h.core)));
    string_result(&mut env, result)
}

/// List sources of notebook `nb`, one per line: `id \t title`.
#[no_mangle]
pub extern "C" fn Java_jp_memorianotebook_Bridge_listSources(
    mut env: JNIEnv,
    _class: JClass,
    nb: jlong,
) -> jstring {
    let result = with_state(|h| Ok(fmt_sources(&h.core, nb as u64)));
    string_result(&mut env, result)
}

/// List chat turns of notebook `nb`, one per line:
/// `role \t text \t cited chunk ids (comma-separated)`.
#[no_mangle]
pub extern "C" fn Java_jp_memorianotebook_Bridge_listTurns(
    mut env: JNIEnv,
    _class: JClass,
    nb: jlong,
) -> jstring {
    let result = with_state(|h| fmt_turns(&h.core, nb as u64));
    string_result(&mut env, result)
}

/// Fetch one cited chunk for the viewer: `source title \t chunk
/// text` (escaped), or an empty string when the id no longer
/// resolves.
#[no_mangle]
pub extern "C" fn Java_jp_memorianotebook_Bridge_getChunk(
    mut env: JNIEnv,
    _class: JClass,
    chunk_id: jlong,
) -> jstring {
    let result = with_state(|h| Ok(fmt_chunk(&h.core, chunk_id as u64)));
    string_result(&mut env, result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn temp_state() -> (CoreState, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "nbapp-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let st = CoreState::new(&dir, EngineCfg::default(), "TinySwallow").unwrap();
        (st, dir)
    }

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "nbapp-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn settings_save_load_roundtrip() {
        let dir = temp_dir();
        let seat = GenSeat::Api;
        let config = ApiConfig {
            base_url: "https://api.example.com/v1".to_string(),
            api_key: "sk-test123".to_string(),
            model: "gpt-4o-mini".to_string(),
        };
        save_settings(&dir, seat, &config).unwrap();
        let (loaded_seat, loaded_config) = load_settings(&dir);
        assert_eq!(loaded_seat, seat);
        assert_eq!(loaded_config.base_url, config.base_url);
        assert_eq!(loaded_config.api_key, config.api_key);
        assert_eq!(loaded_config.model, config.model);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn settings_load_defaults_on_missing() {
        let dir = temp_dir();
        let (seat, config) = load_settings(&dir);
        assert_eq!(seat, GenSeat::Local);
        assert_eq!(config.base_url, "");
        assert_eq!(config.api_key, "");
        assert_eq!(config.model, "");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn settings_load_defaults_on_corrupt() {
        let dir = temp_dir();
        fs::write(dir.join(SETTINGS_FILE), "not json").unwrap();
        let (seat, config) = load_settings(&dir);
        assert_eq!(seat, GenSeat::Local);
        assert_eq!(config.base_url, "");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_model_ids_sample() {
        let payload = r#"{
            "object": "list",
            "data": [
                {"id": "gpt-4o", "object": "model", "owned_by": "openai"},
                {"id": "gpt-4o-mini", "object": "model", "owned_by": "openai"},
                {"id": "gpt-3.5-turbo", "object": "model", "owned_by": "openai"}
            ]
        }"#;
        let ids = parse_model_ids(payload).unwrap();
        assert_eq!(ids, vec!["gpt-4o", "gpt-4o-mini", "gpt-3.5-turbo"]);
    }

    #[test]
    fn parse_model_ids_missing_data() {
        let payload = r#"{"object": "list"}"#;
        let err = parse_model_ids(payload).unwrap_err();
        assert!(err.contains("missing data array"));
    }

    #[test]
    fn parse_chat_content_sample() {
        let payload = r#"{
            "id": "chatcmpl-123",
            "object": "chat.completion",
            "choices": [
                {
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": "Hello, world!"
                    },
                    "finish_reason": "stop"
                }
            ]
        }"#;
        let content = parse_chat_content(payload).unwrap();
        assert_eq!(content, "Hello, world!");
    }

    #[test]
    fn parse_chat_content_missing_choices() {
        let payload = r#"{"id": "x"}"#;
        let err = parse_chat_content(payload).unwrap_err();
        assert!(err.contains("missing choices"));
    }

    #[test]
    fn build_chat_body_shape() {
        let body = build_chat_body("test-model", "user prompt");
        assert_eq!(body["model"], "test-model");
        assert_eq!(body["stream"], false);
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[0]["content"], "user prompt");
    }

    #[test]
    fn esc_keeps_one_line() {
        assert_eq!(esc("a\tb\nc\\d"), "a\\tb\\nc\\\\d");
    }

    #[test]
    fn formats_list_notebooks_sources_and_turns() {
        let (mut st, dir) = temp_state();
        let nb = st.create_notebook("川辺\tの記録", 7).unwrap();
        let src = st
            .add_source(nb.id, SourceKind::Text, "天満川メモ", "", 0)
            .unwrap();
        let long = "南観音は天満川の西岸にある地区です。".repeat(30);
        st.set_source_text(src.id, &long).unwrap();
        let mut emb = BowEmbedder { dim: 64 };
        st.index_source(src.id, &mut emb).unwrap();
        let mut gen = CannedGenerator;
        st.ask(nb.id, "南観音はどこですか", &mut emb, &mut gen, 5, 0)
            .unwrap();

        let nbs = fmt_notebooks(&st);
        assert_eq!(nbs.lines().count(), 1);
        assert!(nbs.starts_with(&format!("{}\t川辺\\tの記録\t7\t1", nb.id)));

        let srcs = fmt_sources(&st, nb.id);
        assert_eq!(srcs, format!("{}\t天満川メモ", src.id));

        let turns = fmt_turns(&st, nb.id).unwrap();
        let lines: Vec<&str> = turns.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("user\t南観音はどこですか\t"));
        assert!(lines[1].starts_with("assistant\t"));
        // The canned answer cites a chunk, so the citation field is non-empty.
        let cites = lines[1].rsplit('\t').next().unwrap();
        assert!(!cites.is_empty(), "expected a cited chunk id: {lines:?}");

        // The cited chunk resolves for the viewer, with its source
        // title, and an unknown id resolves to empty.
        let chunk_id: u64 = cites.split(',').next().unwrap().parse().unwrap();
        let chunk = fmt_chunk(&st, chunk_id);
        let mut fields = chunk.split('\t');
        assert_eq!(fields.next().unwrap(), "天満川メモ");
        assert!(fields.next().unwrap().contains("南観音は天満川の西岸"));
        assert_eq!(fmt_chunk(&st, 999_999), "");

        let _ = std::fs::remove_dir_all(&dir);
    }
}


// ---- Stage 7: audio overview (VOICEVOX) ----

use notebook_core::voice::{
    has_consent, load_summaries, load_voice_pair, record_consent, remove_summary,
    save_voice_pair, voice_by_key, VOICE_CATALOG,
};

fn voice_err<T>(e: String) -> Result<T, notebook_core::CoreError> {
    Err(notebook_core::CoreError::GeneratorError(e))
}

#[no_mangle]
pub extern "C" fn Java_jp_memorianotebook_Bridge_getVoiceCatalog(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    let result = with_state(|host| {
        let data_dir = &host.data_dir;
        let (voice_a, voice_b) = load_voice_pair(data_dir);
        let catalog: Vec<serde_json::Value> = VOICE_CATALOG
            .iter()
            .map(|entry| {
                let installed = data_dir.join("vvm").join(entry.vvm_file).exists();
                let consented = has_consent(data_dir, entry.key);
                serde_json::json!({
                    "key": entry.key,
                    "name": entry.name,
                    "credit": entry.credit,
                    "termsUrl": entry.terms_url,
                    "sizeMb": entry.size_mb,
                    "vvmFile": entry.vvm_file,
                    "installed": installed,
                    "consented": consented,
                    "isA": entry.key == voice_a,
                    "isB": entry.key == voice_b,
                })
            })
            .collect();
        Ok(serde_json::to_string(&catalog).unwrap_or_else(|_| "[]".to_string()))
    });
    string_result(&mut env, result)
}

#[no_mangle]
pub extern "C" fn Java_jp_memorianotebook_Bridge_recordVoiceConsent(
    mut env: JNIEnv,
    _class: JClass,
    key: JString,
) -> jstring {
    let key_str = match jstring_to_string(&mut env, &key) {
        Ok(s) => s,
        Err(e) => return string_result(&mut env, Err(e)),
    };
    let result = with_state(|host| {
        if voice_by_key(&key_str).is_none() {
            return voice_err(format!("unknown voice: {key_str}"));
        }
        record_consent(&host.data_dir, &key_str)
            .map_err(notebook_core::CoreError::GeneratorError)?;
        Ok("OK".to_string())
    });
    string_result(&mut env, result)
}

#[no_mangle]
pub extern "C" fn Java_jp_memorianotebook_Bridge_downloadVoice(
    mut env: JNIEnv,
    _class: JClass,
    key: JString,
) -> jstring {
    let key_str = match jstring_to_string(&mut env, &key) {
        Ok(s) => s,
        Err(e) => return string_result(&mut env, Err(e)),
    };
    let entry = match voice_by_key(&key_str) {
        Some(e) => *e,
        None => {
            return string_result(&mut env, Err(format!("unknown voice: {key_str}")));
        }
    };
    let data_dir = {
        let guard = match STATE.lock() {
            Ok(g) => g,
            Err(_) => return string_result(&mut env, Err("state poisoned".to_string())),
        };
        match guard.as_ref() {
            Some(host) => host.data_dir.clone(),
            None => return string_result(&mut env, Err("not initialized".to_string())),
        }
    };
    if !has_consent(&data_dir, entry.key) {
        return string_result(
            &mut env,
            Err("先に利用規約への同意が必要です".to_string()),
        );
    }
    static VOICE_DOWNLOADING: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);
    if VOICE_DOWNLOADING
        .compare_exchange(
            false,
            true,
            std::sync::atomic::Ordering::SeqCst,
            std::sync::atomic::Ordering::SeqCst,
        )
        .is_err()
    {
        return string_result(&mut env, Err("download already in progress".to_string()));
    }
    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            VOICE_DOWNLOADING.store(false, std::sync::atomic::Ordering::SeqCst);
        }
    }
    let _guard = Guard;
    let dir = data_dir.join("vvm");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return string_result(&mut env, Err(e.to_string()));
    }
    let dest = dir.join(entry.vvm_file);
    if dest.exists() {
        return string_result(&mut env, Ok("OK 0".to_string()));
    }
    let result = notebook_net::download_to_file(&entry.vvm_url(), &dest, |_, _| {})
        .map(|n| format!("OK {n}"))
        .map_err(|e| e.to_string());
    string_result(&mut env, result)
}

#[no_mangle]
pub extern "C" fn Java_jp_memorianotebook_Bridge_setVoicePair(
    mut env: JNIEnv,
    _class: JClass,
    a: JString,
    b: JString,
) -> jstring {
    let a_str = match jstring_to_string(&mut env, &a) {
        Ok(s) => s,
        Err(e) => return string_result(&mut env, Err(e)),
    };
    let b_str = match jstring_to_string(&mut env, &b) {
        Ok(s) => s,
        Err(e) => return string_result(&mut env, Err(e)),
    };
    let result = with_state(|host| {
        save_voice_pair(&host.data_dir, &a_str, &b_str)
            .map_err(notebook_core::CoreError::GeneratorError)?;
        Ok("OK".to_string())
    });
    string_result(&mut env, result)
}

#[no_mangle]
pub extern "C" fn Java_jp_memorianotebook_Bridge_listVoiceSummaries(
    mut env: JNIEnv,
    _class: JClass,
    nb: jlong,
) -> jstring {
    let result = with_state(|host| {
        let mut summaries = load_summaries(&host.data_dir);
        summaries.retain(|s| s.nb == nb as u64);
        summaries.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(serde_json::to_string(&summaries).unwrap_or_else(|_| "[]".to_string()))
    });
    string_result(&mut env, result)
}

#[no_mangle]
pub extern "C" fn Java_jp_memorianotebook_Bridge_deleteVoiceSummary(
    mut env: JNIEnv,
    _class: JClass,
    id: jlong,
) -> jstring {
    let result = with_state(|host| {
        let removed = remove_summary(&host.data_dir, id as u64)
            .map_err(notebook_core::CoreError::GeneratorError)?;
        if let Some(summary) = removed {
            let wav_path = host.data_dir.join("audio").join(&summary.wav_file);
            let _ = std::fs::remove_file(wav_path);
        }
        Ok("OK".to_string())
    });
    string_result(&mut env, result)
}

#[no_mangle]
pub extern "C" fn Java_jp_memorianotebook_Bridge_generateVoiceSummary(
    mut env: JNIEnv,
    _class: JClass,
    nb: jlong,
    focus: JString,
    dict_dir: JString,
    ort_lib: JString,
    at: jlong,
) -> jstring {
    let focus_str = match jstring_to_string(&mut env, &focus) {
        Ok(s) => s,
        Err(e) => return string_result(&mut env, Err(e)),
    };
    let dict_dir_str = match jstring_to_string(&mut env, &dict_dir) {
        Ok(s) => s,
        Err(e) => return string_result(&mut env, Err(e)),
    };
    let ort_lib_str = match jstring_to_string(&mut env, &ort_lib) {
        Ok(s) => s,
        Err(e) => return string_result(&mut env, Err(e)),
    };
    // Snapshot everything the generation needs under a brief lock;
    // the script call (network) and the synthesis (on-device, can
    // take minutes) must not hold the global STATE lock, or every
    // other JNI call stalls behind them.
    struct GenSnap {
        base_url: String,
        api_key: String,
        model: String,
        voice_a_key: String,
        voice_b_key: String,
        style_a: u32,
        style_b: u32,
        path_a: std::path::PathBuf,
        path_b: std::path::PathBuf,
        title: String,
        source_text: String,
        data_dir: std::path::PathBuf,
    }
    let snap = match with_state(|host| {
        // (a) The script is written by the API seat only.
        if host.gen_seat != GenSeat::Api || !host.api_config.is_complete() {
            return voice_err(
                "音声概要の台本には API 設定が必要です。設定で API を設定してください"
                    .to_string(),
            );
        }
        // (b) The pair of voices must both be installed.
        let (voice_a_key, voice_b_key) = load_voice_pair(&host.data_dir);
        let voice_a_entry = match voice_by_key(&voice_a_key) {
            Some(e) => e,
            None => return voice_err(format!("unknown voice: {voice_a_key}")),
        };
        let voice_b_entry = match voice_by_key(&voice_b_key) {
            Some(e) => e,
            None => return voice_err(format!("unknown voice: {voice_b_key}")),
        };
        let vvm_dir = host.data_dir.join("vvm");
        let path_a = vvm_dir.join(voice_a_entry.vvm_file);
        let path_b = vvm_dir.join(voice_b_entry.vvm_file);
        if !path_a.exists() || !path_b.exists() {
            return voice_err(
                "声が未導入です。設定の「対話の声」からダウンロードしてください".to_string(),
            );
        }
        // (c) Notebook title.
        let title = match host
            .core
            .notebooks()
            .iter()
            .find(|n| n.id == nb as u64)
        {
            Some(n) => n.title.clone(),
            None => {
                return Err(notebook_core::CoreError::NotebookNotFound(nb as u64));
            }
        };
        // (d) Source text.
        let source_text = host.core.notebook_source_text(nb as u64, 12000);
        if source_text.trim().is_empty() {
            return voice_err("ソースがありません".to_string());
        }
        Ok(GenSnap {
            base_url: host.api_config.base_url.clone(),
                        api_key: host.api_config.api_key.clone(),
            model: host.api_config.model.clone(),
            style_a: voice_a_entry.style_id,
            style_b: voice_b_entry.style_id,
            voice_a_key,
            voice_b_key,
            path_a,
            path_b,
            title,
            source_text,
            data_dir: host.data_dir.clone(),
        })
    }) {
        Ok(snap) => snap,
        Err(e) => return string_result(&mut env, Err(e)),
    };
    let result: Result<String, String> = (|| {
        use notebook_core::Generator as _;
        // (e) Script via the API generator.
        let focus_opt = if focus_str.trim().is_empty() {
            None
        } else {
            Some(focus_str.as_str())
        };
        let prompt =
            notebook_core::voice::build_script_prompt(&snap.title, &snap.source_text, focus_opt);
        let mut generator = ApiGenerator {
            base_url: snap.base_url.clone(),
                        api_key: snap.api_key.clone(),
            model: snap.model.clone(),
        };
        let script_text = generator.generate(&prompt)?;
        // (f) Parse the dialogue.
        let lines = notebook_core::voice::parse_script(&script_text)
            .map_err(notebook_core::CoreError::GeneratorError)?;
        // (g) Synthesize on-device.
        let mut synth =
            notebook_core::voice::VoiceSynth::new(&ort_lib_str, std::path::Path::new(&dict_dir_str))
                .map_err(notebook_core::CoreError::GeneratorError)?;
        synth
            .load_vvm(&snap.path_a)
            .map_err(notebook_core::CoreError::GeneratorError)?;
        if snap.path_b != snap.path_a {
            synth
                .load_vvm(&snap.path_b)
                .map_err(notebook_core::CoreError::GeneratorError)?;
        }
        let mut wavs = Vec::with_capacity(lines.len());
        for (speaker, text) in &lines {
            let style_id = if *speaker == 0 {
                snap.style_a
            } else {
                snap.style_b
            };
            let wav = synth
                .tts(text, style_id)
                .map_err(notebook_core::CoreError::GeneratorError)?;
            wavs.push(wav);
        }
        let final_wav = notebook_core::voice::concat_wavs(&wavs, 350)
            .map_err(notebook_core::CoreError::GeneratorError)?;
        // (h) Save the WAV.
        let mut id = at as u64;
        let existing = load_summaries(&snap.data_dir);
        while existing.iter().any(|s| s.id == id) {
            id += 1;
        }
        let audio_dir = snap.data_dir.join("audio");
        std::fs::create_dir_all(&audio_dir)
            .map_err(|e| notebook_core::CoreError::GeneratorError(e.to_string()))?;
        let wav_file = format!("{id}.wav");
        std::fs::write(audio_dir.join(&wav_file), &final_wav)
            .map_err(|e| notebook_core::CoreError::GeneratorError(e.to_string()))?;
        // (i) Record it.
        let rec = notebook_core::voice::AudioSummary {
            id,
            nb: nb as u64,
            title: snap.title.clone(),
            wav_file,
            voice_a: snap.voice_a_key.clone(),
            voice_b: snap.voice_b_key.clone(),
            created_at: at as u64,
            duration_sec: notebook_core::voice::wav_duration_sec(&final_wav).unwrap_or(0.0),
        };
        notebook_core::voice::append_summary(&snap.data_dir, &rec)
            .map_err(notebook_core::CoreError::GeneratorError)?;
        serde_json::to_string(&rec)
            .map_err(|e| notebook_core::CoreError::GeneratorError(e.to_string()))
    })()
    .map_err(|e: notebook_core::CoreError| e.to_string());
    string_result(&mut env, result)
}
