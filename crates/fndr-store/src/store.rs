//! The system-of-record store: opens `fndr.sqlite3` in WAL mode with foreign
//! keys enforced and the schema migrated. Domain APIs (record writes, queue
//! operations, deletion-everywhere) land with their pipeline tickets; this
//! slice is the foundation they build on (T-201).

use std::path::Path;

use fndr_privacy::SanitizedUrl;
use rusqlite::{Connection, OptionalExtension};

use crate::migrations;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error(
        "database schema v{on_disk} is newer than this build supports (v{supported}); update FNDR instead of downgrading"
    )]
    SchemaTooNew { on_disk: i64, supported: i64 },
    #[error("invalid deletion scope: {0}")]
    InvalidDeleteScope(String),
}

pub struct Store {
    conn: Connection,
}

/// Capture facts for one record (pipeline stage 8 persists one of these plus
/// its chunks in a single transaction).
#[derive(Debug, Clone)]
pub struct NewRecord {
    pub id: String,
    pub session_id: String,
    pub source: String,
    pub app_name: String,
    pub bundle_id: Option<String>,
    /// Sanitized browser URL metadata. The write path removes credentials,
    /// query strings, and fragments before this crosses into SQLite.
    pub url: Option<SanitizedUrl>,
    pub window_title: String,
    pub captured_at_ms: i64,
    pub created_at_ms: i64,
}

/// Capture metadata retained with a record. Pixel bytes never belong here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureMetadata {
    pub bundle_id: Option<String>,
    pub url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NewChunk {
    pub id: String,
    pub ord: i64,
    pub text: String,
}

/// A chunk awaiting Lance flush, joined with the record fields that become
/// prefilter columns.
#[derive(Debug, Clone)]
pub struct PendingChunk {
    pub chunk_id: String,
    pub record_id: String,
    pub ord: i64,
    pub text: String,
    pub source: String,
    pub captured_at_ms: i64,
}

