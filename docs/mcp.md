# FNDR MCP surface (v2)

The canonical tool set and its rationale live in
[ADR-007](decisions/ADR-007-mcp-surface.md) (14 founding tools plus ratified
P1 additions). This document is the per-tool contract for tools that are
actually implemented; it grows one entry per tool as each lands, per
ADR-007's tool-addition rule (use case, schema round-trip test,
auth-failure test, rate limit, docs entry).

Transport, auth, and posture (bearer token, Origin/Host allowlist, rate
limiting) are one crate-wide concern implemented in `fndr-mcp::auth` and
enforced for every tool before any handler runs; see ADR-007's amendments
for the regression tests that pin this. Today's rate limiting is a single
global window (`fndr_mcp::auth::RateWindow`), not yet scoped per tool —
ADR-007's "per-tool rate limits" goal is still open.

## Implemented tools

### `fndr.search`

Full-text keyword search over durable capture chunks, via
`fndr-retrieval::KeywordRetriever` over `fndr-store::Store` (T-505, wired
into MCP by T-702). This is a plain FTS5 keyword route, not ADR-007's full
`fndr.search` contract (no hybrid ranking, no time-window/app filters, no
surfacing reasons yet).

**Params:** `{ query: string, limit?: number }` (default 10, capped at 50).

**Result:** `{ hits: [{ record_id, chunk_id, source, captured_at_ms, snippet }] }`.
`record_id`/`chunk_id` are the durable, stable IDs also used by deletion and
future evidence/citation tools.

### `fndr.privacy_status`

Reports the local privacy posture without exposing blocklist entries.

**Params:** `{}`.

**Result:** `{ local_default, planner_enabled, configured_blocked_apps, configured_blocked_domains, raw_pixels_persisted }`.

### `fndr.remember_decision`

The only write tool. Appends one row to `fndr-store::Store`'s append-only
`decision_ledger` table; never edits or removes a prior entry, and never
touches ranking.

**Params:** `{ statement: string, record_id?: string, decided_at_ms?: number }`.
`decided_at_ms` defaults to the current time. An empty or whitespace-only
`statement` is rejected (`invalid_params`) before any write.

**Result:** `{ id: number, decided_at_ms: number }`. `id` is the new
`decision_ledger` row's rowid.

## Not yet implemented

`fndr.context_pack`, `fndr.delta`, `fndr.timeline`, `fndr.active_focus`,
`fndr.project_context`, `fndr.recall`, `fndr.source_evidence`,
`fndr.graph_context`, `fndr.open_target`, `fndr.explain_retrieval`,
`fndr.feedback`, and the ratified P1 additions `fndr.answer` and
`fndr.session_story`. See ADR-007 for each tool's purpose and the
Connected Planner amendment for `fndr.propose_action`.
