-- The local, durable audit log of MCP tool calls (ADR-007: "an audit log of
-- tool calls (local only)"). Denials are already logged through tracing; this
-- table is what a person can actually be shown, so T-902's trust moment has
-- something to open.
--
-- Deliberately narrow: tool name, outcome, and whether raw capture text was
-- released. Never the query string, record id, or any capture content. An
-- audit log that copies what it audits becomes a second store of the same
-- sensitive text, which is the opposite of the point.
CREATE TABLE mcp_audit (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    at_ms INTEGER NOT NULL,
    tool TEXT NOT NULL,
    outcome TEXT NOT NULL,
    raw_released INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX idx_mcp_audit_at ON mcp_audit (at_ms);
