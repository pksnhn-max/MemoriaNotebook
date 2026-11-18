//! Stage 6c: the real embedder seat — ruri-v3-30m (int8 ONNX,
//! 256-dim) behind notebook-core's `Embedder` trait.
//!
//! Kept out of the core on purpose (the core's dependency stays
//! serde-only): this crate owns the ONNX Runtime session and the
//! tokenizer. The runtime library is loaded dynamically from a
//! path the host supplies, so the same crate runs on the desktop
//! (x86_64 .so) and on Android (arm64 .so from the official AAR).
//!
//! Pooling follows the model's contract (recon, verified on host
//! 2026-11-17): inputs are `input_ids` + `attention_mask` only;
//! the output `token_embeddings` is mean-pooled with the mask and
//! L2-normalized, giving 256-dim unit vectors.

use notebook_core::{CoreError, CoreResult, Embedder};
use ort::session::Session;
use ort::value::Tensor;
use tokenizers::Tokenizer;

/// Embedding dimension of ruri-v3-30m.
pub const DIM: usize = 256;

/// Identity tag for the index: stored embeddings are only valid
/// for the embedder that produced them. The host compares this
/// against a tag file and re-indexes everything on a mismatch.
pub const TAG: &str = "ruri-v3-30m-256";

const MAX_LEN: usize = 512;

pub struct RuriEmbedder {
    // The environment must outlive the session.
    _env: ort::environment::Environment,
    session: Session,
    tokenizer: Tokenizer,
}

fn seat_err(e: impl std::fmt::Display) -> CoreError {
    CoreError::GeneratorError(format!("embedder: {e}"))
}

impl RuriEmbedder {
    /// Load the runtime from `runtime_path` (libonnxruntime.so),
    /// the model from `model_path` (model_int8.onnx) and the
    /// tokenizer from `tokenizer_path` (tokenizer.json).
    pub fn new(
        runtime_path: &str,
        model_path: &str,
        tokenizer_path: &str,
    ) -> CoreResult<Self> {
        let env = ort::init_from(runtime_path)
            .map_err(seat_err)?
            .build()
            .map_err(seat_err)?;
        let session = Session::builder(&env)
            .map_err(seat_err)?
            .commit_from_file(model_path)
            .map_err(seat_err)?;
        let mut tokenizer =
            Tokenizer::from_file(tokenizer_path).map_err(seat_err)?;
        let trunc = tokenizers::utils::truncation::TruncationParams {
            max_length: MAX_LEN,
            ..Default::default()
        };
        tokenizer.with_truncation(Some(trunc)).map_err(seat_err)?;
        Ok(RuriEmbedder {
            _env: env,
            session,
            tokenizer,
        })
    }
}

impl Embedder for RuriEmbedder {
    fn embed(&mut self, text: &str) -> CoreResult<Vec<f32>> {
        let enc = self.tokenizer.encode(text, true).map_err(seat_err)?;
        let ids: Vec<i64> = enc.get_ids().iter().map(|&x| x as i64).collect();
        let mask: Vec<i64> = enc
            .get_attention_mask()
            .iter()
            .map(|&x| x as i64)
            .collect();
        let n = ids.len();
        if n == 0 {
            return Ok(vec![0.0; DIM]);
        }
        let inputs: Vec<(String, ort::session::SessionInputValue)> = vec![
            (
                "input_ids".to_string(),
                Tensor::from_array((vec![1usize, n], ids))
                    .map_err(seat_err)?
                    .into(),
            ),
            (
                "attention_mask".to_string(),
                Tensor::from_array((vec![1usize, n], mask.clone()))
                    .map_err(seat_err)?
                    .into(),
            ),
        ];
        let outputs = self.session.run(inputs).map_err(seat_err)?;
        let (_shape, data) = outputs[0].try_extract_tensor::<f32>().map_err(seat_err)?;
        if data.len() != n * DIM {
            return Err(seat_err(format!(
                "unexpected embedding shape: {} values for {n} tokens",
                data.len()
            )));
        }
        let mut pooled = vec![0.0f32; DIM];
        let mut msum = 0.0f32;
        for t in 0..n {
            if mask[t] == 1 {
                msum += 1.0;
                for d in 0..DIM {
                    pooled[d] += data[t * DIM + d];
                }
            }
        }
        if msum > 0.0 {
            for v in pooled.iter_mut() {
                *v /= msum;
            }
        }
        let norm = pooled.iter().map(|v| v * v).sum::<f32>().sqrt();
        if norm > 0.0 {
            for v in pooled.iter_mut() {
                *v /= norm;
            }
        }
        Ok(pooled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real-model check, host only: needs RURI_DIR pointing at a
    /// directory with libonnxruntime.so (x86_64), model_int8.onnx
    /// and tokenizer.json. Run with `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn real_model_separates_topics() {
        let base = std::env::var("RURI_DIR").expect("RURI_DIR");
        let mut emb = RuriEmbedder::new(
            &format!("{base}/libonnxruntime.so"),
            &format!("{base}/model_int8.onnx"),
            &format!("{base}/tokenizer.json"),
        )
        .unwrap();
        let q = emb.embed("天満川の水位はどこで測っていますか").unwrap();
        let river = emb
            .embed("天満川の水位は南観音の観測所で測っており、記録は川辺のメモに残している。")
            .unwrap();
        let bread = emb
            .embed("パンの配合は強力粉と薄力粉を混ぜて、発酵は一晩かけるのがよい。")
            .unwrap();
        assert_eq!(q.len(), DIM);
        let cos = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>();
        assert!(cos(&q, &river) > cos(&q, &bread) + 0.1);
    }
}
