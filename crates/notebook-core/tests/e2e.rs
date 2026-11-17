//! Minimal end-to-end harness (stage 4): notebook -> source -> chunks ->
//! index -> search -> grounding -> fake generation -> citations -> turn.
//! The generator seat is faked here on purpose; the real inference seat
//! lands in stage 5 behind the same flow.

use notebook_core::{
    format_grounding, parse_citations, ChatTurn, CoreResult, CoreState, Embedder, EngineCfg,
    Role, SourceKind,
};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Bag-of-words feature-hashing embedder (same family as the unit-test
/// fake): texts sharing words land close together.
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

fn temp_dir() -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("nb-e2e-{}-{}", std::process::id(), nanos));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn e2e_question_to_cited_answer() {
    let t0 = Instant::now();
    let dir = temp_dir();
    let mut st = CoreState::new(&dir, EngineCfg::default(), "e2e").unwrap();

    // 1. Notebook + source with three well-separated topics.
    let nb = st.create_notebook("E2E", 1).unwrap();
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
    assert_eq!(chunks.len(), 3, "three topics must make three chunks");
    println!("stage chunks: {} in {:?}", chunks.len(), t0.elapsed());

    // 2. Index everything.
    let mut emb = BowEmbedder { dim: 64 };
    let indexed = st.index_source(src.id, &mut emb).unwrap();
    assert_eq!(indexed, 3);
    println!("stage index: {} vectors", indexed);

    // 3. Question -> search. The cat chunk must win for a cat question.
    let q = emb.embed("猫 魚").unwrap();
    let hits = st.search(nb.id, &q, 2);
    assert_eq!(hits.len(), 2);
    assert!(
        hits[0].0.text.contains("猫"),
        "top hit must be the cat chunk, got: {}",
        hits[0].0.text
    );
    println!(
        "stage search: top chunk {} score {:.3}, second {:.3}",
        hits[0].0.id, hits[0].1, hits[1].1
    );

    // 4. Grounding text carries the citation markers.
    let grounding = format_grounding(&hits);
    assert!(grounding.contains(&format!("[#{}]", hits[0].0.id)));
    println!("stage grounding: {} chars", grounding.chars().count());

    // 5. Fake generation: an answer that cites the top chunk, plus one
    //    marker that does not exist (must be dropped by validation).
    let answer = format!(
        "猫は魚が好きです[#{}]。存在しない出典[#9999]は落ちるはず。",
        hits[0].0.id
    );
    let cits = parse_citations(&answer, &hits);
    assert_eq!(cits.len(), 1);
    assert_eq!(cits[0].chunk_id, Some(hits[0].0.id));
    assert_eq!(cits[0].source_id, src.id);
    println!("stage citations: {} kept, bogus marker dropped", cits.len());

    // 6. Persist the turn and read it back through a fresh state.
    st.append_turn(&ChatTurn {
        nb: nb.id,
        role: Role::User,
        text: "猫は何が好き？".into(),
        citations: vec![],
        at: 3,
    })
    .unwrap();
    st.append_turn(&ChatTurn {
        nb: nb.id,
        role: Role::Assistant,
        text: answer.clone(),
        citations: cits,
        at: 4,
    })
    .unwrap();
    drop(st);
    let st2 = CoreState::new(&dir, EngineCfg::default(), "e2e").unwrap();
    let turns = st2.turns_of(nb.id).unwrap();
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[1].citations.len(), 1);
    assert_eq!(turns[1].citations[0].chunk_id, Some(hits[0].0.id));
    println!("stage persist: 2 turns round-tripped, total {:?}", t0.elapsed());
}
