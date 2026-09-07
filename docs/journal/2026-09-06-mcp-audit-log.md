# 2026-09-06: the MCP audit log, made structural

## Decision

ADR-007's action item 1 pairs the first eight tools with a tool-call audit
log. The eight tools shipped today and the audit log did not, so I wrote
that down in an ADR amendment and then built it.

Migration 0005 adds `mcp_audit`: when, which tool, whether it succeeded or
was refused, and whether it released raw capture text. Nothing else. No
query string, no record id, no capture content — an audit log that copies
what it audits becomes a second store of the same sensitive text, which is
the opposite of the point.

## Structural, not disciplined

The first instinct was to call an audit helper at the end of each handler.
That is exactly the design where someone later adds a ninth tool, forgets
the call, and nothing complains — and a missing audit entry is invisible by
construction, unlike a missing feature.

So each `#[tool]` method is now a thin wrapper that routes its result
through `FndrMcpServer::audit`, with the real body in a private `_inner`.
No return path, success or refusal, can skip the log. On top of that, a
test compares the set of tools that wrote audit entries against
`FndrMcpServer::tool_router().list_all()`, so adding a tool without an
audit wrapper fails the suite with a message saying so.

## A design I backed out of

The first version of `audit` inferred `raw_released` from an `AtomicBool`
on the server, set by `source_evidence` before returning. That is a race:
two concurrent evidence calls, one gated and one not, could attribute the
raw release to the wrong entry — in the one log whose entire purpose is
being trustworthy about raw releases. The flag is now passed explicitly
from the wrapper, which reads `include_raw` from its own parameters.

## What is verified

The set-equality test above. Plus: three `source_evidence` calls — one
releasing raw text, one withholding it, one refused — produce three audit
entries, exactly one flagged `raw_released`, exactly one `refused`. Store
tests cover newest-first ordering and the flag round-tripping.

## Known wart

A failed audit write fails the call. For `fndr.remember_decision` that
means an appended decision can be reported as an error, and a retry appends
a second ledger entry. That is the deliberate trade: a duplicate ledger row
is visible to its owner, and an unaudited write is not.

## Explicitly not done

Retention: `mcp_audit` grows without bound and no policy prunes it (T-207
owns retention generally). No UI surface either — `recent_tool_calls` is a
plain method, deliberately not an MCP tool, since the audit log is for the
person who owns the machine and not for the agents it audits. T-902's
"one-click audit log" trust moment now has something real to open.
