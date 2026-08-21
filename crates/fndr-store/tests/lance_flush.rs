//! T-202 write-path tests: flush success, retry-after-failure, empty-batch
//! no-op (no Lance version churn), and the wrong-dimension refusal. Uses a
//! deterministic test embedder; per the invariants, test code is the only
//! place a non-real embedder may exist.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use fndr_inference::{CHUNK_EMBEDDING_V1, EmbedError, Embedder, EmbeddingSpec};
use fndr_store::{LanceWriter, NewChunk, NewRecord, Store};

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
