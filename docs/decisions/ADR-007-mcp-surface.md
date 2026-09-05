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

## Amendment (2026-08-21, walking skeleton validation)

The `fndr.` dotted tool namespace is confirmed legal per the MCP tool-name specification (letters, digits, underscore, dash, dot; validated in the official Rust SDK), so the inventory keeps its names unchanged. The auth-always posture shipped with the first tool: bearer with constant-time compare, Host and Origin allowlists, a global rate limit, uniform deny bodies with audit logging, and the named regression tests (`mcp_rejects_unauthenticated_loopback`, web-origin rejection with a valid token) exercising the real socket with raw HTTP. Note for T-701: rmcp 3.1's streamable HTTP server carries its own Host/Origin allowlist configuration underneath our middleware; keep both layers (defense in depth).

## Amendment (2026-09-05, Connected Planner contract)

ADR-008 adds an experimental Connected Planner mode without changing the 14
founding-tool count, the ratified P1 additions, or the auth-always posture.
It is disabled by default and is not an app-owned provider integration.

**Runtime skills are resources, not tools.** `fndr://runtime-skills` lists
locally validated skill metadata; `fndr://runtime-skills/{skill_id}` returns
the exact reviewed `SKILL.md` content and declared capability ids. The
resource response never includes credentials, hidden instructions, capture
payloads, or executable code. Listing and reading require the
`planner.read_skills` token scope, an explicit rate limit, and an audit event.
Skill resources are unavailable while Connected Planner is disabled.

**One experimental proposal tool:** `fndr.propose_action` accepts a planner's
structured `ActionProposal` and returns either a locally stored, owner-visible
proposal identifier or a typed refusal. It cannot execute any capability. The
schema requires `capability_id`, canonical arguments, rationale, and evidence
citations; it returns verification state, risk label, and the requirement for
a separate action approval. In alpha, the only accepted identifiers are
`memory.open_target` and `git.status.short` as defined in ADR-008. This tool
requires `planner.propose_action`, has a per-tool rate limit, writes an audit
event, rejects unknown or malformed input before persistence, and has schema
round-trip and auth-failure tests.

**Context-pack delivery is explicit.** `fndr.context_pack` remains local by
default. A future `delivery: "planner_export"` request may only create a
`PlannerExportDraft`, never send it. It requires a `planner.export` scope,
the destination label, all normal citation and token-budget behavior, and the
ADR-004 preview/one-time approval flow. `include_raw` remains an independent
explicit gate. A changed draft digest, destination, policy version, or expiry
must return a typed refusal rather than a new implicit approval.

The implementation PR must add `docs/mcp.md` examples that execute against a
dev server and prove: disabled mode hides planner resources; unauthenticated
or wrongly scoped calls fail; draft approval cannot execute an action; and
the two alpha capability ids reject arguments outside their narrow contracts.
