//! Owner-facing search over the local vault.
//!
//! Until now the only way to query FNDR's memory was `fndr.search` over MCP:
//! an agent could read the machine owner's memory and the owner could not.
//! This module is the engine half of the fix, and it deliberately calls the
//! same `fndr-retrieval` merge helpers `fndr.search` calls rather than
//! re-implementing them (ARCHITECTURE.md section 4.2: "the same function
//! serves Tauri IPC, MCP tools, and future companion routes").
//!
//! Two rules shape the code here:
//!
//! 1. **Read-only, and independent of capture.** Search opens the vault with
//!    `Store::open_read_only`, the same accessor the audit viewer uses, so it
//!    can never create a vault, migrate a schema, or take a writer. Searching
//!    does not require capture to be running.
//! 2. **No `Store` is alive across the `.await`.** `fndr_retrieval::vector_hits`
//!    is async and takes no `Store` precisely so the embedding round trip and
//!    Lance disk I/O happen with no database handle held; the synchronous
//!    `tag_vector_hits_with_snippets` re-reads the store afterwards. The phase
//!    structure below preserves that split.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use fndr_inference::{
    CHUNK_EMBEDDING_V1, Embedder, GgufEmbedder, ModelWorkerHandle, Priority, QueuedEmbedder,
};
use fndr_retrieval::TaggedHit;
use fndr_store::Store;
use fndr_types::{
    MemorySearchHit, MemorySearchResults, MemoryVaultState, SearchRoute, VectorRouteState,
};

/// Results returned when the caller does not ask for a specific number.
pub const DEFAULT_SEARCH_LIMIT: u32 = 10;

/// The ceiling the shell applies to a requested limit. It matches
/// `fndr.search`'s own cap so both surfaces bound the same store the same way.
pub const SEARCH_LIMIT_CAP: u32 = 50;

/// How long the query-side model stays resident after the last search. The
/// capture path uses ten minutes because it embeds on a flush cadence; a
/// person typing queries comes in bursts, so this is shorter and reclaims the
/// model's RAM sooner between sessions.
const SEARCH_MODEL_IDLE_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// The local files one search reads. Resolved from the same launch options
/// capture uses, so the reader looks where the writer writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemorySearchPaths {
    pub database_path: PathBuf,
    pub index_dir: PathBuf,
    pub model_path: PathBuf,
}

impl MemorySearchPaths {
    pub fn resolve(data_dir: &Path, model_override: Option<&Path>) -> Self {
        Self {
            database_path: data_dir.join("vault.sqlite3"),
            index_dir: data_dir.join("index"),
            model_path: crate::app::resolved_model_path(data_dir, model_override),
        }
    }
}

/// The shell's query-side model worker, held in Tauri managed state.
///
/// The worker thread is spawned on the first search that needs it and loads
/// the GGUF lazily; `ModelWorkerHandle` unloads it again after
/// `SEARCH_MODEL_IDLE_TIMEOUT`, so an app that is never searched never pays
/// for the model. Searches run at `Priority::Interactive`, which is the whole
/// reason to reuse the existing worker instead of loading a model per query:
/// a person waiting on a query jumps ahead of backfill embedding work.
///
/// Known cost, accepted deliberately for this slice: `RealCaptureScheduler`
/// owns a second `ModelWorkerHandle` for capture-side embedding, so while
/// capture is running and both are hot, two copies of the model can be
/// resident. Sharing one worker across both means threading it through the
/// capture lifecycle's config, which belongs to the capture slice, not this
/// one.
#[derive(Default)]
pub struct ShellSearchModel {
    worker: Mutex<Option<Arc<ModelWorkerHandle>>>,
}

impl ShellSearchModel {
    /// An interactive-priority embedder, or `None` when no local model file
    /// exists at `model_path`. `None` is not a quiet fallback: the caller
    /// reports it as `VectorRouteState::ModelMissing` on the screen.
    pub fn embedder(&self, model_path: &Path) -> Option<QueuedEmbedder> {
        if !model_path.is_file() {
            return None;
        }
        let mut worker = self
            .worker
            .lock()
            .expect("shell search model mutex is not poisoned");
        let handle = match worker.as_ref() {
            Some(handle) => Arc::clone(handle),
            None => {
                let model_path = model_path.to_path_buf();
                let handle = Arc::new(ModelWorkerHandle::spawn(
                    move || {
                        Ok(
                            Box::new(GgufEmbedder::load(&model_path, CHUNK_EMBEDDING_V1.clone())?)
                                as Box<dyn Embedder>,
                        )
                    },
                    SEARCH_MODEL_IDLE_TIMEOUT,
                ));
                *worker = Some(Arc::clone(&handle));
                handle
            }
        };
        // The guard is released here, before the caller's `.await`.
        Some(QueuedEmbedder::new(
            handle,
            Priority::Interactive,
            CHUNK_EMBEDDING_V1.clone(),
        ))
    }
}

