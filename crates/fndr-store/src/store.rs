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

/// Hard ceiling on keyword-route results, applied regardless of a caller's
/// requested limit. Named so `explain_chunk_search` can report it instead of
/// a caller discovering it by getting fewer rows than asked for.
pub const SEARCH_LIMIT_CAP: i64 = 50;

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

/// A not-yet-indexed record eligible for an in-flight continuity decision.
/// It is deliberately unavailable once its chunk has reached Lance, avoiding
/// an unsafe derived-index mutation path.
#[derive(Debug, Clone)]
pub struct PendingContinuityCandidate {
    pub record_id: String,
    pub chunk_id: String,
    pub app_name: String,
    pub url: Option<String>,
    pub window_title: String,
    pub text: String,
    pub captured_at_ms: i64,
}

/// A keyword-route hit from SQLite truth. The retrieval crate owns ranking
/// composition; this storage boundary only exposes indexed chunk evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkSearchHit {
    pub chunk_id: String,
    pub record_id: String,
    pub source: String,
    pub captured_at_ms: i64,
    pub snippet: String,
}

/// One chunk's stored evidence, in capture order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkEvidence {
    pub chunk_id: String,
    pub ord: i64,
    pub text: String,
}

/// Everything SQLite retains behind one capture record: its non-pixel
/// metadata plus its chunks' stored text. Callers decide whether the text
/// itself may cross a surface boundary (MCP's `include_raw` gate).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordEvidence {
    pub record_id: String,
    pub session_id: String,
    pub source: String,
    pub app_name: String,
    pub bundle_id: Option<String>,
    pub url: Option<String>,
    pub window_title: String,
    pub captured_at_ms: i64,
    pub chunks: Vec<ChunkEvidence>,
}

/// Bucket width for grouped chronological activity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimelineGranularity {
    Hour,
    Day,
}

impl TimelineGranularity {
    fn width_ms(self) -> i64 {
        match self {
            Self::Hour => 60 * 60 * 1_000,
            Self::Day => 24 * 60 * 60 * 1_000,
        }
    }
}

/// One app's activity inside one time bucket. `bucket_start_ms` is an
/// absolute instant, already corrected for the caller's UTC offset, so the
/// caller never re-derives boundaries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityBucket {
    pub bucket_start_ms: i64,
    pub app_name: String,
    pub record_count: i64,
}

/// How much one app contributed to a window of new captures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppChange {
    pub app_name: String,
    pub record_count: i64,
}

/// What changed since an instant: totals and per-app counts, never content.
/// Built for cheap repeated polling.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ChangeSummary {
    pub record_count: i64,
    pub newest_captured_at_ms: Option<i64>,
    pub apps: Vec<AppChange>,
}

/// Why a keyword search returned what it did. Reports the query as the
/// index actually saw it, not as the caller wrote it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchExplanation {
    /// The terms the query reduced to after punctuation was stripped.
    pub terms: Vec<String>,
    /// The FTS expression actually run, or `None` when nothing survived
    /// normalization (in which case the search returns empty, not an error).
    pub fts_expression: Option<String>,
    /// Matches ignoring any limit, so a caller can tell "few matches" from
    /// "many matches, truncated".
    pub total_matches: i64,
    /// The hard ceiling `search_chunks` applies regardless of the requested
    /// limit.
    pub store_limit_cap: i64,
}

/// One owner rating of a surfaced result. Recorded only; nothing reads it
/// into a ranker (ADR-007: "logged, never silently mutates ranking").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResultFeedback {
    pub id: i64,
    pub at_ms: i64,
    pub record_id: Option<String>,
    pub chunk_id: Option<String>,
    pub query: String,
    pub rating: String,
    pub note: String,
}

/// One recorded MCP tool call. Deliberately carries no query text, record
/// id, or capture content: an audit log that copies what it audits becomes a
/// second store of the same sensitive text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEntry {
    pub id: i64,
    pub at_ms: i64,
    pub tool: String,
    pub outcome: String,
    pub raw_released: bool,
}

