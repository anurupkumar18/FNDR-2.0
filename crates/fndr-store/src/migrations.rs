//! Forward-only embedded migrations tracked by SQLite's user_version.
//! Each migration runs in one transaction; a database newer than this binary
//! is a typed error, never a silent downgrade.

use rusqlite::Connection;

use crate::StoreError;

/// Append-only. Never edit a shipped migration; add the next file.
const MIGRATIONS: &[&str] = &[
    include_str!("migrations/0001_schema_v1.sql"),
    include_str!("migrations/0002_chunk_flush.sql"),
    include_str!("migrations/0003_capture_metadata.sql"),
    include_str!("migrations/0004_chunk_fts.sql"),
];

pub fn schema_version() -> i64 {
    MIGRATIONS.len() as i64
}

pub fn apply(conn: &mut Connection) -> Result<(), StoreError> {
    let current: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if current > schema_version() {
        return Err(StoreError::SchemaTooNew {
            on_disk: current,
            supported: schema_version(),
        });
    }
    for (index, sql) in MIGRATIONS.iter().enumerate().skip(current as usize) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", (index + 1) as i64)?;
        tx.commit()?;
    }
    Ok(())
}
