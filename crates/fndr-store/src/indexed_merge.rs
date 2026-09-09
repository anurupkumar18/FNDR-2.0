//! T-307: the Lance-safe indexed-record merge/update protocol.
//!
//! # The problem
//!
//! Continuity merging edits the evidence of an existing capture in place, so
//! the original record and chunk ids survive and the timeline keeps one entry
//! instead of a burst of near-duplicates. That was previously only legal for
//! a chunk SQLite still owned (`flushed_at_ms = 0`), because Lance has no
//! upsert: `Table::add` is an append. Editing the text of an already-indexed
//! chunk and re-adding it would leave two rows for one chunk id, one of them
//! carrying text that truth no longer holds. Refusing the merge instead was
//! not free either: it silently discarded the continuity decision as soon as
//! a flush cycle happened to land first, which is a timing-dependent
//! behaviour change and exactly the kind of quiet skip invariant 4 bans.
//!
//! # The protocol
//!
//! One extra column of truth makes the derivative repairable:
//! `chunks.index_state` (`fndr_types::ChunkIndexState`) says what Lance holds
//! for that chunk, and `chunks.revision` is an optimistic-concurrency token
//! bumped on every text change. Both live in SQLite, so the derivative still
//! holds no unique information (ADR-002); dropping `index/` and rebuilding
//! remains a complete recovery.
//!
//! A merge is a single SQLite transaction (see [`Store::merge_capture`]):
//! update the record's metadata, replace the chunk text, bump `revision`, and
//! move `Indexed -> Superseded` (`Pending` stays `Pending`, since no Lance
//! row exists yet). Truth is complete and correct at that commit. The
//! derivative is repaired by the next flush cycle, which is exactly the
//! sub-minute eventual consistency ADR-002 already accepts between the two
//! stores; nothing new is introduced.
//!
//! The flush cycle ([`LanceWriter::flush_once`]) therefore runs four steps:
//!
//! 1. **Lance delete** of every chunk id in the batch that is `Superseded`,
//!    removing the stale row (and any duplicate of it).
//! 2. **SQLite mark-indexing**: every chunk in the batch becomes
//!    `Superseded`. This is the durable statement "a Lance row for this chunk
//!    may exist from here on".
//! 3. **Lance add** of the freshly embedded batch.
//! 4. **SQLite stamp**: each chunk becomes `Indexed` if its `revision` still
//!    matches the revision that was embedded, and `Superseded` if it does not.
//!
//! # Ordering rationale
//!
//! [`crate::delete_everywhere`] deletes from the derived index *first*,
//! because its safety property is that no failure may leave searchable
//! content behind after a deletion is reported complete. An update has a
//! different safety property, so it needs a different, and deliberately
//! opposite, ordering: the merged evidence is *new truth*, and truth is
//! written first. Losing it to a crash while repairing a rebuildable index
//! would be inverting ADR-002. Every step after the merge commit only
//! destroys or re-derives index state, so every one of them is idempotent and
//! safely replayable.
//!
//! Within the flush, delete precedes add for the same reason it does in
//! deletion: a stale row must never coexist with its replacement, so the only
//! legal intermediate state is "row missing", never "two rows".
//!
//! # Crash-window analysis
//!
//! Let M be the merge commit and 1..4 the flush steps above.
//!
//! - **Crash before M.** Nothing happened. The incoming capture is lost the
//!   same way any pre-persistence crash loses it; both stores are consistent.
//! - **Crash after M, before 1.** Truth holds merged evidence. Lance holds
//!   the pre-merge row, and SQLite records that fact as `Superseded`. SQLite
//!   FTS (the keyword route) already serves merged truth. The vector route
//!   can return the older text of the same chunk of the same record for at
//!   most one flush interval; it cannot return content that truth deleted for
//!   an owner, because owner deletion runs `delete_everywhere`, not this path.
//!   The state is typed, queryable, and counted in `FlushReport`, not silent.
//! - **Crash after 1, before 2.** The stale row is gone; the chunk is still
//!   `Superseded`. The next cycle re-issues the delete (0 rows) and continues.
//!   The chunk is missing from the vector index in the meantime, and says so.
//! - **Crash after 2, before 3.** Chunks are `Superseded` with no row added.
//!   Next cycle deletes (0 rows for pending chunks, the just-added row for any
//!   re-run) and adds. Converges.
//! - **Crash after 3, before 4.** The fresh row is in Lance and the chunk is
//!   still `Superseded`, so the next cycle deletes exactly that row and adds
//!   it again. This is why step 2 exists: without it the chunk would still
//!   read `Pending`, the next cycle would add a second row, and the duplicate
//!   would survive until a rebuild.
//! - **Crash after 4.** Consistent.
//! - **Merge racing a flush.** Both writers serialise on the SQLite write
//!   lock, and neither trusts the row it read earlier. `merge_capture`
//!   computes the new `index_state` from the row's *current* value inside its
//!   own transaction, so a flush that stamped `Indexed` in between is picked
//!   up correctly. Symmetrically, step 4 compares `revision` against the
//!   revision that was actually embedded: a merge that lands mid-flush leaves
//!   the chunk `Superseded` rather than falsely `Indexed`, so the next cycle
//!   replaces the row it just wrote. A second merge into a candidate that
//!   another writer already merged fails its `revision` guard and returns
//!   [`CaptureMergeOutcome::CandidateChanged`], which the caller sees and
//!   handles by storing a new record rather than by silently dropping data.
//!
//! In every window the invariant holds: SQLite is complete and authoritative,
//! Lance is either correct, missing a row, or holding exactly one stale row
//! that truth knows about, and one further flush cycle converges.

