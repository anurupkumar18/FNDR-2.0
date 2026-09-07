# 2026-09-06: fndr.remember_decision, the only write tool

## Decision

ADR-007's single write tool now exists. `fndr.remember_decision` appends one
row to schema v1's `decision_ledger` table, which has been in the migration
since T-201 with no reader or writer until now. `Store::remember_decision`
is the one new engine function: an append-only insert returning the new
rowid. No new table, migration, or lifecycle state was needed.

## What is verified

`fndr-store` unit tests cover an append with no cited record, an append
citing a record, and the append-only property (two calls produce two
distinct rows, never an overwrite). The MCP test covers the tool's schema
round trip and proves an empty or whitespace-only statement is rejected with
`invalid_params` before any write happens. Auth is not re-tested per tool
because it is enforced in the transport middleware ahead of every handler;
`mcp_rejects_unauthenticated_loopback` already pins that for the whole
surface.

`docs/mcp.md` now exists and carries the per-tool contract ADR-007's
tool-addition rule requires, documenting all three implemented tools and
listing the eleven that are not.

## Explicitly not done

No per-tool rate limit: `fndr-mcp::auth::RateWindow` is still one global
window, so ADR-007's "per-tool rate limits" goal stays open and is recorded
as such in both `docs/mcp.md` and the T-702 ledger row. No reader for the
ledger either: `fndr.recall` and the `fndr://recent-decisions` resource are
still unstarted, so entries are write-only from MCP's side today.

## Landmines

The ledger is append-only by contract, not just by convention. Anything
that later edits or deletes a `decision_ledger` row (including a deletion
sweep) must be a deliberate, separately reviewed decision; the FK is
`ON DELETE SET NULL` precisely so deleting a cited record does not erase the
decision that referenced it.
