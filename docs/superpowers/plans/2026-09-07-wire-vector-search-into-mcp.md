# Wire Vector Search into `fndr.search` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `fndr.search` (the MCP tool) returns real semantic results, not just FTS keyword matches — a query with no literal keyword overlap with stored text (a paraphrase) still surfaces the right memory, using the vector route that already exists and is already proven correct, just never called from MCP.

**Architecture:** `fndr-mcp::FndrMcpServer` gains an optional vector route (an `Embedder` plus the Lance index directory). `search_inner` runs the existing `KeywordRetriever` exactly as today, and — only when the server was constructed with vector-route pieces — also runs the existing `fndr-retrieval::VectorRetriever`, then merges the two hit lists (keyword first, vector-only hits appended, deduped by `chunk_id`, no score fusion per ADR-006). `crates/fndr-mcp/src/main.rs` gains an optional `--model` CLI flag that, when given, loads the existing `GgufEmbedder` and passes the vector route in; when absent, behavior is byte-identical to today (typed, visible: every hit already carries which route found it).

**Tech Stack:** Existing `fndr-retrieval::VectorRetriever` (already implemented, tested), existing `fndr-inference::GgufEmbedder` (already implements `Embedder`), existing `fndr-store::Store::record_evidence` (for building vector-hit snippets, since Lance rows carry no FTS match markers).

---

## Context

This plan replaces an earlier, much larger plan (`docs/superpowers/plans/2026-09-07-fndr-mvp-desktop-app.md` in the original checkout, not present in this worktree) that assumed an empty `fndr-capture`/`fndr-retrieval`/`fndr-inference` and no Tauri app. Between writing that plan and starting execution, PR #18 ("Alpha spine") merged into `main` with a large body of already-completed work: a real Tauri desktop shell (`crates/fndr-shell`, capture lifecycle, trust window), a full staged ScreenCaptureKit capture pipeline (`fndr-capture`: `admission.rs`, `dedup.rs`, `foreground.rs`, `pipeline.rs`, `sampling.rs`), a real embedder (`fndr-inference::GgufEmbedder`), and a real, tested vector retrieval route (`fndr-retrieval::VectorRetriever`). This plan is scoped to the one gap that survived that merge and still matches the original ask ("no semantic/vector search"): **`fndr.search`, the only search surface an agent or the desktop UI can reach, still calls `KeywordRetriever` only.** `docs/demo/PRESENTER-CARD.md`'s own "Claims to avoid" section states this explicitly: *"Vector retrieval is an opt-in local benchmark route only; the MCP and UI retrieval story remains keyword FTS, with no hybrid, RRF, or reranking."*

Verified directly from source in this worktree (not assumed):
- `crates/fndr-mcp/src/server.rs::search_inner` (line ~500) calls only `KeywordRetriever::new(&store).search(&query, limit)`.
- `fndr-retrieval::VectorRetriever::new(index_dir).search(query, limit, embedder).await -> Result<Vec<VectorHit>, VectorSearchError>` already exists, is already unit-tested (`vector_route_queries_the_same_flushed_lance_derivative`), and already queries the real Lance chunk table via `nearest_to(vector)`.
- `VectorHit { record_id, chunk_id, source, captured_at_ms, distance: f32 }` — no `text`/`snippet` field (Lance rows carry no FTS match markers), so a snippet must be built separately via `Store::record_evidence`.
- `fndr-inference::GgufEmbedder::load(model_path, spec) -> Result<Self, EmbedError>` implements `Embedder` fully (`embed_documents` + a default-provided `embed_query`) and is already loaded successfully today, just only for document-side embedding inside `fndr-shell::capture_scheduler` — never for query-side search.
- `fndr-mcp/Cargo.toml` already depends on `fndr-retrieval`; it does **not** yet depend on `fndr-inference`.
- `crates/fndr-mcp/src/main.rs` parses only `--store` and `--port`; no model/index-dir flag exists.
- The repo's own test convention for exercising `VectorRetriever` without a slow real model load is a small deterministic fake `Embedder` (`TestEmbedder` in `fndr-retrieval`'s own test module, 2-dim vectors keyed on a keyword in the input text) — this plan's new tests follow the same pattern; it is legitimate for testing retrieval *mechanics*, not for the eval-gated ranking-quality claims `make bench` is responsible for.
- `ADR-006` (referenced directly in `VectorRetriever`'s own doc comment) prohibits raw score fusion between keyword and vector results before a benchmark justifies a fusion formula — this plan's merge strategy (keyword hits first, vector-only hits appended, no combined score) respects that by construction, not by omission.

**Out of scope (explicitly, do not build):** RRF fusion, reranking, hybrid scoring, temporal/metadata-prefiltered routes, `fndr.context_pack`'s own retrieval route (still keyword-only after this plan — a separate, later change), any UI search box (the desktop shell's `crates/fndr-shell/ui/` has no search element at all today; adding one is a distinct piece of work this plan does not include), changes to the capture pipeline, changes to `GgufEmbedder` or `VectorRetriever` themselves (both are already correct and tested — this plan only calls them from a new place).

