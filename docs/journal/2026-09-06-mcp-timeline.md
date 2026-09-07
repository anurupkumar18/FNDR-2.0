# 2026-09-06: fndr.timeline and the UTC-offset question

## Decision

`fndr.timeline` answers "which apps was I in, when" from data that already
existed: `memory_records` carries `captured_at_ms` and `app_name` and is
already indexed on capture time. `Store::activity_buckets` is the one new
engine read, grouping records per app per hour or day bucket. The tool
returns counts only; no capture text passes through it, and a test
serializes the whole response and asserts the stored string is absent.

## What is verified

Bucket boundaries follow a caller-supplied `utc_offset_minutes` rather than
UTC, because a UTC-aligned day answers "what did I do yesterday" wrongly for
most of the world. A record at 23:30 UTC lands in local day 0 for a caller
at UTC-5 and local day 1 for a caller at UTC+9; both cases are pinned by
test. `bucket_start_ms` is returned as an absolute instant already corrected
for that offset, so no caller re-derives boundaries and gets a different
answer than the store did.

Refusals are typed: `to_ms` before `from_ms`, and an offset outside
`-720..=840`, are both `invalid_params` rather than a silently empty
timeline. `truncated` is set when `limit` clipped the result, so a partial
timeline cannot be read as a complete one.

Writing the first offset test, I asserted local midnight at UTC-5 was five
hours *before* the epoch boundary rather than five hours after. The
implementation was right and the test expectation was wrong; the failure
caught a sign error in my own reasoning, which is the argument for pinning
timezone behavior with concrete instants instead of prose.

## Explicitly not done

ADR-007's flexible `time_window` convention (shorthand strings like `"today"`
or `"7d"`, unix ms, or a from/to object) is not implemented. This tool takes
explicit unix-ms bounds. Building the shared parser for exactly one caller
would be speculative; it lands when the second tool needs it, and both
`docs/mcp.md` and the T-702 ledger row say so rather than implying the
convention exists.

Also not done: session- and project-granularity grouping (ADR-007 names
session/hour/day/app/project). Sessions have IDs but no lifecycle owner yet
(T-307's remaining half), and projects have no data model at all, so
offering those grains would return empty groupings that read as "no
activity" rather than "not implemented".

## Landmines

`activity_buckets` does integer division on `captured_at_ms`, which is sound
only because capture timestamps are positive epoch values. If a synthetic or
pre-1970 timestamp ever reaches this table, bucket alignment for negative
values would need explicit floor-division semantics.
