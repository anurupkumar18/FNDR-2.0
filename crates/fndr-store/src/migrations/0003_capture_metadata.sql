-- Capture metadata needed for safe URL-only records and later deletion filters.
-- URL values reach this column only through fndr-privacy's sanitizing write
-- path, which strips credentials, query strings, and fragments.
ALTER TABLE memory_records ADD COLUMN bundle_id TEXT;
ALTER TABLE memory_records ADD COLUMN url TEXT;

CREATE INDEX idx_records_url ON memory_records (url) WHERE url IS NOT NULL;
