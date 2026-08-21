//! The embedding contract seam consumed by the store's Lance writer.
//!
//! This is the trait boundary only. The real implementation (T-402) carries
//! the full ADR-003 rules: Qwen3-Embedding-0.6B official Q8_0, instruction
//! asymmetry between documents and queries, and the 768d matryoshka rule
//! (truncate the 1024d output to 768, then L2-renormalize) implemented
//! app-side inside the contract. No mock implementation may reach a
//! production path (invariant 4); test embedders live in test code.

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

    /// Embed document-side texts (already composed embedding text). Every
    /// returned vector must have exactly `spec().dim` elements; writers
    /// refuse anything else.
    fn embed_documents(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError>;
}
