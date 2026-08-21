//! The single Lance writer (T-202, ADR-002). Nothing else in the codebase
//! writes Lance; the index stays a rebuildable derivative of SQLite truth.
//!
//! Spike-derived rules (docs/spikes/T-208-lance-findings.md):
//! - every commit is a Lance version, so flushes are batched and an empty
//!   batch commits nothing (no version churn);
//! - BTree and FTS indexes are created with the table (cheap); the vector
//!   index is background maintenance (T-203/T-204), never built here;
//! - SQLite is stamped only after the Lance commit succeeds, so a failed
//!   flush leaves truth intact and the next cycle retries.

use std::path::Path;
use std::sync::Arc;

use arrow_array::types::Float32Type;
use arrow_array::{FixedSizeListArray, Int64Array, RecordBatchIterator, StringArray};
use arrow_schema::{DataType, Field, Schema};
use lancedb::index::Index;
use lancedb::index::scalar::{BTreeIndexBuilder, FtsIndexBuilder};
use lancedb::table::Table;

use fndr_inference::{EmbedError, Embedder};

use crate::{Store, StoreError};

/// Default flush cadence bounds (ADR-002: 30 to 60 seconds or batch size).
/// The engine scheduler (pipeline stage 9) owns the timer; these are the
/// agreed constants it reads.
pub const FLUSH_INTERVAL_SECS_MIN: u64 = 30;
pub const FLUSH_INTERVAL_SECS_MAX: u64 = 60;
pub const FLUSH_BATCH_SIZE: usize = 256;

#[derive(Debug, thiserror::Error)]
pub enum FlushError {
    #[error("store: {0}")]
    Store(#[from] StoreError),
    #[error("embedding: {0}")]
    Embed(#[from] EmbedError),
    #[error(
        "embedder returned {got} dims for chunk {chunk_id}, contract requires {expected}; write refused"
    )]
    WrongDimension {
        chunk_id: String,
        got: usize,
        expected: usize,
    },
    #[error("lance: {0}")]
    Lance(#[from] lancedb::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlushReport {
    pub written: usize,
    /// True when the batch was full, meaning more work is likely pending and
    /// the scheduler should run again soon instead of waiting a full cycle.
    pub batch_was_full: bool,
}

pub struct LanceWriter {
    uri: String,
}

impl LanceWriter {
    /// `index_dir` is the app data `index/` directory (ARCHITECTURE section 5).
    pub fn new(index_dir: &Path) -> Self {
        Self {
            uri: index_dir.to_string_lossy().into_owned(),
        }
    }

    fn chunk_schema(dim: usize) -> Arc<Schema> {
        Arc::new(Schema::new(vec![
            Field::new("id", DataType::Utf8, false),
            Field::new("record_id", DataType::Utf8, false),
            Field::new("ord", DataType::Int64, false),
            Field::new("text", DataType::Utf8, false),
            Field::new("source", DataType::Utf8, false),
            Field::new("captured_at_ms", DataType::Int64, false),
            Field::new(
                "vector",
                DataType::FixedSizeList(
                    Arc::new(Field::new("item", DataType::Float32, true)),
                    dim as i32,
                ),
                false,
            ),
        ]))
    }