---

## Task 1: `fndr-mcp` depends on `fndr-inference`

**Files:**
- Modify: `crates/fndr-mcp/Cargo.toml`

- [ ] **Step 1: Add the dependency**

Edit `crates/fndr-mcp/Cargo.toml`, add to `[dependencies]` (alphabetical position matches the existing `fndr-privacy`/`fndr-retrieval`/`fndr-store` block):

```toml
[dependencies]
fndr-inference = { path = "../fndr-inference" }
fndr-privacy = { path = "../fndr-privacy" }
fndr-retrieval = { path = "../fndr-retrieval" }
fndr-store = { path = "../fndr-store" }
rmcp = { version = "3.1", features = ["transport-streamable-http-server"] }
axum = "0.8"
tokio = { version = "1", features = ["rt-multi-thread", "macros", "net", "signal"] }
schemars = "1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
# Constant-time token comparison (ADR-007: never a timing oracle).
subtle = "2"
getrandom = "0.3"
thiserror = "2"
tracing = "0.1"
```

- [ ] **Step 2: Verify it resolves**

Run: `cargo build -p fndr-mcp`
Expected: PASS (may take a few minutes if `fndr-inference`'s `llama-cpp-2` dependency isn't already built in this worktree's target directory — this is normal, not a failure signal).

- [ ] **Step 3: Commit**

```bash
git add crates/fndr-mcp/Cargo.toml
git commit -m "fndr-mcp: depend on fndr-inference for the query-side embedder"
```

---

## Task 2: `FndrMcpServer` gains an optional vector route

**Files:**
- Modify: `crates/fndr-mcp/src/server.rs`

- [ ] **Step 1: Write the failing tests first**

Add to the bottom of `crates/fndr-mcp/src/server.rs` (find the existing `#[cfg(test)] mod tests { ... }` block — if the file doesn't already have one, add it; if it does, add these functions inside it, alongside a small deterministic fake embedder following the exact pattern already used in `fndr-retrieval`'s own test module):

```rust
#[cfg(test)]
mod vector_route_tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use fndr_inference::{EmbedError, EmbeddingSpec};
    use fndr_store::{LanceWriter, NewChunk, NewRecord};

    use super::*;

    /// Same pattern as fndr-retrieval's own TestEmbedder: deterministic,
    /// fast, keyed on a substring, exercises the plumbing without a real
    /// model load. Not a claim about ranking quality (that's make bench's
    /// job).
    struct TestEmbedder {
        spec: EmbeddingSpec,
    }

    impl fndr_inference::Embedder for TestEmbedder {
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
            "fndr-mcp-vector-route-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[tokio::test]
    async fn search_without_vector_route_behaves_exactly_as_before() {
        let dir = scratch("keyword-only");
        let mut store = Store::open(&dir.join("vault.sqlite3")).unwrap();
        store
            .insert_capture(
                &NewRecord {
                    id: "r1".into(),
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
                    id: "c1".into(),
                    ord: 0,
                    text: "the suspension bridge inspection is due".into(),
                }],
            )
            .unwrap();

        let server = FndrMcpServer::new(store);
        let Json(out) = server
            .search_inner(Parameters(SearchParams {
                query: "bridge".into(),
                limit: None,
            }))
            .unwrap();
        assert_eq!(out.hits.len(), 1);
        assert_eq!(out.hits[0].route, "keyword");
        assert!(!out.vector_route_available);

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn search_with_vector_route_finds_a_semantic_match_keyword_would_miss() {
        let dir = scratch("vector-fills-gap");
        let mut store = Store::open(&dir.join("vault.sqlite3")).unwrap();
        store
            .insert_capture(
                &NewRecord {
                    id: "r-bridge".into(),
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
                    id: "c-bridge".into(),
                    ord: 0,
                    text: "the suspension bridge inspection is due".into(),
                }],
            )
            .unwrap();

        let embedder: std::sync::Arc<dyn fndr_inference::Embedder> =
            std::sync::Arc::new(TestEmbedder {
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

        // "crossing report" shares zero literal words with the stored text,
        // so KeywordRetriever alone would return nothing; TestEmbedder maps
        // any text without "bridge" to the same vector as one with it is
        // near, proving this is the vector route's hit, not keyword's.
        let server = FndrMcpServer::with_vector_route(store, Blocklist::default(), embedder, index_dir);
        let Json(out) = server
            .search_inner(Parameters(SearchParams {
                query: "crossing report".into(),
                limit: None,
            }))
            .unwrap();
        assert_eq!(out.hits.len(), 1);
        assert_eq!(out.hits[0].chunk_id, "c-bridge");
        assert_eq!(out.hits[0].route, "vector");
        assert!(out.vector_route_available);

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn a_hit_found_by_both_routes_is_not_duplicated() {
        let dir = scratch("dedupe");
        let mut store = Store::open(&dir.join("vault.sqlite3")).unwrap();
        store
            .insert_capture(
                &NewRecord {
                    id: "r-bridge".into(),
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
                    id: "c-bridge".into(),
                    ord: 0,
                    text: "the suspension bridge inspection is due".into(),
                }],
            )
            .unwrap();

        let embedder: std::sync::Arc<dyn fndr_inference::Embedder> =
            std::sync::Arc::new(TestEmbedder {
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
        let Json(out) = server
            .search_inner(Parameters(SearchParams {
                query: "bridge".into(),
                limit: None,
            }))
            .unwrap();
        assert_eq!(out.hits.len(), 1);
        assert_eq!(out.hits[0].route, "keyword");

        std::fs::remove_dir_all(dir).unwrap();
    }
}
```

- [ ] **Step 2: Run the tests, confirm they fail to compile**

Run: `cargo test -p fndr-mcp vector_route_tests`
Expected: FAIL to compile — `FndrMcpServer::with_vector_route` doesn't exist yet, `SearchHitOut` has no `route` field, `SearchOutput` has no `vector_route_available` field.

- [ ] **Step 3: Extend `SearchHitOut` and `SearchOutput`**

Edit `crates/fndr-mcp/src/server.rs` around the existing struct definitions (line ~60):

```rust
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct SearchHitOut {
    pub record_id: String,
    pub chunk_id: String,
    pub source: String,
    pub captured_at_ms: f64,
    pub snippet: String,
    /// Which route produced this hit: "keyword" or "vector". A hit found by
    /// both routes reports "keyword" (its snippet carries real match
    /// markers; the vector route's does not).
    pub route: String,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct SearchOutput {
    pub hits: Vec<SearchHitOut>,
    /// True when this server was constructed with a query-side embedder and
    /// Lance index directory, so a caller can tell "no semantic results"
    /// apart from "semantic search wasn't even attempted" (invariant 4: no
    /// silent degradation).
    pub vector_route_available: bool,
}
```

- [ ] **Step 4: Extend `FndrMcpServer`'s fields and constructors**

Edit the struct and its `impl` block (line ~413):

```rust
#[derive(Clone)]
pub struct FndrMcpServer {
    // Mutex because rusqlite's Connection is Send but not Sync. The real
    // engine gets a proper connection strategy with T-201.
    store: Arc<Mutex<Store>>,
    blocklist: Blocklist,
    // Both present or both absent — never partially configured. Absent
    // means "no model was given at launch," a typed, visible state
    // (SearchOutput.vector_route_available), never a silent skip.
    vector_route: Option<(Arc<dyn fndr_inference::Embedder>, PathBuf)>,
}
```

Add `use std::path::PathBuf;` to the top of the file if not already imported (check the existing `use` block first — the file already imports `std::sync::{Arc, Mutex}` per the struct above, so add `PathBuf` alongside).

```rust
#[tool_router(server_handler)]
impl FndrMcpServer {
    pub fn new(store: Store) -> Self {
        Self::with_blocklist(store, Blocklist::default())
    }

    pub fn with_blocklist(store: Store, blocklist: Blocklist) -> Self {
        Self {
            store: Arc::new(Mutex::new(store)),
            blocklist,
            vector_route: None,
        }
    }

    /// Like `with_blocklist`, plus a query-side embedder and the Lance index
    /// directory it should query. `fndr-mcp`'s CLI entrypoint calls this
    /// only when launched with `--model`; every other caller (including all
    /// existing tests) keeps using `new`/`with_blocklist` unchanged.
    pub fn with_vector_route(
        store: Store,
        blocklist: Blocklist,
        embedder: Arc<dyn fndr_inference::Embedder>,
        index_dir: PathBuf,
    ) -> Self {
        Self {
            store: Arc::new(Mutex::new(store)),
            blocklist,
            vector_route: Some((embedder, index_dir)),
        }
    }

    // ... existing methods (registered_tool_names, recent_tool_calls, audit) unchanged ...
```

- [ ] **Step 5: Rewrite `search_inner` to merge both routes**

Replace the existing `search_inner` (line ~500):

```rust
    fn search_inner(
        &self,
        Parameters(SearchParams { query, limit }): Parameters<SearchParams>,
    ) -> Result<Json<SearchOutput>, ErrorData> {
        let limit = limit.unwrap_or(10).min(50) as usize;
        let store = self
            .store
            .lock()
            .map_err(|_| ErrorData::internal_error("store lock poisoned", None))?;

        let keyword_hits = KeywordRetriever::new(&store)
            .search(&query, limit)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;

        let mut seen: std::collections::HashSet<String> =
            keyword_hits.iter().map(|h| h.chunk_id.clone()).collect();
        let mut hits: Vec<SearchHitOut> = keyword_hits
            .into_iter()
            .map(|h| SearchHitOut {
                record_id: h.record_id,
                chunk_id: h.chunk_id,
                source: h.source,
                captured_at_ms: h.captured_at_ms as f64,
                snippet: h.snippet,
                route: "keyword".to_owned(),
            })
            .collect();

        let vector_route_available = self.vector_route.is_some();
        if let Some((embedder, index_dir)) = &self.vector_route {
            let vector_hits = tauri_free_block_on(fndr_retrieval::VectorRetriever::new(index_dir).search(
                &query,
                limit,
                embedder.as_ref(),
            ))
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;

            for hit in vector_hits {
                if !seen.insert(hit.chunk_id.clone()) {
                    continue; // already present via the keyword route
                }
                let snippet = self
                    .vector_hit_snippet(&store, &hit)
                    .unwrap_or_default();
                hits.push(SearchHitOut {
                    record_id: hit.record_id,
                    chunk_id: hit.chunk_id,
                    source: hit.source,
                    captured_at_ms: hit.captured_at_ms as f64,
                    snippet,
                    route: "vector".to_owned(),
                });
            }
        }
        hits.truncate(limit);

        Ok(Json(SearchOutput {
            hits,
            vector_route_available,
        }))
    }

    /// A vector hit carries no FTS match markers (Lance rows have no text
    /// snippet), so this builds a plain truncated excerpt from the same
    /// durable store the keyword route reads, instead of returning an empty
    /// or fabricated snippet.
    fn vector_hit_snippet(
        &self,
        store: &Store,
        hit: &fndr_retrieval::VectorHit,
    ) -> Option<String> {
        const SNIPPET_CHARS: usize = 200;
        let evidence = store.record_evidence(&hit.record_id).ok()??;
        let chunk = evidence.chunks.into_iter().find(|c| c.chunk_id == hit.chunk_id)?;
        if chunk.text.chars().count() <= SNIPPET_CHARS {
            Some(chunk.text)
        } else {
            let truncated: String = chunk.text.chars().take(SNIPPET_CHARS).collect();
            Some(format!("{truncated}…"))
        }
    }
```

`search_inner` is a synchronous method (`fn`, not `async fn`) today because `KeywordRetriever::search` is synchronous, but `VectorRetriever::search` is `async`. Since `FndrMcpServer`'s `#[tool]` methods are called from within a `tokio` runtime already (the MCP server runs on one), block on the vector search with `tokio::runtime::Handle::current().block_on(...)` rather than making `search_inner` itself `async` — changing its signature would ripple into the `#[tool]` macro wrapper and every caller, which is out of scope for this plan. Add this helper near the top of the file (after the `use` block):

```rust
/// Runs a future to completion from inside a synchronous method that is
/// itself already called from within a tokio runtime (every `#[tool]`
/// handler is). Named for what it does, not for Tauri (this crate has no
/// Tauri dependency) — it just avoids a bare, easy-to-misread
/// `tokio::runtime::Handle::current().block_on(...)` at each call site.
fn tauri_free_block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Handle::current().block_on(future)
}
```

If this blocks the async runtime's worker thread in a way `cargo clippy -p fndr-mcp -- -D warnings` flags (some clippy configurations warn on `block_on` inside async contexts), the fallback is `tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(future))`, which is safe specifically for a multi-threaded runtime (already what `fndr-mcp`'s `tokio` dependency is configured with, per `rt-multi-thread` in `Cargo.toml`). Use whichever variant actually satisfies `-D warnings`; both are correct, only clippy's opinion differs.

- [ ] **Step 6: Run the tests, confirm they pass**

Run: `cargo test -p fndr-mcp vector_route_tests`
Expected: PASS, 3 tests (`search_without_vector_route_behaves_exactly_as_before`, `search_with_vector_route_finds_a_semantic_match_keyword_would_miss`, `a_hit_found_by_both_routes_is_not_duplicated`).

- [ ] **Step 7: Run the full existing `fndr-mcp` test suite to confirm nothing regressed**

Run: `cargo test -p fndr-mcp`
Expected: PASS — every pre-existing test (auth, audit, the 12 MCP tools, the skeleton e2e round-trip) still passes unchanged. `FndrMcpServer::new(store)` still compiles and behaves identically for every existing caller (it now sets `vector_route: None` internally, which is exactly the previous implicit behavior).

- [ ] **Step 8: Lint clean**

Run: `cargo clippy -p fndr-mcp --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add crates/fndr-mcp/src/server.rs
git commit -m "fndr-mcp: fndr.search merges the vector route when one is configured"
```

---

## Task 3: `fndr-mcp`'s CLI loads a model and wires the vector route

**Files:**
- Modify: `crates/fndr-mcp/src/main.rs`

- [ ] **Step 1: Write the failing test for the new CLI flag**

`LaunchOptions` already has a `#[cfg(test)] mod tests` block (line ~92). Add a case for the new optional flags:

