# 2026-09-06: durable keyword retrieval slice

## Decision

T-505 starts with one low-RAM, durable keyword route. `Store` owns an
external-content SQLite FTS5 index over `chunks`; `fndr-retrieval` exposes
that evidence through `KeywordRetriever`. The index uses Porter stemming so
the existing index/indexes regression is covered without loading a model.

## What is verified

- Migration 0004 backfills existing vaults and trigger-maintains the FTS index
  for future chunk writes, edits, and foreign-key-cascade deletes.
- Search returns durable record/chunk IDs, source, capture time, and an FTS
  snippet; queries are normalized into quoted conjunctions rather than passed
  through as FTS syntax.
- A deletion-everywhere integration test proves an owner-deleted record no
  longer appears in this route, including when no Lance table exists.

## Explicitly not done

This is not Lance FTS, vector retrieval, temporal retrieval, hybrid/RRF,
reranking, context packing, an MCP tool, or a UI. It makes no claim about
retrieval quality beyond the tested keyword behavior.

## Landmines

- Never accept raw FTS query syntax at an external boundary; it changes query
  meaning and can become a parser-error surface.
- The FTS table is derived data, not another truth store. Keep all writes and
  owner deletion flowing through SQLite `chunks`.
- Do not add model loading to this route: its purpose is a useful fallback on
  an 8 GB laptop while vector work remains gated by benchmarks.

## Verification

`CARGO_BUILD_JOBS=1 cargo test -p fndr-store -p fndr-retrieval`, followed by
the serial workspace `make test` gate.
