//! Stage 6a scaffold: the Android vessel's Rust side.
//!
//! A thin JNI bridge over `notebook-core`. The Kotlin host owns one
//! Activity and calls these functions; all state lives in one
//! `CoreState` behind a mutex (the core's single-owner contract is
//! honoured by serializing here).
//!
//! Seats are placeholders in this scaffold: a bag-of-words embedder
//! and a canned generator stand in until stage 6c (ruri) and 6d/6e
//! (TinySwallow / API) land behind the same traits.
//!
//! The JNI class is `jp.memorianotebook.Bridge` — provisional until
//! the package name is ruled.

use jni::objects::{JClass, JString};
use jni::sys::{jboolean, jlong, jstring};
use jni::JNIEnv;
use notebook_core::{
    ChatTurn, CoreResult, CoreState, Embedder, EngineCfg, Generator, SourceKind,
};
use std::sync::Mutex;

static STATE: Mutex<Option<CoreState>> = Mutex::new(None);

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

/// Placeholder generator: cites the first grounding marker it finds
/// in the prompt, so the citation path is exercised end to end on
/// device. Replaced by real models in stage 6d/6e.
struct CannedGenerator;

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

/// One line per notebook: `id \t title \t created_at`.
fn fmt_notebooks(st: &CoreState) -> String {
    st.notebooks()
        .iter()
        .map(|n| format!("{}\t{}\t{}", n.id, esc(&n.title), n.created_at))
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

fn with_state<T>(f: impl FnOnce(&mut CoreState) -> CoreResult<T>) -> Result<T, String> {
    let mut guard = STATE.lock().map_err(|_| "state poisoned".to_string())?;
    let st = guard.as_mut().ok_or("not initialized")?;
    f(st).map_err(|e| e.to_string())
}

fn jstring_to_string(env: &mut JNIEnv, s: &JString) -> Result<String, String> {
    Ok(env.get_string(s).map_err(|e| e.to_string())?.into())
}

/// Initialize the core at `data_dir`. Returns 1 on success, 0 on failure.
#[no_mangle]
pub extern "C" fn Java_jp_memorianotebook_Bridge_init(
    mut env: JNIEnv,
    _class: JClass,
    data_dir: JString,
) -> jboolean {
    let dir = match jstring_to_string(&mut env, &data_dir) {
        Ok(d) => d,
        Err(_) => return 0,
    };
    match CoreState::new(std::path::Path::new(&dir), EngineCfg::default(), "TinySwallow") {
        Ok(st) => {
            let mut guard = match STATE.lock() {
                Ok(g) => g,
                Err(_) => return 0,
            };
            *guard = Some(st);
            1
        }
        Err(_) => 0,
    }
}

/// Create a notebook; returns its id, or -1 on failure.
#[no_mangle]
pub extern "C" fn Java_jp_memorianotebook_Bridge_createNotebook(
    mut env: JNIEnv,
    _class: JClass,
    title: JString,
) -> jlong {
    let title = match jstring_to_string(&mut env, &title) {
        Ok(t) => t,
        Err(_) => return -1,
    };
    with_state(|st| {
        let nb = st.create_notebook(&title, 0)?;
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
    with_state(|st| {
        let src = st.add_source(nb as u64, SourceKind::Text, &title, "", 0)?;
        st.set_source_text(src.id, &text)?;
        let mut emb = BowEmbedder { dim: 64 };
        st.index_source(src.id, &mut emb)?;
        Ok(src.id as jlong)
    })
    .unwrap_or(-1)
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
    let result = with_state(|st| {
        let mut emb = BowEmbedder { dim: 64 };
        let mut gen = CannedGenerator;
        let turn: ChatTurn = st.ask(nb as u64, &question, &mut emb, &mut gen, 5, 0)?;
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

fn string_result(env: &mut JNIEnv, result: Result<String, String>) -> jstring {
    let text = match result {
        Ok(t) => t,
        Err(e) => format!("ERR: {e}"),
    };
    env.new_string(text)
        .map(|s| s.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

/// List notebooks, one per line: `id \t title \t created_at`.
#[no_mangle]
pub extern "C" fn Java_jp_memorianotebook_Bridge_listNotebooks(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    let result = with_state(|st| Ok(fmt_notebooks(st)));
    string_result(&mut env, result)
}

/// List sources of notebook `nb`, one per line: `id \t title`.
#[no_mangle]
pub extern "C" fn Java_jp_memorianotebook_Bridge_listSources(
    mut env: JNIEnv,
    _class: JClass,
    nb: jlong,
) -> jstring {
    let result = with_state(|st| Ok(fmt_sources(st, nb as u64)));
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
    let result = with_state(|st| fmt_turns(st, nb as u64));
    string_result(&mut env, result)
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(nbs.starts_with(&format!("{}\t川辺\\tの記録\t7", nb.id)));

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

        let _ = std::fs::remove_dir_all(&dir);
    }
}