/// One entry read back from the append-only decision ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerDecision {
    pub id: i64,
    pub decided_at_ms: i64,
    pub statement: String,
    pub record_id: Option<String>,
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

    /// Recent one-chunk records still owned by SQLite's pending queue. This
    /// supports in-flight continuity only; indexed records require a future
    /// Lance-safe replacement protocol before their evidence may be edited.
    pub fn pending_continuity_candidates(
        &self,
        captured_after_ms: i64,
        limit: usize,
    ) -> Result<Vec<PendingContinuityCandidate>, StoreError> {
        let mut statement = self.conn.prepare(
            "SELECT r.id, c.id, r.app_name, r.url, r.window_title, c.text, r.captured_at_ms
             FROM memory_records r JOIN chunks c ON c.record_id = r.id
             WHERE c.flushed_at_ms = 0 AND c.ord = 0 AND r.captured_at_ms >= ?1
             ORDER BY r.captured_at_ms DESC LIMIT ?2",
        )?;
        statement
            .query_map((captured_after_ms, limit.min(64) as i64), |row| {
                Ok(PendingContinuityCandidate {
                    record_id: row.get(0)?,
                    chunk_id: row.get(1)?,
                    app_name: row.get(2)?,
                    url: row.get(3)?,
                    window_title: row.get(4)?,
                    text: row.get(5)?,
                    captured_at_ms: row.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// Atomically refresh one unflushed record with its deterministic merged
    /// text. The original record and chunk IDs survive, so a later Lance flush
    /// emits exactly one vector row rather than leaving a stale derived row.
    pub fn merge_pending_capture(
        &mut self,
        candidate: &PendingContinuityCandidate,
        incoming: &NewRecord,
        merged_text: &str,
    ) -> Result<bool, StoreError> {
        let tx = self.conn.transaction()?;
        let updated_record = tx.execute(
            "UPDATE memory_records SET app_name = ?1, bundle_id = ?2, url = ?3,
                    window_title = ?4, captured_at_ms = ?5
             WHERE id = ?6 AND EXISTS (
                SELECT 1 FROM chunks WHERE id = ?7 AND record_id = ?6 AND flushed_at_ms = 0
             )",
            (
                &incoming.app_name,
                &incoming.bundle_id,
                incoming.url.as_ref().map(SanitizedUrl::as_str),
                &incoming.window_title,
                incoming.captured_at_ms,
                &candidate.record_id,
                &candidate.chunk_id,
            ),
        )?;
        if updated_record == 0 {
            tx.rollback()?;
            return Ok(false);
        }
        tx.execute(
            "UPDATE chunks SET text = ?1 WHERE id = ?2 AND flushed_at_ms = 0",
            (merged_text, &candidate.chunk_id),
        )?;
        tx.commit()?;
        Ok(true)
    }

    /// Search durable capture chunks through SQLite FTS5. User text becomes a
    /// conjunction of quoted terms, rather than raw FTS syntax, so punctuation
    /// cannot change query semantics or surface an SQLite parse error.
    pub fn search_chunks(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<ChunkSearchHit>, StoreError> {
        let Some(query) = fts_query(query) else {
            return Ok(Vec::new());
        };
        let mut statement = self.conn.prepare(
            "SELECT c.id, c.record_id, r.source, r.captured_at_ms,
                    snippet(chunks_fts, 0, '[', ']', '…', 12)
             FROM chunks_fts
             JOIN chunks c ON c.rowid = chunks_fts.rowid
             JOIN memory_records r ON r.id = c.record_id
             WHERE chunks_fts MATCH ?1
             ORDER BY bm25(chunks_fts), r.captured_at_ms DESC, c.ord
             LIMIT ?2",
        )?;
        statement
            // Clamp before the cast, never after: a usize past i64::MAX
            // casts to -1, and SQLite reads LIMIT -1 as no limit at all.
            .query_map(
                (query, limit.min(SEARCH_LIMIT_CAP as usize) as i64),
                |row| {
                    Ok(ChunkSearchHit {
                        chunk_id: row.get(0)?,
                        record_id: row.get(1)?,
                        source: row.get(2)?,
                        captured_at_ms: row.get(3)?,
                        snippet: row.get(4)?,
                    })
                },
            )?
            .collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
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

    /// Grouped chronological activity: per app, per time bucket, how many
    /// records were captured in `[start_ms, end_ms]`. `utc_offset_minutes`
    /// shifts bucket boundaries onto the caller's local day or hour, because
    /// UTC-aligned days answer "what did I do yesterday" wrongly for most of
    /// the world.
    pub fn activity_buckets(
        &self,
        start_ms: i64,
        end_ms: i64,
        granularity: TimelineGranularity,
        utc_offset_minutes: i64,
        limit: usize,
    ) -> Result<Vec<ActivityBucket>, StoreError> {
        let width = granularity.width_ms();
        let offset = utc_offset_minutes * 60 * 1_000;
        let mut statement = self.conn.prepare(
            "SELECT ((captured_at_ms + ?3) / ?4) * ?4 - ?3 AS bucket_start,
                    app_name,
                    COUNT(*) AS records
             FROM memory_records
             WHERE captured_at_ms >= ?1 AND captured_at_ms <= ?2
             GROUP BY bucket_start, app_name
             ORDER BY bucket_start, records DESC, app_name
             LIMIT ?5",
        )?;
        let rows = statement
            .query_map((start_ms, end_ms, offset, width, limit as i64), |row| {
                Ok(ActivityBucket {
                    bucket_start_ms: row.get(0)?,
                    app_name: row.get(1)?,
                    record_count: row.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Read one record's retained metadata and its chunks' stored text, in
    /// capture order. This is the evidence behind a search hit; the caller
    /// owns the decision to expose the text itself.
    pub fn record_evidence(&self, record_id: &str) -> Result<Option<RecordEvidence>, StoreError> {
        let record = self
            .conn
            .query_row(
                "SELECT session_id, source, app_name, bundle_id, url, window_title,
                        captured_at_ms
                 FROM memory_records WHERE id = ?1",
                [record_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, i64>(6)?,
                    ))
                },
            )
            .optional()?;
        let Some((session_id, source, app_name, bundle_id, url, window_title, captured_at_ms)) =
            record
        else {
            return Ok(None);
        };

        let mut statement = self
            .conn
            .prepare("SELECT id, ord, text FROM chunks WHERE record_id = ?1 ORDER BY ord")?;
        let chunks = statement
            .query_map([record_id], |row| {
                Ok(ChunkEvidence {
                    chunk_id: row.get(0)?,
                    ord: row.get(1)?,
                    text: row.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Some(RecordEvidence {
            record_id: record_id.to_owned(),
            session_id,
            source,
            app_name,
            bundle_id,
            url,
            window_title,
            captured_at_ms,
            chunks,
        }))
    }

    /// Explain a keyword search without running it: what the query became,
    /// and how many chunks match it in total.
    pub fn explain_chunk_search(&self, query: &str) -> Result<SearchExplanation, StoreError> {
        let terms = fts_terms(query);
        let fts_expression = fts_query(query);
        let total_matches = match &fts_expression {
            Some(expression) => self.conn.query_row(
                "SELECT COUNT(*) FROM chunks_fts WHERE chunks_fts MATCH ?1",
                [expression],
                |row| row.get(0),
            )?,
            None => 0,
        };
        Ok(SearchExplanation {
            terms,
            fts_expression,
            total_matches,
            store_limit_cap: SEARCH_LIMIT_CAP,
        })
    }

    /// Record one rating of a surfaced result. Write-only as far as
    /// retrieval is concerned: no ranking path reads this table, and any
    /// future one has to arrive through ADR-006's bench gate.
    pub fn record_feedback(
        &self,
        at_ms: i64,
        record_id: Option<&str>,
        chunk_id: Option<&str>,
        query: &str,
        rating: &str,
        note: &str,
    ) -> Result<i64, StoreError> {
        self.conn.execute(
            "INSERT INTO result_feedback (at_ms, record_id, chunk_id, query, rating, note)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            (at_ms, record_id, chunk_id, query, rating, note),
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Read recorded feedback, newest first.
    pub fn recent_feedback(&self, limit: usize) -> Result<Vec<ResultFeedback>, StoreError> {
        let mut statement = self.conn.prepare(
            "SELECT id, at_ms, record_id, chunk_id, query, rating, note
             FROM result_feedback
             ORDER BY at_ms DESC, id DESC
             LIMIT ?1",
        )?;
        let rows = statement
            .query_map([limit as i64], |row| {
                Ok(ResultFeedback {
                    id: row.get(0)?,
                    at_ms: row.get(1)?,
                    record_id: row.get(2)?,
                    chunk_id: row.get(3)?,
                    query: row.get(4)?,
                    rating: row.get(5)?,
                    note: row.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// The id of the most recently captured record, if any. Kept separate
    /// from `record_evidence` so callers compose the two rather than growing
    /// a second "read a record" path.
    pub fn latest_record_id(&self) -> Result<Option<String>, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT id FROM memory_records ORDER BY captured_at_ms DESC, id DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Record one MCP tool call. `raw_released` marks the calls that handed
    /// back stored capture text, which is the event a person most needs to
    /// see when they open their audit log.
    pub fn record_tool_call(
        &self,
        at_ms: i64,
        tool: &str,
        outcome: &str,
        raw_released: bool,
    ) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO mcp_audit (at_ms, tool, outcome, raw_released)
             VALUES (?1, ?2, ?3, ?4)",
            (at_ms, tool, outcome, raw_released),
        )?;
        Ok(())
    }

    /// Read recorded tool calls, newest first.
    pub fn recent_tool_calls(&self, limit: usize) -> Result<Vec<AuditEntry>, StoreError> {
        let mut statement = self.conn.prepare(
            "SELECT id, at_ms, tool, outcome, raw_released
             FROM mcp_audit
             ORDER BY at_ms DESC, id DESC
             LIMIT ?1",
        )?;
        let rows = statement
            .query_map([limit as i64], |row| {
                Ok(AuditEntry {
                    id: row.get(0)?,
                    at_ms: row.get(1)?,
                    tool: row.get(2)?,
                    outcome: row.get(3)?,
                    raw_released: row.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Summarize captures at or after `since_ms`: how many, the newest
    /// instant, and the busiest apps. Counts only, so a caller can poll this
    /// repeatedly without moving any capture content.
    pub fn changes_since(
        &self,
        since_ms: i64,
        app_limit: usize,
    ) -> Result<ChangeSummary, StoreError> {
        let mut statement = self.conn.prepare(
            "SELECT app_name, COUNT(*) AS records, MAX(captured_at_ms) AS newest
             FROM memory_records
             WHERE captured_at_ms >= ?1
             GROUP BY app_name
             ORDER BY records DESC, app_name",
        )?;
        let rows = statement
            .query_map([since_ms], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let record_count = rows.iter().map(|(_, count, _)| count).sum();
        let newest_captured_at_ms = rows.iter().map(|(_, _, newest)| *newest).max();
        let apps = rows
            .into_iter()
            .take(app_limit)
            .map(|(app_name, record_count, _)| AppChange {
                app_name,
                record_count,
            })
            .collect();
        Ok(ChangeSummary {
            record_count,
            newest_captured_at_ms,
            apps,
        })
    }

    /// Read the decision ledger, newest first. `since_ms` bounds the read to
    /// decisions made at or after that instant.
    pub fn recent_decisions(
        &self,
        since_ms: Option<i64>,
        limit: usize,
    ) -> Result<Vec<LedgerDecision>, StoreError> {
        let mut statement = self.conn.prepare(
            "SELECT id, decided_at_ms, statement, record_id
             FROM decision_ledger
             WHERE decided_at_ms >= ?1
             ORDER BY decided_at_ms DESC, id DESC
             LIMIT ?2",
        )?;
        let rows = statement
            .query_map((since_ms.unwrap_or(i64::MIN), limit as i64), |row| {
                Ok(LedgerDecision {
                    id: row.get(0)?,
                    decided_at_ms: row.get(1)?,
                    statement: row.get(2)?,
                    record_id: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Append one entry to the append-only decision ledger (schema v1). This
    /// is the only durable write MCP's `fndr.remember_decision` performs; it
    /// never edits or removes prior entries.
    pub fn remember_decision(
        &self,
        decided_at_ms: i64,
        statement: &str,
        record_id: Option<&str>,
    ) -> Result<i64, StoreError> {
        self.conn.execute(
            "INSERT INTO decision_ledger (decided_at_ms, statement, record_id)
             VALUES (?1, ?2, ?3)",
            (decided_at_ms, statement, record_id),
        )?;
        Ok(self.conn.last_insert_rowid())
    }
}

/// The terms an FTS query is reduced to: punctuation stripped, empties
/// dropped. Split out from `fts_query` so `explain_chunk_search` can show a
/// caller what their words actually became.
fn fts_terms(query: &str) -> Vec<String> {
    query
        .split_whitespace()
        .map(|term| {
            term.chars()
                .filter(|character| character.is_alphanumeric() || matches!(character, '_' | '-'))
                .collect::<String>()
        })
        .filter(|term| !term.is_empty())
        .collect()
}

fn fts_query(query: &str) -> Option<String> {
    let terms = fts_terms(query);
    (!terms.is_empty()).then(|| {
        terms
            .iter()
            .map(|term| format!("\"{term}\""))
            .collect::<Vec<_>>()
            .join(" AND ")
    })
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
            "mcp_audit",
            "result_feedback",
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
    fn punctuation_only_fts_query_is_empty() {
        let store = Store::open_in_memory().unwrap();
        assert!(store.search_chunks("!!! \"()", 10).unwrap().is_empty());
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

    fn insert_record_at(store: &Store, id: &str, app: &str, captured_at_ms: i64) {
        store
            .conn()
            .execute(
                "INSERT INTO memory_records (id, source, app_name, captured_at_ms, created_at_ms)
                 VALUES (?1, 'screen', ?2, ?3, ?3)",
                (id, app, captured_at_ms),
            )
            .unwrap();
    }

    #[test]
    fn activity_buckets_group_by_app_within_each_hour() {
        let store = Store::open_in_memory().unwrap();
        let hour = 60 * 60 * 1_000;
        insert_record_at(&store, "a", "Safari", hour + 1);
        insert_record_at(&store, "b", "Safari", hour + 2);
        insert_record_at(&store, "c", "Terminal", hour + 3);
        insert_record_at(&store, "d", "Safari", 3 * hour);

        let buckets = store
            .activity_buckets(0, 10 * hour, TimelineGranularity::Hour, 0, 50)
            .unwrap();
        assert_eq!(buckets.len(), 3);
        // Busiest app first inside a bucket, buckets in chronological order.
        assert_eq!(buckets[0].bucket_start_ms, hour);
        assert_eq!(buckets[0].app_name, "Safari");
        assert_eq!(buckets[0].record_count, 2);
        assert_eq!(buckets[1].app_name, "Terminal");
        assert_eq!(buckets[2].bucket_start_ms, 3 * hour);
    }

    #[test]
    fn activity_buckets_exclude_records_outside_the_window() {
        let store = Store::open_in_memory().unwrap();
        let hour = 60 * 60 * 1_000;
        insert_record_at(&store, "before", "Safari", hour);
        insert_record_at(&store, "inside", "Safari", 5 * hour);
        insert_record_at(&store, "after", "Safari", 9 * hour);

        let buckets = store
            .activity_buckets(4 * hour, 6 * hour, TimelineGranularity::Hour, 0, 50)
            .unwrap();
        assert_eq!(buckets.len(), 1);
        assert_eq!(buckets[0].record_count, 1);
        assert_eq!(buckets[0].bucket_start_ms, 5 * hour);
    }

    #[test]
    fn day_buckets_follow_the_callers_utc_offset() {
        let store = Store::open_in_memory().unwrap();
        let hour = 60 * 60 * 1_000;
        let day = 24 * hour;
        // 23:30 UTC on day 0 is 18:30 the same local day at UTC-5, but
        // 08:30 the *next* local day at UTC+9.
        insert_record_at(&store, "late", "Safari", day - (30 * 60 * 1_000));

        // Local midnight at UTC-5 is 05:00 UTC, so day 0's bucket starts there.
        let west = store
            .activity_buckets(0, 5 * day, TimelineGranularity::Day, -300, 50)
            .unwrap();
        assert_eq!(west[0].bucket_start_ms, 5 * hour, "local day 0 at UTC-5");

        let east = store
            .activity_buckets(0, 5 * day, TimelineGranularity::Day, 540, 50)
            .unwrap();
        assert_eq!(
            east[0].bucket_start_ms,
            day - (9 * hour),
            "local day 1 at UTC+9"
        );
    }

    #[test]
    fn record_evidence_returns_metadata_and_chunks_in_capture_order() {
        let mut store = Store::open_in_memory().unwrap();
        store
            .insert_capture(
                &NewRecord {
                    id: "r1".into(),
                    session_id: "s1".into(),
                    source: "screen".into(),
                    app_name: "Safari".into(),
                    bundle_id: Some("com.apple.Safari".into()),
                    url: None,
                    window_title: "release notes".into(),
                    captured_at_ms: 42,
                    created_at_ms: 42,
                },
                &[
                    NewChunk {
                        id: "c2".into(),
                        ord: 1,
                        text: "second".into(),
                    },
                    NewChunk {
                        id: "c1".into(),
                        ord: 0,
                        text: "first".into(),
                    },
                ],
            )
            .unwrap();

        let evidence = store.record_evidence("r1").unwrap().expect("record exists");
        assert_eq!(evidence.app_name, "Safari");
        assert_eq!(evidence.bundle_id.as_deref(), Some("com.apple.Safari"));
        assert_eq!(evidence.captured_at_ms, 42);
        let ords: Vec<i64> = evidence.chunks.iter().map(|c| c.ord).collect();
        assert_eq!(ords, vec![0, 1], "chunks come back in capture order");
        assert_eq!(evidence.chunks[0].text, "first");
    }

    #[test]
    fn record_evidence_for_an_unknown_record_is_none_not_an_error() {
        let store = Store::open_in_memory().unwrap();
        assert!(store.record_evidence("missing").unwrap().is_none());
    }

    #[test]
    fn an_absurd_limit_still_clamps_to_the_cap() {
        let mut store = Store::open_in_memory().unwrap();
        // More rows than the cap, so an unclamped limit is distinguishable
        // from a clamped one. With only a handful this test would pass
        // against the bug it exists to catch.
        let rows = SEARCH_LIMIT_CAP + 5;
        for index in 0..rows {
            store
                .insert_capture(
                    &NewRecord {
                        id: format!("r{index}"),
                        session_id: "s1".into(),
                        source: "screen".into(),
                        app_name: "Terminal".into(),
                        bundle_id: None,
                        url: None,
                        window_title: "notes".into(),
                        captured_at_ms: 1_000 + index,
                        created_at_ms: 1_000 + index,
                    },
                    &[NewChunk {
                        id: format!("c{index}"),
                        ord: 0,
                        text: "shared searchable token".into(),
                    }],
                )
                .unwrap();
        }

        // usize::MAX casts to -1 as i64, and SQLite reads LIMIT -1 as no
        // limit, so the clamp has to happen before the cast. Unclamped this
        // returns every row; clamped it returns the cap.
        let hits = store.search_chunks("searchable", usize::MAX).unwrap();
        assert_eq!(hits.len(), SEARCH_LIMIT_CAP as usize);
    }

    #[test]
    fn explain_chunk_search_shows_the_query_the_index_actually_saw() {
        let mut store = Store::open_in_memory().unwrap();
        store
            .insert_capture(
                &NewRecord {
                    id: "r1".into(),
                    session_id: "s1".into(),
                    source: "screen".into(),
                    app_name: "Terminal".into(),
                    bundle_id: None,
                    url: None,
                    window_title: "notes".into(),
                    captured_at_ms: 1_000,
                    created_at_ms: 1_000,
                },
                &[NewChunk {
                    id: "c1".into(),
                    ord: 0,
                    text: "the index was rebuilt after the crash".into(),
                }],
            )
            .unwrap();

        let explained = store.explain_chunk_search("index, rebuilt!").unwrap();
        assert_eq!(
            explained.terms,
            vec!["index", "rebuilt"],
            "punctuation is stripped before the index sees the query"
        );
        assert_eq!(explained.total_matches, 1);
        assert_eq!(explained.store_limit_cap, SEARCH_LIMIT_CAP);

        // Every term must match, so one absent word empties the result.
        let narrowed = store
            .explain_chunk_search("index rebuilt kangaroo")
            .unwrap();
        assert_eq!(
            narrowed.total_matches, 0,
            "terms are ANDed: one unmatched word returns nothing"
        );

        let nothing_usable = store.explain_chunk_search("!!! ???").unwrap();
        assert!(nothing_usable.terms.is_empty());
        assert!(nothing_usable.fts_expression.is_none());
        assert_eq!(nothing_usable.total_matches, 0);
    }

    #[test]
    fn feedback_survives_deleting_the_record_it_rated() {
        let mut store = Store::open_in_memory().unwrap();
        insert_record_at(&store, "r1", "Safari", 1_000);
        store
            .record_feedback(2_000, Some("r1"), Some("c1"), "migrations", "helpful", "")
            .unwrap();

        store.delete_records(&["r1".to_owned()]).unwrap();

        let feedback = store.recent_feedback(10).unwrap();
        assert_eq!(
            feedback.len(),
            1,
            "deleting a memory must not erase that its surfacing was rated"
        );
        assert_eq!(
            feedback[0].record_id, None,
            "the citation is dropped, the rating is not"
        );
        assert_eq!(feedback[0].query, "migrations");
        assert_eq!(feedback[0].rating, "helpful");
    }

    #[test]
    fn latest_record_id_tracks_the_newest_capture_and_is_none_when_empty() {
        let store = Store::open_in_memory().unwrap();
        assert!(store.latest_record_id().unwrap().is_none());

        insert_record_at(&store, "older", "Safari", 1_000);
        insert_record_at(&store, "newest", "Terminal", 3_000);
        insert_record_at(&store, "middle", "Notes", 2_000);

        assert_eq!(
            store.latest_record_id().unwrap().as_deref(),
            Some("newest"),
            "newest by capture time, not by insertion order"
        );
    }

    #[test]
    fn recorded_tool_calls_come_back_newest_first_with_the_raw_release_flag() {
        let store = Store::open_in_memory().unwrap();
        store
            .record_tool_call(1_000, "fndr.search", "ok", false)
            .unwrap();
        store
            .record_tool_call(2_000, "fndr.source_evidence", "ok", true)
            .unwrap();

        let entries = store.recent_tool_calls(10).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].tool, "fndr.source_evidence");
        assert!(
            entries[0].raw_released,
            "a raw-text release must be visible in the audit log"
        );
        assert_eq!(entries[1].tool, "fndr.search");
        assert!(!entries[1].raw_released);
    }

    #[test]
    fn changes_since_totals_every_app_even_when_the_app_list_is_capped() {
        let store = Store::open_in_memory().unwrap();
        insert_record_at(&store, "old", "Safari", 500);
        insert_record_at(&store, "a", "Safari", 1_000);
        insert_record_at(&store, "b", "Safari", 1_500);
        insert_record_at(&store, "c", "Terminal", 2_000);
        insert_record_at(&store, "d", "Notes", 2_500);

        let summary = store.changes_since(1_000, 1).unwrap();
        assert_eq!(summary.record_count, 4, "the pre-window record is excluded");
        assert_eq!(summary.newest_captured_at_ms, Some(2_500));
        assert_eq!(summary.apps.len(), 1, "app list respects its cap");
        assert_eq!(summary.apps[0].app_name, "Safari", "busiest app first");
        assert_eq!(summary.apps[0].record_count, 2);
    }

    #[test]
    fn changes_since_a_future_instant_is_an_empty_summary() {
        let store = Store::open_in_memory().unwrap();
        insert_record_at(&store, "a", "Safari", 1_000);
        let summary = store.changes_since(9_000, 10).unwrap();
        assert_eq!(summary.record_count, 0);
        assert_eq!(summary.newest_captured_at_ms, None);
        assert!(summary.apps.is_empty());
    }

    #[test]
    fn recent_decisions_are_newest_first_and_respect_since_and_limit() {
        let store = Store::open_in_memory().unwrap();
        store.remember_decision(1_000, "oldest", None).unwrap();
        store.remember_decision(2_000, "middle", None).unwrap();
        store.remember_decision(3_000, "newest", None).unwrap();

        let all = store.recent_decisions(None, 10).unwrap();
        let statements: Vec<&str> = all.iter().map(|d| d.statement.as_str()).collect();
        assert_eq!(statements, vec!["newest", "middle", "oldest"]);

        let since = store.recent_decisions(Some(2_000), 10).unwrap();
        assert_eq!(since.len(), 2, "since_ms is inclusive of its own instant");

        let capped = store.recent_decisions(None, 1).unwrap();
        assert_eq!(capped.len(), 1);
        assert_eq!(capped[0].statement, "newest");
    }

    #[test]
    fn remember_decision_appends_without_requiring_a_record() {
        let store = Store::open_in_memory().unwrap();
        let id = store
            .remember_decision(1_000, "ship the walking skeleton first", None)
            .unwrap();
        let (statement, record_id): (String, Option<String>) = store
            .conn()
            .query_row(
                "SELECT statement, record_id FROM decision_ledger WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(statement, "ship the walking skeleton first");
        assert_eq!(record_id, None);
    }

    #[test]
    fn remember_decision_can_cite_a_record_and_never_overwrites_prior_entries() {
        let store = Store::open_in_memory().unwrap();
        insert_record(&store, "r1");
        let first = store.remember_decision(1_000, "first", Some("r1")).unwrap();
        let second = store
            .remember_decision(2_000, "second", Some("r1"))
            .unwrap();
        assert_ne!(first, second, "append-only: each call adds a new row");
        let count: i64 = store
            .conn()
            .query_row("SELECT COUNT(*) FROM decision_ledger", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 2);
    }
}
