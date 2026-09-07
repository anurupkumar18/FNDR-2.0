-- Owner feedback on a surfaced result (ADR-007's `fndr.feedback`: "logged,
-- never silently mutates ranking"). Nothing reads this into a ranker; it is
-- an evidence trail for deliberate, bench-gated ranking work later.
--
-- This table intentionally stores the query text, unlike `mcp_audit` which
-- refuses to. Feedback without the query it was given for cannot be replayed
-- as an eval case, which is the only reason to collect it. The owner also
-- initiates each row explicitly, rather than it accruing from background
-- capture.
CREATE TABLE result_feedback (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    at_ms INTEGER NOT NULL,
    -- SET NULL, not CASCADE: deleting a memory must not erase the fact that
    -- its surfacing was rated, or deletion would quietly rewrite eval history.
    record_id TEXT REFERENCES memory_records (id) ON DELETE SET NULL,
    chunk_id TEXT,
    query TEXT NOT NULL DEFAULT '',
    rating TEXT NOT NULL,
    note TEXT NOT NULL DEFAULT ''
);

CREATE INDEX idx_result_feedback_at ON result_feedback (at_ms);
