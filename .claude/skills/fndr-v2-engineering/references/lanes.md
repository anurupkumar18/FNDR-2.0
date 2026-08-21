# Lanes, boundaries, and contracts

Four lanes own disjoint crates; contracts are the only crossing points. When a task spans lanes, change the contract first (types, schema, tool definition), get the owning lane's review, then implement both sides.

## Crate ownership map

| Lane | Owns | Consumes |
|---|---|---|
| ml | `fndr-inference`, `fndr-memory`, `fndr-retrieval` (co-owned with backend), `fndr-bench` | store API, types |
| backend | `fndr-types`, `fndr-textsignal`, `fndr-capture`, `fndr-ocr`, `fndr-privacy`, `fndr-store`, `fndr-graph`, `fndr-mcp` | inference API (model queue), types |
| frontend | `ui/` | generated IPC types, engine API via shell commands, tokens |
| platform | `fndr-shell`, `apps/helper` (Swift), `fndr-companion` (contract), `fndr-downloader`, `fndr-updater`, CI, release | everything, at arm's length |

## The contracts

### Engine API (backend owns)
The Rust functions `fndr-shell` and `fndr-mcp` call. Typed errors; no stringly results. Rule: the UI and MCP must be servable by the same function; surface-specific shaping happens in composition, not scoring or storage.

### Generated IPC types (backend produces, frontend consumes)
specta/tauri-specta generation runs at build time. Hand-written TS mirrors are banned by lint. If the frontend needs a field, the change starts in `fndr-types`.

### Events (backend to frontend)
Push-only for always-on state: emit on change with fingerprint suppression. Adding a poller for always-on state is a review-blocking defect (v1 ADR-011's half-finished migration is the cautionary tale). Rarely-running progress states may poll while visible.

### MCP contract (backend owns, ml feeds quality)
The canonical tool set lives in ADR-007's table, the single inventory of record (14 founding tools plus ratified P1 additions). Adding a tool needs: a use case no existing tool covers, an ADR-007 amendment, a schema round-trip test, an auth-failure test, a rate limit, and a `docs/mcp.md` entry. Removing or renaming bumps the manifest version.

### Sidecar protocol (platform owns)
Versioned JSON over stdio to `apps/helper`. Engine treats the sidecar as optional: absence is a typed unavailable state (meetings degrade visibly, nothing else is affected). Protocol changes need fixture-replay tests on both sides.

### Bench interface (ml owns)
`make bench` on the corpus dir produces the metrics file. Baselines are committed; CI compares. Anyone may run it; only ml changes what it measures, and such changes are announced (they re-baseline everyone).

## Per-lane review checklists

**ml reviews ask:** is the change measured (bench delta present)? Does every heuristic carry attribution? Is model residency budgeted (load/unload, priority)? Is the prompt ported byte-identical or the change called out? Is anything trained/tuned on the eval set (leakage)?

**backend reviews ask:** does SQLite stay the only truth (Lance written solely via the flush writer)? Are deletes routed through deletion-everywhere? Are new columns in the migration with tests? Do errors stay typed end to end? Is the capture hot path free of blocking syscalls and direct LLM calls?

**frontend reviews ask:** generated types only? Tokens only (no raw hex; the lint should catch it)? Built from the component library, with no hand-rolled buttons, panels, or inputs, and reviewed against the design language spec (the anti-slop bar: consistent type scale and spacing, no gradient soup, no boxes-in-containers)? Push events, not polls? Does the component consume the engine API rather than duplicating derivation logic client-side? Are the v1-ported interaction contracts (omnibar keyboard model, lifecycle chips) preserved?

**platform reviews ask:** does the change alter the signing identity, entitlements, or permission surface (TCC re-grant risk)? Do CI gates still cover it? Is the sidecar lifecycle supervised (restart, health)? Does the release pipeline remain one-tag-push?

## Cross-lane change protocol

1. Open the contract change (types/schema/tool def) as its own commit or PR; tag the owning lane.
2. Land both sides behind the contract within the same milestone; a contract with only one side implemented does not ship enabled (no decorative plumbing).
3. If the contract change is breaking, the PR names every consumer and updates them in the same change or links the tickets that will.
