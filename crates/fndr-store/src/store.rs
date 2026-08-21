//! The system-of-record store: opens `fndr.sqlite3` in WAL mode with foreign
//! keys enforced and the schema migrated. Domain APIs (record writes, queue
//! operations, deletion-everywhere) land with their pipeline tickets; this
//! slice is the foundation they build on (T-201).

use std::path::Path;

use rusqlite::Connection;

use crate::migrations;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error(
        "database schema v{on_disk} is newer than this build supports (v{supported}); update FNDR instead of downgrading"
    )]
    SchemaTooNew { on_disk: i64, supported: i64 },
}

pub struct Store {
    conn: Connection,
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
