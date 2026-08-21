# ADR-007: MCP surface: canonical tool set, auth-always transport

**Status:** Proposed
**Date:** 2026-08-19
**Deciders:** FNDR v2 team (4)

## Context

MCP is FNDR's headline interface. The POC shipped 51 tools across four naming conventions with at least four duplicate groups, a 5,197-line single-file server, an SSE session layer that never pushed messages, and a default posture in which any web page could read the entire memory store over loopback (auth off by default, `allow_origin(Any)` in every mode) while the companion server let any LAN host mint a full-permission token. Those defaults are being patched in v1 separately; v2 designs them out.

## Decision

**Transport:** the official MCP Rust SDK (Tier 2), streamable HTTP on the current stateless spec (2026-07-28). No legacy SSE compatibility layer. Loopback bind by default; `tunnel` and `public` modes exist but change only bind and documentation, never auth posture.

**Security posture, all modes:** bearer token required unconditionally (constant-time compare); strict `Origin` and `Host` validation (DNS-rebinding defense) with an explicit allowlist, no wildcard CORS; per-client scoped tokens with revocation; per-tool rate limits; an audit log of tool calls (local only); discovery file written with owner-only permissions and no secrets beyond the token reference; TLS optional for loopback, required for non-loopback.

**Tool surface: 14 canonical tools, one `fndr.` namespace,** deduplicated from the POC's vocabulary (concept provenance in parentheses):

| Tool | Purpose |
|---|---|
| `fndr.search` | Hybrid memory search; filters for time window, app, project; returns cards with surfacing reasons (merges 4 POC search tools) |
| `fndr.context_pack` | The headline: budgeted, cited context for a goal/topic, with `depth` and `token_budget` (merges 4 POC pack tools) |
| `fndr.delta` | Changes since a session timestamp, for cheap repeated calls (POC `fndr_diff`) |
| `fndr.timeline` | Grouped chronological activity (session/hour/day/app/project granularity) |
| `fndr.active_focus` | Current app/window/project/task inference |
| `fndr.project_context` | Per-project summary, files, errors, decisions, todos |
| `fndr.recall` | Decisions, errors, blockers, todos in one tool with a `kind` parameter (merges 4 POC tools) |
| `fndr.source_evidence` | Evidence for a card or memory; raw text behind an explicit `include_raw` gate |
| `fndr.graph_context` | Bounded typed-graph neighborhood for a project or node |
| `fndr.open_target` | Resolve a memory to a URL / file / app reopen target |
| `fndr.explain_retrieval` | Why results surfaced, what was dropped or redacted |
| `fndr.feedback` | Rate a result (logged, never silently mutates ranking) |
| `fndr.privacy_status` | Capture/redaction/auth posture, blocklist count |
| `fndr.remember_decision` | The only write tool: append to the decision ledger |

Carried conventions: the flexible `time_window` schema (shorthand string, unix ms, or from/to object), the `content` plus `structuredContent` result envelope, the three `fndr://` resources (privacy settings, open todos, recent decisions), and the prompt-template registry with real per-prompt argument schemas.

**Ratified P1 additions (amendment 2026-08-20):** two tools beyond the 14 founding members are ratified and follow the tool-addition rule (use case, schema round-trip test, auth-failure test, rate limit, docs entry): `fndr.answer` (grounded Q&A layered on `fndr.context_pack`, per-claim citation checks, three-state verdict; T-711, month 5) and `fndr.session_story` (cited narrative reconstruction of a captured work session; T-709, month 5). Wherever a tool count appears in other documents it means "14 founding plus ratified P1 additions"; this ADR's table is the single inventory of record.

**Versioning:** the tool contract is documented in-repo (`docs/mcp.md` v2) and versioned; breaking changes bump a manifest version surfaced in `initialize`.

## Options considered

**A (chosen): 14 canonical tools, hardened single posture.** Small enough for an agent to reason about (tool-selection quality degrades with redundant tools), large enough to cover every POC use case; one auth story.

**B: port the POC's 51-tool surface and harden it.** Rejected: four naming conventions and duplicate groups measurably confuse tool-selecting agents, and every extra tool is contract surface to maintain.

**C: fewer, mega-parameterized tools (3 to 5).** Rejected: overloaded parameter unions are worse for agent tool selection than well-named single-purpose tools, and per-tool rate/permission scoping becomes impossible.

**D: local socket transport only (no HTTP).** Simplest security story but excludes tunnel/remote use cases the product explicitly wants (locked decision to keep deployment modes). Rejected; loopback-default HTTP with auth-always achieves the same effective posture.

## Trade-off analysis

The main tension is capability breadth vs agent usability and security surface. Deduplicating to 14 named tools keeps the agent's tool-selection problem tractable, keeps rate/permission scoping per-tool, and keeps the contract documentable on one page, at the cost of a migration mapping for any v1 user (published in `docs/mcp.md`).

## Consequences

- Easier: agent integration quality (fewer, clearer tools), security review (one posture), contract testing (each tool gets schema round-trip and auth-failure tests).
- Harder: tunnel/public modes need real setup docs since auth is never bypassable.
- Revisit: server-initiated notifications (resource subscriptions) once a real client needs them; scoped multi-client permissions when more than one agent class connects routinely.

## Action items

1. [ ] MCP crate on the official Rust SDK with auth middleware, origin/host validation, and the audit log (month 2, first 8 tools).
2. [ ] Remaining tools plus resources and prompts (month 3).
3. [ ] Contract doc with per-tool schemas and the v1-to-v2 tool mapping (month 3).
4. [ ] Connect-your-agent onboarding step with Claude Desktop/Code snippets (month 3).
