//! Proof that the two seams this session built actually compose: a capture
//! goes through the real safety gate and `Store::insert_capture`
//! (`persist_capture`, T-803 down payment), then the real `LanceWriter`
//! embeds it with the real GGUF-backed `Embedder` (T-402) and lands a real
//! vector row in Lance — no mock anywhere on the path.
//!
//! This is dev/demo tooling, not a production scheduler: the real call
//! site for repeated flushes is the model-worker priority queue (T-403),
//! not this one-shot binary.
//!
//! Requires the pinned model downloaded first:
//! `cargo run -p fndr-downloader --example fetch_model`
//! Then: `cargo run -p fndr-memory --example end_to_end_flush`

use std::path::Path;

use fndr_inference::{CHUNK_EMBEDDING_V1, GgufEmbedder};
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
        PersistCaptureOutcome::Skipped { reason } => {
            panic!("unexpected skip for a normal capture: {reason:?}")
        }
    }

    println!("loading the real embedder from {}", model_path.display());
    let embedder = GgufEmbedder::load(&model_path, CHUNK_EMBEDDING_V1).expect("load embedder");

    let writer = LanceWriter::new(&dir.join("index"));
    let report = writer
        .flush_once(&mut store, &embedder, 1_755_000_001_000)
        .await
        .expect("flush with the real embedder");
    println!("flushed {} chunk(s) with the real embedder", report.written);
    assert_eq!(report.written, 1, "the one seeded chunk should flush");
    assert!(
        store.pending_chunks(10).unwrap().is_empty(),
        "the flushed chunk must be marked flushed, not left pending"
    );

    println!(
        "end-to-end proof complete: real capture -> real safety gate -> real SQLite -> \
         real Lance vector, via the real GGUF embedder (no mock on the path)"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