use fndr_privacy::SanitizedUrl;
use fndr_types::ChunkIndexState;
use rusqlite::OptionalExtension;

use crate::{NewRecord, PendingChunk, Store, StoreError};

/// A record eligible for an in-flight continuity decision.
///
/// Unlike its predecessor this is not restricted to chunks SQLite still owns;
/// `index_state` reports where the candidate's evidence currently lives so a
/// caller can tell a purely-SQLite merge from one that also schedules index
/// repair.
#[derive(Debug, Clone)]
pub struct ContinuityCandidate {
    pub record_id: String,
    pub chunk_id: String,
    pub app_name: String,
    pub url: Option<String>,
    pub window_title: String,
    pub text: String,
    pub captured_at_ms: i64,
    pub index_state: ChunkIndexState,
    /// The revision observed when the candidate was read. `merge_capture`
    /// refuses to write over a different one.
    pub revision: i64,
}

/// What a merge attempt actually did. Every outcome is observable; there is
/// no variant that means "something happened, we are not sure what".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureMergeOutcome {
    /// Truth updated; the chunk had never reached Lance, so the ordinary
    /// flush will index it once, exactly as before this ticket.
    Merged,
    /// Truth updated; a stale Lance row exists and the chunk is now
    /// `Superseded`, so the next flush deletes it before adding the
    /// replacement.
    MergedIndexRepairPending,
    /// Another writer changed this chunk between the read and the write. No
    /// evidence was overwritten; the caller must fall back to storing a new
    /// record.
    CandidateChanged,
}

impl CaptureMergeOutcome {
    /// True when the derived index still has to be repaired for this merge.
    pub fn needs_index_repair(self) -> bool {
        matches!(self, CaptureMergeOutcome::MergedIndexRepairPending)
    }
}

/// How many chunks the flush stamp could actually confirm as indexed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IndexStampReport {
    /// Rows in Lance that match truth.
    pub indexed: usize,
    /// Rows in Lance whose chunk was merged while the flush was in flight;
    /// they stay `Superseded` and the next cycle replaces them.
    pub superseded: usize,
}