```rust
    #[test]
    fn parses_optional_model_and_index_dir() {
        assert_eq!(
            LaunchOptions::parse(
                [
                    "--store", "/tmp/fndr.sqlite3",
                    "--model", "/tmp/model.gguf",
                    "--index-dir", "/tmp/index",
                ]
                .map(str::to_owned)
            ),
            Ok(LaunchOptions {
                store_path: PathBuf::from("/tmp/fndr.sqlite3"),
                port: 0,
                model_path: Some(PathBuf::from("/tmp/model.gguf")),
                index_dir: Some(PathBuf::from("/tmp/index")),
            })
        );
    }

    #[test]
    fn model_and_index_dir_default_to_none() {
        let opts =
            LaunchOptions::parse(["--store", "/tmp/fndr.sqlite3"].map(str::to_owned)).unwrap();
        assert_eq!(opts.model_path, None);
        assert_eq!(opts.index_dir, None);
    }
```

- [ ] **Step 2: Run it, confirm it fails to compile**

Run: `cargo test -p fndr-mcp --bin fndr-mcp parses_optional_model_and_index_dir`
Expected: FAIL to compile — `LaunchOptions` has no `model_path`/`index_dir` fields yet.

- [ ] **Step 3: Extend `LaunchOptions` and its parser**

