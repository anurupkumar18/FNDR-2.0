//! SQLite schema and migrations, the single Lance writer, batched flush, compaction, rebuild, deletion-everywhere.
//!
//! `Store` is the system of record: schema v1 (T-201) with forward-only
//! embedded migrations. Domain APIs land with their pipeline tickets; the
//! Lance writer and flush are T-202.
//!
//! `SkeletonStore` is the walking-skeleton stand-in (T-109); it dies when the
//! real read/write paths replace it in E02/E03.
//!
//! `backup`/`export` are the owner-facing portability surface (T-209): a
//! restorable snapshot of truth plus config, and a readable JSONL dump. The
//! `fndr-vault` binary in `src/bin/` is their CLI.

mod backup;
mod deletion;
mod export;
mod lance_writer;
mod migrations;
mod skeleton;
mod store;

pub use backup::{
    BACKUP_CONFIG_DIR, BACKUP_FORMAT, BACKUP_FORMAT_VERSION, BACKUP_MANIFEST_FILE, BackupError,
    BackupReport, BackupSkipReason, RestorePolicy, RestoreReport, SUPERSEDED_INDEX_PREFIX,
    SkippedEntry, VAULT_DATABASE_FILE, VaultLayout, backup_vault, restore_vault,
};
pub use deletion::{DeletionError, DeletionReport, delete_everywhere};
pub use export::{
    EXPORT_DECISIONS_FILE, EXPORT_FORMAT, EXPORT_FORMAT_VERSION, EXPORT_MANIFEST_FILE,
    EXPORT_README_FILE, EXPORT_RECORDS_FILE, ExportError, ExportReport, export_vault,
};
pub use lance_writer::{
    FLUSH_BATCH_SIZE, FLUSH_INTERVAL_SECS_MAX, FLUSH_INTERVAL_SECS_MIN, FlushError, FlushReport,
    LanceWriter, RebuildReport,
};
pub use skeleton::{SearchHit, SkeletonStore};
pub use store::{
    ActivityBucket, AppChange, AuditEntry, CaptureMetadata, ChangeSummary, ChunkEvidence,
    ChunkSearchHit, DeleteScope, LedgerDecision, NewChunk, NewRecord, PendingChunk,
    PendingContinuityCandidate, RecordEvidence, ResultFeedback, SEARCH_LIMIT_CAP,
    SearchExplanation, Store, StoreError, TimelineGranularity,
};
