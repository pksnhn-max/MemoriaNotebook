//! Shared fakes for the host core E2E tests. The embedder and the
//! generator are the two seats the brief allows to fake; everything
//! between them (chunking, indexing, search, grounding, the citation
//! gate, persistence) is the real core code.
#![allow(dead_code)]

use notebook_core::{CoreError, CoreResult, Embedder, Generator};
use std::collections::VecDeque;
use std::time::{SystemTime, UNIX_EPOCH};

/// Bag-of-words feature-hashing embedder (same family as the one in
/// tests/e2e.rs): texts sharing words land close together.
pub struct BowEmbedder {
    pub dim: usize,
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

pub fn temp_dir(tag: &str) -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "nb-w5-{}-{}-{}",
        tag,
        std::process::id(),
        nanos
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A generator that answers from a fixed queue, recording every prompt
/// it was called with. `generate_streaming` replays the answer in
/// pieces of `piece_chars` characters (0 = one whole piece), so tests
/// can watch the Token/Reset event sequence at the sink.
pub struct QueueGenerator {
    pub responses: VecDeque<String>,
    pub prompts: Vec<String>,
    pub piece_chars: usize,
}

impl QueueGenerator {
    pub fn new(responses: Vec<String>, piece_chars: usize) -> Self {
        QueueGenerator {
            responses: responses.into(),
            prompts: Vec::new(),
            piece_chars,
        }
    }
}

impl Generator for QueueGenerator {
    fn generate(&mut self, prompt: &str) -> CoreResult<String> {
        self.prompts.push(prompt.to_string());
        self.responses
            .pop_front()
            .ok_or_else(|| CoreError::GeneratorError("queue exhausted".into()))
    }

    fn generate_streaming(
        &mut self,
        prompt: &str,
        on_piece: &mut dyn FnMut(&str),
    ) -> CoreResult<String> {
        let text = self.generate(prompt)?;
        if self.piece_chars == 0 {
            if !text.is_empty() {
                on_piece(&text);
            }
        } else {
            let chars: Vec<char> = text.chars().collect();
            for piece in chars.chunks(self.piece_chars) {
                let s: String = piece.iter().collect();
                on_piece(&s);
            }
        }
        Ok(text)
    }
}