Replace the struct and `parse` function:

```rust
#[derive(Debug, PartialEq, Eq)]
struct LaunchOptions {
    store_path: PathBuf,
    port: u16,
    /// When given together with index_dir, fndr.search also queries the
    /// real vector route. Absent by default: an alpha demo host with no
    /// model still serves keyword search exactly as before.
    model_path: Option<PathBuf>,
    index_dir: Option<PathBuf>,
}

impl LaunchOptions {
    fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut store_path = None;
        let mut port = 0;
        let mut model_path = None;
        let mut index_dir = None;
        let mut args = args.into_iter();

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--store" => {
                    let value = args
                        .next()
                        .ok_or_else(|| "--store requires a path".to_owned())?;
                    store_path = Some(PathBuf::from(value));
                }
                "--port" => {
                    let value = args
                        .next()
                        .ok_or_else(|| "--port requires a number".to_owned())?;
                    port = value
                        .parse::<u16>()
                        .map_err(|_| "--port must be between 0 and 65535".to_owned())?;
                }
                "--model" => {
                    let value = args
                        .next()
                        .ok_or_else(|| "--model requires a path".to_owned())?;
                    model_path = Some(PathBuf::from(value));
                }
                "--index-dir" => {
                    let value = args
                        .next()
                        .ok_or_else(|| "--index-dir requires a path".to_owned())?;
                    index_dir = Some(PathBuf::from(value));
                }
                "--help" | "-h" => return Err(String::new()),
                other => return Err(format!("unknown option: {other}")),
            }
        }

        let store_path = store_path.ok_or_else(|| "--store is required".to_owned())?;
        if model_path.is_some() != index_dir.is_some() {
            return Err("--model and --index-dir must be given together".to_owned());
        }
        Ok(Self {
            store_path,
            port,
            model_path,
            index_dir,
        })
    }
}
```