/// Run one owner search: keyword route first, then the semantic route for
/// anything keyword did not already find.
///
/// `embedder` being `None` means no local model is installed; that becomes a
/// visible `VectorRouteState`, never a keyword-only answer presented as the
/// whole answer (invariant 4 / PRD P0.11).
pub async fn search_memories(
    paths: &MemorySearchPaths,
    embedder: Option<&dyn Embedder>,
    query: &str,
    limit: usize,
) -> Result<MemorySearchResults, String> {
    let vector_route_before_query = unavailable_vector_route(paths, embedder);

    // An absent vault truthfully means nothing has been captured into this
    // data directory yet. It is not an error and must not create a database.
    if !paths.database_path.exists() {
        return Ok(MemorySearchResults {
            query: query.to_owned(),
            hits: Vec::new(),
            vault: MemoryVaultState::NotCreated,
            vector_route: vector_route_before_query.unwrap_or(VectorRouteState::IndexMissing),
        });
    }

    let mut app_names = AppNameCache::default();
    let mut seen: HashSet<String> = HashSet::new();

    // Phase 1: keyword. The store is opened and dropped inside this block, so
    // no database handle survives into the await below.
    let mut hits = {
        let store = open_vault(&paths.database_path)?;
        let tagged = fndr_retrieval::keyword_hits(&store, query, limit, &mut seen)
            .map_err(|_| SEARCH_UNAVAILABLE.to_owned())?;
        app_names.hits_from(&store, tagged)
    };

    // Phase 2: the semantic route. No store is open here: this is the embed
    // plus Lance round trip only. The thread parked on this await is waiting
    // on the model worker's channel; llama.cpp itself runs on that worker's
    // own thread.
    let vector_route = match (vector_route_before_query, embedder) {
        (Some(state), _) => state,
        (None, None) => VectorRouteState::ModelMissing,
        (None, Some(embedder)) => {
            match fndr_retrieval::vector_hits(embedder, &paths.index_dir, query, limit, &mut seen)
                .await
            {
                Ok(raw_hits) => {
                    // Phase 3: the store is reopened for snippet building
                    // only, mirroring the keyword phase's scope.
                    if !raw_hits.is_empty() {
                        let store = open_vault(&paths.database_path)?;
                        let tagged =
                            fndr_retrieval::tag_vector_hits_with_snippets(&store, raw_hits);
                        hits.extend(app_names.hits_from(&store, tagged));
                    }
                    VectorRouteState::Available
                }
                Err(error) => {
                    // Logged locally, reported as a state: the webview gets a
                    // stable code, never a dependency error string.
                    eprintln!("FNDR search vector route unavailable: {error}");
                    VectorRouteState::Failed
                }
            }
        }
    };

    hits.truncate(limit);
    Ok(MemorySearchResults {
        query: query.to_owned(),
        hits,
        vault: MemoryVaultState::Ready,
        vector_route,
    })
}

/// The stable operator code every search failure reports. Store and retrieval
/// errors can name local paths, so their text stays in the log.
const SEARCH_UNAVAILABLE: &str = "memory_search_unavailable";

fn open_vault(database_path: &Path) -> Result<Store, String> {
    Store::open_read_only(database_path).map_err(|error| {
        eprintln!("FNDR search could not open the local vault: {error}");
        SEARCH_UNAVAILABLE.to_owned()
    })
}

/// The reasons the semantic route cannot run, known before any query work.
/// `None` means it can be attempted.
fn unavailable_vector_route(
    paths: &MemorySearchPaths,
    embedder: Option<&dyn Embedder>,
) -> Option<VectorRouteState> {
    if embedder.is_none() {
        return Some(VectorRouteState::ModelMissing);
    }
    if !paths.index_dir.is_dir() {
        return Some(VectorRouteState::IndexMissing);
    }
    None
}

