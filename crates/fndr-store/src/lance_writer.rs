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
use fndr_types::ChunkIndexState;

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
    /// Chunks in this batch whose stale Lance row was deleted before the new
    /// one was added (T-307 index repair). Zero on an ordinary flush.
    pub stale_rows_removed: usize,
    /// Chunks that were merged while this flush was in flight. They stay
    /// `Superseded` and the next cycle replaces the row just written. This is
    /// reported rather than swallowed: a persistently non-zero count means
    /// merges are outrunning the flush cadence.
    pub raced_by_merge: usize,
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

    /// Flush up to `FLUSH_BATCH_SIZE` chunks the index owes work for: read
    /// from SQLite, embed, delete any stale rows, commit one Lance batch, then
    /// stamp SQLite. An empty queue returns without touching Lance at all.
    ///
    /// This is steps 1 to 4 of the T-307 protocol; the ordering rationale and
    /// the crash-window analysis for every step live in
    /// [`crate::indexed_merge`] and are not repeated here.
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
                stale_rows_removed: 0,
                raced_by_merge: 0,
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

        // Step 1: drop any stale row before its replacement exists, so the
        // only legal intermediate state is "row missing", never "two rows".
        let stale: Vec<String> = pending
            .iter()
            .filter(|c| c.index_state == ChunkIndexState::Superseded)
            .map(|c| c.chunk_id.clone())
            .collect();
        let stale_rows_removed = self.delete_chunk_rows(&table, &stale).await?;

        // Step 2: from here on a Lance row for these chunks may exist. This
        // durable statement is what makes a crash during the add converge
        // instead of leaving an undetectable duplicate.
        let batch_ids: Vec<String> = pending.iter().map(|c| c.chunk_id.clone()).collect();
        store.mark_chunks_indexing(&batch_ids)?;

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

        // Step 4: only now is the truth stamped, and only for the revision
        // that was actually embedded. A crash before this line leaves every
        // chunk `Superseded`, so the next cycle deletes the rows just written
        // and re-adds them rather than duplicating them.
        let stamps: Vec<(String, i64)> = pending
            .iter()
            .map(|c| (c.chunk_id.clone(), c.revision))
            .collect();
        let stamp = store.mark_chunks_indexed(&stamps, now_ms)?;

        Ok(FlushReport {
            written: pending.len(),
            batch_was_full: pending.len() == FLUSH_BATCH_SIZE,
            stale_rows_removed,
            raced_by_merge: stamp.superseded,
        })
    }

    /// Delete the derived rows for specific chunk ids. Unlike
    /// [`Self::delete_records`] this is index repair, not owner deletion:
    /// SQLite truth keeps the chunk and the next add re-derives its row.
    async fn delete_chunk_rows(
        &self,
        table: &Table,
        chunk_ids: &[String],
    ) -> Result<usize, FlushError> {
        if chunk_ids.is_empty() {
            return Ok(0);
        }
        let predicate = format!("id IN ({})", sql_string_list(chunk_ids));
        Ok(table.delete(&predicate).await?.num_deleted_rows as usize)
    }

    /// Remove the derived rows for records that an owner is deleting. SQLite
    /// remains untouched until this succeeds, so a Lance outage cannot leave
    /// searchable private content behind after a deletion is reported as
    /// complete. A missing table means no index has ever been built and is a
    /// successful no-op.
    pub async fn delete_records(
        &self,
        record_ids: &[String],
        table_name: &str,
    ) -> Result<usize, FlushError> {
        if record_ids.is_empty() {
            return Ok(0);
        }
        let db = lancedb::connect(&self.uri).execute().await?;
        let table = match db.open_table(table_name).execute().await {
            Ok(table) => table,
            Err(lancedb::Error::TableNotFound { .. }) => return Ok(0),
            Err(error) => return Err(error.into()),
        };
        let predicate = format!("record_id IN ({})", sql_string_list(record_ids));
        let result = table.delete(&predicate).await?;
        Ok(result.num_deleted_rows as usize)
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

/// Quote a set of ids for a Lance SQL `IN (...)` predicate. Single quotes are
/// doubled, so an id containing one cannot change the predicate's shape.
fn sql_string_list(values: &[String]) -> String {
    values
        .iter()
        .map(|v| format!("'{}'", v.replace('\'', "''")))
        .collect::<Vec<_>>()
        .join(", ")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RebuildReport {
    pub chunks: usize,
    pub batches: usize,
}
