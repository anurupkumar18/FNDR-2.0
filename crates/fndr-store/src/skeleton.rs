//! Deliberately minimal store for the walking skeleton (T-109): one records
//! table, an external-content FTS5 index, insert and search. WAL mode from
//! day one because the real store (T-201) will live in WAL.
//!
//! FTS uses the porter stemmer: the bench sample exposed that unstemmed
//! unicode61 misses morphological variants ("index" vs "indexes"), and the
//! keyword route in the real stack needs stemming for the same reason.

use std::path::Path;

use rusqlite::Connection;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    pub record_id: i64,
    pub source: String,
    pub captured_at_ms: i64,
    /// FTS5 snippet with match markers.
    pub snippet: String,
    /// bm25 rank; lower is better (SQLite convention).
    pub rank: f64,
}

pub struct SkeletonStore {
    conn: Connection,
}

impl SkeletonStore {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let conn = Connection::open(path)?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self, StoreError> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self, StoreError> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS records (
                id INTEGER PRIMARY KEY,
                captured_at_ms INTEGER NOT NULL,
                source TEXT NOT NULL,
                text TEXT NOT NULL
            );
            CREATE VIRTUAL TABLE IF NOT EXISTS records_fts USING fts5(
                text,
                content='records',
                content_rowid='id',
                tokenize='porter unicode61'
            );
            CREATE TRIGGER IF NOT EXISTS records_ai AFTER INSERT ON records BEGIN
                INSERT INTO records_fts(rowid, text) VALUES (new.id, new.text);
            END;
            CREATE TRIGGER IF NOT EXISTS records_ad AFTER DELETE ON records BEGIN
                INSERT INTO records_fts(records_fts, rowid, text)
                VALUES ('delete', old.id, old.text);
            END;",
        )?;
        Ok(Self { conn })
    }

    pub fn insert_record(
        &self,
        captured_at_ms: i64,
        source: &str,
        text: &str,
    ) -> Result<i64, StoreError> {
        self.conn.execute(
            "INSERT INTO records (captured_at_ms, source, text) VALUES (?1, ?2, ?3)",
            (captured_at_ms, source, text),
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Insert preserving the caller's id (bench corpora reference records by
    /// id, so load order must not renumber them).
    pub fn insert_record_with_id(
        &self,
        id: i64,
        captured_at_ms: i64,
        source: &str,
        text: &str,
    ) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO records (id, captured_at_ms, source, text) VALUES (?1, ?2, ?3, ?4)",
            (id, captured_at_ms, source, text),
        )?;
        Ok(())
    }

    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT r.id, r.source, r.captured_at_ms,
                    snippet(records_fts, 0, '[', ']', ' … ', 12) AS snip,
                    bm25(records_fts) AS rank
             FROM records_fts
             JOIN records r ON r.id = records_fts.rowid
             WHERE records_fts MATCH ?1
             ORDER BY rank
             LIMIT ?2",
        )?;
        let hits = stmt
            .query_map((query, limit as i64), |row| {
                Ok(SearchHit {
                    record_id: row.get(0)?,
                    source: row.get(1)?,
                    captured_at_ms: row.get(2)?,
                    snippet: row.get(3)?,
                    rank: row.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(hits)
    }

    pub fn record_count(&self) -> Result<i64, StoreError> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM records", [], |r| r.get(0))?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_search_round_trip() {
        let store = SkeletonStore::open_in_memory().unwrap();
        store
            .insert_record(
                1000,
                "screen",
                "reviewing the quarterly retrieval benchmark numbers",
            )
            .unwrap();
        store
            .insert_record(2000, "screen", "editing the walking skeleton design note")
            .unwrap();

        let hits = store.search("skeleton", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].record_id, 2);
        assert!(hits[0].snippet.contains("[skeleton]"));

        let hits = store.search("benchmark", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].source, "screen");
    }

    #[test]
    fn search_no_match_is_empty_not_error() {
        let store = SkeletonStore::open_in_memory().unwrap();
        store.insert_record(1000, "screen", "hello world").unwrap();
        assert!(store.search("zebra", 10).unwrap().is_empty());
    }

    #[test]
    fn delete_trigger_keeps_fts_in_sync() {
        let store = SkeletonStore::open_in_memory().unwrap();
        let id = store
            .insert_record(1000, "screen", "ephemeral note")
            .unwrap();
        store
            .conn
            .execute("DELETE FROM records WHERE id = ?1", [id])
            .unwrap();
        assert!(store.search("ephemeral", 10).unwrap().is_empty());
    }
}
