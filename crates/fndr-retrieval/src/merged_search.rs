//! Shared keyword+vector merge logic (ARCHITECTURE.md section 4.2: "the
//! same function serves Tauri IPC, MCP tools, and future companion
//! routes"). fndr-mcp's `fndr.search` and the desktop shell's search
//! command both call these instead of each re-implementing the merge,
//! dedup, and snippet-building rules.

use std::collections::HashSet;
use std::path::Path;

use fndr_inference::Embedder;
use fndr_store::Store;

use crate::{KeywordRetriever, VectorHit, VectorRetriever, VectorSearchError};

/// One search result, tagged with which route found it. No combined score
/// exists across routes (ADR-006: raw score fusion needs a benchmark to
/// justify it first) -- this is presentation-order merging, not ranking.
#[derive(Debug, Clone, PartialEq)]
pub struct TaggedHit {
    pub record_id: String,
    pub chunk_id: String,
    pub source: String,
    pub captured_at_ms: i64,
    pub snippet: String,
    pub route: &'static str,
}

/// Keyword-route hits, tagged `route: "keyword"`. Every returned chunk_id
/// is also inserted into `seen`, so a caller can pass the same set into
/// `vector_hits` afterward to dedup against these.
pub fn keyword_hits(
    store: &Store,
    query: &str,
    limit: usize,
    seen: &mut HashSet<String>,
) -> Result<Vec<TaggedHit>, fndr_store::StoreError> {
    let hits = KeywordRetriever::new(store).search(query, limit)?;
    Ok(hits
        .into_iter()
        .map(|h| {
            seen.insert(h.chunk_id.clone());
            TaggedHit {
                record_id: h.record_id,
                chunk_id: h.chunk_id,
                source: h.source,
                captured_at_ms: h.captured_at_ms,
                snippet: h.snippet,
                route: "keyword",
            }
        })
        .collect())
}

/// Vector-route hits not already in `seen` (which is mutated to include
/// them). Takes no `Store`: this is the embed+Lance round trip only, so a
/// caller holding `store` behind a shared `Mutex` can run this with the
/// lock fully released and only acquire it afterward, for the snippet
/// step in `tag_vector_hits_with_snippets`.
pub async fn vector_hits(
    embedder: &dyn Embedder,
    index_dir: &Path,
    query: &str,
    limit: usize,
    seen: &mut HashSet<String>,
) -> Result<Vec<VectorHit>, VectorSearchError> {
    let vector_hits = VectorRetriever::new(index_dir)
        .search(query, limit, embedder)
        .await?;

    Ok(vector_hits
        .into_iter()
        .filter(|hit| seen.insert(hit.chunk_id.clone()))
        .collect())
}

/// Tags already-fetched vector hits `route: "vector"` and builds each
/// snippet from `store` (Lance rows carry no FTS match markers, so this
/// reads the durable text back out instead of returning an empty or
/// fabricated snippet). Synchronous and store-only, so a caller can hold
/// its store lock for exactly this span, not the `vector_hits` round trip
/// that produced the input.
pub fn tag_vector_hits_with_snippets(store: &Store, hits: Vec<VectorHit>) -> Vec<TaggedHit> {
    hits.into_iter()
        .map(|hit| {
            let snippet = vector_hit_snippet(store, &hit).unwrap_or_default();
            TaggedHit {
                record_id: hit.record_id,
                chunk_id: hit.chunk_id,
                source: hit.source,
                captured_at_ms: hit.captured_at_ms,
                snippet,
                route: "vector",
            }
        })
        .collect()
}

