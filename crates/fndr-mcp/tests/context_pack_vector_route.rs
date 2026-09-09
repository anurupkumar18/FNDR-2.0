//! `fndr.context_pack` merging the vector route alongside its existing
//! keyword route, with the pack's budget and citation invariants still
//! holding once semantic hits are in the mix. These call the public
//! `context_pack` tool method (not the private `context_pack_inner`),
//! matching this crate's convention of testing `FndrMcpServer` through its
//! public API and exercising the `.audit(...)` wrapper every tool call goes
//! through.
//!
//! Kept separate from `vector_route.rs` (which covers `fndr.search`) so each
//! tool surface has its own test binary, as `auth_surface.rs` and
//! `skeleton_e2e.rs` already do.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use fndr_inference::{EmbedError, Embedder, EmbeddingSpec};
use fndr_mcp::{ContextPackParams, FndrMcpServer};
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
        "fndr-mcp-context-pack-vector-{name}-{}-{}",
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

fn pack_params(goal: &str, token_budget: Option<u32>) -> ContextPackParams {
    ContextPackParams {
        goal: goal.to_owned(),
        token_budget,
        max_records: None,
    }
}

#[test]
fn context_pack_without_a_vector_route_is_unchanged_and_says_so() {
    let mut store = Store::open_in_memory().unwrap();
    insert_chunk(
        &mut store,
        "r1",
        "c1",
        "the suspension bridge inspection is due",
    );

    let server = FndrMcpServer::new(store);
    let pack = server
        .context_pack(Parameters(pack_params("bridge", None)))
        .expect("tool call")
        .0;

    assert_eq!(pack.items.len(), 1);
    assert_eq!(pack.items[0].chunk_id, "c1");
    assert_eq!(pack.items[0].route, "keyword");
    assert_eq!(pack.retrieval_route, "keyword");
    assert!(
        !pack.vector_route_available,
        "a keyword-only pack must say semantic search was never attempted"
    );
    assert_eq!(pack.dropped_for_budget, 0);
    assert_eq!(pack.vector_dropped_for_budget, 0);
}

