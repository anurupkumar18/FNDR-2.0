//! T-202 write-path tests: flush success, retry-after-failure, empty-batch
//! no-op (no Lance version churn), and the wrong-dimension refusal. Uses a
//! deterministic test embedder; per the invariants, test code is the only
//! place a non-real embedder may exist.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use fndr_inference::{
    CHUNK_EMBEDDING_V1, EmbedError, Embedder, EmbeddingSpec, ModelWorkerHandle, Priority,
    QueuedEmbedder,
};
use fndr_privacy::sanitize_url_for_storage;
use fndr_store::{DeleteScope, LanceWriter, NewChunk, NewRecord, Store, delete_everywhere};

struct TestEmbedder {
    fail: AtomicBool,
    dim_override: Option<usize>,
}

impl TestEmbedder {
    fn good() -> Self {
        Self {
            fail: AtomicBool::new(false),
            dim_override: None,
        }
    }
    fn failing() -> Self {
        Self {
            fail: AtomicBool::new(true),
            dim_override: None,
        }
    }
    fn wrong_dim() -> Self {
        Self {
            fail: AtomicBool::new(false),
            dim_override: Some(5),
        }
    }
}

impl Embedder for TestEmbedder {
    fn spec(&self) -> &EmbeddingSpec {
        &CHUNK_EMBEDDING_V1
    }

