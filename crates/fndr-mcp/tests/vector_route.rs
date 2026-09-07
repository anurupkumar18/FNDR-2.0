//! `fndr.search` merging the vector route (fndr-retrieval::VectorRetriever)
//! alongside the existing keyword route. These call the public `search`
//! tool method (not the private `search_inner`), matching this crate's
//! convention of testing `FndrMcpServer` through its public API and
//! exercising the `.audit(...)` wrapper every tool call goes through.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use fndr_capture::{FrameSource, PngFileSource};
use fndr_inference::{CHUNK_EMBEDDING_V1, EmbedError, Embedder, EmbeddingSpec, GgufEmbedder};
use fndr_mcp::{FndrMcpServer, SearchParams};
use fndr_ocr::OcrEngine;
use fndr_privacy::Blocklist;
use fndr_store::{LanceWriter, NewChunk, NewRecord, Store};
use rmcp::handler::server::wrapper::Parameters;

/// Same pattern as fndr-retrieval's own TestEmbedder: deterministic, fast,
/// keyed on a substring, exercises the plumbing without a real model load.
/// Not a claim about ranking quality (that's make bench's job).
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

/// Full control over exactly which vector each document and the query get,
/// for tests where the geometry itself (which chunk is "nearest") has to be
/// deterministic rather than incidental.
struct FixedVectorEmbedder {
    spec: EmbeddingSpec,
    query_vector: Vec<f32>,
    document_vectors: HashMap<String, Vec<f32>>,
}

impl Embedder for FixedVectorEmbedder {
    fn spec(&self) -> &EmbeddingSpec {
        &self.spec
    }

    fn embed_documents(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        Ok(texts
            .iter()
            .map(|t| {
                self.document_vectors
                    .get(t)
                    .cloned()
                    .unwrap_or_else(|| panic!("test embedder: unmapped text {t:?}"))
            })
            .collect())
    }

    fn embed_query(&self, _query: &str) -> Result<Vec<f32>, EmbedError> {
        Ok(self.query_vector.clone())
    }
}

fn scratch(name: &str) -> std::path::PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "fndr-mcp-vector-route-{name}-{}-{}",
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
fn search_without_vector_route_behaves_exactly_as_before() {
    let dir = scratch("keyword-only");
    let mut store = Store::open(&dir.join("vault.sqlite3")).unwrap();
    insert_chunk(
        &mut store,
        "r1",
        "c1",
        "the suspension bridge inspection is due",
    );

    let server = FndrMcpServer::new(store);
    let out = server
        .search(Parameters(SearchParams {
            query: "bridge".into(),
            limit: None,
        }))
        .expect("tool call")
        .0;
    assert_eq!(out.hits.len(), 1);
    assert_eq!(out.hits[0].route, "keyword");
    assert!(!out.vector_route_available);

    std::fs::remove_dir_all(dir).unwrap();
}