- [ ] **Step 4: Run the new tests, confirm they pass**

Run: `cargo test -p fndr-mcp --bin fndr-mcp parses_optional_model_and_index_dir model_and_index_dir_default_to_none`
Expected: PASS, 2 tests.

Run: `cargo test -p fndr-mcp --bin fndr-mcp`
Expected: PASS — the pre-existing `parses_required_store_and_optional_port` and `refuses_missing_store_and_invalid_port` tests still pass (their expected `LaunchOptions` values need `model_path: None, index_dir: None` added to their `Ok(...)` literals — update those two existing test assertions to include the two new fields, matching the pattern above).

- [ ] **Step 5: Wire model loading into `print_usage` and `main`**

```rust
fn print_usage() {
    eprintln!("usage: fndr-mcp --store PATH [--port PORT] [--model PATH --index-dir PATH]");
}

fn main() {
    let options = LaunchOptions::parse(std::env::args().skip(1)).unwrap_or_else(|error| {
        if !error.is_empty() {
            eprintln!("FNDR MCP launch options: {error}");
        }
        print_usage();
        std::process::exit(if error.is_empty() { 0 } else { 2 });
    });
    let store = Store::open(&options.store_path).unwrap_or_else(|error| {
        eprintln!(
            "FNDR MCP could not open {}: {error}",
            options.store_path.display()
        );
        std::process::exit(1);
    });

    let server = match (options.model_path, options.index_dir) {
        (Some(model_path), Some(index_dir)) => {
            let spec = fndr_inference::CHUNK_EMBEDDING_V1;
            let embedder = fndr_inference::GgufEmbedder::load(&model_path, spec)
                .unwrap_or_else(|error| {
                    eprintln!("FNDR MCP could not load {}: {error}", model_path.display());
                    std::process::exit(1);
                });
            println!("Vector route enabled: {}", model_path.display());
            FndrMcpServer::with_vector_route(
                store,
                fndr_privacy::Blocklist::default(),
                std::sync::Arc::new(embedder),
                index_dir,
            )
        }
        (None, None) => {
            println!("Vector route disabled (no --model given); fndr.search is keyword-only.");
            FndrMcpServer::new(store)
        }
        _ => unreachable!("LaunchOptions::parse already rejected a partial pair"),
    };

    let token = generate_token();
    let runtime = tokio::runtime::Runtime::new().expect("Tokio runtime must initialize");

    runtime.block_on(async move {
        let (addr, handle) = serve_loopback(server, token.clone(), options.port)
            .await
            .unwrap_or_else(|error| {
                eprintln!("FNDR MCP could not bind: {error}");
                std::process::exit(1);
            });
        println!("FNDR MCP serving at http://{addr}/mcp");
        println!("Authorization: Bearer {token}");
        println!("\nAdd to Claude Code:");
        println!(
            "  claude mcp add fndr --transport http http://{addr}/mcp --header \"Authorization: Bearer {token}\""
        );
        println!("\nCtrl-C to stop. The bearer token is process-local; do not record or commit it.");

        let _ = tokio::signal::ctrl_c().await;
        handle.abort();
    });
}
```

