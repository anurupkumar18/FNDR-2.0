//! T-307: the Lance-safe indexed-record merge/update path.
//!
//! The happy path is the least interesting case here. These tests exercise the
//! crash windows and the merge/flush race described in
//! `fndr_store::indexed_merge`, and assert convergence: after one further
//! flush cycle, Lance holds exactly one row per chunk and that row carries the
//! text SQLite calls truth.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use fndr_inference::{CHUNK_EMBEDDING_V1, EmbedError, Embedder, EmbeddingSpec};
use fndr_store::{CaptureMergeOutcome, LanceWriter, NewChunk, NewRecord, Store};
use fndr_types::ChunkIndexState;

const TABLE: &str = "chunks_v1_qwen768";

struct TestEmbedder;

impl Embedder for TestEmbedder {
    fn spec(&self) -> &EmbeddingSpec {
        &CHUNK_EMBEDDING_V1
    }
    fn embed_documents(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        Ok(texts.iter().map(|t| seeded_vector(t)).collect())
    }
}

/// An embedder that performs a real concurrent merge, through a second SQLite
/// connection, at exactly the moment the flush has read its batch and not yet
/// stamped it. That is the merge-racing-a-flush window, made deterministic.
struct MergingRacerEmbedder {
    db_path: PathBuf,
    fired: AtomicBool,
    merged_text: String,
}

impl Embedder for MergingRacerEmbedder {
    fn spec(&self) -> &EmbeddingSpec {
        &CHUNK_EMBEDDING_V1
    }
    fn embed_documents(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        if !self.fired.swap(true, Ordering::SeqCst) {
            let mut other = Store::open(&self.db_path).unwrap();
            let candidate = other.continuity_candidates(0, 8).unwrap().remove(0);
            let incoming = record_at("ignored", candidate.captured_at_ms + 1_000);
            let outcome = other
                .merge_capture(&candidate, &incoming, &self.merged_text)
                .unwrap();
            assert_ne!(outcome, CaptureMergeOutcome::CandidateChanged);
        }
        Ok(texts.iter().map(|t| seeded_vector(t)).collect())
    }
}

