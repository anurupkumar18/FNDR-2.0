//! Model registry, llama.cpp session management, the model-worker priority queue, embedding contract.

mod registry;

pub use registry::{MODELS, ModelArtifact, artifact, bytes_needed};