Note `FndrMcpServer` needs to be constructed *before* entering the `tokio::runtime::Runtime::new()...block_on` — `GgufEmbedder::load` is synchronous (confirmed: `pub fn load(model_path: &Path, spec: EmbeddingSpec) -> Result<Self, EmbedError>`, no `async`), so this ordering is correct and doesn't need its own runtime.

- [ ] **Step 6: Verify the binary builds**

Run: `cargo build -p fndr-mcp --bin fndr-mcp`
Expected: PASS.

- [ ] **Step 7: Verify lint and the full crate test suite once more**

Run: `cargo clippy -p fndr-mcp --all-targets -- -D warnings && cargo test -p fndr-mcp`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add crates/fndr-mcp/src/main.rs
git commit -m "fndr-mcp: --model/--index-dir flags enable the vector route at launch"
```

---

## Task 4: End-to-end verification with a real model

**Files:** none (verification only)

- [ ] **Step 1: Seed a small real vault using the existing alpha runbook flow**

Follow `docs/demo/ALPHA-RUNBOOK.md`'s existing pattern to create a scratch vault and capture at least one real record (via the desktop shell or the `fndr-mcp --example skeleton` path it documents) so a real Lance index with real embedded chunks exists on disk:

```bash
alpha_tmp=$(mktemp -d /tmp/fndr-vector-mcp.XXXXXX)
```

Use whichever seeding path the runbook already documents to populate `$alpha_tmp/desktop/vault.sqlite3` and its Lance `index/` directory with at least one record whose text is specific enough to paraphrase (e.g., capture a screen showing a distinctive sentence).

- [ ] **Step 2: Start `fndr-mcp` with the vector route enabled**

Run:
```bash
cargo run -p fndr-mcp -- --store "$alpha_tmp/desktop/vault.sqlite3" \
  --model models/Qwen3-Embedding-0.6B-Q8_0.gguf \
  --index-dir "$alpha_tmp/desktop/index"
