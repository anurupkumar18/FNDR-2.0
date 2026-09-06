//! Routes, RRF fusion, reranking, relevance gates, diversity, evidence packs,
//! and context-pack budgeting. T-505 begins with the one real route that does
//! not require a loaded model: SQLite FTS over durable capture chunks.

use fndr_store::{Store, StoreError};

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

#[cfg(test)]
mod tests {
    use fndr_store::{NewChunk, NewRecord};

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
}