/// Per-search memo of record id to application name.
///
/// A `TaggedHit` carries the record's capture `source` ("screen"), not the
/// foreground app, so the app name comes from the record itself. Hits from
/// one browsing session usually share a record, and `record_evidence` also
/// loads that record's chunk text, so looking it up once per record rather
/// than once per hit keeps a bounded page of results to a handful of reads.
#[derive(Default)]
struct AppNameCache {
    by_record: HashMap<String, Option<String>>,
}

impl AppNameCache {
    fn hits_from(&mut self, store: &Store, tagged: Vec<TaggedHit>) -> Vec<MemorySearchHit> {
        tagged
            .into_iter()
            .map(|hit| MemorySearchHit {
                app_name: self.app_name(store, &hit.record_id),
                route: match hit.route {
                    "vector" => SearchRoute::Vector,
                    _ => SearchRoute::Keyword,
                },
                record_id: hit.record_id,
                chunk_id: hit.chunk_id,
                captured_at_ms: hit.captured_at_ms as f64,
                snippet: hit.snippet,
            })
            .collect()
    }

    fn app_name(&mut self, store: &Store, record_id: &str) -> Option<String> {
        if let Some(cached) = self.by_record.get(record_id) {
            return cached.clone();
        }
        let app_name = store
            .record_evidence(record_id)
            .ok()
            .flatten()
            .map(|evidence| evidence.app_name);
        self.by_record.insert(record_id.to_owned(), app_name.clone());
        app_name
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use fndr_inference::{EmbedError, EmbeddingSpec};
    use fndr_store::{LanceWriter, NewChunk, NewRecord};

    use super::*;

    /// A deterministic two-dimension embedder. Test embedders live in test
    /// code (invariant 4); nothing here ships in the binary.
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
                    if text.contains("kayak") {
                        vec![1.0, 0.0]
                    } else {
                        vec![0.0, 1.0]
                    }
                })
                .collect())
        }
    }

    /// A `process::id()` plus nanosecond path is not unique across threads in
    /// one `cargo test` process (lessons.md 2026-09-06), so the counter is
    /// what actually separates concurrent tests.
    fn scratch(name: &str) -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "fndr-shell-search-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn seed_vault(data_dir: &Path, app_name: &str, text: &str) {
        let mut store = Store::open(&data_dir.join("vault.sqlite3")).unwrap();
        store
            .insert_capture(
                &NewRecord {
                    id: "r1".into(),
                    session_id: "s1".into(),
                    source: "screen".into(),
                    app_name: app_name.to_owned(),
                    bundle_id: None,
                    url: None,
                    window_title: "fixture".into(),
                    captured_at_ms: 1_700_000_000_000,
                    created_at_ms: 1_700_000_000_000,
                },
                &[NewChunk {
                    id: "c1".into(),
                    ord: 0,
                    text: text.to_owned(),
                }],
            )
            .unwrap();
    }

    fn test_embedder(table: &'static str) -> TestEmbedder {
        TestEmbedder {
            spec: EmbeddingSpec {
                model_id: "test-shell-search",
                dim: 2,
                lance_table: table,
            },
        }
    }

    #[tokio::test]
    async fn an_absent_vault_reports_not_created_instead_of_an_empty_result() {
        let dir = scratch("no-vault");
        let paths = MemorySearchPaths::resolve(&dir, Some(Path::new("/tmp/absent-model.gguf")));

        let results = search_memories(&paths, None, "kayak", 10).await.unwrap();

        assert_eq!(results.vault, MemoryVaultState::NotCreated);
        assert!(results.hits.is_empty());
        assert_eq!(results.query, "kayak");
        assert!(!paths.database_path.exists(), "search must not create a vault");

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn a_missing_model_is_a_visible_state_not_a_silent_keyword_only_answer() {
        let dir = scratch("model-missing");
        seed_vault(&dir, "Safari", "the kayak rental opens at nine");
        let paths = MemorySearchPaths::resolve(&dir, Some(Path::new("/tmp/absent-model.gguf")));

        let results = search_memories(&paths, None, "kayak", 10).await.unwrap();

        assert_eq!(results.vault, MemoryVaultState::Ready);
        assert_eq!(results.vector_route, VectorRouteState::ModelMissing);
        assert_eq!(results.hits.len(), 1);
        assert_eq!(results.hits[0].route, SearchRoute::Keyword);
        assert_eq!(results.hits[0].app_name.as_deref(), Some("Safari"));
        assert_eq!(results.hits[0].captured_at_ms, 1_700_000_000_000.0);
        assert!(results.hits[0].snippet.contains("kayak"));

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn a_present_model_without_an_index_reports_index_missing() {
        let dir = scratch("index-missing");
        seed_vault(&dir, "Notes", "the kayak rental opens at nine");
        let paths = MemorySearchPaths::resolve(&dir, Some(Path::new("/tmp/absent-model.gguf")));
        let embedder = test_embedder("shell_search_index_missing");

        let results = search_memories(&paths, Some(&embedder), "kayak", 10)
            .await
            .unwrap();

        assert_eq!(results.vector_route, VectorRouteState::IndexMissing);
        assert_eq!(results.hits.len(), 1, "keyword results still stand");

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn both_routes_run_and_each_hit_reports_the_route_that_found_it() {
        let dir = scratch("both-routes");
        let mut store = Store::open(&dir.join("vault.sqlite3")).unwrap();
        for (record_id, chunk_id, app_name, text) in [
            ("r-kayak", "c-kayak", "Safari", "the kayak rental opens at nine"),
            ("r-other", "c-other", "Notes", "unrelated groceries list"),
        ] {
            store
                .insert_capture(
                    &NewRecord {
                        id: record_id.into(),
                        session_id: "s1".into(),
                        source: "screen".into(),
                        app_name: app_name.into(),
                        bundle_id: None,
                        url: None,
                        window_title: "fixture".into(),
                        captured_at_ms: 1_700_000_000_000,
                        created_at_ms: 1_700_000_000_000,
                    },
                    &[NewChunk {
                        id: chunk_id.into(),
                        ord: 0,
                        text: text.into(),
                    }],
                )
                .unwrap();
        }

        let embedder = test_embedder("shell_search_both_routes");
        let index_dir = dir.join("index");
        LanceWriter::new(&index_dir)
            .flush_once(&mut store, &embedder, 1_700_000_001_000)
            .await
            .unwrap();
        drop(store);

        let paths = MemorySearchPaths::resolve(&dir, Some(Path::new("/tmp/absent-model.gguf")));
        let results = search_memories(&paths, Some(&embedder), "kayak", 10)
            .await
            .unwrap();

        assert_eq!(results.vector_route, VectorRouteState::Available);
        // The keyword route finds the literal match; the vector route adds
        // the chunk keyword did not have, deduped against it.
        assert_eq!(results.hits[0].route, SearchRoute::Keyword);
        assert_eq!(results.hits[0].chunk_id, "c-kayak");
        assert!(
            results.hits.iter().any(|hit| hit.route == SearchRoute::Vector),
            "expected the semantic route to contribute a hit: {:?}",
            results.hits
        );
        assert_eq!(
            results
                .hits
                .iter()
                .filter(|hit| hit.chunk_id == "c-kayak")
                .count(),
            1,
            "a chunk found by keyword must not be repeated by the vector route"
        );

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn results_are_bounded_by_the_requested_limit() {
        let dir = scratch("limit");
        let mut store = Store::open(&dir.join("vault.sqlite3")).unwrap();
        for index in 0..5 {
            store
                .insert_capture(
                    &NewRecord {
                        id: format!("r{index}"),
                        session_id: "s1".into(),
                        source: "screen".into(),
                        app_name: "Notes".into(),
                        bundle_id: None,
                        url: None,
                        window_title: "fixture".into(),
                        captured_at_ms: 1_700_000_000_000 + index,
                        created_at_ms: 1_700_000_000_000 + index,
                    },
                    &[NewChunk {
                        id: format!("c{index}"),
                        ord: 0,
                        text: "the kayak rental opens at nine".into(),
                    }],
                )
                .unwrap();
        }
        drop(store);

        let paths = MemorySearchPaths::resolve(&dir, Some(Path::new("/tmp/absent-model.gguf")));
        let results = search_memories(&paths, None, "kayak", 2).await.unwrap();

        assert_eq!(results.hits.len(), 2);

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn no_model_file_means_no_embedder_and_no_worker_thread() {
        let model = ShellSearchModel::default();
        assert!(model.embedder(Path::new("/tmp/fndr-absent-model.gguf")).is_none());
    }

    #[test]
    fn search_paths_resolve_where_capture_writes() {
        let dir = Path::new("/tmp/fndr-search-paths");
        let paths = MemorySearchPaths::resolve(dir, None);

        assert_eq!(paths.database_path, dir.join("vault.sqlite3"));
        assert_eq!(paths.index_dir, dir.join("index"));
        assert_eq!(
            paths.model_path,
            crate::app::resolved_model_path(dir, None),
            "search must look for the model where capture loads it"
        );
    }
}