```
Expected: prints `Vector route enabled: models/Qwen3-Embedding-0.6B-Q8_0.gguf` before the usual `FNDR MCP serving at ...` lines.

- [ ] **Step 3: Issue a paraphrased search over the same MCP connection the runbook already documents**

Using the printed bearer token and the `claude mcp add` line (or a direct HTTP call matching this repo's existing test pattern in `crates/fndr-mcp/tests/`), call `fndr.search` with a query that paraphrases the captured content rather than repeating its literal words.

Expected (real, observed): the response includes a hit with `"route": "vector"` and `"vector_route_available": true` — not an empty result, and not a hit whose route is `"keyword"` (which would mean the paraphrase accidentally shared a literal term, not a real vector-route proof; if that happens, pick a query with genuinely zero word overlap and retry).

- [ ] **Step 4: Confirm the keyword-only path still works unchanged (regression check)**

Stop the server (Ctrl-C), restart it without `--model`/`--index-dir`:
```bash
cargo run -p fndr-mcp -- --store "$alpha_tmp/desktop/vault.sqlite3"
```
Expected: prints `Vector route disabled (no --model given); fndr.search is keyword-only.` Search still works for literal keyword queries, every hit's `route` is `"keyword"`, `vector_route_available` is `false`.

- [ ] **Step 5: Clean up**

```bash
rm -rf "$alpha_tmp"
```

- [ ] **Step 6: Run the full verification gate**

Run: `make test`
Expected: PASS.

- [ ] **Step 7: No commit** (verification only).

---

## Task 5: Update docs to match reality

**Files:**
- Modify: `docs/demo/DEMO-READINESS.md`
- Modify: `docs/demo/PRESENTER-CARD.md`
- Modify: `docs/demo/ALPHA-RUNBOOK.md`
- Modify: `docs/ROADMAP-TICKETS.md`

- [ ] **Step 1: Update `docs/demo/DEMO-READINESS.md`**

In the "Verified now" table, update the Retrieval row (and add a new row if the existing one only covers the bench-only claim) to state that `fndr.search` now merges a real vector route when `fndr-mcp` is launched with `--model`, citing Task 4's verification as the evidence, and keep the existing FTS-baseline claim as-is (both are true, not a replacement). Keep the boundary column honest: this is still not hybrid/RRF/reranked, and still has no UI surface.

- [ ] **Step 2: Update `docs/demo/PRESENTER-CARD.md`**

Find the "Claims to avoid" line: *"Vector retrieval is an opt-in local benchmark route only; the MCP and UI retrieval story remains keyword FTS, with no hybrid, RRF, or reranking."* Replace it with an accurate statement: vector retrieval is now available through `fndr.search` when the server is launched with `--model`/`--index-dir` (results are merged, not fused/ranked together — still no hybrid/RRF/reranking), and the UI still has no search surface at all (that remains a claim to avoid).

- [ ] **Step 3: Update `docs/demo/ALPHA-RUNBOOK.md`**

Add the `--model`/`--index-dir` flags to the documented `fndr-mcp` launch command (terminal 2 in the existing runbook), noting they're optional and what changes when they're given (matches Task 3's CLI).

- [ ] **Step 4: Update `docs/ROADMAP-TICKETS.md`**

Find T-505's status line (currently "Partial 2026-09-06", noting the vector route reads the Lance derivative but isn't wired into MCP/UI/context packs). Update it to note MCP wiring landed for `fndr.search` specifically (not `fndr.context_pack`, not the UI, not RRF/hybrid — keep those listed as still missing). Find T-702's status line similarly and note the same narrow addition; do not mark either ticket "Done" — both still have real missing scope (RRF, hybrid, temporal routes, UI, context-pack integration).

- [ ] **Step 5: Verify no other doc references the now-stale "MCP retrieval story remains keyword FTS" claim**

Run: `grep -rn "remains keyword FTS\|MCP-served vector retrieval" docs/`
Expected: only matches in files already updated in Steps 1-3 above (or none, if the exact phrasing differs slightly — read any remaining hits and update them the same way).

- [ ] **Step 6: Commit**

```bash
git add docs/demo/DEMO-READINESS.md docs/demo/PRESENTER-CARD.md docs/demo/ALPHA-RUNBOOK.md docs/ROADMAP-TICKETS.md
git commit -m "docs: fndr.search now serves a real vector route when launched with --model"
```

---

## Verification summary (what proves this plan actually worked)

- `cargo test -p fndr-mcp` is green, including the 3 new vector-route tests and the 2 new CLI-flag tests, with every pre-existing test unchanged and passing.
- A real `fndr-mcp --model ... --index-dir ...` run returns a `route: "vector"` hit for a genuine paraphrase query with zero literal keyword overlap with the captured text (Task 4, real execution, not assumed).
- The same server launched without `--model` behaves byte-identical to before this plan (regression-safe).
- `docs/demo/PRESENTER-CARD.md` no longer claims something the code contradicts.
- `make test` is green throughout — every task leaves the workspace fully tested and lint-clean.
