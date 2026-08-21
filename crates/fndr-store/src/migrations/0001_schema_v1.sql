-- Schema v1 (T-201), the ARCHITECTURE section 5 domains.
-- Rules: split memory schema (facts, derived text, scores separate); vectors
-- live only in Lance; lifecycle columns hold fndr-types discriminants;
-- FKs are real and cascade so deletion-everywhere (T-206) has one spine.

-- Memory: capture facts only. Derived text and scores live in their own
-- tables (the v1 104-field mixed table is the anti-pattern).
CREATE TABLE memory_records (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL DEFAULT '',
    source TEXT NOT NULL,
    app_name TEXT NOT NULL DEFAULT '',
    window_title TEXT NOT NULL DEFAULT '',
    captured_at_ms INTEGER NOT NULL,
    created_at_ms INTEGER NOT NULL,
    lifecycle INTEGER NOT NULL DEFAULT 0,
    reviewed_at_ms INTEGER NOT NULL DEFAULT 0,
    reviewer_generation INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_records_captured_at ON memory_records (captured_at_ms);
CREATE INDEX idx_records_session ON memory_records (session_id);
CREATE INDEX idx_records_lifecycle ON memory_records (lifecycle);

CREATE TABLE memory_texts (
    record_id TEXT PRIMARY KEY
        REFERENCES memory_records (id) ON DELETE CASCADE,
    raw_text TEXT NOT NULL DEFAULT '',
    cleaned_text TEXT NOT NULL DEFAULT '',
    summary TEXT NOT NULL DEFAULT '',
    embedding_text TEXT NOT NULL DEFAULT '',
    updated_at_ms INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE memory_scores (
    record_id TEXT NOT NULL
        REFERENCES memory_records (id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    value REAL NOT NULL,
    PRIMARY KEY (record_id, name)
);

-- Chunks: text plus spans. Vectors live only in Lance (ADR-002).
CREATE TABLE chunks (
    id TEXT PRIMARY KEY,
    record_id TEXT NOT NULL
        REFERENCES memory_records (id) ON DELETE CASCADE,
    ord INTEGER NOT NULL,
    text TEXT NOT NULL,
    span_start INTEGER NOT NULL DEFAULT 0,
    span_end INTEGER NOT NULL DEFAULT 0,
    UNIQUE (record_id, ord)
);
CREATE INDEX idx_chunks_record ON chunks (record_id);

-- Graph: typed nodes and edges, FK-checked (kind discriminants arrive with
-- the fndr-graph taxonomy port, T-1101).
CREATE TABLE graph_nodes (
    id TEXT PRIMARY KEY,
    kind INTEGER NOT NULL,
    name TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL
);
CREATE INDEX idx_nodes_kind_name ON graph_nodes (kind, name);

CREATE TABLE graph_edges (
    src TEXT NOT NULL REFERENCES graph_nodes (id) ON DELETE CASCADE,
    dst TEXT NOT NULL REFERENCES graph_nodes (id) ON DELETE CASCADE,
    kind INTEGER NOT NULL,
    weight REAL NOT NULL DEFAULT 1.0,
    created_at_ms INTEGER NOT NULL,
    PRIMARY KEY (src, dst, kind)
);
CREATE INDEX idx_edges_dst ON graph_edges (dst);

CREATE TABLE node_mentions (
    node_id TEXT NOT NULL REFERENCES graph_nodes (id) ON DELETE CASCADE,
    record_id TEXT NOT NULL REFERENCES memory_records (id) ON DELETE CASCADE,
    PRIMARY KEY (node_id, record_id)
);
CREATE INDEX idx_mentions_record ON node_mentions (record_id);

CREATE TABLE entity_aliases (
    node_id TEXT NOT NULL REFERENCES graph_nodes (id) ON DELETE CASCADE,
    alias TEXT NOT NULL,
    PRIMARY KEY (node_id, alias)
);
CREATE INDEX idx_aliases_alias ON entity_aliases (alias);

-- Tasks.
CREATE TABLE tasks (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    status INTEGER NOT NULL DEFAULT 0,
    source_record_id TEXT
        REFERENCES memory_records (id) ON DELETE SET NULL,
    due_at_ms INTEGER NOT NULL DEFAULT 0,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);
CREATE INDEX idx_tasks_status ON tasks (status);

-- Meetings.
CREATE TABLE meetings (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL DEFAULT '',
    started_at_ms INTEGER NOT NULL,
    ended_at_ms INTEGER NOT NULL DEFAULT 0,
    created_at_ms INTEGER NOT NULL
);

CREATE TABLE meeting_segments (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    meeting_id TEXT NOT NULL
        REFERENCES meetings (id) ON DELETE CASCADE,
    started_at_ms INTEGER NOT NULL,
    ended_at_ms INTEGER NOT NULL DEFAULT 0,
    speaker TEXT NOT NULL DEFAULT '',
    text TEXT NOT NULL DEFAULT ''
);
CREATE INDEX idx_segments_meeting ON meeting_segments (meeting_id);

-- Decision ledger (append-only surface for "what did we decide").
CREATE TABLE decision_ledger (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    decided_at_ms INTEGER NOT NULL,
    statement TEXT NOT NULL,
    record_id TEXT
        REFERENCES memory_records (id) ON DELETE SET NULL
);

-- Durable review queue (replaces the v1 in-memory VecDeque; a batch survives
-- shutdown).
CREATE TABLE review_queue (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    record_id TEXT NOT NULL UNIQUE
        REFERENCES memory_records (id) ON DELETE CASCADE,
    enqueued_at_ms INTEGER NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    next_attempt_at_ms INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_review_next_attempt ON review_queue (next_attempt_at_ms);

-- App state.
CREATE TABLE settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

CREATE TABLE devices (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL DEFAULT '',
    paired_at_ms INTEGER NOT NULL,
    last_seen_ms INTEGER NOT NULL DEFAULT 0
);

-- Tokens are stored hashed, never raw (ADR-007).
CREATE TABLE tokens (
    id TEXT PRIMARY KEY,
    kind INTEGER NOT NULL,
    hash TEXT NOT NULL,
    device_id TEXT
        REFERENCES devices (id) ON DELETE CASCADE,
    created_at_ms INTEGER NOT NULL,
    last_used_ms INTEGER NOT NULL DEFAULT 0
);
