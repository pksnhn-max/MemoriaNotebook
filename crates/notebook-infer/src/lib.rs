//! Local generation seat: llama.cpp (via llama-cpp-2) behind core's
//! `Generator` trait. The model file is loaded once; every `generate`
//! call runs in a FRESH context — no conversational state is carried
//! inside the model, the ledger (core) is the only memory.

use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::LlamaModel;
use llama_cpp_2::sampling::LlamaSampler;
use notebook_core::{CoreError, CoreResult, Generator};
use std::num::NonZeroU32;
use std::path::PathBuf;

pub struct LlamaGenerator {
    backend: LlamaBackend,
    model: LlamaModel,
    pub model_path: PathBuf,
    max_tokens: usize,
}

fn cerr<E: std::fmt::Debug>(what: &str) -> impl Fn(E) -> CoreError + '_ {
    move |e| CoreError::GeneratorError(format!("{what}: {e:?}"))
}

impl LlamaGenerator {
    pub fn load(model_path: PathBuf) -> CoreResult<Self> {
        let backend = LlamaBackend::init().map_err(cerr("llama backend"))?;
        let model = LlamaModel::load_from_file(&backend, &model_path, &LlamaModelParams::default())
            .map_err(cerr("llama model load"))?;
        Ok(Self {
            backend,
            model,
            model_path,
            max_tokens: 512,
        })
    }
}

impl Generator for LlamaGenerator {
    fn generate(&mut self, prompt: &str) -> CoreResult<String> {
        self.generate_streaming(prompt, &mut |_| {})
    }

    fn generate_streaming(
        &mut self,
        prompt: &str,
        on_piece: &mut dyn FnMut(&str),
    ) -> CoreResult<String> {
        let vocab = self.model.vocab();
        let ctx_params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(4096))
            .with_n_threads(4)
            .with_n_threads_batch(4);
        let mut ctx = self
            .model
            .new_context(&self.backend, ctx_params)
            .map_err(cerr("llama context"))?;

        // ChatML wrap: core's prompt is template-agnostic plain text;
        // dressing it for the model is this seat's job.
        let dressed = format!("<|im_start|>user\n{prompt}<|im_end|>\n<|im_start|>assistant\n");
        let tokens = vocab.tokenize(dressed.as_bytes(), true, true);
        if tokens.len() + self.max_tokens > 4096 {
            return Err(CoreError::GeneratorError(
                "prompt too long for context".to_string(),
            ));
        }
        let mut batch = LlamaBatch::new(4096, 1);
        let last = tokens.len() - 1;
        for (i, tok) in tokens.iter().enumerate() {
            batch.add(*tok, i as i32, &[0], i == last).map_err(cerr("batch add"))?;
        }
        ctx.decode(&mut batch).map_err(cerr("decode prompt"))?;

        let mut sampler = LlamaSampler::chain_simple([
            LlamaSampler::top_k(40),
            LlamaSampler::top_p(0.9, 1),
            LlamaSampler::temp(0.7),
            LlamaSampler::dist(1234),
        ]);
        let mut n_cur = tokens.len() as i32;
        let mut out = String::new();
        let mut decoder = encoding_rs::UTF_8.new_decoder();
        let mut piece: Vec<u8> = Vec::new();
        for _ in 0..self.max_tokens {
            let tok = sampler.sample(&ctx, batch.n_tokens() - 1);
            sampler.accept(tok);
            if vocab.is_eog(tok) {
                break;
            }
            piece.clear();
            vocab.token_to_piece_into(tok, &mut piece, true, None);
            let max = decoder.max_utf8_buffer_length(piece.len()).unwrap_or(32);
            let mut s = String::with_capacity(max);
            decoder.decode_to_string(&piece, &mut s, false);
            out.push_str(&s);
            on_piece(&s);
            batch.clear();
            batch.add(tok, n_cur, &[0], true).map_err(cerr("batch add gen"))?;
            n_cur += 1;
            ctx.decode(&mut batch).map_err(cerr("decode gen"))?;
        }
        // Flush the decoder so a trailing partial sequence surfaces
        // (as U+FFFD) instead of vanishing.
        let mut tail = String::new();
        decoder.decode_to_string(&[], &mut tail, true);
        out.push_str(&tail);
        if !tail.is_empty() {
            on_piece(&tail);
        }
        Ok(out.trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use notebook_core::{CoreState, EngineCfg, SourceKind};

    /// Real-model E2E through core's ask(). Runs only when
    /// TINYSWALLOW_GGUF points at the Q5_K_M file.
    #[test]
    fn e2e_ask_with_real_model_when_env_set() {
        let Ok(path) = std::env::var("TINYSWALLOW_GGUF") else {
            return;
        };
        let dir = std::env::temp_dir().join(format!("nb-infer-e2e-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut st = CoreState::new(&dir, EngineCfg::default(), "m").unwrap();
        let nb = st.create_notebook("推論テスト", 1).unwrap();
        let src = st
            .add_source(nb.id, SourceKind::Text, "川辺の記録", "/kawabe.txt", 2)
            .unwrap();
        st.set_source_text(
            src.id,
            "天満川の水位は南観音の観測所で測っている。記録は川辺の集会所に保管される。\n\nパンの配合は強力粉と薄力粉を混ぜ、発酵は一晩かける。",
        )
        .unwrap();
        // Index with a trivial embedder (BoW) so search can run.
        struct Bow;
        impl notebook_core::Embedder for Bow {
            fn embed(&mut self, text: &str) -> CoreResult<Vec<f32>> {
                let mut v = vec![0f32; 64];
                for w in text.split(|c: char| c.is_whitespace() || "、。！？".contains(c)) {
                    if w.is_empty() {
                        continue;
                    }
                    let mut h: u64 = 0xcbf29ce484222325;
                    for b in w.as_bytes() {
                        h ^= *b as u64;
                        h = h.wrapping_mul(0x100000001b3);
                    }
                    v[(h % 64) as usize] += 1.0;
                }
                Ok(v)
            }
        }
        let mut emb = Bow;
        st.index_source(src.id, &mut emb).unwrap();
        let mut gen = LlamaGenerator::load(PathBuf::from(path)).unwrap();
        let turn = st
            .ask(nb.id, "天満川の水位はどこで測っていますか", &mut emb, &mut gen, 5, 3)
            .unwrap();
        assert!(turn.text.contains("南観音"), "answer: {}", turn.text);
        assert!(!turn.citations.is_empty(), "citations: {}", turn.text);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
