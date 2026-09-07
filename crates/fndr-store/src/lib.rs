//! SQLite schema and migrations, the single Lance writer, batched flush, compaction, rebuild, deletion-everywhere.
//!
//! `Store` is the system of record: schema v1 (T-201) with forward-only
//! embedded migrations. Domain APIs land with their pipeline tickets; the
//! Lance writer and flush are T-202.
//!
//! `SkeletonStore` is the walking-skeleton stand-in (T-109); it dies when the
//! real read/write paths replace it in E02/E03.

mod deletion;
mod lance_writer;
mod migrations;
mod skeleton;
mod store;

pub use deletion::{DeletionError, DeletionReport, delete_everywhere};
pub use lance_writer::{
    FLUSH_BATCH_SIZE, FLUSH_INTERVAL_SECS_MAX, FLUSH_INTERVAL_SECS_MIN, FlushError, FlushReport,
    LanceWriter, RebuildReport,
};
pub use skeleton::{SearchHit, SkeletonStore};
pub use store::{
    ActivityBucket, AppChange, CaptureMetadata, ChangeSummary, ChunkEvidence, ChunkSearchHit,
    DeleteScope, LedgerDecision, NewChunk, NewRecord, PendingChunk, PendingContinuityCandidate,
    RecordEvidence, Store, StoreError, TimelineGranularity,
};
