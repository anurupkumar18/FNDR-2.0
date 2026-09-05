//! Model registry, llama.cpp session management, the model-worker priority queue, embedding contract.

mod embedding;
mod gguf_embedder;
mod model_worker;
mod registry;

pub use embedding::{
    CHUNK_EMBEDDING_V1, EmbedError, Embedder, EmbeddingSpec, QUERY_INSTRUCTION,
    query_embedding_text, truncate_and_renormalize,
};
pub use gguf_embedder::GgufEmbedder;
pub use model_worker::{ModelWorkerHandle, Priority};
pub use registry::{MODELS, ModelArtifact, artifact, bytes_needed};