/// A durable-memory selection used by T-206. Domain matching shares the
/// privacy crate's parsed-host semantics; raw SQL substring matching is never
/// used for owner deletion requests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeleteScope {
    RecordIds(Vec<String>),
    TimeRange { start_ms: i64, end_ms: i64 },
    Domain(String),
    All,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self, StoreError> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Self, StoreError> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        migrations::apply(&mut conn)?;
        Ok(Self { conn })
    }

    pub fn schema_version(&self) -> Result<i64, StoreError> {
        Ok(self
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))?)
    }

    /// Test access. Domain modules built on this store (T-202+) get their own
    /// crate-internal accessor when they exist; until then this stays
    /// test-only rather than shipping unused scaffolding.
    #[cfg(test)]
    pub(crate) fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Persist one capture: record plus chunks, atomically.
    pub fn insert_capture(
        &mut self,
        record: &NewRecord,
        chunks: &[NewChunk],
    ) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO memory_records
                 (id, session_id, source, app_name, bundle_id, url, window_title,
                  captured_at_ms, created_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            (
                &record.id,
                &record.session_id,
                &record.source,
                &record.app_name,
                &record.bundle_id,
                record.url.as_ref().map(SanitizedUrl::as_str),
                &record.window_title,
                record.captured_at_ms,
                record.created_at_ms,
            ),
        )?;
        for chunk in chunks {
            tx.execute(
                "INSERT INTO chunks (id, record_id, ord, text) VALUES (?1, ?2, ?3, ?4)",
                (&chunk.id, &record.id, chunk.ord, &chunk.text),
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Read the non-pixel metadata retained for one capture record.
    pub fn capture_metadata(&self, record_id: &str) -> Result<Option<CaptureMetadata>, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT bundle_id, url FROM memory_records WHERE id = ?1",
                [record_id],
                |row| {
                    Ok(CaptureMetadata {
                        bundle_id: row.get(0)?,
                        url: row.get(1)?,
                    })
                },
            )
            .optional()?)
    }

    /// Chunks not yet flushed to Lance, oldest capture first.
    pub fn pending_chunks(&self, limit: usize) -> Result<Vec<PendingChunk>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT c.id, c.record_id, c.ord, c.text, r.source, r.captured_at_ms
             FROM chunks c JOIN memory_records r ON r.id = c.record_id
             WHERE c.flushed_at_ms = 0
             ORDER BY r.captured_at_ms, c.ord
             LIMIT ?1",
        )?;
        let rows = stmt
            .query_map([limit as i64], |row| {
                Ok(PendingChunk {
                    chunk_id: row.get(0)?,
                    record_id: row.get(1)?,
                    ord: row.get(2)?,
                    text: row.get(3)?,
                    source: row.get(4)?,
                    captured_at_ms: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Stamp chunks as flushed. Called only after a successful Lance commit;
    /// a failed flush leaves rows pending so the next cycle retries.
    pub fn mark_chunks_flushed(
        &mut self,
        chunk_ids: &[String],
        now_ms: i64,
    ) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        for id in chunk_ids {
            tx.execute(
                "UPDATE chunks SET flushed_at_ms = ?1 WHERE id = ?2",
                (now_ms, id),
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Reset every chunk to pending (the rebuild path re-flushes everything).
    pub fn reset_flush_state(&mut self) -> Result<usize, StoreError> {
        Ok(self
            .conn
            .execute("UPDATE chunks SET flushed_at_ms = 0", [])?)
    }

    /// Resolve a deletion scope to stable record IDs before touching either
    /// store. The caller deletes Lance rows first, then passes these IDs to
    /// `delete_records`; that ordering never leaves raw searchable content in
    /// the derived index after a successful owner deletion.
    pub fn record_ids_for_delete(&self, scope: &DeleteScope) -> Result<Vec<String>, StoreError> {
        match scope {
            DeleteScope::RecordIds(ids) => {
                let mut found = Vec::new();
                for id in ids {
                    let exists: Option<String> = self
                        .conn
                        .query_row("SELECT id FROM memory_records WHERE id = ?1", [id], |row| {
                            row.get(0)
                        })
                        .optional()?;
                    if let Some(id) = exists {
                        found.push(id);
                    }
                }
                found.sort();
                found.dedup();
                Ok(found)
            }
            DeleteScope::TimeRange { start_ms, end_ms } => {
                if start_ms > end_ms {
                    return Err(StoreError::InvalidDeleteScope(
                        "time range start must not be after its end".to_owned(),
                    ));
                }
                self.record_ids_from_query(
                    "SELECT id FROM memory_records
                     WHERE captured_at_ms >= ?1 AND captured_at_ms <= ?2
                     ORDER BY id",
                    (*start_ms, *end_ms),
                )
            }
            DeleteScope::Domain(domain) => {
                if fndr_privacy::normalize_domain(domain).is_none() {
                    return Err(StoreError::InvalidDeleteScope(
                        "domain must be a valid host or HTTP(S) URL".to_owned(),
                    ));
                }
                let mut statement = self.conn.prepare(
                    "SELECT id, url FROM memory_records WHERE url IS NOT NULL ORDER BY id",
                )?;
                let rows = statement.query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?;
                rows.filter_map(|row| match row {
                    Ok((id, url)) if fndr_privacy::url_matches_domain_suffix(&url, domain) => {
                        Some(Ok(id))
                    }
                    Ok(_) => None,
                    Err(error) => Some(Err(error)),
                })
                .collect::<Result<Vec<_>, _>>()
                .map_err(StoreError::from)
            }
            DeleteScope::All => {
                self.record_ids_from_query("SELECT id FROM memory_records ORDER BY id", [])
            }
        }
    }

    /// Permanently remove resolved records from SQLite truth. Foreign-key
    /// cascades remove chunks and related durable queues in the same
    /// transaction; callers must remove matching derived index rows first.
    pub fn delete_records(&mut self, record_ids: &[String]) -> Result<usize, StoreError> {
        let tx = self.conn.transaction()?;
        let mut deleted = 0;
        for id in record_ids {
            deleted += tx.execute("DELETE FROM memory_records WHERE id = ?1", [id])?;
        }
        tx.commit()?;
        Ok(deleted)
    }

    fn record_ids_from_query<P>(&self, query: &str, params: P) -> Result<Vec<String>, StoreError>
    where
        P: rusqlite::Params,
    {
        let mut statement = self.conn.prepare(query)?;
        Ok(statement
            .query_map(params, |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fndr_types::ReviewLifecycle;

    fn insert_record(store: &Store, id: &str) {
        store
            .conn()
            .execute(
                "INSERT INTO memory_records (id, source, captured_at_ms, created_at_ms)
                 VALUES (?1, 'screen', 1000, 1000)",
                [id],
            )
            .unwrap();
    }

    #[test]
    fn fresh_database_migrates_to_latest() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(
            store.schema_version().unwrap(),
            migrations::schema_version()
        );
        // Every section-5 domain table exists.
        for table in [
            "memory_records",
            "memory_texts",
            "memory_scores",
            "chunks",
            "graph_nodes",
            "graph_edges",
            "node_mentions",
            "entity_aliases",
            "tasks",
            "meetings",
            "meeting_segments",
            "decision_ledger",
            "review_queue",
            "settings",
            "devices",
            "tokens",
        ] {
            let count: i64 = store
                .conn()
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    [table],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(count, 1, "missing table {table}");
        }
    }

    #[test]
    fn v2_database_upgrades_with_capture_metadata_columns() {
        let dir = std::env::temp_dir().join(format!("fndr-metadata-v2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("fndr.sqlite3");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(include_str!("migrations/0001_schema_v1.sql"))
                .unwrap();
            conn.execute_batch(include_str!("migrations/0002_chunk_flush.sql"))
                .unwrap();
            conn.pragma_update(None, "user_version", 2_i64).unwrap();
        }

        let store = Store::open(&path).unwrap();
        assert_eq!(
            store.schema_version().unwrap(),
            migrations::schema_version()
        );
        let mut statement = store
            .conn()
            .prepare("PRAGMA table_info(memory_records)")
            .unwrap();
        let columns: Vec<String> = statement
            .query_map([], |row| row.get(1))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(columns.contains(&"bundle_id".to_owned()));
        assert!(columns.contains(&"url".to_owned()));

        drop(statement);
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn insert_capture_retains_only_explicit_capture_metadata() {
        let mut store = Store::open_in_memory().unwrap();
        store
            .insert_capture(
                &NewRecord {
                    id: "r1".into(),
                    session_id: "s1".into(),
                    source: "screen".into(),
                    app_name: "Safari".into(),
                    bundle_id: Some("com.apple.Safari".into()),
                    url: Some(
                        fndr_privacy::sanitize_url_for_storage("https://docs.example.com/fndr")
                            .unwrap(),
                    ),
                    window_title: "FNDR docs".into(),
                    captured_at_ms: 1_000,
                    created_at_ms: 1_000,
                },
                &[],
            )
            .unwrap();

        assert_eq!(
            store.capture_metadata("r1").unwrap(),
            Some(CaptureMetadata {
                bundle_id: Some("com.apple.Safari".into()),
                url: Some("https://docs.example.com/fndr".into()),
            })
        );
        assert_eq!(store.capture_metadata("missing").unwrap(), None);
    }

    #[test]
    fn reopening_is_idempotent() {
        let dir = std::env::temp_dir().join(format!("fndr-migrate-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("fndr.sqlite3");
        {
            let store = Store::open(&path).unwrap();
            insert_record(&store, "r1");
        }
        let store = Store::open(&path).unwrap();
        let count: i64 = store
            .conn()
            .query_row("SELECT COUNT(*) FROM memory_records", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn newer_schema_on_disk_is_a_typed_error() {
        let dir = std::env::temp_dir().join(format!("fndr-toonew-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("fndr.sqlite3");
        {
            let conn = Connection::open(&path).unwrap();
            conn.pragma_update(None, "user_version", 9999_i64).unwrap();
        }
        match Store::open(&path) {
            Err(StoreError::SchemaTooNew { on_disk, supported }) => {
                assert_eq!(on_disk, 9999);
                assert_eq!(supported, migrations::schema_version());
            }
            Err(other) => panic!("expected SchemaTooNew, got {other:?}"),
            Ok(_) => panic!("expected SchemaTooNew, got a working store"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn graph_edges_enforce_fk_integrity() {
        let store = Store::open_in_memory().unwrap();
        let result = store.conn().execute(
            "INSERT INTO graph_edges (src, dst, kind, created_at_ms)
             VALUES ('ghost-a', 'ghost-b', 1, 1000)",
            [],
        );
        assert!(result.is_err(), "edge to nonexistent nodes must fail");

        store
            .conn()
            .execute_batch(
                "INSERT INTO graph_nodes (id, kind, name, created_at_ms)
                 VALUES ('n1', 1, 'alpha', 1000), ('n2', 2, 'beta', 1000);
                 INSERT INTO graph_edges (src, dst, kind, created_at_ms)
                 VALUES ('n1', 'n2', 1, 1000);",
            )
            .unwrap();

        // Deleting a node cascades its edges: no orphans, ever.
        store
            .conn()
            .execute("DELETE FROM graph_nodes WHERE id = 'n1'", [])
            .unwrap();
        let edges: i64 = store
            .conn()
            .query_row("SELECT COUNT(*) FROM graph_edges", [], |r| r.get(0))
            .unwrap();
        assert_eq!(edges, 0);
    }

    #[test]
    fn record_delete_cascades_to_derived_tables() {
        let store = Store::open_in_memory().unwrap();
        insert_record(&store, "r1");
        store
            .conn()
            .execute_batch(
                "INSERT INTO memory_texts (record_id, raw_text) VALUES ('r1', 'text');
                 INSERT INTO memory_scores (record_id, name, value) VALUES ('r1', 'salience', 0.7);
                 INSERT INTO chunks (id, record_id, ord, text) VALUES ('c1', 'r1', 0, 'chunk');
                 INSERT INTO review_queue (record_id, enqueued_at_ms) VALUES ('r1', 1000);",
            )
            .unwrap();
        store
            .conn()
            .execute("DELETE FROM memory_records WHERE id = 'r1'", [])
            .unwrap();
        for table in ["memory_texts", "memory_scores", "chunks", "review_queue"] {
            let count: i64 = store
                .conn()
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                .unwrap();
            assert_eq!(count, 0, "{table} must cascade on record delete");
        }
    }

    #[test]
    fn lifecycle_discriminants_round_trip_through_storage() {
        let store = Store::open_in_memory().unwrap();
        insert_record(&store, "r1");
        store
            .conn()
            .execute(
                "UPDATE memory_records SET lifecycle = ?1 WHERE id = 'r1'",
                [i64::from(ReviewLifecycle::ReviewedLocal)],
            )
            .unwrap();
        let raw: i64 = store
            .conn()
            .query_row(
                "SELECT lifecycle FROM memory_records WHERE id = 'r1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            ReviewLifecycle::try_from(raw).unwrap(),
            ReviewLifecycle::ReviewedLocal
        );
    }
}
