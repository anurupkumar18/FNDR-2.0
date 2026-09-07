//! T-206 owner deletion across SQLite truth and the derived Lance index.

use fndr_inference::EmbeddingSpec;

use crate::{DeleteScope, FlushError, LanceWriter, Store, StoreError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeletionReport {
    pub records: usize,
    pub indexed_chunks: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum DeletionError {
    #[error("store: {0}")]
    Store(#[from] StoreError),
    #[error("derived index: {0}")]
    Index(#[from] FlushError),
}

/// Delete a fully-resolved scope from both stores. Index deletion happens
/// first; if it fails, SQLite truth is retained and the owner sees an error.
/// If SQLite fails after index deletion, truth remains authoritative and the
/// caller can rebuild the index, which cannot resurrect already-deleted data.
pub async fn delete_everywhere(
    store: &mut Store,
    writer: &LanceWriter,
    scope: &DeleteScope,
    embedding: &EmbeddingSpec,
) -> Result<DeletionReport, DeletionError> {
    let record_ids = store.record_ids_for_delete(scope)?;
    let indexed_chunks = writer
        .delete_records(&record_ids, embedding.lance_table)
        .await?;
    let records = store.delete_records(&record_ids)?;
    Ok(DeletionReport {
        records,
        indexed_chunks,
    })
}