/// A vector hit carries no FTS match markers (Lance rows have no text
/// snippet), so this builds a plain truncated excerpt from the same
/// durable store the keyword route reads, instead of returning an empty
/// or fabricated snippet. Moved here from fndr-mcp/src/server.rs verbatim
/// (behavior-preserving extraction).
fn vector_hit_snippet(store: &Store, hit: &crate::VectorHit) -> Option<String> {
    const SNIPPET_CHARS: usize = 200;
    let evidence = store.record_evidence(&hit.record_id).ok()??;
    let chunk = evidence
        .chunks
        .into_iter()
        .find(|c| c.chunk_id == hit.chunk_id)?;
    if chunk.text.chars().count() <= SNIPPET_CHARS {
        Some(chunk.text)
    } else {
        let truncated: String = chunk.text.chars().take(SNIPPET_CHARS).collect();
        Some(format!("{truncated}…"))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use fndr_inference::{EmbedError, EmbeddingSpec};
    use fndr_store::{LanceWriter, NewChunk, NewRecord};

    use super::*;

    struct TestEmbedder {
        spec: EmbeddingSpec,
    }

    impl Embedder for TestEmbedder {
        fn spec(&self) -> &EmbeddingSpec {
            &self.spec
        }

        fn embed_documents(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
            Ok(texts
                .iter()
                .map(|text| {
                    if text.contains("bridge") {
                        vec![1.0, 0.0]
                    } else {
                        vec![0.0, 1.0]
                    }
                })
                .collect())
        }
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "fndr-retrieval-merged-search-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn insert_chunk(store: &mut Store, record_id: &str, chunk_id: &str, text: &str) {
        store
            .insert_capture(
                &NewRecord {
                    id: record_id.to_owned(),
                    session_id: "s1".into(),
                    source: "screen".into(),
                    app_name: "Notes".into(),
                    bundle_id: None,
                    url: None,
                    window_title: "fixture".into(),
                    captured_at_ms: 42,
                    created_at_ms: 42,
                },
                &[NewChunk {
                    id: chunk_id.to_owned(),
                    ord: 0,
                    text: text.to_owned(),
                }],
            )
            .unwrap();
    }

    #[test]
    fn keyword_hits_finds_a_literal_match_and_marks_it_seen() {
        let mut store = Store::open_in_memory().unwrap();
        insert_chunk(
            &mut store,
            "r1",
            "c1",
            "the suspension bridge inspection is due",
        );

        let mut seen = HashSet::new();
        let hits = keyword_hits(&store, "bridge", 10, &mut seen).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].chunk_id, "c1");
        assert_eq!(hits[0].route, "keyword");
        assert!(seen.contains("c1"));
    }

    #[tokio::test]
    async fn vector_hits_finds_a_new_chunk_and_tagging_builds_a_real_snippet() {
        let dir = scratch("new-hit");
        let mut store = Store::open(&dir.join("vault.sqlite3")).unwrap();
        insert_chunk(
            &mut store,
            "r-bridge",
            "c-bridge",
            "the suspension bridge inspection is due",
        );

        let embedder = TestEmbedder {
            spec: EmbeddingSpec {
                model_id: "test-vector",
                dim: 2,
                lance_table: "test_merged_search_new_hit",
            },
        };
        let index_dir = dir.join("index");
        LanceWriter::new(&index_dir)
            .flush_once(&mut store, &embedder, 43)
            .await
            .unwrap();

        let mut seen = HashSet::new();
        let raw_hits = vector_hits(&embedder, &index_dir, "bridge", 10, &mut seen)
            .await
            .unwrap();
        assert!(seen.contains("c-bridge"));

        let hits = tag_vector_hits_with_snippets(&store, raw_hits);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].chunk_id, "c-bridge");
        assert_eq!(hits[0].route, "vector");
        assert!(hits[0].snippet.contains("suspension bridge"));

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn vector_hits_excludes_a_chunk_already_in_seen() {
        let dir = scratch("dedupe");
        let mut store = Store::open(&dir.join("vault.sqlite3")).unwrap();
        insert_chunk(
            &mut store,
            "r-bridge",
            "c-bridge",
            "the suspension bridge inspection is due",
        );

        let embedder = TestEmbedder {
            spec: EmbeddingSpec {
                model_id: "test-vector",
                dim: 2,
                lance_table: "test_merged_search_dedupe",
            },
        };
        let index_dir = dir.join("index");
        LanceWriter::new(&index_dir)
            .flush_once(&mut store, &embedder, 43)
            .await
            .unwrap();

        // Already found by the keyword phase: vector phase must not
        // duplicate it.
        let mut seen = HashSet::new();
        seen.insert("c-bridge".to_owned());
        let hits = vector_hits(&embedder, &index_dir, "bridge", 10, &mut seen)
            .await
            .unwrap();
        assert!(hits.is_empty());

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn a_long_vector_hit_snippet_is_truncated_to_200_chars() {
        let dir = scratch("snippet-truncation");
        let mut store = Store::open(&dir.join("vault.sqlite3")).unwrap();
        let long_text = "alpha ".repeat(60); // 360 characters, past the 200-char cap
        insert_chunk(&mut store, "r-long", "c-long", long_text.trim());

        let embedder = TestEmbedder {
            spec: EmbeddingSpec {
                model_id: "test-vector",
                dim: 2,
                lance_table: "test_merged_search_snippet_len",
            },
        };
        let index_dir = dir.join("index");
        LanceWriter::new(&index_dir)
            .flush_once(&mut store, &embedder, 43)
            .await
            .unwrap();

        let mut seen = HashSet::new();
        let raw_hits = vector_hits(
            &embedder,
            &index_dir,
            "something else entirely",
            10,
            &mut seen,
        )
        .await
        .unwrap();
        let hits = tag_vector_hits_with_snippets(&store, raw_hits);
        assert_eq!(hits.len(), 1);
        let snippet = &hits[0].snippet;
        assert!(
            snippet.ends_with('…'),
            "expected an ellipsis-truncated snippet, got {snippet:?}"
        );
        assert_eq!(
            snippet.chars().count(),
            201, // 200 kept characters plus the ellipsis
            "snippet should be capped at 200 characters plus the ellipsis, got {snippet:?}"
        );

        std::fs::remove_dir_all(dir).unwrap();
    }
}
