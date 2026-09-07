//! Routes, RRF fusion, reranking, relevance gates, diversity, evidence packs,
//! and context-pack budgeting. T-505 begins with the one real route that does
//! not require a loaded model: SQLite FTS over durable capture chunks.

use std::path::{Path, PathBuf};

use arrow_array::{Float32Array, Int64Array, StringArray};
use fndr_inference::{EmbedError, Embedder};
use fndr_store::{Store, StoreError};
use futures::TryStreamExt;
use lancedb::query::{ExecutableQuery, QueryBase};

/// An evidence-bearing result from the keyword route. It intentionally carries
/// stable record and chunk IDs so later composition, deletion, and citation
/// surfaces all resolve through the same engine path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeywordHit {
    pub record_id: String,
    pub chunk_id: String,
    pub source: String,
    pub captured_at_ms: i64,
    pub snippet: String,
}

/// The first production retrieval route. This is not a second store or a
/// mock: it queries the FTS index maintained alongside SQLite truth. Vector,
/// hybrid, and reranking routes join this one stack later behind ADR-006's
/// benchmark gates.
pub struct KeywordRetriever<'a> {
    store: &'a Store,
}

impl<'a> KeywordRetriever<'a> {
    pub fn new(store: &'a Store) -> Self {
        Self { store }
    }

    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<KeywordHit>, StoreError> {
        self.store.search_chunks(query, limit).map(|hits| {
            hits.into_iter()
                .map(|hit| KeywordHit {
                    record_id: hit.record_id,
                    chunk_id: hit.chunk_id,
                    source: hit.source,
                    captured_at_ms: hit.captured_at_ms,
                    snippet: hit.snippet,
                })
                .collect()
        })
    }
}

/// One semantic route over the rebuildable Lance derivative. It is deliberately
/// separate from keyword search: T-505's later RRF step is where ranks may be
/// fused, and ADR-006 requires a benchmark number before that happens.
pub struct VectorRetriever {
    index_dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VectorHit {
    pub record_id: String,
    pub chunk_id: String,
    pub source: String,
    pub captured_at_ms: i64,
    /// Lance's raw distance; lower is nearer. It is not combined with any
    /// keyword score here because raw score fusion is prohibited by ADR-006.
    pub distance: f32,
}

#[derive(Debug, thiserror::Error)]
pub enum VectorSearchError {
    #[error("embedding: {0}")]
    Embed(#[from] EmbedError),
    #[error("query vector has {got} dimensions; index contract requires {expected}")]
    WrongDimension { got: usize, expected: usize },
    #[error("vector index is unavailable: {0}")]
    Lance(#[from] lancedb::Error),
    #[error("vector index returned an invalid result row")]
    InvalidRow,
}

impl VectorRetriever {
    pub fn new(index_dir: impl AsRef<Path>) -> Self {
        Self {
            index_dir: index_dir.as_ref().to_path_buf(),
        }
    }

    /// Embed a query using the contract's query-side instruction, then run a
    /// nearest-neighbor search against the matching Lance table. Empty queries
    /// avoid model work and return no results, matching the keyword route.
    pub async fn search(
        &self,
        query: &str,
        limit: usize,
        embedder: &dyn Embedder,
    ) -> Result<Vec<VectorHit>, VectorSearchError> {
        if query.trim().is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let spec = embedder.spec();
        let vector = embedder.embed_query(query)?;
        if vector.len() != spec.dim {
            return Err(VectorSearchError::WrongDimension {
                got: vector.len(),
                expected: spec.dim,
            });
        }
        let db = lancedb::connect(self.index_dir.to_string_lossy().as_ref())
            .execute()
            .await?;
        let table = db.open_table(spec.lance_table).execute().await?;
        let batches = table
            .query()
            .limit(limit)
            .nearest_to(vector)?
            .execute()
            .await?
            .try_collect::<Vec<_>>()
            .await?;
        let mut hits = Vec::new();
        for batch in batches {
            let ids = string_column(&batch, "id")?;
            let record_ids = string_column(&batch, "record_id")?;
            let sources = string_column(&batch, "source")?;
            let captured = int64_column(&batch, "captured_at_ms")?;
            let distances = float32_column(&batch, "_distance")?;
            for row in 0..batch.num_rows() {
                hits.push(VectorHit {
                    chunk_id: ids.value(row).to_owned(),
                    record_id: record_ids.value(row).to_owned(),
                    source: sources.value(row).to_owned(),
                    captured_at_ms: captured.value(row),
                    distance: distances.value(row),
                });
            }
        }
        Ok(hits)
    }
}

fn string_column<'a>(
    batch: &'a arrow_array::RecordBatch,
    name: &str,
) -> Result<&'a StringArray, VectorSearchError> {
    batch
        .column_by_name(name)
        .and_then(|column| column.as_any().downcast_ref())
        .ok_or(VectorSearchError::InvalidRow)
}

fn int64_column<'a>(
    batch: &'a arrow_array::RecordBatch,
    name: &str,
) -> Result<&'a Int64Array, VectorSearchError> {
    batch
        .column_by_name(name)
        .and_then(|column| column.as_any().downcast_ref())
        .ok_or(VectorSearchError::InvalidRow)
}

