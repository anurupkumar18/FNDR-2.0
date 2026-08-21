//! T-205: crash-recovery and rebuild convergence. The vulnerable window in
//! the flush protocol is after the Lance commit and before the SQLite stamp;
//! a crash there re-flushes the batch and duplicates rows in the derived
//! index. The rebuild command must converge back to exactly SQLite truth.

use std::path::PathBuf;

use fndr_inference::{CHUNK_EMBEDDING_V1, EmbedError, Embedder, EmbeddingSpec};
use fndr_store::{LanceWriter, NewChunk, NewRecord, Store};

struct TestEmbedder;

impl Embedder for TestEmbedder {
    fn spec(&self) -> &EmbeddingSpec {
        &CHUNK_EMBEDDING_V1
    }
    fn embed_documents(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        Ok(texts
            .iter()
            .map(|t| {
                let seed = t.len() as f32 + 1.0;
                (0..self.spec().dim)
                    .map(|i| (seed + i as f32).sin())
                    .collect()
            })
            .collect())
    }
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fndr-t205-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

async fn lance_ids(index_dir: &std::path::Path) -> Vec<String> {
    use futures::TryStreamExt;
    use lancedb::query::ExecutableQuery;
    let db = lancedb::connect(index_dir.to_str().unwrap())
        .execute()
        .await
        .unwrap();
    let table = db.open_table("chunks_v1_qwen768").execute().await.unwrap();
    let batches: Vec<arrow_array::RecordBatch> = table
        .query()
        .execute()
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    let mut ids = Vec::new();
    for batch in &batches {
        let col = batch
            .column_by_name("id")
            .unwrap()
            .as_any()
            .downcast_ref::<arrow_array::StringArray>()
            .unwrap();
        ids.extend(col.iter().flatten().map(String::from));
    }
    ids.sort();
    ids
}

#[tokio::test]
async fn crash_window_duplicates_then_rebuild_converges_to_truth() {
    let dir = scratch("crash");
    let mut store = Store::open(&dir.join("fndr.sqlite3")).unwrap();
    let record = NewRecord {
        id: "r1".into(),
        session_id: "s1".into(),
        source: "screen".into(),
        app_name: "Terminal".into(),
        window_title: "fndr".into(),
        captured_at_ms: 1_755_000_000_000,
        created_at_ms: 1_755_000_000_000,
    };
    let chunks: Vec<NewChunk> = (0..5)
        .map(|ord| NewChunk {
            id: format!("c{ord}"),
            ord,
            text: format!("chunk {ord} about the rebuild convergence test"),
        })
        .collect();
    store.insert_capture(&record, &chunks).unwrap();

    let writer = LanceWriter::new(&dir.join("index"));
    let embedder = TestEmbedder;
    writer.flush_once(&mut store, &embedder, 1).await.unwrap();

    // Simulate the crash window: Lance committed, SQLite stamps lost.
    store.reset_flush_state().unwrap();
    writer.flush_once(&mut store, &embedder, 2).await.unwrap();
    let ids = lance_ids(&dir.join("index")).await;
    assert_eq!(ids.len(), 10, "crash window produced duplicates (expected)");

    // Rebuild converges back to exactly SQLite truth.
    let report = writer.rebuild(&mut store, &embedder, 3).await.unwrap();
    assert_eq!(report.chunks, 5);
    assert_eq!(report.batches, 1);
    let ids = lance_ids(&dir.join("index")).await;
    assert_eq!(
        ids,
        vec!["c0", "c1", "c2", "c3", "c4"],
        "exact truth, no dupes"
    );
    assert!(store.pending_chunks(10).unwrap().is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn rebuild_on_missing_table_is_a_fresh_build() {
    let dir = scratch("fresh");
    let mut store = Store::open(&dir.join("fndr.sqlite3")).unwrap();
    let record = NewRecord {
        id: "r1".into(),
        session_id: "s1".into(),
        source: "screen".into(),
        app_name: "Terminal".into(),
        window_title: "fndr".into(),
        captured_at_ms: 1_755_000_000_000,
        created_at_ms: 1_755_000_000_000,
    };
    store
        .insert_capture(
            &record,
            &[NewChunk {
                id: "c0".into(),
                ord: 0,
                text: "solo chunk".into(),
            }],
        )
        .unwrap();

    let writer = LanceWriter::new(&dir.join("index"));
    let report = writer.rebuild(&mut store, &TestEmbedder, 1).await.unwrap();
    assert_eq!(report.chunks, 1);
    assert_eq!(lance_ids(&dir.join("index")).await, vec!["c0"]);

    let _ = std::fs::remove_dir_all(&dir);
}
