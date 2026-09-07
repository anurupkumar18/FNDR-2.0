# FNDR MCP surface (v2)

The canonical tool set and its rationale live in
[ADR-007](decisions/ADR-007-mcp-surface.md) (14 founding tools plus ratified
P1 additions). This document is the per-tool contract for tools that are
actually implemented; it grows one entry per tool as each lands, per
ADR-007's tool-addition rule (use case, schema round-trip test,
auth-failure test, rate limit, docs entry). Nine of the fourteen are
implemented today.

Transport, auth, and posture (bearer token, Origin/Host allowlist, rate
limiting) are one crate-wide concern implemented in `fndr-mcp::auth` and
enforced for every tool before any handler runs; see ADR-007's amendments
for the regression tests that pin this. Today's rate limiting is a single
global window (`fndr_mcp::auth::RateWindow`), not yet scoped per tool —
ADR-007's "per-tool rate limits" goal is still open.

## The audit log

Every tool call is recorded locally in SQLite's `mcp_audit` table: when,
which tool, whether it succeeded or was refused, and whether it released
raw capture text. Nothing else — no query string, no record id, no capture
content. An audit log that copies what it audits becomes a second store of
the same sensitive text.

Auditing is structural, not a convention: each `#[tool]` method is a thin
wrapper that routes its result through `FndrMcpServer::audit`, so no return
path can skip it, and a test asserts the set of tools that wrote audit
entries equals the set the router has registered. Adding a tool without
auditing it fails that test.

Read it with `FndrMcpServer::recent_tool_calls`. That is deliberately not
an MCP tool: the audit log is for the person who owns the machine, not for
the agents being audited by it.

Auth denials are logged separately, through `tracing` on the
`fndr_mcp::audit` target, because they are rejected before any handler and
therefore before any tool name is known.

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

### `fndr.timeline`

Grouped chronological activity over a window: which apps were active, in
which time buckets, and how many records each produced. Counts only; no
capture text ever crosses this tool.

**Params:** `{ from_ms: number, to_ms: number, granularity?: "hour" | "day", utc_offset_minutes?: number, limit?: number }`.
`granularity` defaults to `day`, `utc_offset_minutes` to `0`, `limit` to
200 (capped at 1000).

**Result:** `{ from_ms, to_ms, granularity, truncated, buckets: [{ bucket_start_ms, app_name, record_count }] }`.

`bucket_start_ms` is an absolute instant already corrected for
`utc_offset_minutes`, so a caller never re-derives boundaries; UTC-aligned
days answer "what did I do yesterday" wrongly outside UTC. `truncated` is
true when `limit` clipped the result, so a partial timeline is never read as
a complete one. `to_ms` before `from_ms`, or an offset outside
`-720..=840`, is a typed refusal.

ADR-007's flexible `time_window` shorthand (`"today"`, `"7d"`, from/to
object) is **not** implemented; this tool takes explicit unix-ms bounds.
The shared parser lands when a second tool needs it.

### `fndr.active_focus`

What the newest capture says someone was doing, with its age and whether
it is recent enough to still be called current.

**Params:** `{ stale_after_ms?: number }`, defaulting to five minutes —
`fndr-capture`'s deep-idle threshold, past which the sampler itself stops
believing the screen represents what someone is doing.

**Result:** `{ status, app_name?, window_title?, url?, bundle_id?, record_id?, captured_at_ms?, age_ms?, stale_after_ms }`.

`status` is `active`, `stale`, or `none`. It exists so a caller cannot
report a three-hour-old observation as what someone is "currently" doing:
`none` means nothing has ever been captured, and `stale` means there is an
observation but it is older than the caller's own tolerance. `age_ms`
makes that measurable rather than just labelled. A negative
`stale_after_ms` is a typed refusal.

Project and task inference, which ADR-007's entry also names, are not
implemented — neither has a data model.

### `fndr.delta`

What was captured since an instant: totals and the busiest apps. Built for
cheap repeated polling, so it carries counts only, never capture text.

**Params:** `{ since_ms: number, app_limit?: number }`. `app_limit`
defaults to 10, capped at 100.

**Result:** `{ since_ms, record_count, newest_captured_at_ms?, apps: [{ app_name, record_count }] }`.

`record_count` counts every record in the window regardless of how many
apps `app_limit` listed, so a capped app list never understates the total.
`newest_captured_at_ms` is absent when nothing was captured; otherwise feed
it back as the next call's `since_ms` to continue polling.

### `fndr.source_evidence`

The evidence behind one memory, resolved from a `record_id` that
`fndr.search` returned. Capture metadata and chunk shape are always
returned; the stored capture text is not.

**Params:** `{ record_id: string, include_raw?: boolean }`. `include_raw`
defaults to `false`.

**Result:** `{ record_id, session_id, source, app_name, bundle_id, url, window_title, captured_at_ms, raw_included, chunks: [{ chunk_id, ord, text_len, text? }] }`.

`text` is present on a chunk only when `include_raw` was explicitly `true`;
`text_len` is always present so a caller can judge a record's substance
without moving its content. `raw_included` echoes the gate's state so a
caller never infers it from an absent field. An unknown `record_id` is a
typed refusal (`invalid_params`), never an empty success.

### `fndr.open_target`

Resolve one memory to something reopenable, from the metadata the record
already retained.

**Params:** `{ record_id: string }`.

**Result:** `{ record_id, kind, url?, bundle_id?, app_name, window_title, reason? }`.

`kind` is `url` when the record kept a page URL, `app` when it kept only a
bundle identifier, and `unavailable` when it kept neither — in which case
`reason` says so rather than returning a blank target. Returned URLs are
the sanitized ones the write path stored: credentials, query strings, and
fragments never reached storage, so a reopened link is the page, not the
session. An unknown `record_id` is a typed refusal.

### `fndr.recall`

Recall structured knowledge by kind. Only `decision` has a data model
today, backed by the same `decision_ledger` `fndr.remember_decision`
writes.

**Params:** `{ kind: "decision" | "error" | "blocker" | "todo", since_ms?: number, limit?: number }`.
`limit` defaults to 20, capped at 200. `since_ms` is inclusive.

**Result:** `{ kind, decisions: [{ id, decided_at_ms, statement, record_id }] }`,
newest first.

`error`, `blocker`, and `todo` are **typed refusals** (`invalid_params`),
not empty lists. An empty list would be read by an agent as "nothing was
recorded", which is a silent degradation; a refusal says the kind is not
implemented. They become real answers when their data models exist.

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

`fndr.context_pack`, `fndr.project_context`, `fndr.graph_context`,
`fndr.explain_retrieval`, `fndr.feedback`, and the
ratified P1 additions `fndr.answer` and `fndr.session_story`. See ADR-007
for each tool's purpose and the Connected Planner amendment for
`fndr.propose_action`.
