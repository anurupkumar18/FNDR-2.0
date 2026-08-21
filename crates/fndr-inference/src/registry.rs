//! The model registry (T-401): every model FNDR can use, pinned by exact
//! file, size, and SHA-256, with required/optional semantics (v1 ADR-012:
//! a missing required model is a typed, visible state, never a mock or a
//! zero-vector). Downloads execute in fndr-downloader (the only crate
//! allowed HTTP besides fndr-updater, ADR-004); this module owns the facts.

/// One pinned downloadable artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelArtifact {
    /// Stable registry id, referenced by config and logs.
    pub id: &'static str,
    pub purpose: &'static str,
    /// Upstream repository (provenance; the url is what gets fetched).
    pub repo: &'static str,
    pub filename: &'static str,
    pub url: &'static str,
    /// SHA-256 of the file content, hex. Verified after download and on
    /// registry preflight; a mismatch deletes the artifact and fails loudly.
    pub sha256: &'static str,
    pub size_bytes: u64,
    /// Required models gate the features that need them with a typed
    /// unavailable state; optional ones degrade their feature visibly.
    pub required: bool,
}

/// Registry of record (ADR-003 lineup). Append or replace entries only with
/// an ADR-003 amendment in the same PR.
///
/// The reranker is deliberately absent: ADR-003 named a "ggml-org
/// conversion" that does not exist under that name on the Hub; the source
/// and quant get pinned by the ml lane with the T-506 eval (candidates
/// exist, for example mradermacher/Qwen3-Reranker-0.6B-GGUF). The VLM waits
/// on the T-408 mtmd spike.
pub const MODELS: &[ModelArtifact] = &[ModelArtifact {
    id: "qwen3-embedding-0.6b-q8_0",
    purpose: "chunk and query embeddings (CHUNK_EMBEDDING_V1)",
    repo: "Qwen/Qwen3-Embedding-0.6B-GGUF",
    filename: "Qwen3-Embedding-0.6B-Q8_0.gguf",
    url: "https://huggingface.co/Qwen/Qwen3-Embedding-0.6B-GGUF/resolve/main/Qwen3-Embedding-0.6B-Q8_0.gguf",
    sha256: "06507c7b42688469c4e7298b0a1e16deff06caf291cf0a5b278c308249c3e439",
    size_bytes: 639_150_592,
    required: true,
}];

pub fn artifact(id: &str) -> Option<&'static ModelArtifact> {
    MODELS.iter().find(|m| m.id == id)
}

/// Disk preflight (v1 ADR-012 semantics): bytes needed to fetch everything
/// in `ids` that is not already present and verified at `models_dir`.
pub fn bytes_needed(models_dir: &std::path::Path, ids: &[&str]) -> u64 {
    ids.iter()
        .filter_map(|id| artifact(id))
        .filter(|m| {
            let path = models_dir.join(m.filename);
            !path.is_file()
                || path
                    .metadata()
                    .map(|meta| meta.len() != m.size_bytes)
                    .unwrap_or(true)
        })
        .map(|m| m.size_bytes)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_entries_are_well_formed() {
        assert!(!MODELS.is_empty());
        let mut ids: Vec<&str> = MODELS.iter().map(|m| m.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), MODELS.len(), "duplicate registry ids");
        for m in MODELS {
            assert!(
                m.url.starts_with("https://huggingface.co/"),
                "{}: pinned artifacts come from the pinned host",
                m.id
            );
            assert!(m.url.ends_with(m.filename), "{}: url/filename drift", m.id);
            assert_eq!(m.sha256.len(), 64, "{}: sha256 must be 64 hex chars", m.id);
            assert!(
                m.sha256.bytes().all(|b| b.is_ascii_hexdigit()),
                "{}: sha256 must be hex",
                m.id
            );
            assert!(m.size_bytes > 0, "{}: size must be pinned", m.id);
        }
    }

    #[test]
    fn bytes_needed_counts_missing_and_wrong_size() {
        let dir = std::env::temp_dir().join(format!("fndr-reg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let ids = ["qwen3-embedding-0.6b-q8_0"];
        let m = artifact(ids[0]).unwrap();
        assert_eq!(bytes_needed(&dir, &ids), m.size_bytes, "missing file");

        std::fs::write(dir.join(m.filename), b"stub").unwrap();
        assert_eq!(bytes_needed(&dir, &ids), m.size_bytes, "wrong size counts");

        assert_eq!(bytes_needed(&dir, &["unknown-id"]), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
