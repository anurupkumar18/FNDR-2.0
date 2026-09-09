//! T-209's acceptance criterion: a backup taken while capture is actively
//! writing restores to a working vault.
//!
//! An idle-database test would prove nothing here. The failure this ticket
//! exists to prevent is the torn copy: a live WAL database copied file by file
//! while a writer commits, producing a database that opens and then lies. So
//! the headline test keeps a second thread committing captures for the whole
//! duration of the backup, and then checks the restored vault for internal
//! consistency, not merely for opening.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::time::{Duration, Instant};

use fndr_store::{
    NewChunk, NewRecord, RestorePolicy, Store, VaultLayout, backup_vault, export_vault,
    restore_vault,
};

/// Every record is written with this many chunks in one transaction. A torn
/// snapshot shows up as a record whose chunks did not all arrive.
const CHUNKS_PER_RECORD: i64 = 3;
/// Bounds the writer thread so a broken test cannot spin forever.
const MAX_RECORDS: i64 = 5_000;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fndr-t209-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn capture(index: i64) -> (NewRecord, Vec<NewChunk>) {
    (
        NewRecord {
            id: format!("r{index}"),
            session_id: "live-session".into(),
            source: "screen".into(),
            app_name: "Safari".into(),
            bundle_id: Some("com.apple.Safari".into()),
            url: None,
            window_title: format!("window {index}"),
            captured_at_ms: 1_000 + index,
            created_at_ms: 1_000 + index,
        },
        (0..CHUNKS_PER_RECORD)
            .map(|ord| NewChunk {
                id: format!("c{index}-{ord}"),
                ord,
                text: format!("captured moment {index} segment {ord} recoverable"),
            })
            .collect(),
    )
}