// multi_thread: context_pack_inner's block_in_place bridge for the vector
// route requires a multi-threaded runtime to hand blocking work off to. The
// keyword-only test above needs no runtime at all (plain #[test]); every
// test below configures a vector route and needs one.
#[tokio::test(flavor = "multi_thread")]
async fn context_pack_finds_a_paraphrase_the_keyword_route_provably_misses() {
    let dir = scratch("paraphrase");
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
            lance_table: "test_pack_vector_paraphrase",
        },
    });
    let index_dir = dir.join("index");
    LanceWriter::new(&index_dir)
        .flush_once(&mut store, embedder.as_ref(), 43)
        .await
        .unwrap();

    // "crossing report" shares zero literal words with the stored text, so
    // the keyword route alone returns nothing; with only one row in the
    // Lance table the vector route returns it regardless of exact distance.
    // This proves the wiring (query embedded, Lance queried, hit packed and
    // tagged), not a claim about ranking quality.
    let goal = "crossing report";
    let server = FndrMcpServer::with_vector_route(store, Blocklist::default(), embedder, index_dir);
    let pack = server
        .context_pack(Parameters(pack_params(goal, None)))
        .expect("tool call")
        .0;

    assert_eq!(pack.items.len(), 1, "the semantic hit reaches the pack");
    assert_eq!(pack.items[0].chunk_id, "c-bridge");
    assert_eq!(pack.items[0].route, "vector");
    assert_eq!(
        pack.items[0].text, "the suspension bridge inspection is due",
        "the pack carries the full capture text, not a search snippet"
    );
    assert!(pack.vector_route_available);
    assert_eq!(pack.retrieval_route, "keyword+vector");

    // The same goal against a keyword-only server over the same captured
    // data packs nothing at all. Without this the test above could pass on
    // an accidental keyword match rather than on the vector route.
    let keyword_only = FndrMcpServer::new(Store::open(&dir.join("vault.sqlite3")).unwrap());
    let keyword_only_pack = keyword_only
        .context_pack(Parameters(pack_params(goal, None)))
        .expect("tool call")
        .0;
    assert!(
        keyword_only_pack.items.is_empty(),
        "the goal must share no keyword-searchable terms with the capture text"
    );
    assert_eq!(keyword_only_pack.dropped_for_budget, 0);

    // A pack carrying capture text is a raw release whichever route found
    // it; the vector route must not open an unaudited path to that text.
    let audited = server
        .recent_tool_calls(10)
        .expect("audit readable")
        .into_iter()
        .find(|entry| entry.tool == "fndr.context_pack")
        .expect("context_pack is audited");
    assert!(audited.raw_released);

    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_chunk_found_by_both_routes_is_packed_once() {
    let dir = scratch("dedupe");
    let mut store = Store::open(&dir.join("vault.sqlite3")).unwrap();
    insert_chunk(
        &mut store,
        "r-bridge",
        "c-bridge",
        "the suspension bridge inspection is due",
    );
    // A second chunk only the vector route can reach. Without it this test
    // would pass just as well against a context_pack that never ran the
    // vector route at all, which would make it theater rather than a dedup
    // test: c-other's presence in the pack is what proves the route ran.
    insert_chunk(&mut store, "r-other", "c-other", "unrelated meeting notes");

    let embedder: Arc<dyn Embedder> = Arc::new(TestEmbedder {
        spec: EmbeddingSpec {
            model_id: "test-vector",
            dim: 2,
            lance_table: "test_pack_vector_dedupe",
        },
    });
    let index_dir = dir.join("index");
    LanceWriter::new(&index_dir)
        .flush_once(&mut store, embedder.as_ref(), 43)
        .await
        .unwrap();

    // "bridge" matches both the keyword route (literal term) and the vector
    // route (TestEmbedder's bridge-keyed vector) for c-bridge. Packing it
    // twice would silently bill the caller's token budget twice for one
    // memory. c-other matches neither literally nor by vector affinity, but
    // is returned by the nearest-neighbor search because the table holds
    // only two rows.
    let server = FndrMcpServer::with_vector_route(store, Blocklist::default(), embedder, index_dir);
    let pack = server
        .context_pack(Parameters(pack_params("bridge", None)))
        .expect("tool call")
        .0;

    let packed: Vec<&str> = pack.items.iter().map(|i| i.chunk_id.as_str()).collect();
    assert_eq!(
        packed,
        vec!["c-bridge", "c-other"],
        "the shared chunk once, then the vector-only chunk"
    );
    assert_eq!(
        pack.items
            .iter()
            .filter(|i| i.chunk_id == "c-bridge")
            .count(),
        1,
        "a chunk both routes found is packed exactly once"
    );
    assert_eq!(
        pack.items[0].route, "keyword",
        "a chunk both routes found keeps its keyword tag"
    );
    assert_eq!(
        pack.items[1].route, "vector",
        "and the vector route did run, so the dedup above is real"
    );
    assert_eq!(pack.dropped_for_budget, 0);
    assert_eq!(pack.vector_dropped_for_budget, 0);

    std::fs::remove_dir_all(dir).unwrap();
}

/// The keyword chunk is short; the vector-only chunk is deliberately longer
/// than the 200-character search-snippet cap, so a pack that packed the
/// vector route's *snippet* instead of the stored text would fail here.
const KEYWORD_TEXT: &str = "the suspension bridge inspection is due";

fn vector_only_text() -> String {
    // 309 characters, well past `tag_vector_hits_with_snippets`'s 200-char
    // cap (the test below asserts the length rather than trusting this sum).
    "reservoir spillway maintenance "
        .repeat(10)
        .trim()
        .to_owned()
}

