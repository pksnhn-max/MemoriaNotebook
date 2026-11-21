//! Host core E2E: the citation gate, driven through the real split
//! flow (ask_prepare -> ask_generate_with_sink -> ask_commit) with a
//! scripted streaming generator. The gate's ruled shape (2026-11-18):
//! with grounding hits present, an answer with no valid citation is
//! retried exactly once on the sterner prompt; if the retry also has
//! no valid citation, the recorded answer is the fixed refusal
//! 「資料にありません。」. Streamed pieces are provisional: a Reset
//! event discards everything shown for the failed attempt.

mod common;

use common::{temp_dir, BowEmbedder, QueueGenerator};
use notebook_core::{
    ask_generate_with_sink, CoreState, EngineCfg, Role, SourceKind, StreamEvent,
};

/// One notebook, one source, three well-separated topics, indexed.
fn staged() -> (std::path::PathBuf, CoreState, u64, u64, BowEmbedder) {
    let dir = temp_dir("gate");
    let mut st = CoreState::new(&dir, EngineCfg::default(), "w5-gate").unwrap();
    let nb = st.create_notebook("Gate", 1).unwrap();
    let src = st
        .add_source(nb.id, SourceKind::Text, "animals", "/animals.txt", 2)
        .unwrap();
    let text = format!(
        "{}\n\n{}\n\n{}",
        "猫 は 魚 が 好き で 昼寝 を する 。 ".repeat(30),
        "犬 は 公園 を 散歩 し て 走る 。 ".repeat(30),
        "鳥 は 空 を 飛び 歌 を 歌う 。 ".repeat(30)
    );
    let chunks = st.set_source_text(src.id, &text).unwrap();
    assert_eq!(chunks.len(), 3);
    let mut emb = BowEmbedder { dim: 64 };
    assert_eq!(st.index_source(src.id, &mut emb).unwrap(), 3);
    (dir, st, nb.id, src.id, emb)
}

fn record(events: &mut Vec<String>) -> impl FnMut(StreamEvent) + '_ {
    move |ev| match ev {
        StreamEvent::Token(t) => events.push(format!("T:{t}")),
        StreamEvent::Reset => events.push("R".to_string()),
    }
}

fn tokens(events: &[String]) -> Vec<String> {
    events
        .iter()
        .filter(|e| e.starts_with("T:"))
        .map(|e| e[2..].to_string())
        .collect()
}

#[test]
fn gate_retry_adopts_the_cited_retry_answer() {
    let (dir, mut st, nb, src, mut emb) = staged();
    let prepared = st.ask_prepare(nb, "猫 魚", &mut emb, 2).unwrap();
    assert_eq!(prepared.hits.len(), 2);
    let top_id = prepared.hits[0].0.id;
    assert!(prepared.hits[0].0.text.contains("猫"));

    // First attempt cites only a marker that does not exist; the retry
    // cites the top chunk for real.
    let first = "猫は魚が好きです[#9999]。".to_string();
    let second = format!("猫は魚が好きです[#{top_id}]。");
    let mut gen = QueueGenerator::new(vec![first.clone(), second.clone()], 4);

    let mut events: Vec<String> = Vec::new();
    let answer = ask_generate_with_sink(&prepared, &mut gen, &mut record(&mut events)).unwrap();

    // The generator saw the prompt, then the sterner retry prompt.
    assert_eq!(gen.prompts, vec![prepared.prompt.clone(), prepared.retry_prompt.clone()]);
    // Event shape: first attempt's pieces, one Reset, retry's pieces.
    let resets = events.iter().filter(|e| *e == "R").count();
    assert_eq!(resets, 1, "events: {events:?}");
    let reset_at = events.iter().position(|e| e == "R").unwrap();
    assert_eq!(tokens(&events[..reset_at]).concat(), first);
    assert_eq!(tokens(&events[reset_at + 1..]).concat(), second);
    // The retry answer is the one adopted, with its citation validated.
    assert_eq!(answer.text, second);
    assert_eq!(answer.citations.len(), 1);
    assert_eq!(answer.citations[0].chunk_id, Some(top_id));
    assert_eq!(answer.citations[0].source_id, src);

    // Committed and read back through a fresh state.
    let turn = st.ask_commit(prepared, answer, 10).unwrap();
    assert_eq!(turn.role, Role::Assistant);
    drop(st);
    let st2 = CoreState::new(&dir, EngineCfg::default(), "w5-gate").unwrap();
    let turns = st2.turns_of(nb).unwrap();
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[1].text, second);
    assert_eq!(turns[1].citations.len(), 1);
    assert_eq!(turns[1].citations[0].chunk_id, Some(top_id));
}

#[test]
fn gate_refuses_with_the_fixed_text_after_two_citationless_answers() {
    let (dir, mut st, nb, _src, mut emb) = staged();
    let prepared = st.ask_prepare(nb, "猫 魚", &mut emb, 2).unwrap();
    assert!(!prepared.hits.is_empty());

    let first = "根拠なしの答えです。".to_string();
    let second = "やはり分かりません[#9999]。".to_string();
    let mut gen = QueueGenerator::new(vec![first.clone(), second.clone()], 3);

    let mut events: Vec<String> = Vec::new();
    let answer = ask_generate_with_sink(&prepared, &mut gen, &mut record(&mut events)).unwrap();

    assert_eq!(gen.prompts.len(), 2, "exactly one retry");
    // Two Resets: one discarding the first attempt, one for the refusal.
    let resets = events.iter().filter(|e| *e == "R").count();
    assert_eq!(resets, 2, "events: {events:?}");
    assert_eq!(events.last().unwrap(), "R");
    assert_eq!(answer.text, "資料にありません。");
    assert!(answer.citations.is_empty());

    let turn = st.ask_commit(prepared, answer, 10).unwrap();
    assert_eq!(turn.text, "資料にありません。");
    drop(st);
    let st2 = CoreState::new(&dir, EngineCfg::default(), "w5-gate").unwrap();
    let turns = st2.turns_of(nb).unwrap();
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[1].text, "資料にありません。");
    assert!(turns[1].citations.is_empty());
}

#[test]
fn streaming_tokens_flow_in_order_without_reset_when_first_answer_is_cited() {
    let (_dir, mut st, nb, _src, mut emb) = staged();
    let prepared = st.ask_prepare(nb, "猫 魚", &mut emb, 2).unwrap();
    let top_id = prepared.hits[0].0.id;
    let full = format!("猫は魚が好きで[#{top_id}]、昼寝をします。");
    let mut gen = QueueGenerator::new(vec![full.clone()], 3);

    let mut events: Vec<String> = Vec::new();
    let answer = ask_generate_with_sink(&prepared, &mut gen, &mut record(&mut events)).unwrap();

    assert_eq!(gen.prompts.len(), 1, "no retry when the first answer is cited");
    assert!(
        !events.iter().any(|e| e == "R"),
        "no Reset expected: {events:?}"
    );
    let pieces = tokens(&events);
    assert!(pieces.len() > 1, "the answer must arrive in pieces: {pieces:?}");
    assert_eq!(pieces.concat(), full);
    assert_eq!(answer.text, full);
    assert_eq!(answer.citations.len(), 1);
    assert_eq!(answer.citations[0].chunk_id, Some(top_id));
}
