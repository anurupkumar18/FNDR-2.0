//! Proof that the three seams this session built actually compose: a
//! capture goes through the real safety gate and `Store::insert_capture`
//! (`persist_capture`, T-803 down payment), then the real `LanceWriter`
//! embeds it — via the real model-worker queue (T-403), which lazily
//! loads the real GGUF-backed `Embedder` (T-402) — and lands a real
//! vector row in Lance. No mock anywhere on the path, and no direct
//! `GgufEmbedder` call from this binary's own logic (only from inside the
//! queue's loader closure, which is the sanctioned construction site per
//! `scripts/check-llm-call-sites.sh`).
//!
//! Requires the pinned model downloaded first:
//! `cargo run -p fndr-downloader --example fetch_model`
//! Then: `cargo run -p fndr-memory --example end_to_end_flush`

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use fndr_inference::{CHUNK_EMBEDDING_V1, GgufEmbedder, ModelWorkerHandle, Priority};
use fndr_memory::{CaptureForPersistence, PersistCaptureOutcome, persist_capture};
use fndr_privacy::Blocklist;
use fndr_store::{LanceWriter, Store};

#[tokio::main]
async fn main() {
    let model_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../models/Qwen3-Embedding-0.6B-Q8_0.gguf");
    if !model_path.is_file() {
        eprintln!(
            "model not found at {}; run: cargo run -p fndr-downloader --example fetch_model",
            model_path.display()
        );
        std::process::exit(1);
    }

    let dir = std::env::temp_dir().join(format!("fndr-e2e-flush-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let mut store = Store::open(&dir.join("fndr.sqlite3")).expect("open store");

    let capture = CaptureForPersistence {
        record_id: "e2e-record-1",
        session_id: "e2e-session-1",
        chunk_id: "e2e-chunk-1",
        source: "screen",
        app_name: "VS Code",
        bundle_id: Some("com.microsoft.VSCode"),
        url: None,
        window_title: "fndr end-to-end proof",
        ocr_text: "Deciding to use RRF fusion for hybrid search, revisit after the bench lands.",
        captured_at_ms: 1_755_000_000_000,
        created_at_ms: 1_755_000_000_000,
    };

    match persist_capture(&mut store, capture, &Blocklist::default()).expect("persist_capture") {
        PersistCaptureOutcome::Stored {
            record_id,
            redaction_count,
        } => println!("stored record {record_id} (redactions: {redaction_count})"),
        PersistCaptureOutcome::Merged {
            record_id,
            redaction_count,
        } => println!("merged into record {record_id} (redactions: {redaction_count})"),
        PersistCaptureOutcome::Skipped { reason } => {
            panic!("unexpected skip for a normal capture: {reason:?}")
        }
    }

    println!(
        "starting the model-worker queue (real model loads lazily on first job): {}",
        model_path.display()
    );
    let worker_model_path: PathBuf = model_path.clone();
    let worker = Arc::new(ModelWorkerHandle::spawn(
        move || {
            GgufEmbedder::load(&worker_model_path, CHUNK_EMBEDDING_V1)
                .map(|e| Box::new(e) as Box<dyn fndr_inference::Embedder>)
        },
        Duration::from_secs(60),
    ));
    let queued =
        fndr_inference::QueuedEmbedder::new(worker, Priority::Backfill, CHUNK_EMBEDDING_V1);

    let writer = LanceWriter::new(&dir.join("index"));
    let report = writer
        .flush_once(&mut store, &queued, 1_755_000_001_000)
        .await
        .expect("flush through the queue");
    println!(
        "flushed {} chunk(s) via the model-worker queue",
        report.written
    );
    assert_eq!(report.written, 1, "the one seeded chunk should flush");
    assert!(
        store.pending_chunks(10).unwrap().is_empty(),
        "the flushed chunk must be marked flushed, not left pending"
    );

    println!(
        "end-to-end proof complete: real capture -> real safety gate -> real SQLite -> \
         model-worker queue -> real GGUF embedder -> real Lance vector (no mock on the path)"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
