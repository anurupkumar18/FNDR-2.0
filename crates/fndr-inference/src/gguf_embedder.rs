//! The concrete Qwen3-Embedding-0.6B GGUF `Embedder` (T-402), backed by
//! llama.cpp via `llama-cpp-2`. Loads the pinned artifact from the model
//! registry, runs on CPU by default (enable the `metal` feature for Metal
//! acceleration on Apple Silicon; off by default so this crate keeps
//! building everywhere, including CI runners without a GPU/Metal backend).
//!
//! One `LlamaBackend`/`LlamaModel` per `GgufEmbedder`, shared across calls
//! (invariant 4: expensive resources are constructed once); a fresh
//! `LlamaContext` is created per `embed_documents` call and its KV cache is
//! cleared between texts, since each text is an independent sequence, not a
//! continuation of the previous one.

use std::num::NonZeroU32;
use std::path::Path;

use llama_cpp_2::context::params::{LlamaContextParams, LlamaPoolingType};
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{AddBos, LlamaModel};

use crate::embedding::{EmbedError, Embedder, EmbeddingSpec, truncate_and_renormalize};

/// The context window given to each text. Chunk text is composed and
/// byte-budgeted upstream (fndr-textsignal); this is a generous ceiling, not
/// a claim about the pipeline's own limits.
const N_CTX_TOKENS: u32 = 2048;

pub struct GgufEmbedder {
    backend: LlamaBackend,
    model: LlamaModel,
    spec: EmbeddingSpec,
}

impl GgufEmbedder {
    /// Load `model_path` (a GGUF file) as the given contract's backing
    /// model. The caller is responsible for having verified the artifact
    /// against the registry (`fndr_downloader::download_verified` /
    /// `verify_file`) before calling this — this constructor does not
    /// re-check the checksum, only that llama.cpp can load the file.
    pub fn load(model_path: &Path, spec: EmbeddingSpec) -> Result<Self, EmbedError> {
        let backend = LlamaBackend::init()
            .map_err(|e| EmbedError::Unavailable(format!("llama backend init: {e}")))?;
        let model = LlamaModel::load_from_file(&backend, model_path, &LlamaModelParams::default())
            .map_err(|e| {
                EmbedError::Unavailable(format!("loading {}: {e}", model_path.display()))
            })?;
        Ok(Self {
            backend,
            model,
            spec,
        })
    }

    fn embed_one(&self, text: &str) -> Result<Vec<f32>, EmbedError> {
        let ctx_params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(N_CTX_TOKENS))
            .with_embeddings(true)
            .with_pooling_type(LlamaPoolingType::Last);
        let mut ctx = self
            .model
            .new_context(&self.backend, ctx_params)
            .map_err(|e| EmbedError::Failed(format!("context init: {e}")))?;

        let tokens = self
            .model
            .str_to_token(text, AddBos::Always)
            .map_err(|e| EmbedError::Failed(format!("tokenize: {e}")))?;
        if tokens.is_empty() {
            return Err(EmbedError::Failed(
                "tokenizer produced zero tokens for non-empty input".to_owned(),
            ));
        }
        if tokens.len() as u32 > N_CTX_TOKENS {
            return Err(EmbedError::Failed(format!(
                "text tokenizes to {} tokens, exceeds the {N_CTX_TOKENS}-token context window",
                tokens.len()
            )));
        }

        let mut batch = LlamaBatch::new(tokens.len(), 1);
        batch
            .add_sequence(&tokens, 0, false)
            .map_err(|e| EmbedError::Failed(format!("batch construction: {e}")))?;

        ctx.clear_kv_cache();
        ctx.decode(&mut batch)
            .map_err(|e| EmbedError::Failed(format!("decode: {e}")))?;

        let raw = ctx
            .embeddings_seq_ith(0)
            .map_err(|e| EmbedError::Failed(format!("embeddings extraction: {e}")))?;
        if raw.iter().all(|x| *x == 0.0) {
            return Err(EmbedError::Failed(
                "model returned an all-zero embedding (invariant 4: never written)".to_owned(),
            ));
        }
        Ok(raw.to_vec())
    }
}

impl Embedder for GgufEmbedder {
    fn spec(&self) -> &EmbeddingSpec {
        &self.spec
    }

    fn embed_documents(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        texts
            .iter()
            .map(|text| {
                let native = self.embed_one(text)?;
                Ok(truncate_and_renormalize(&native, self.spec.dim))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CHUNK_EMBEDDING_V1;

    /// This is the AC's "construction probe (dimension, non-zero)" — but it
    /// needs the real ~640MB GGUF model on disk (`models/`, gitignored,
    /// never present on a fresh checkout or in CI) and takes real CPU time
    /// to run inference. It MUST stay `#[ignore]`d: `cargo test --workspace`
    /// runs in CI on every PR with no model file present, so an un-ignored
    /// version of this test would fail on every single PR, forever. Run it
    /// explicitly after `cargo run -p fndr-downloader --example fetch_model`:
    /// `cargo test -p fndr-inference -- --ignored`.
    #[test]
    #[ignore = "needs the real GGUF model downloaded to models/ (fetch_model example); never runs in CI"]
    fn construction_probe_dimension_and_non_zero() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../models/Qwen3-Embedding-0.6B-Q8_0.gguf");
        let embedder =
            GgufEmbedder::load(&path, CHUNK_EMBEDDING_V1).expect("model should load from disk");

        let docs = embedder
            .embed_documents(&["a chunk of captured screen text".to_owned()])
            .expect("embedding a normal document should succeed");
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].len(), CHUNK_EMBEDDING_V1.dim);
        assert!(
            docs[0].iter().any(|x| *x != 0.0),
            "embedding must not be all-zero"
        );
        let norm: f32 = docs[0].iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            (norm - 1.0).abs() < 1e-3,
            "truncated embedding should be L2-renormalized, got norm {norm}"
        );

        let query_vec = embedder
            .embed_query("what did I write about the schema")
            .expect("embedding a query should succeed");
        assert_eq!(query_vec.len(), CHUNK_EMBEDDING_V1.dim);
        assert_ne!(
            query_vec, docs[0],
            "a real query and a real document should not embed identically"
        );
    }
}