async fn fixed_vector_server(dir: &std::path::Path, table: &'static str) -> FndrMcpServer {
    let mut store = Store::open(&dir.join("vault.sqlite3")).unwrap();
    let vector_text = vector_only_text();
    insert_chunk(&mut store, "r-kw", "c-kw", KEYWORD_TEXT);
    insert_chunk(&mut store, "r-vec", "c-vec", &vector_text);

    // c-kw (the keyword match) is embedded far from the query; c-vec (not a
    // keyword match at all) is embedded identically to the query, so it is
    // the nearest vector candidate and is guaranteed to be a genuinely new,
    // non-deduped vector hit rather than an accidental non-match.
    let mut document_vectors = HashMap::new();
    document_vectors.insert(KEYWORD_TEXT.to_owned(), vec![0.0, 1.0]);
    document_vectors.insert(vector_text, vec![1.0, 0.0]);
    let embedder: Arc<dyn Embedder> = Arc::new(FixedVectorEmbedder {
        spec: EmbeddingSpec {
            model_id: "test-vector",
            dim: 2,
            lance_table: table,
        },
        query_vector: vec![1.0, 0.0],
        document_vectors,
    });
    let index_dir = dir.join("index");
    LanceWriter::new(&index_dir)
        .flush_once(&mut store, embedder.as_ref(), 43)
        .await
        .unwrap();

    FndrMcpServer::with_vector_route(store, Blocklist::default(), embedder, index_dir)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_vector_hit_is_cited_and_billed_to_the_budget_like_a_keyword_hit() {
    let dir = scratch("citations");
    let server = fixed_vector_server(&dir, "test_pack_vector_citations").await;

    let pack = server
        .context_pack(Parameters(pack_params("bridge", None)))
        .expect("tool call")
        .0;

    assert_eq!(pack.items.len(), 2, "one hit from each route");
    assert_eq!(pack.items[0].chunk_id, "c-kw");
    assert_eq!(pack.items[0].route, "keyword");
    assert_eq!(pack.items[1].chunk_id, "c-vec");
    assert_eq!(pack.items[1].route, "vector");

    // Every item is cited, whichever route found it: a resolvable record and
    // chunk ID plus the capture metadata that makes the citation readable.
    for item in &pack.items {
        assert!(!item.record_id.is_empty(), "every item cites its record");
        assert!(!item.chunk_id.is_empty(), "and its chunk");
        assert_eq!(item.app_name, "Notes");
        assert_eq!(item.window_title, "fixture");
        assert_eq!(item.captured_at_ms, 42.0);
        assert!(item.estimated_tokens > 0);
    }

    // The vector item carries the whole stored chunk, not the 200-character
    // snippet the search route builds for the same hit.
    assert_eq!(pack.items[1].text, vector_only_text());
    assert!(pack.items[1].text.chars().count() > 200);

    // The vector item's text is billed to the budget, not packed for free:
    // the total is the estimate over both chunks, not the keyword one alone.
    let both_chunks = (KEYWORD_TEXT.len() + vector_only_text().len()) as u32;
    assert_eq!(pack.estimated_tokens_used, both_chunks.div_ceil(4));
    assert!(pack.estimated_tokens_used > (KEYWORD_TEXT.len() as u32).div_ceil(4));
    assert_eq!(pack.dropped_for_budget, 0);
    assert_eq!(pack.vector_dropped_for_budget, 0);

    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_vector_hit_dropped_by_the_budget_is_reported_as_such() {
    let dir = scratch("budget-drop");
    let server = fixed_vector_server(&dir, "test_pack_vector_budget_drop").await;

    // 20 estimated tokens is 80 characters: enough for the 39-character
    // keyword chunk, nowhere near enough to also fit the 300-character
    // vector-only chunk behind it.
    assert!(KEYWORD_TEXT.len() < 80 && KEYWORD_TEXT.len() + vector_only_text().len() > 80);
    let pack = server
        .context_pack(Parameters(pack_params("bridge", Some(20))))
        .expect("tool call")
        .0;

    assert_eq!(pack.items.len(), 1, "only the keyword chunk fits");
    assert_eq!(pack.items[0].chunk_id, "c-kw");
    assert_eq!(
        pack.dropped_for_budget, 1,
        "a thin pack must not look like a thin memory"
    );
    assert_eq!(
        pack.vector_dropped_for_budget, 1,
        "and a caller must be able to tell a dropped semantic hit apart from \
         the vector route having found nothing"
    );
    assert!(pack.vector_route_available);

    std::fs::remove_dir_all(dir).unwrap();
}
