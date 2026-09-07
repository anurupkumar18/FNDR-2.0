//! The embedding contract seam consumed by the store's Lance writer.
//!
//! The trait boundary, the instruction/prefix asymmetry
//! (`query_embedding_text`), and the 768d matryoshka rule
//! (`truncate_and_renormalize`) live here (T-402), all app-side per ADR-003
//! so they work regardless of which concrete model backs `Embedder`. Still
//! missing: a concrete Qwen3-Embedding-0.6B Q8_0 GGUF implementation of the
//! trait itself. No mock implementation may reach a production path
//! (invariant 4); test embedders live in test code.

/// One embedding model contract. The Lance table name carries model and
/// dimension (ARCHITECTURE section 5): contracts move together, so a model
/// or dimension change is a new spec with a new table, never an in-place
/// mutation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddingSpec {
    pub model_id: &'static str,
    pub dim: usize,
    pub lance_table: &'static str,
}

/// The v1 chunk-embedding contract (ADR-003).
pub const CHUNK_EMBEDDING_V1: EmbeddingSpec = EmbeddingSpec {
    model_id: "qwen3-embedding-0.6b-q8_0",
    dim: 768,
    lance_table: "chunks_v1_qwen768",
};

#[derive(Debug, thiserror::Error)]
pub enum EmbedError {
    /// Typed unavailability (invariant 4): the caller surfaces this state;
    /// nothing writes zero vectors or falls back to a mock.
    #[error("embedding model unavailable: {0}")]
    Unavailable(String),
    #[error("embedding failed: {0}")]
    Failed(String),
}

pub trait Embedder: Send + Sync {
    fn spec(&self) -> &EmbeddingSpec;

    /// Embed document-side texts (already composed embedding text, no
    /// instruction prefix). Every returned vector must have exactly
    /// `spec().dim` elements; writers refuse anything else.
    fn embed_documents(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError>;

    /// Embed one query. The default wraps `query` with the instruction
    /// prefix (ADR-006 index/query asymmetry) and reuses the document path,
    /// so every `Embedder` gets the asymmetry for free; override only if the
    /// concrete model needs a different query-side mechanism.
    fn embed_query(&self, query: &str) -> Result<Vec<f32>, EmbedError> {
        let text = query_embedding_text(query);
        self.embed_documents(std::slice::from_ref(&text))?
            .into_iter()
            .next()
            .ok_or_else(|| {
                EmbedError::Failed("embedder returned no vector for one query text".to_owned())
            })
    }
}

/// The instruction used on the query side only (Qwen3-Embedding convention:
/// queries carry a task instruction, documents never do). Kept as one named
/// constant so index and query text composition cannot drift independently.
pub const QUERY_INSTRUCTION: &str =
    "Given a search query, retrieve relevant passages that answer the query";

/// Build the exact text sent to the embedder for a query. Never apply this
/// to document/chunk text — the index/query asymmetry this contract requires
/// depends on documents being embedded as-is.
pub fn query_embedding_text(query: &str) -> String {
    format!("Instruct: {QUERY_INSTRUCTION}\nQuery: {query}")
}

/// The matryoshka rule (ADR-006): truncate a native embedding to its leading
/// `target_dim` dimensions, then L2-renormalize. Run this on the raw model
/// output before it is treated as a `spec().dim`-length contract vector. A
/// zero vector renormalizes to itself rather than dividing by zero; the
/// writer's non-zero probe (T-402 AC) is the place that rejects it.
pub fn truncate_and_renormalize(vector: &[f32], target_dim: usize) -> Vec<f32> {
    let mut truncated: Vec<f32> = vector.iter().take(target_dim).copied().collect();
    let norm: f32 = truncated.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in &mut truncated {
            *x /= norm;
        }
    }
    truncated
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Records exactly what text it was asked to embed, so tests can assert
    /// on the index/query asymmetry without a real model (invariant 4: this
    /// stays test-only code, never a production path).
    struct RecordingEmbedder {
        spec: EmbeddingSpec,
        received: Mutex<Vec<String>>,
        dim: usize,
    }

    impl Embedder for RecordingEmbedder {
        fn spec(&self) -> &EmbeddingSpec {
            &self.spec
        }

        fn embed_documents(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
            self.received.lock().unwrap().extend(texts.iter().cloned());
            Ok(texts.iter().map(|_| vec![1.0; self.dim]).collect())
        }
    }

    fn recording_embedder(dim: usize) -> RecordingEmbedder {
        RecordingEmbedder {
            spec: EmbeddingSpec {
                model_id: "test",
                dim,
                lance_table: "test_table",
            },
            received: Mutex::new(Vec::new()),
            dim,
        }
    }

    #[test]
    fn truncate_and_renormalize_yields_unit_length_leading_dims() {
        let native = vec![3.0_f32, 4.0, 0.0, 0.0]; // norm 5 over the first two dims
        let out = truncate_and_renormalize(&native, 2);
        assert_eq!(out, vec![0.6, 0.8]);
        let norm: f32 = out.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            (norm - 1.0).abs() < 1e-6,
            "renormalized vector must be unit length, got {norm}"
        );
    }

    #[test]
    fn truncate_and_renormalize_of_zero_vector_stays_zero() {
        assert_eq!(
            truncate_and_renormalize(&[0.0, 0.0, 0.0], 2),
            vec![0.0, 0.0]
        );
    }

    #[test]
    fn query_text_carries_the_instruction_prefix_documents_never_do() {
        let query = "what did I decide about the schema";
        let text = query_embedding_text(query);
        assert!(text.contains(QUERY_INSTRUCTION));
        assert!(text.contains(query));
        assert_ne!(
            text, query,
            "query embedding text must differ from raw document text"
        );
    }

    #[test]
    fn embed_query_default_impl_applies_the_prefix_before_calling_embed_documents() {
        let embedder = recording_embedder(3);
        let vector = embedder.embed_query("find my notes").unwrap();
        assert_eq!(vector.len(), 3);
        let received = embedder.received.lock().unwrap();
        assert_eq!(received.len(), 1);
        assert_eq!(received[0], query_embedding_text("find my notes"));
        assert_ne!(
            received[0], "find my notes",
            "embed_documents must never see the raw, un-prefixed query text"
        );
    }

    #[test]
    fn embed_documents_receives_text_completely_unprefixed() {
        let embedder = recording_embedder(3);
        embedder
            .embed_documents(&["a chunk of captured text".to_owned()])
            .unwrap();
        assert_eq!(
            embedder.received.lock().unwrap()[0],
            "a chunk of captured text"
        );
    }
}