    fn embed_documents(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        if self.fail.load(Ordering::Relaxed) {
            return Err(EmbedError::Unavailable("test outage".into()));
        }
        let dim = self.dim_override.unwrap_or(self.spec().dim);
        Ok(texts
            .iter()
            .map(|t| {
                let seed = t.len() as f32 + 1.0;
                (0..dim).map(|i| (seed + i as f32).sin()).collect()
            })
            .collect())
    }
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fndr-t202-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn seed_capture(store: &mut Store, record_id: &str, n_chunks: usize) {
    let record = NewRecord {
        id: record_id.to_string(),
        session_id: "s1".into(),
        source: "screen".into(),
        app_name: "Terminal".into(),
        bundle_id: None,
        url: None,
        window_title: "fndr".into(),
        captured_at_ms: 1_755_000_000_000,
        created_at_ms: 1_755_000_000_000,
    };
    let chunks: Vec<NewChunk> = (0..n_chunks)
        .map(|ord| NewChunk {
            id: format!("{record_id}-c{ord}"),
            ord: ord as i64,
            text: format!("chunk {ord} of {record_id} discussing the flush writer"),
        })
        .collect();
    store.insert_capture(&record, &chunks).unwrap();
}

fn seed_capture_at(store: &mut Store, record_id: &str, captured_at_ms: i64, url: Option<&str>) {
    let record = NewRecord {
        id: record_id.to_string(),
        session_id: "s1".into(),
        source: "screen".into(),
        app_name: "Terminal".into(),
        bundle_id: None,
        url: url.and_then(sanitize_url_for_storage),
        window_title: "fndr".into(),
        captured_at_ms,
        created_at_ms: captured_at_ms,
    };
    store
        .insert_capture(
            &record,
            &[NewChunk {
                id: format!("{record_id}-c0"),
                ord: 0,
                text: format!("chunk of {record_id}"),
            }],
        )
        .unwrap();
}

#[tokio::test]
async fn flush_writes_marks_and_skips_when_drained() {
    let dir = scratch("happy");
    let mut store = Store::open(&dir.join("fndr.sqlite3")).unwrap();
    seed_capture(&mut store, "r1", 3);
    seed_capture(&mut store, "r2", 2);

    let writer = LanceWriter::new(&dir.join("index"));
    let embedder = TestEmbedder::good();

    let report = writer.flush_once(&mut store, &embedder, 42).await.unwrap();
    assert_eq!(report.written, 5);
    assert!(store.pending_chunks(10).unwrap().is_empty());

    // Table exists with the contract name, rows, and the two cheap indexes.
    let db = lancedb::connect(dir.join("index").to_str().unwrap())
        .execute()
        .await
        .unwrap();
    let table = db.open_table("chunks_v1_qwen768").execute().await.unwrap();
    assert_eq!(table.count_rows(None).await.unwrap(), 5);
    let index_names: Vec<String> = table
        .list_indices()
        .await
        .unwrap()
        .into_iter()
        .map(|i| i.name)
        .collect();
    assert!(index_names.iter().any(|n| n.contains("captured_at_ms")));
    assert!(index_names.iter().any(|n| n.contains("text")));
    let version_after_flush = table.version().await.unwrap();

    // Drained: the next flush must not create a Lance version (spike rule).
    let report = writer.flush_once(&mut store, &embedder, 43).await.unwrap();
    assert_eq!(report.written, 0);
    assert_eq!(table.version().await.unwrap(), version_after_flush);

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn failed_flush_leaves_truth_intact_and_retries() {
    let dir = scratch("retry");
    let mut store = Store::open(&dir.join("fndr.sqlite3")).unwrap();
    seed_capture(&mut store, "r1", 4);

    let writer = LanceWriter::new(&dir.join("index"));
    let outage = TestEmbedder::failing();
    let result = writer.flush_once(&mut store, &outage, 42).await;
    assert!(result.is_err(), "outage must surface, not vanish");
    assert_eq!(store.pending_chunks(10).unwrap().len(), 4, "still pending");

    let report = writer
        .flush_once(&mut store, &TestEmbedder::good(), 43)
        .await
        .unwrap();
    assert_eq!(report.written, 4, "retry drains the same batch");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn wrong_dimension_write_is_refused() {
    let dir = scratch("dim");
    let mut store = Store::open(&dir.join("fndr.sqlite3")).unwrap();
    seed_capture(&mut store, "r1", 1);

    let writer = LanceWriter::new(&dir.join("index"));
    let result = writer
        .flush_once(&mut store, &TestEmbedder::wrong_dim(), 42)
        .await;
    match result {
        Err(fndr_store::FlushError::WrongDimension { got, expected, .. }) => {
            assert_eq!(got, 5);
            assert_eq!(expected, 768);
        }
        other => panic!("expected WrongDimension, got {other:?}"),
    }
    assert_eq!(
        store.pending_chunks(10).unwrap().len(),
        1,
        "nothing stamped"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// T-403 integration: `LanceWriter::flush_once` needs only `&dyn Embedder`,
/// so a `QueuedEmbedder` (routing through a real `ModelWorkerHandle`) slots
/// in with zero changes to `LanceWriter` itself.
#[tokio::test]
async fn flush_once_works_through_the_model_worker_queue() {
    let dir = scratch("via-queue");
    let mut store = Store::open(&dir.join("fndr.sqlite3")).unwrap();
    seed_capture(&mut store, "r1", 2);

    let worker = Arc::new(ModelWorkerHandle::spawn(
        || Ok(Box::new(TestEmbedder::good()) as Box<dyn Embedder>),
        Duration::from_secs(30),
    ));
    let queued = QueuedEmbedder::new(worker, Priority::Backfill, CHUNK_EMBEDDING_V1);

    let writer = LanceWriter::new(&dir.join("index"));
    let report = writer.flush_once(&mut store, &queued, 42).await.unwrap();
    assert_eq!(report.written, 2);
    assert!(store.pending_chunks(10).unwrap().is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn deletion_everywhere_removes_suffix_domain_records_from_sqlite_and_lance() {
    let dir = scratch("delete-domain");
    let mut store = Store::open(&dir.join("fndr.sqlite3")).unwrap();
    seed_capture_at(
        &mut store,
        "bank-root",
        10,
        Some("https://bank.com/account"),
    );
    seed_capture_at(
        &mut store,
        "bank-subdomain",
        20,
        Some("https://online.bank.com/account"),
    );
    seed_capture_at(&mut store, "burbank", 30, Some("https://burbank.com/"));

    let writer = LanceWriter::new(&dir.join("index"));
    writer
        .flush_once(&mut store, &TestEmbedder::good(), 42)
        .await
        .unwrap();

    let report = delete_everywhere(
        &mut store,
        &writer,
        &DeleteScope::Domain("bank.com".into()),
        &CHUNK_EMBEDDING_V1,
    )
    .await
    .unwrap();
    assert_eq!(report.records, 2);
    assert_eq!(report.indexed_chunks, 2);
    assert_eq!(
        store.record_ids_for_delete(&DeleteScope::All).unwrap(),
        vec!["burbank"]
    );

    let db = lancedb::connect(dir.join("index").to_str().unwrap())
        .execute()
        .await
        .unwrap();
    let table = db.open_table("chunks_v1_qwen768").execute().await.unwrap();
    assert_eq!(table.count_rows(None).await.unwrap(), 1);

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn deletion_scopes_cover_record_time_and_all_without_a_derived_table() {
    let dir = scratch("delete-scopes");
    let mut store = Store::open(&dir.join("fndr.sqlite3")).unwrap();
    seed_capture_at(&mut store, "early", 10, None);
    seed_capture_at(&mut store, "middle", 20, None);
    seed_capture_at(&mut store, "late", 30, None);
    let writer = LanceWriter::new(&dir.join("index"));

    assert_eq!(
        delete_everywhere(
            &mut store,
            &writer,
            &DeleteScope::RecordIds(vec!["early".into(), "missing".into()]),
            &CHUNK_EMBEDDING_V1,
        )
        .await
        .unwrap()
        .records,
        1
    );
    assert_eq!(
        delete_everywhere(
            &mut store,
            &writer,
            &DeleteScope::TimeRange {
                start_ms: 15,
                end_ms: 25,
            },
            &CHUNK_EMBEDDING_V1,
        )
        .await
        .unwrap()
        .records,
        1
    );
    assert_eq!(
        delete_everywhere(&mut store, &writer, &DeleteScope::All, &CHUNK_EMBEDDING_V1,)
            .await
            .unwrap()
            .records,
        1
    );
    assert!(
        store
            .record_ids_for_delete(&DeleteScope::All)
            .unwrap()
            .is_empty()
    );

    let _ = std::fs::remove_dir_all(&dir);
}