    async fn open_or_create_table(
        &self,
        table_name: &str,
        dim: usize,
    ) -> Result<Table, FlushError> {
        let db = lancedb::connect(&self.uri).execute().await?;
        match db.open_table(table_name).execute().await {
            Ok(table) => Ok(table),
            Err(lancedb::Error::TableNotFound { .. }) => {
                let table = db
                    .create_empty_table(table_name, Self::chunk_schema(dim))
                    .execute()
                    .await?;
                // Cheap indexes ship with the table (spike: ~16 ms BTree,
                // ~360 ms FTS at 100k rows). The vector index is T-203/T-204.
                table
                    .create_index(
                        &["captured_at_ms"],
                        Index::BTree(BTreeIndexBuilder::default()),
                    )
                    .execute()
                    .await?;
                table
                    .create_index(&["text"], Index::FTS(FtsIndexBuilder::default()))
                    .execute()
                    .await?;
                Ok(table)
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Flush up to `FLUSH_BATCH_SIZE` pending chunks: read from SQLite,
    /// embed, commit one Lance batch, then stamp SQLite. An empty pending
    /// set returns without touching Lance at all.
    pub async fn flush_once(
        &self,
        store: &mut Store,
        embedder: &dyn Embedder,
        now_ms: i64,
    ) -> Result<FlushReport, FlushError> {
        let pending = store.pending_chunks(FLUSH_BATCH_SIZE)?;
        if pending.is_empty() {
            return Ok(FlushReport {
                written: 0,
                batch_was_full: false,
            });
        }

        let spec = embedder.spec();
        let texts: Vec<String> = pending.iter().map(|c| c.text.clone()).collect();
        let vectors = embedder.embed_documents(&texts)?;
        for (chunk, vector) in pending.iter().zip(&vectors) {
            if vector.len() != spec.dim {
                return Err(FlushError::WrongDimension {
                    chunk_id: chunk.chunk_id.clone(),
                    got: vector.len(),
                    expected: spec.dim,
                });
            }
        }

        let table = self
            .open_or_create_table(spec.lance_table, spec.dim)
            .await?;
        let schema = Self::chunk_schema(spec.dim);
        let batch = arrow_array::RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(StringArray::from_iter_values(
                    pending.iter().map(|c| c.chunk_id.as_str()),
                )),
                Arc::new(StringArray::from_iter_values(
                    pending.iter().map(|c| c.record_id.as_str()),
                )),
                Arc::new(Int64Array::from_iter_values(pending.iter().map(|c| c.ord))),
                Arc::new(StringArray::from_iter_values(
                    pending.iter().map(|c| c.text.as_str()),
                )),
                Arc::new(StringArray::from_iter_values(
                    pending.iter().map(|c| c.source.as_str()),
                )),
                Arc::new(Int64Array::from_iter_values(
                    pending.iter().map(|c| c.captured_at_ms),
                )),
                Arc::new(
                    FixedSizeListArray::from_iter_primitive::<Float32Type, _, _>(
                        vectors
                            .iter()
                            .map(|v| Some(v.iter().copied().map(Some).collect::<Vec<_>>())),
                        spec.dim as i32,
                    ),
                ),
            ],
        )
        .expect("batch construction from validated columns");
        let reader: Box<dyn arrow_array::RecordBatchReader + Send> =
            Box::new(RecordBatchIterator::new(vec![Ok(batch)], schema));
        table.add(reader).execute().await?;

        // Only now is the truth stamped; a crash before this line re-flushes
        // the batch (Lance dedup/compaction is maintenance's problem, and the
        // rebuild command converges regardless).
        let ids: Vec<String> = pending.iter().map(|c| c.chunk_id.clone()).collect();
        store.mark_chunks_flushed(&ids, now_ms)?;

        Ok(FlushReport {
            written: pending.len(),
            batch_was_full: pending.len() == FLUSH_BATCH_SIZE,
        })
    }

    /// `fndr index rebuild` (T-205): drop the derived table and re-flush
    /// everything from SQLite truth. The recovery answer for any Lance
    /// corruption, schema change, or crash-window duplicate: the index is
    /// disposable, the truth is not (ADR-002).
    pub async fn rebuild(
        &self,
        store: &mut Store,
        embedder: &dyn Embedder,
        now_ms: i64,
    ) -> Result<RebuildReport, FlushError> {
        let db = lancedb::connect(&self.uri).execute().await?;
        match db.drop_table(embedder.spec().lance_table, &[]).await {
            Ok(()) => {}
            Err(lancedb::Error::TableNotFound { .. }) => {}
            Err(e) => return Err(e.into()),
        }
        store.reset_flush_state()?;

        let mut report = RebuildReport {
            chunks: 0,
            batches: 0,
        };
        loop {
            let flush = self.flush_once(store, embedder, now_ms).await?;
            if flush.written == 0 {
                return Ok(report);
            }
            report.chunks += flush.written;
            report.batches += 1;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RebuildReport {
    pub chunks: usize,
    pub batches: usize,
}