/// Spin until `condition` holds, failing loudly rather than hanging.
fn wait_until(what: &str, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Assert the vault at `database` is internally consistent for a prefix of the
/// writer's stream: exactly `records` records, ids `r0..r{records-1}` with no
/// gaps, every one carrying all of its chunks, and the FTS index agreeing with
/// the chunk table.
fn assert_consistent_prefix(store: &Store, records: i64, chunks: i64) {
    assert_eq!(
        chunks,
        records * CHUNKS_PER_RECORD,
        "chunk count must be exactly {CHUNKS_PER_RECORD} per record"
    );
    for index in 0..records {
        let evidence = store
            .record_evidence(&format!("r{index}"))
            .unwrap()
            .unwrap_or_else(|| panic!("record r{index} is missing from a {records}-record prefix"));
        assert_eq!(
            evidence.chunks.len() as i64,
            CHUNKS_PER_RECORD,
            "record r{index} lost part of its transaction"
        );
    }
    assert!(
        store.record_evidence(&format!("r{records}")).unwrap().is_none(),
        "a consistent snapshot cannot contain a record past its own count"
    );
    let explanation = store.explain_chunk_search("recoverable").unwrap();
    assert_eq!(
        explanation.total_matches, chunks,
        "the FTS index must cover exactly the chunks the snapshot carries"
    );
    assert!(!store.search_chunks("recoverable", 10).unwrap().is_empty());
}

#[test]
fn backup_under_active_capture_restores_to_a_working_vault() {
    let root = scratch("live-capture");
    let data_dir = root.join("vault");
    fs::create_dir_all(&data_dir).unwrap();
    let layout = VaultLayout::default();
    let database = layout.database_path(&data_dir);
    drop(Store::open(&database).unwrap());

    let stop = Arc::new(AtomicBool::new(false));
    let written = Arc::new(AtomicI64::new(0));
    let writer = {
        let database = database.clone();
        let stop = Arc::clone(&stop);
        let written = Arc::clone(&written);
        std::thread::spawn(move || {
            let mut store = Store::open(&database).expect("the writer opens the live vault");
            let mut index = 0;
            while !stop.load(Ordering::Relaxed) && index < MAX_RECORDS {
                let (record, chunks) = capture(index);
                store
                    .insert_capture(&record, &chunks)
                    .expect("a live capture write must not fail during a backup");
                index += 1;
                written.store(index, Ordering::SeqCst);
            }
        })
    };

    // Only take the backup once capture is demonstrably in flight.
    wait_until("capture to start writing", || {
        written.load(Ordering::SeqCst) >= 50
    });
    let committed_before = written.load(Ordering::SeqCst);

    let backup_dir = root.join("backup");
    let report = backup_vault(&data_dir, &backup_dir, &layout).unwrap();

    // Keep writing after the snapshot boundary, so the source vault provably
    // moved on and the snapshot is a point in time rather than the end state.
    wait_until("capture to continue past the snapshot", || {
        written.load(Ordering::SeqCst) >= report.records + 20
    });
    stop.store(true, Ordering::SeqCst);
    writer.join().unwrap();
    let total = written.load(Ordering::SeqCst);

    assert!(
        report.records >= committed_before,
        "the snapshot dropped writes that were already committed: {} < {committed_before}",
        report.records
    );
    assert!(
        report.records < total,
        "the writer never overlapped the backup: snapshot {} vs final {total}",
        report.records
    );

    // Restore into a fresh data directory and open it the way the app does.
    let restored_dir = root.join("restored");
    let restore = restore_vault(
        &backup_dir,
        &restored_dir,
        RestorePolicy::RefuseIfVaultExists,
        &layout,
    )
    .unwrap();
    assert_eq!(restore.records, report.records);
    assert_eq!(restore.chunks, report.chunks);
    assert!(!restore.replaced_vault);
    assert!(restore.index_rebuild_required);

    let mut restored = Store::open(&layout.database_path(&restored_dir)).unwrap();
    assert_eq!(
        restored.schema_version().unwrap(),
        Store::open_in_memory().unwrap().schema_version().unwrap(),
        "a restored vault is at the schema this build speaks"
    );
    assert_consistent_prefix(&restored, report.records, report.chunks);

    // A working vault is one that still takes writes.
    let (record, chunks) = capture(MAX_RECORDS + 1);
    restored.insert_capture(&record, &chunks).unwrap();
    assert!(
        !restored
            .search_chunks(&format!("moment {}", MAX_RECORDS + 1), 5)
            .unwrap()
            .is_empty()
    );

    // The source vault is untouched by its own backup, and still growing-safe.
    let source = Store::open(&database).unwrap();
    assert_consistent_prefix(&source, total, total * CHUNKS_PER_RECORD);

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_snapshot_carries_committed_wal_content_that_was_never_checkpointed() {
    let root = scratch("wal-content");
    let data_dir = root.join("vault");
    fs::create_dir_all(&data_dir).unwrap();
    let layout = VaultLayout::default();
    let database = layout.database_path(&data_dir);

    let mut store = Store::open(&database).unwrap();
    for index in 0..200 {
        let (record, chunks) = capture(index);
        store.insert_capture(&record, &chunks).unwrap();
    }
    // Deliberately no checkpoint and no close: the newest commits live in the
    // WAL, which is exactly the content a naive file copy loses.
    assert!(
        wal_bytes(&data_dir, &layout) > 0,
        "the test needs an unwritten-back WAL to be meaningful"
    );

    let backup_dir = root.join("backup");
    let report = backup_vault(&data_dir, &backup_dir, &layout).unwrap();
    assert_eq!(report.records, 200);
    assert_eq!(report.chunks, 200 * CHUNKS_PER_RECORD);
    // The snapshot is one self-contained file: no WAL travels with it.
    assert!(!backup_dir.join("vault.sqlite3-wal").exists());
    assert!(!backup_dir.join("vault.sqlite3-shm").exists());

    let restored_dir = root.join("restored");
    restore_vault(
        &backup_dir,
        &restored_dir,
        RestorePolicy::RefuseIfVaultExists,
        &layout,
    )
    .unwrap();
    let restored = Store::open(&layout.database_path(&restored_dir)).unwrap();
    assert_consistent_prefix(&restored, 200, 200 * CHUNKS_PER_RECORD);

    drop(store);
    let _ = fs::remove_dir_all(&root);
}

fn wal_bytes(data_dir: &Path, layout: &VaultLayout) -> u64 {
    let wal = data_dir.join(format!("{}-wal", layout.database));
    fs::metadata(wal).map(|meta| meta.len()).unwrap_or_default()
}

#[test]
fn export_runs_against_a_vault_that_is_being_written() {
    let root = scratch("live-export");
    let data_dir = root.join("vault");
    fs::create_dir_all(&data_dir).unwrap();
    let layout = VaultLayout::default();
    let database = layout.database_path(&data_dir);
    drop(Store::open(&database).unwrap());

    let stop = Arc::new(AtomicBool::new(false));
    let written = Arc::new(AtomicI64::new(0));
    let writer = {
        let database = database.clone();
        let stop = Arc::clone(&stop);
        let written = Arc::clone(&written);
        std::thread::spawn(move || {
            let mut store = Store::open(&database).expect("the writer opens the live vault");
            let mut index = 0;
            while !stop.load(Ordering::Relaxed) && index < MAX_RECORDS {
                let (record, chunks) = capture(index);
                store.insert_capture(&record, &chunks).expect("live capture");
                index += 1;
                written.store(index, Ordering::SeqCst);
            }
        })
    };
    wait_until("capture to start writing", || {
        written.load(Ordering::SeqCst) >= 50
    });

    let destination = root.join("export");
    let report = export_vault(&data_dir, &destination, &layout).unwrap();

    stop.store(true, Ordering::SeqCst);
    writer.join().unwrap();

    // Every line is complete JSON and every record carries its whole
    // transaction: a mid-write export must never emit a half-written moment.
    let lines: Vec<serde_json::Value> = fs::read_to_string(destination.join("records.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).expect("every exported line is valid JSON"))
        .collect();
    assert_eq!(lines.len() as i64, report.records);
    assert!(report.records >= 50);
    for (index, line) in lines.iter().enumerate() {
        assert_eq!(line["record_id"], format!("r{index}"));
        assert_eq!(
            line["chunks"].as_array().map(Vec::len),
            Some(CHUNKS_PER_RECORD as usize)
        );
    }

    let _ = fs::remove_dir_all(&root);
}
