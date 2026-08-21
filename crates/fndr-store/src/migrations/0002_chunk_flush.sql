-- T-202: chunk flush tracking. A chunk with flushed_at_ms = 0 is pending for
-- the Lance writer; it is stamped only after a successful Lance commit, so a
-- failed flush retries by construction.
ALTER TABLE chunks ADD COLUMN flushed_at_ms INTEGER NOT NULL DEFAULT 0;
CREATE INDEX idx_chunks_unflushed ON chunks (flushed_at_ms) WHERE flushed_at_ms = 0;
