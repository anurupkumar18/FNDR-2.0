-- T-505's first real keyword route. Chunks remain SQLite truth; this FTS5
-- table is an external-content index kept current by the same transaction
-- that writes or deletes chunk rows. Porter stemming is deliberate: the
-- benchmark's "index" versus "indexes" regression must not return.
CREATE VIRTUAL TABLE chunks_fts USING fts5(
    text,
    content='chunks',
    content_rowid='rowid',
    tokenize='porter unicode61'
);

CREATE TRIGGER chunks_fts_insert AFTER INSERT ON chunks BEGIN
    INSERT INTO chunks_fts(rowid, text) VALUES (new.rowid, new.text);
END;

CREATE TRIGGER chunks_fts_delete AFTER DELETE ON chunks BEGIN
    INSERT INTO chunks_fts(chunks_fts, rowid, text)
    VALUES ('delete', old.rowid, old.text);
END;

CREATE TRIGGER chunks_fts_update AFTER UPDATE OF text ON chunks BEGIN
    INSERT INTO chunks_fts(chunks_fts, rowid, text)
    VALUES ('delete', old.rowid, old.text);
    INSERT INTO chunks_fts(rowid, text) VALUES (new.rowid, new.text);
END;

-- Populate an existing v3 vault as part of its forward-only migration.
INSERT INTO chunks_fts(rowid, text) SELECT rowid, text FROM chunks;