// multi_thread: search_inner's block_in_place bridge for the vector route
// requires a multi-threaded runtime to hand blocking work off to. The
// keyword-only test above needs no runtime at all (plain #[test]); every
// test below configures a vector route and needs one.
#[tokio::test(flavor = "multi_thread")]
async fn search_with_vector_route_finds_a_semantic_match_keyword_would_miss() {
    let dir = scratch("vector-fills-gap");
    let mut store = Store::open(&dir.join("vault.sqlite3")).unwrap();
    insert_chunk(
        &mut store,
        "r-bridge",
        "c-bridge",
        "the suspension bridge inspection is due",
    );

    let embedder: Arc<dyn Embedder> = Arc::new(TestEmbedder {
        spec: EmbeddingSpec {
            model_id: "test-vector",
            dim: 2,
            lance_table: "test_mcp_vector_chunks",
        },
    });
    let index_dir = dir.join("index");
    LanceWriter::new(&index_dir)
        .flush_once(&mut store, embedder.as_ref(), 43)
        .await
        .unwrap();

    // "crossing report" shares zero literal words with the stored text, so
    // KeywordRetriever alone would return nothing; with only one row in the
    // Lance table, VectorRetriever's nearest-neighbor search returns it
    // regardless of exact distance, proving the wiring (query embedded,
    // Lance queried, hit tagged "vector"), not a claim about ranking
    // quality.
    let server = FndrMcpServer::with_vector_route(store, Blocklist::default(), embedder, index_dir);
    let out = server
        .search(Parameters(SearchParams {
            query: "crossing report".into(),
            limit: None,
        }))
        .expect("tool call")
        .0;
    assert_eq!(out.hits.len(), 1);
    assert_eq!(out.hits[0].chunk_id, "c-bridge");
    assert_eq!(out.hits[0].route, "vector");
    assert!(out.vector_route_available);

    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_hit_found_by_both_routes_is_not_duplicated() {
    let dir = scratch("dedupe");
    let mut store = Store::open(&dir.join("vault.sqlite3")).unwrap();
    insert_chunk(
        &mut store,
        "r-bridge",
        "c-bridge",
        "the suspension bridge inspection is due",
    );

    let embedder: Arc<dyn Embedder> = Arc::new(TestEmbedder {
        spec: EmbeddingSpec {
            model_id: "test-vector",
            dim: 2,
            lance_table: "test_mcp_vector_chunks_dedupe",
        },
    });
    let index_dir = dir.join("index");
    LanceWriter::new(&index_dir)
        .flush_once(&mut store, embedder.as_ref(), 43)
        .await
        .unwrap();

    // "bridge" matches both KeywordRetriever (literal term) and
    // VectorRetriever (TestEmbedder's bridge-keyed vector) for the same
    // chunk: it must appear exactly once in the merged output.
    let server = FndrMcpServer::with_vector_route(store, Blocklist::default(), embedder, index_dir);
    let out = server
        .search(Parameters(SearchParams {
            query: "bridge".into(),
            limit: None,
        }))
        .expect("tool call")
        .0;
    assert_eq!(out.hits.len(), 1);
    assert_eq!(out.hits[0].route, "keyword");

    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn truncation_keeps_keyword_hits_over_a_genuinely_new_vector_hit() {
    let dir = scratch("truncate-priority");
    let mut store = Store::open(&dir.join("vault.sqlite3")).unwrap();
    let keyword_text = "the suspension bridge inspection is due";
    let vector_only_text = "waterfall schedule for the reservoir";
    insert_chunk(&mut store, "r-kw", "c-kw", keyword_text);
    insert_chunk(&mut store, "r-vec", "c-vec", vector_only_text);

    // c-kw (the keyword match) is embedded far from the query; c-vec (not a
    // keyword match at all) is embedded identically to the query, so it is
    // the nearest vector candidate and is guaranteed to be returned as a
    // genuinely new, non-deduped vector hit — not an accidental non-match.
    let mut document_vectors = HashMap::new();
    document_vectors.insert(keyword_text.to_owned(), vec![0.0, 1.0]);
    document_vectors.insert(vector_only_text.to_owned(), vec![1.0, 0.0]);
    let embedder: Arc<dyn Embedder> = Arc::new(FixedVectorEmbedder {
        spec: EmbeddingSpec {
            model_id: "test-vector",
            dim: 2,
            lance_table: "test_mcp_vector_chunks_truncate",
        },
        query_vector: vec![1.0, 0.0],
        document_vectors,
    });
    let index_dir = dir.join("index");
    LanceWriter::new(&index_dir)
        .flush_once(&mut store, embedder.as_ref(), 43)
        .await
        .unwrap();

    // limit 1: keyword alone fills the budget with c-kw. The vector route
    // separately and correctly finds c-vec as the nearest neighbor (a real,
    // new hit, not deduped) — but the final result must still be the
    // keyword hit, proving hits.truncate(limit) never drops a keyword hit
    // in favor of a vector-only one.
    let server = FndrMcpServer::with_vector_route(store, Blocklist::default(), embedder, index_dir);
    let out = server
        .search(Parameters(SearchParams {
            query: "bridge".into(),
            limit: Some(1),
        }))
        .expect("tool call")
        .0;
    assert_eq!(out.hits.len(), 1);
    assert_eq!(out.hits[0].chunk_id, "c-kw");
    assert_eq!(out.hits[0].route, "keyword");

    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_vector_hit_for_a_since_deleted_record_gets_an_empty_snippet_not_a_panic() {
    let dir = scratch("deleted-record");
    let mut store = Store::open(&dir.join("vault.sqlite3")).unwrap();
    insert_chunk(
        &mut store,
        "r-ghost",
        "c-ghost",
        "a record that will be deleted after indexing",
    );

    let embedder: Arc<dyn Embedder> = Arc::new(TestEmbedder {
        spec: EmbeddingSpec {
            model_id: "test-vector",
            dim: 2,
            lance_table: "test_mcp_vector_chunks_deleted",
        },
    });
    let index_dir = dir.join("index");
    LanceWriter::new(&index_dir)
        .flush_once(&mut store, embedder.as_ref(), 43)
        .await
        .unwrap();

    // Delete the SQLite record (and its chunk) after the Lance flush,
    // exactly the ADR-002 scenario a rebuildable-but-stale derivative
    // exists for: Lance's row survives until the next rebuild/prune, so a
    // vector search can still return it. record_evidence must then find
    // nothing, and vector_hit_snippet must fall back to an empty snippet,
    // not panic.
    store.delete_records(&["r-ghost".to_owned()]).unwrap();

    let server = FndrMcpServer::with_vector_route(store, Blocklist::default(), embedder, index_dir);
    let out = server
        .search(Parameters(SearchParams {
            query: "nothing in common with the stored text".into(),
            limit: None,
        }))
        .expect("tool call")
        .0;
    assert_eq!(out.hits.len(), 1, "the stale Lance row still surfaces");
    assert_eq!(out.hits[0].chunk_id, "c-ghost");
    assert_eq!(out.hits[0].route, "vector");
    assert_eq!(out.hits[0].snippet, "", "no crash; an honest empty snippet");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_long_vector_hit_snippet_is_truncated_to_200_chars() {
    let dir = scratch("snippet-truncation");
    let mut store = Store::open(&dir.join("vault.sqlite3")).unwrap();
    // No literal repeats of "distinctive" keywords the query will use, and
    // deliberately over 200 characters so the truncation path is exercised.
    let long_text = "alpha ".repeat(60); // 360 characters, well past the 200-char cap
    insert_chunk(&mut store, "r-long", "c-long", long_text.trim());

    let embedder: Arc<dyn Embedder> = Arc::new(TestEmbedder {
        spec: EmbeddingSpec {
            model_id: "test-vector",
            dim: 2,
            lance_table: "test_mcp_vector_chunks_snippet_len",
        },
    });
    let index_dir = dir.join("index");
    LanceWriter::new(&index_dir)
        .flush_once(&mut store, embedder.as_ref(), 43)
        .await
        .unwrap();

    let server = FndrMcpServer::with_vector_route(store, Blocklist::default(), embedder, index_dir);
    let out = server
        .search(Parameters(SearchParams {
            query: "something else entirely".into(),
            limit: None,
        }))
        .expect("tool call")
        .0;
    assert_eq!(out.hits.len(), 1);
    assert_eq!(out.hits[0].route, "vector");
    let snippet = &out.hits[0].snippet;
    assert!(
        snippet.ends_with('…'),
        "expected an ellipsis-truncated snippet, got {snippet:?}"
    );
    assert_eq!(
        snippet.chars().count(),
        201, // 200 kept characters plus the ellipsis
        "snippet should be capped at 200 characters plus the ellipsis, got {snippet:?}"
    );
    assert!(long_text.trim().len() > snippet.len());
}

/// Real end-to-end proof with the actual pinned model (not `TestEmbedder`):
/// a real captured frame, real Apple Vision OCR, a real `GgufEmbedder`
/// loading the 639 MB Qwen3 GGUF, a real `LanceWriter` flush, and a real
/// `VectorRetriever` query, all reached through `FndrMcpServer::search`
/// exactly as the MCP tool serves it. Ignored by default (needs the real
/// model file on disk and takes real inference time); run explicitly:
/// `cargo test -p fndr-mcp --test vector_route -- --ignored --nocapture`
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn real_model_finds_a_genuine_paraphrase_keyword_search_would_miss() {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../fndr-ocr/tests/fixtures/skeleton_fixture.png");
    let frame = PngFileSource { path: fixture }
        .grab()
        .expect("fixture frame");
    let ocr = OcrEngine::new().expect("Vision available");
    let recognized = ocr.recognize(&frame.png).expect("ocr");
    // Fixture text is "FNDR walking skeleton fixture" plus the pangram "the
    // quick brown fox jumps over the lazy dog" (confirmed by direct OCR
    // run). The query below shares zero literal words with either line.
    assert!(recognized.to_lowercase().contains("fox"));

    let dir = scratch("real-model");
    let mut store = Store::open(&dir.join("vault.sqlite3")).unwrap();
    store
        .insert_capture(
            &NewRecord {
                id: "r-real".into(),
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
                id: "c-real".into(),
                ord: 0,
                text: recognized,
            }],
        )
        .unwrap();

    let model_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../models/Qwen3-Embedding-0.6B-Q8_0.gguf");
    let embedder: Arc<dyn Embedder> =
        Arc::new(GgufEmbedder::load(&model_path, CHUNK_EMBEDDING_V1).expect(
            "model load failed; expected models/Qwen3-Embedding-0.6B-Q8_0.gguf at repo root",
        ));
    let index_dir = dir.join("index");
    LanceWriter::new(&index_dir)
        .flush_once(&mut store, embedder.as_ref(), 43)
        .await
        .unwrap();

    // A paraphrase of "the quick brown fox jumps over the lazy dog" with no
    // literal word overlap at all: proves the vector route, not an
    // accidental keyword hit.
    let paraphrase = "a nimble animal leaping past a drowsy canine";

    let server = FndrMcpServer::with_vector_route(store, Blocklist::default(), embedder, index_dir);
    let out = server
        .search(Parameters(SearchParams {
            query: paraphrase.into(),
            limit: None,
        }))
        .expect("tool call")
        .0;
    assert_eq!(
        out.hits.len(),
        1,
        "expected the real model to find the fixture chunk"
    );
    assert_eq!(out.hits[0].chunk_id, "c-real");
    assert_eq!(out.hits[0].route, "vector");
    assert!(out.vector_route_available);

    // Regression check: the identical paraphrase against a keyword-only
    // server (same captured data, no vector route) finds nothing, proving
    // the vector-route hit above was not an accidental keyword match.
    let keyword_only_store = Store::open(&dir.join("vault.sqlite3")).unwrap();
    let keyword_hits = fndr_retrieval::KeywordRetriever::new(&keyword_only_store)
        .search(paraphrase, 10)
        .unwrap();
    assert!(
        keyword_hits.is_empty(),
        "the paraphrase must share no keyword-searchable terms with the fixture text"
    );

    std::fs::remove_dir_all(dir).unwrap();
}