/// Inherent methods live here rather than in `store.rs` (already ~1500 lines,
/// over the repo's ~600-line rule) so this protocol reads as one unit.
impl Store {
    /// Recent one-chunk records eligible for a continuity merge, newest first.
    ///
    /// This intentionally no longer filters on flush state: an indexed
    /// candidate is mergeable through the protocol documented above.
    pub fn continuity_candidates(
        &self,
        captured_after_ms: i64,
        limit: usize,
    ) -> Result<Vec<ContinuityCandidate>, StoreError> {
        let mut statement = self.conn().prepare(
            "SELECT r.id, c.id, r.app_name, r.url, r.window_title, c.text,
                    r.captured_at_ms, c.index_state, c.revision
             FROM memory_records r JOIN chunks c ON c.record_id = r.id
             WHERE c.ord = 0 AND r.captured_at_ms >= ?1
             ORDER BY r.captured_at_ms DESC LIMIT ?2",
        )?;
        let rows = statement
            .query_map((captured_after_ms, limit.min(64) as i64), |row| {
                Ok((
                    ContinuityCandidate {
                        record_id: row.get(0)?,
                        chunk_id: row.get(1)?,
                        app_name: row.get(2)?,
                        url: row.get(3)?,
                        window_title: row.get(4)?,
                        text: row.get(5)?,
                        captured_at_ms: row.get(6)?,
                        index_state: ChunkIndexState::Pending,
                        revision: row.get(8)?,
                    },
                    row.get::<_, i64>(7)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|(mut candidate, raw_state)| {
                candidate.index_state = ChunkIndexState::try_from(raw_state).map_err(
                    |e: fndr_types::UnknownDiscriminant| {
                        StoreError::UnknownDiscriminant(e.to_string())
                    },
                )?;
                Ok(candidate)
            })
            .collect()
    }

    /// Atomically fold an incoming capture into an existing record.
    ///
    /// Step M of the protocol in this module's documentation. The record and
    /// chunk ids survive, `revision` is bumped, and the chunk moves to
    /// `Superseded` when (and only when) a Lance row for it may already
    /// exist. The new state is computed from the row's live value inside the
    /// transaction, never from the possibly-stale `candidate.index_state`.
    pub fn merge_capture(
        &mut self,
        candidate: &ContinuityCandidate,
        incoming: &NewRecord,
        merged_text: &str,
    ) -> Result<CaptureMergeOutcome, StoreError> {
        let tx = self.conn_mut().transaction()?;
        // The revision guard is the whole concurrency story: it fails if any
        // other writer touched this chunk's text since it was read.
        let updated_chunk = tx.execute(
            "UPDATE chunks
                SET text = ?1,
                    revision = revision + 1,
                    index_state = CASE index_state WHEN ?2 THEN ?2 ELSE ?3 END
              WHERE id = ?4 AND record_id = ?5 AND revision = ?6",
            (
                merged_text,
                i64::from(ChunkIndexState::Pending),
                i64::from(ChunkIndexState::Superseded),
                &candidate.chunk_id,
                &candidate.record_id,
                candidate.revision,
            ),
        )?;
        if updated_chunk == 0 {
            tx.rollback()?;
            return Ok(CaptureMergeOutcome::CandidateChanged);
        }
        tx.execute(
            "UPDATE memory_records SET app_name = ?1, bundle_id = ?2, url = ?3,
                    window_title = ?4, captured_at_ms = ?5
             WHERE id = ?6",
            (
                &incoming.app_name,
                &incoming.bundle_id,
                incoming.url.as_ref().map(SanitizedUrl::as_str),
                &incoming.window_title,
                incoming.captured_at_ms,
                &candidate.record_id,
            ),
        )?;
        let state: i64 = tx.query_row(
            "SELECT index_state FROM chunks WHERE id = ?1",
            [&candidate.chunk_id],
            |row| row.get(0),
        )?;
        tx.commit()?;
        match ChunkIndexState::try_from(state).map_err(|e: fndr_types::UnknownDiscriminant| {
            StoreError::UnknownDiscriminant(e.to_string())
        })? {
            ChunkIndexState::Pending => Ok(CaptureMergeOutcome::Merged),
            _ => Ok(CaptureMergeOutcome::MergedIndexRepairPending),
        }
    }

    /// Chunks the derived index still owes work for: everything not
    /// `Indexed`, oldest capture first. `Superseded` entries carry a stale
    /// Lance row that the flush must delete before adding the replacement.
    pub fn pending_chunks(&self, limit: usize) -> Result<Vec<PendingChunk>, StoreError> {
        let mut stmt = self.conn().prepare(
            "SELECT c.id, c.record_id, c.ord, c.text, r.source, r.captured_at_ms,
                    c.index_state, c.revision
             FROM chunks c JOIN memory_records r ON r.id = c.record_id
             WHERE c.index_state != ?1
             ORDER BY r.captured_at_ms, c.ord
             LIMIT ?2",
        )?;
        let rows = stmt
            .query_map((i64::from(ChunkIndexState::Indexed), limit as i64), |row| {
                Ok((
                    PendingChunk {
                        chunk_id: row.get(0)?,
                        record_id: row.get(1)?,
                        ord: row.get(2)?,
                        text: row.get(3)?,
                        source: row.get(4)?,
                        captured_at_ms: row.get(5)?,
                        index_state: ChunkIndexState::Pending,
                        revision: row.get(7)?,
                    },
                    row.get::<_, i64>(6)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|(mut chunk, raw_state)| {
                chunk.index_state = ChunkIndexState::try_from(raw_state).map_err(
                    |e: fndr_types::UnknownDiscriminant| {
                        StoreError::UnknownDiscriminant(e.to_string())
                    },
                )?;
                Ok(chunk)
            })
            .collect()
    }

    /// Step 2: declare that a Lance row for each of these chunks may exist
    /// from now on. Called after the stale-row delete and before the add, so
    /// a crash during the add can never be mistaken for "never indexed".
    pub fn mark_chunks_indexing(&mut self, chunk_ids: &[String]) -> Result<(), StoreError> {
        let tx = self.conn_mut().transaction()?;
        for id in chunk_ids {
            tx.execute(
                "UPDATE chunks SET index_state = ?1 WHERE id = ?2",
                (i64::from(ChunkIndexState::Superseded), id),
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Step 4: confirm the Lance commit against the revision that was
    /// actually embedded. A chunk whose revision moved during the flush stays
    /// `Superseded` and is reported, never quietly marked clean.
    pub fn mark_chunks_indexed(
        &mut self,
        stamps: &[(String, i64)],
        now_ms: i64,
    ) -> Result<IndexStampReport, StoreError> {
        let tx = self.conn_mut().transaction()?;
        let mut report = IndexStampReport::default();
        for (id, revision) in stamps {
            tx.execute(
                "UPDATE chunks
                    SET flushed_at_ms = ?1,
                        index_state = CASE WHEN revision = ?2 THEN ?3 ELSE ?4 END
                  WHERE id = ?5",
                (
                    now_ms,
                    revision,
                    i64::from(ChunkIndexState::Indexed),
                    i64::from(ChunkIndexState::Superseded),
                    id,
                ),
            )?;
            let state: i64 = tx.query_row(
                "SELECT index_state FROM chunks WHERE id = ?1",
                [id],
                |row| row.get(0),
            )?;
            if state == i64::from(ChunkIndexState::Indexed) {
                report.indexed += 1;
            } else {
                report.superseded += 1;
            }
        }
        tx.commit()?;
        Ok(report)
    }

    /// Reset every chunk to `Pending` (the rebuild path drops the Lance table
    /// and re-flushes everything, so no row may be assumed to exist).
    pub fn reset_flush_state(&mut self) -> Result<usize, StoreError> {
        Ok(self.conn_mut().execute(
            "UPDATE chunks SET flushed_at_ms = 0, index_state = ?1",
            [i64::from(ChunkIndexState::Pending)],
        )?)
    }

    /// Read one chunk's index lifecycle. Used by the flush report surface and
    /// by tests that assert convergence after a simulated crash.
    pub fn chunk_index_state(
        &self,
        chunk_id: &str,
    ) -> Result<Option<(ChunkIndexState, i64)>, StoreError> {
        let row: Option<(i64, i64)> = self
            .conn()
            .query_row(
                "SELECT index_state, revision FROM chunks WHERE id = ?1",
                [chunk_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        match row {
            None => Ok(None),
            Some((state, revision)) => Ok(Some((
                ChunkIndexState::try_from(state).map_err(
                    |e: fndr_types::UnknownDiscriminant| {
                        StoreError::UnknownDiscriminant(e.to_string())
                    },
                )?,
                revision,
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NewChunk;

    fn record_for(id: &str, captured_at_ms: i64) -> NewRecord {
        NewRecord {
            id: id.into(),
            session_id: "s1".into(),
            source: "screen".into(),
            app_name: "Terminal".into(),
            bundle_id: None,
            url: None,
            window_title: "fndr".into(),
            captured_at_ms,
            created_at_ms: captured_at_ms,
        }
    }

    fn seeded() -> Store {
        let mut store = Store::open_in_memory().unwrap();
        store
            .insert_capture(
                &record_for("r1", 1_000),
                &[NewChunk {
                    id: "c0".into(),
                    ord: 0,
                    text: "alpha".into(),
                }],
            )
            .unwrap();
        store
    }

    #[test]
    fn a_fresh_chunk_starts_pending_at_revision_one() {
        let store = seeded();
        assert_eq!(
            store.chunk_index_state("c0").unwrap(),
            Some((ChunkIndexState::Pending, 1))
        );
        assert_eq!(store.chunk_index_state("missing").unwrap(), None);
    }

    #[test]
    fn an_unknown_index_state_on_disk_is_a_typed_error_not_a_default() {
        // A row this build cannot classify must never be silently treated as
        // Pending: that would re-add a row Lance may already hold.
        let store = seeded();
        store
            .conn()
            .execute("UPDATE chunks SET index_state = 99", [])
            .unwrap();
        assert!(matches!(
            store.pending_chunks(10).unwrap_err(),
            StoreError::UnknownDiscriminant(_)
        ));
        assert!(matches!(
            store.continuity_candidates(0, 8).unwrap_err(),
            StoreError::UnknownDiscriminant(_)
        ));
        assert!(matches!(
            store.chunk_index_state("c0").unwrap_err(),
            StoreError::UnknownDiscriminant(_)
        ));
    }

    #[test]
    fn the_stamp_only_confirms_the_revision_that_was_embedded() {
        let mut store = seeded();
        let embedded = vec![("c0".to_string(), 1_i64)];
        let candidate = store.continuity_candidates(0, 8).unwrap().remove(0);
        store
            .merge_capture(&candidate, &record_for("r1", 2_000), "alpha beta")
            .unwrap();
        // The flush now tries to stamp revision 1, which is no longer current.
        let report = store.mark_chunks_indexed(&embedded, 5).unwrap();
        assert_eq!(report.indexed, 0);
        assert_eq!(report.superseded, 1);
        assert_eq!(
            store.chunk_index_state("c0").unwrap().unwrap().0,
            ChunkIndexState::Superseded
        );

        // Stamping the revision that is actually current does confirm it.
        let report = store
            .mark_chunks_indexed(&[("c0".to_string(), 2)], 6)
            .unwrap();
        assert_eq!(report.indexed, 1);
        assert_eq!(report.superseded, 0);
    }

    #[test]
    fn a_rebuild_reset_forgets_every_claim_about_lance() {
        let mut store = seeded();
        store
            .mark_chunks_indexed(&[("c0".to_string(), 1)], 5)
            .unwrap();
        assert_eq!(
            store.chunk_index_state("c0").unwrap().unwrap().0,
            ChunkIndexState::Indexed
        );
        assert_eq!(store.reset_flush_state().unwrap(), 1);
        assert_eq!(
            store.chunk_index_state("c0").unwrap().unwrap().0,
            ChunkIndexState::Pending
        );
    }
}
