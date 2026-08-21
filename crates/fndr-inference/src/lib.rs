//! Model registry, llama.cpp session management, the model-worker priority queue, embedding contract.

mod embedding;

pub use embedding::{CHUNK_EMBEDDING_V1, EmbedError, Embedder, EmbeddingSpec};