fn seeded_vector(text: &str) -> Vec<f32> {
    let seed = text.len() as f32 + 1.0;
    (0..CHUNK_EMBEDDING_V1.dim)
        .map(|i| (seed + i as f32).sin())
        .collect()
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fndr-t307-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn record_at(id: &str, captured_at_ms: i64) -> NewRecord {
    NewRecord {
        id: id.into(),
        session_id: "s1".into(),
        source: "screen".into(),
        app_name: "Terminal".into(),
        bundle_id: None,
        url: None,
        window_title: "fndr".into(),
        captured_at_ms,
        created_at_ms: captured_at_ms,
    }
}

fn seed(store: &mut Store, id: &str, text: &str) {
    store
        .insert_capture(
            &record_at(id, 1_755_000_000_000),
            &[NewChunk {
                id: format!("{id}-c0"),
                ord: 0,
                text: text.into(),
            }],
        )
        .unwrap();
}

/// Every row Lance holds, as (chunk id, text). Duplicates are visible because
/// nothing here deduplicates.
async fn lance_rows(index_dir: &Path) -> Vec<(String, String)> {
    use futures::TryStreamExt;
    use lancedb::query::ExecutableQuery;
    let db = lancedb::connect(index_dir.to_str().unwrap())
        .execute()
        .await
        .unwrap();
    let table = db.open_table(TABLE).execute().await.unwrap();
    let batches: Vec<arrow_array::RecordBatch> = table
        .query()
        .execute()
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    let mut rows = Vec::new();
    for batch in &batches {
        let ids = batch
            .column_by_name("id")
            .unwrap()
            .as_any()
            .downcast_ref::<arrow_array::StringArray>()
            .unwrap();
        let texts = batch
            .column_by_name("text")
            .unwrap()
            .as_any()
            .downcast_ref::<arrow_array::StringArray>()
            .unwrap();
        for i in 0..batch.num_rows() {
            rows.push((ids.value(i).to_string(), texts.value(i).to_string()));
        }
    }
    rows.sort();
    rows
}

fn state_of(store: &Store, chunk_id: &str) -> ChunkIndexState {
    store.chunk_index_state(chunk_id).unwrap().unwrap().0
}

#[tokio::test]
async fn merging_an_indexed_record_replaces_its_row_with_no_stale_and_no_duplicate() {
    let dir = scratch("replace");
    let index = dir.join("index");
    let mut store = Store::open(&dir.join("fndr.sqlite3")).unwrap();
    seed(&mut store, "r1", "alpha evidence");

    let writer = LanceWriter::new(&index);
    writer
        .flush_once(&mut store, &TestEmbedder, 1)
        .await
        .unwrap();
    assert_eq!(state_of(&store, "r1-c0"), ChunkIndexState::Indexed);
    assert_eq!(
        lance_rows(&index).await,
        vec![("r1-c0".into(), "alpha evidence".into())]
    );

    // The merge is SQLite-only and complete at its commit; the index owes a
    // repair and says so, rather than the merge being refused.
    let candidate = store.continuity_candidates(0, 8).unwrap().remove(0);
    assert_eq!(candidate.index_state, ChunkIndexState::Indexed);
    let outcome = store
        .merge_capture(
            &candidate,
            &record_at("r2", 1_755_000_060_000),
            "alpha evidence and beta evidence",
        )
        .unwrap();
    assert_eq!(outcome, CaptureMergeOutcome::MergedIndexRepairPending);
    assert!(outcome.needs_index_repair());
    assert_eq!(state_of(&store, "r1-c0"), ChunkIndexState::Superseded);
    assert_eq!(
        lance_rows(&index).await,
        vec![("r1-c0".into(), "alpha evidence".into())],
        "the stale row is still there, and truth knows it"
    );

    let report = writer
        .flush_once(&mut store, &TestEmbedder, 2)
        .await
        .unwrap();
    assert_eq!(report.written, 1);
    assert_eq!(report.stale_rows_removed, 1);
    assert_eq!(report.raced_by_merge, 0);
    assert_eq!(
        lance_rows(&index).await,
        vec![("r1-c0".into(), "alpha evidence and beta evidence".into())],
        "exactly one row, carrying truth"
    );
    assert_eq!(state_of(&store, "r1-c0"), ChunkIndexState::Indexed);
    assert!(store.pending_chunks(10).unwrap().is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}

/// Crash window: the Lance add committed and the SQLite stamp never ran.
/// `mark_chunks_indexing` is exactly the durable state the flush leaves behind
/// before its add, so replaying it reproduces the lost-stamp state faithfully.
#[tokio::test]
async fn a_crash_between_the_index_write_and_the_sqlite_stamp_converges() {
    let dir = scratch("lost-stamp");
    let index = dir.join("index");
    let mut store = Store::open(&dir.join("fndr.sqlite3")).unwrap();
    seed(&mut store, "r1", "alpha evidence");

    let writer = LanceWriter::new(&index);
    writer
        .flush_once(&mut store, &TestEmbedder, 1)
        .await
        .unwrap();

    // The row is in Lance; pretend the process died before step 4.
    store.mark_chunks_indexing(&["r1-c0".to_string()]).unwrap();
    assert_eq!(state_of(&store, "r1-c0"), ChunkIndexState::Superseded);

    let report = writer
        .flush_once(&mut store, &TestEmbedder, 2)
        .await
        .unwrap();
    assert_eq!(report.written, 1);
    assert_eq!(
        report.stale_rows_removed, 1,
        "the row written before the crash is removed, not duplicated"
    );
    assert_eq!(
        lance_rows(&index).await,
        vec![("r1-c0".into(), "alpha evidence".into())],
        "one row, not two"
    );
    assert_eq!(state_of(&store, "r1-c0"), ChunkIndexState::Indexed);

    let _ = std::fs::remove_dir_all(&dir);
}

/// Crash window: the stale-row delete succeeded and the process died before
/// anything else. Truth is untouched, the chunk is still owed a row, and the
/// next cycle supplies it.
#[tokio::test]
async fn a_crash_after_the_stale_delete_leaves_a_visible_gap_that_the_next_cycle_fills() {
    let dir = scratch("lost-add");
    let index = dir.join("index");
    let mut store = Store::open(&dir.join("fndr.sqlite3")).unwrap();
    seed(&mut store, "r1", "alpha evidence");

    let writer = LanceWriter::new(&index);
    writer
        .flush_once(&mut store, &TestEmbedder, 1)
        .await
        .unwrap();
    let candidate = store.continuity_candidates(0, 8).unwrap().remove(0);
    store
        .merge_capture(
            &candidate,
            &record_at("r2", 1_755_000_060_000),
            "alpha evidence and beta evidence",
        )
        .unwrap();

    // A flush whose embedder fails: the batch is abandoned after nothing has
    // been written, so the chunk stays Superseded and is retried.
    struct Outage;
    impl Embedder for Outage {
        fn spec(&self) -> &EmbeddingSpec {
            &CHUNK_EMBEDDING_V1
        }
        fn embed_documents(&self, _: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
            Err(EmbedError::Unavailable("test outage".into()))
        }
    }
    assert!(writer.flush_once(&mut store, &Outage, 2).await.is_err());
    assert_eq!(
        state_of(&store, "r1-c0"),
        ChunkIndexState::Superseded,
        "a failed flush is not a clean index"
    );
    assert_eq!(store.pending_chunks(10).unwrap().len(), 1);

    let report = writer
        .flush_once(&mut store, &TestEmbedder, 3)
        .await
        .unwrap();
    assert_eq!(report.stale_rows_removed, 1);
    assert_eq!(
        lance_rows(&index).await,
        vec![("r1-c0".into(), "alpha evidence and beta evidence".into())]
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// A merge that commits between the flush's read and its stamp must not be
/// stamped away. The flush reports the race and the next cycle converges.
#[tokio::test]
async fn a_merge_racing_a_flush_is_reported_and_converges_on_the_next_cycle() {
    let dir = scratch("race");
    let index = dir.join("index");
    let db_path = dir.join("fndr.sqlite3");
    let mut store = Store::open(&db_path).unwrap();
    seed(&mut store, "r1", "alpha evidence");

    let writer = LanceWriter::new(&index);
    let racer = MergingRacerEmbedder {
        db_path: db_path.clone(),
        fired: AtomicBool::new(false),
        merged_text: "alpha evidence and beta evidence".into(),
    };
    let report = writer.flush_once(&mut store, &racer, 1).await.unwrap();
    assert_eq!(report.written, 1);
    assert_eq!(
        report.raced_by_merge, 1,
        "the flush saw its own batch move under it and said so"
    );
    assert_eq!(
        state_of(&store, "r1-c0"),
        ChunkIndexState::Superseded,
        "never falsely Indexed"
    );
    assert_eq!(
        lance_rows(&index).await,
        vec![("r1-c0".into(), "alpha evidence".into())],
        "the row written mid-race carries the pre-merge text"
    );

    let report = writer
        .flush_once(&mut store, &TestEmbedder, 2)
        .await
        .unwrap();
    assert_eq!(report.stale_rows_removed, 1);
    assert_eq!(report.raced_by_merge, 0);
    assert_eq!(
        lance_rows(&index).await,
        vec![("r1-c0".into(), "alpha evidence and beta evidence".into())],
        "one row, carrying truth"
    );
    assert_eq!(state_of(&store, "r1-c0"), ChunkIndexState::Indexed);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_second_merge_against_a_stale_candidate_is_refused_not_silently_applied() {
    let mut store = Store::open_in_memory().unwrap();
    seed(&mut store, "r1", "alpha evidence");
    let candidate = store.continuity_candidates(0, 8).unwrap().remove(0);

    assert_eq!(
        store
            .merge_capture(&candidate, &record_at("r2", 2), "alpha and beta")
            .unwrap(),
        CaptureMergeOutcome::Merged,
        "an unflushed chunk still merges with no index work owed"
    );
    assert_eq!(state_of(&store, "r1-c0"), ChunkIndexState::Pending);

    // The same candidate, read before that write, must not overwrite it.
    assert_eq!(
        store
            .merge_capture(&candidate, &record_at("r3", 3), "alpha and gamma")
            .unwrap(),
        CaptureMergeOutcome::CandidateChanged
    );
    let (state, revision) = store.chunk_index_state("r1-c0").unwrap().unwrap();
    assert_eq!(state, ChunkIndexState::Pending);
    assert_eq!(revision, 2, "exactly one text change was applied");
    assert_eq!(store.pending_chunks(10).unwrap()[0].text, "alpha and beta");
}
