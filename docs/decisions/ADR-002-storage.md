# ADR-002: Storage: SQLite system of record, LanceDB derived retrieval index

**Status:** Proposed
**Date:** 2026-08-19
**Deciders:** FNDR v2 team (4)

## Context

Workload: continuous ingestion of small structured records (one capture every few seconds when active), hybrid retrieval (vector similarity, BM25 lexical, metadata filters, typed-graph traversal), a few million rows per year, embedded in a desktop app on 8 to 16 GB machines, crash-safe across laptop sleep and power loss, Apache 2.0 compatible.

POC evidence (code audit): LanceDB 0.27 held 20 tables with **no vector, FTS, or scalar index anywhere**; every query and point lookup was a full scan without column projection; a 104-field memory table mixed capture facts, derived text, four vector columns, and lifecycle; eight tables hid their payload in opaque JSON strings; there was no compaction or version pruning; and `is_soft_deleted` was written but never filtered. Separately, research confirmed Lance's MVCC append-only commit model produces version and small-fragment sprawl under per-capture writes unless batched and pruned, and that compaction temporarily grows disk.

## Decision

Two stores with a strict role split:

- **SQLite (WAL mode) is the system of record.** It holds memory records (a normalized, split schema: capture facts, derived text, lifecycle, scores as separate tables), tasks, meetings and segments, the typed graph (nodes and edges tables, traversal via recursive CTEs and proper indexes), entity aliases, the decision ledger, the review queue (durable, replacing the POC's in-memory VecDeque), and app state. One file, one WAL, the best crash-safety story available.
- **LanceDB is a derived, rebuildable retrieval index.** It holds chunk and record embedding tables, image vectors, and Lance-native BM25 FTS over retrieval text, with metadata columns for prefiltering. It is populated by batched flush from SQLite (30 to 60 second cadence or batch-size threshold), carries real indexes (IVF_PQ or equivalent vector index, FTS index, scalar index on ids), and runs scheduled `optimize` plus version pruning. Any corruption or schema migration is handled by rebuild from SQLite, never data loss.

Deletion semantics: permanent deletes execute against SQLite and Lance in one operation, verified by test (PRD P0.9). Retention jobs operate on SQLite and trigger index deletes.

## Options considered

### Option A: SQLite + LanceDB (chosen)

| Dimension | Assessment |
|---|---|
| Crash safety | SQLite WAL for truth; Lance manifest commits are atomic, and rebuildability removes the remaining risk |
| Hybrid search | Lance-native BM25 (stress-tested upstream at 41M docs), RRF fusion, pluggable reranker trait, incremental index updates with flat-scan tail |
| Ops burden | Batched flush plus scheduled compact/prune; two stores but only two |
| Graph fit | SQLite recursive CTEs at a few million edges with indexes is milliseconds; no embedded vector store has a credible native graph |
| License | Apache 2.0 both |

**Pros:** drops the POC's separate Tantivy dependency (Lance FTS is native now); each store does what it is best at; the rebuild property converts the scariest failure mode into a maintenance task.
**Cons:** dual-write discipline (mitigated: one writer module owns the flush; Lance is never written except through it); eventual consistency between truth and index inside the flush window (acceptable: sub-minute).

### Option B: All-SQLite (sqlite-vec + FTS5 + graph tables)

**Pros:** one file, perfect crash story, trivially maintainable.
**Cons:** sqlite-vec stable releases are brute-force scan (~68 ms per 100k rows at 384d, extrapolating to multi-second at millions of rows and larger dims); its ANN work (IVF/DiskANN) is alpha after a year-long maintenance scare. Rejected as primary; remains the fallback if Lance becomes untenable, since the rebuild property makes swapping the index store cheap.

### Option C: LanceDB-only (POC shape)

**Cons:** the POC demonstrated the failure mode: no relational integrity, JSON-blob side tables, no durable queue, version sprawl under per-capture writes, and full-scan point lookups. An append-only MVCC store is the wrong system of record for a desktop app. Rejected.

### Option D: LadybugDB (Kuzu fork: graph + vector + FTS in one)

The only credible one-store answer, but the fork is under a year old. Watch; consider for the graph layer later if multi-hop traversal becomes core and SQLite CTEs run out.

### Option E: Qdrant Edge / usearch / DuckDB-VSS / embedded Postgres

Qdrant Edge is beta with API churn and still needs a relational store beside it. usearch means hand-rolled persistence and fusion. DuckDB-VSS has experimental HNSW persistence with documented crash-corruption risk and RAM-resident indexes. PGlite is alpha and single-connection. All rejected.

## Trade-off analysis

The core insight from both the audit and research: every embedded retrieval structure in 2026 is either derived-index-shaped or beta, so **architecting for rebuildability is the real crash-safety strategy.** Making SQLite the truth buys relational integrity, durable queues, and graph queries for free, and demotes Lance to a role it is excellent at (indexed hybrid retrieval) while neutralizing its weaknesses (version sprawl, migration risk).

## Consequences

- Easier: schema evolution (rebuild the index), permanent deletion, testing (engine tests run against tempdir SQLite without models), the durable review queue, real graph queries.
- Harder: one flush/consistency module must be built well and early; two backup surfaces.
- Revisit: vector index parameters at 1M+ rows; LadybugDB for graph if CTE traversal hits limits; sqlite-vec if its ANN stabilizes and one-store simplicity becomes worth it.

## Action items

1. [ ] Storage crate with the split SQLite schema, the single Lance writer, and the flush/compaction scheduler (month 1).
2. [ ] `fndr index rebuild` command and a crash-recovery test (kill during flush, verify truth intact, rebuild converges to recall parity within epsilon on the bench corpus; byte-equality is not achievable because ANN index training is nondeterministic).
3. [ ] Deletion-everywhere test fixture (PRD P0.9) before any UI exists.
4. [ ] The app's single-instance lock also guards the database directory, so a stray CLI invocation (`fndr doctor`, rebuild) cannot open a second Lance writer while the app runs.

## Amendment (2026-08-20, plan review)

Due diligence confirmed both the failure mode (version/fragment sprawl under single-record writes, documented upstream) and this ADR's counter-pattern as the maintainer-recommended one. Item 2's acceptance was respecified and item 4 added. Budget roughly 2x live-index disk headroom during compaction. Pin the lancedb crate version exactly (fast release cadence, no 1.0; the underlying format is declared stable with compatibility commitments).

## Amendment (2026-08-21, T-208 spike result)

The spike measured everything this ADR assumes, from Rust, on a 100k-row 768d fixture: GO on the index design. Measured behavior and the resulting directives for T-202/T-203/T-204 are in `docs/spikes/T-208-lance-findings.md`. The two binding corrections: default prune reclaims nothing for our write pattern (the maintenance scheduler must prune explicitly with `older_than=0` plus `delete_unverified=true`, safe under the instance lock), and IVF_PQ builds are tens of seconds at 100k rows so vector-index creation is background maintenance while BTree and FTS indexes are created with the table.
