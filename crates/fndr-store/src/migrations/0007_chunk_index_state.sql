-- T-307: the Lance-safe indexed-record merge/update protocol.
--
-- `flushed_at_ms` alone could only answer "has this chunk ever reached
-- Lance", which is why continuity merging was restricted to chunks SQLite
-- still owned. Two columns replace that binary with a three-state lifecycle
-- plus an optimistic-concurrency token:
--
--   index_state (fndr_types::ChunkIndexState discriminants)
--     0 Pending     no Lance row for this chunk id may be assumed to exist
--     1 Indexed     a Lance row exists and matches this chunk's text
--     2 Superseded  a Lance row may exist and does NOT match; it must be
--                   deleted by chunk id before a fresh row is added
--
--   revision  bumped on every text change. The flush embeds a revision and
--             stamps only that revision as Indexed; a merge that lands mid
--             flush leaves the chunk Superseded instead of falsely Indexed.
--
-- Backfill: an already-flushed chunk has a Lance row that matches its text,
-- so it is Indexed; everything else stays Pending.
ALTER TABLE chunks ADD COLUMN index_state INTEGER NOT NULL DEFAULT 0;
ALTER TABLE chunks ADD COLUMN revision INTEGER NOT NULL DEFAULT 1;

UPDATE chunks SET index_state = 1 WHERE flushed_at_ms != 0;

-- The flush queue is now "anything not Indexed", so the partial index
-- follows that predicate rather than the old flushed_at_ms = 0 one.
DROP INDEX IF EXISTS idx_chunks_unflushed;
CREATE INDEX idx_chunks_index_work ON chunks (index_state) WHERE index_state != 1;