fn float32_column<'a>(
    batch: &'a arrow_array::RecordBatch,
    name: &str,
) -> Result<&'a Float32Array, VectorSearchError> {
    batch
        .column_by_name(name)
        .and_then(|column| column.as_any().downcast_ref())
        .ok_or(VectorSearchError::InvalidRow)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use fndr_inference::EmbeddingSpec;
    use fndr_store::{LanceWriter, NewChunk, NewRecord};

    use super::*;

    #[test]
    fn porter_keyword_route_returns_durable_chunk_evidence() {
        let mut store = Store::open_in_memory().unwrap();
        store
            .insert_capture(
                &NewRecord {
                    id: "r1".into(),
                    session_id: "s1".into(),
                    source: "screen".into(),
                    app_name: "Finder".into(),
                    bundle_id: None,
                    url: None,
                    window_title: "Index maintenance".into(),
                    captured_at_ms: 42,
                    created_at_ms: 42,
                },
                &[NewChunk {
                    id: "c1".into(),
                    ord: 0,
                    text: "the index was rebuilt after the crash".into(),
                }],
            )
            .unwrap();

        let hits = KeywordRetriever::new(&store)
            .search("indexes crash", 10)
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].record_id, "r1");
        assert_eq!(hits[0].chunk_id, "c1");
        assert!(hits[0].snippet.contains("index"));
    }

    #[test]
    fn empty_query_is_an_empty_result() {
        let store = Store::open_in_memory().unwrap();
        assert!(
            KeywordRetriever::new(&store)
                .search("   ", 10)
                .unwrap()
                .is_empty()
        );
    }

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
            "fndr-vector-route-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[tokio::test]
    async fn vector_route_queries_the_same_flushed_lance_derivative() {
        let dir = scratch("happy");
        let mut store = Store::open(&dir.join("vault.sqlite3")).unwrap();
        for (record_id, chunk_id, text) in [
            (
                "r-bridge",
                "c-bridge",
                "the suspension bridge inspection is due",
            ),
            (
                "r-garden",
                "c-garden",
                "the garden watering schedule is ready",
            ),
        ] {
            store
                .insert_capture(
                    &NewRecord {
                        id: record_id.into(),
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
                        id: chunk_id.into(),
                        ord: 0,
                        text: text.into(),
                    }],
                )
                .unwrap();
        }
        let embedder = TestEmbedder {
            spec: EmbeddingSpec {
                model_id: "test-vector",
                dim: 2,
                lance_table: "test_vector_chunks",
            },
        };
        LanceWriter::new(&dir.join("index"))
            .flush_once(&mut store, &embedder, 43)
            .await
            .unwrap();

        let hits = VectorRetriever::new(dir.join("index"))
            .search("bridge", 2, &embedder)
            .await
            .unwrap();
        assert_eq!(hits[0].chunk_id, "c-bridge");
        assert_eq!(hits[0].record_id, "r-bridge");
        assert!(hits[0].distance <= hits[1].distance);

        std::fs::remove_dir_all(dir).unwrap();
    }
}
