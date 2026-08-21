# T-208 spike findings: Lance FTS, prefilters, hybrid, and maintenance from Rust

Date: 2026-08-21. Harness: `spikes/lance-spike` (standalone crate, outside the
workspace on purpose). lancedb 0.37.1 / lance 10, arrow 58, rustc 1.98.0,
Apple Silicon dev machine (not the bench reference machine; treat latencies as
shape, not published numbers). Fixture: 100,000 rows, 768-dim vectors, text,
source and timestamp prefilter columns, ingested in 100 batches of 1,000 (the
T-202 flush shape). Two runs; numbers were stable.

## Verdict: GO on the ADR-002 index design

Everything ADR-002 assumes works from the Rust crate, with two behaviors the
T-202/T-204 designs must respect (below).

## Measured behavior

| What | Measured |
|---|---|
| Ingest 100k rows x 768d, 100 batches | 0.9 s, 296.7 MB, one Lance version per batch commit |
| BTree scalar index build (id, timestamp) | 15.7 ms / 11.7 ms |
| FTS BM25 index build (text) | 362.7 ms |
| IVF_PQ vector index build (defaults) | 26.7 s |
| Vector top10, no index (flat scan) | 30.0 ms |
| Vector top10, IVF_PQ | 6.8 ms (seed row present, spot check only) |
| Vector top10 + SQL prefilter (source + time window) | 14.7 ms, filter respected |
| FTS multi-term BM25 | 5.4 ms, correct topical hits |
| FTS unique-token lookup | 0.9 ms, exactly one hit |
| Native hybrid (FTS + vector, RRF, `execute_hybrid`) | 4.4 ms |
| Scalar point lookup (`id = 4242`) | 1.3 ms |
| `optimize(All)`: compact + default prune + index optimize | 511 ms; 100 fragments to 1; disk 310 to 625 MB; prune removed nothing |
| Explicit prune (`older_than=0`, `delete_unverified=true`) | 56.6 ms; reclaimed 331 MB (106 versions); disk back to 309 MB; search intact |

## What this pins down for the storage tickets

1. **Every commit is a version (T-202).** 100 batch appends produced 100
   versions. The 30 to 60 second batched flush is the right default; never
   per-record commits.
2. **Default prune is a no-op for our write pattern (T-204).** It keeps
   versions inside a retention window and will not touch files newer than 7
   days, so compaction doubles disk and nothing comes back until an explicit
   prune with `older_than=0` and `delete_unverified=true`. That flag is safe
   only single-process; FNDR's instance lock already guards the data dir
   (ARCHITECTURE section 2), so the T-204 scheduler can and must prune
   explicitly. "Disk returns after prune" is measurable and fast.
3. **Index build scheduling (T-203).** BTree and FTS are cheap enough to
   create at table creation. IVF_PQ takes tens of seconds at 100k rows and
   belongs in background maintenance, with incremental index optimize on
   flush cycles picking up new rows between rebuilds.
4. **Pre-index behavior is acceptable, not silent.** A flat scan at 100k rows
   is 30 ms, so a young vault works before IVF_PQ exists; the state still
   surfaces as a typed "vector index pending" status, never quietly.
5. **Hybrid exists natively.** `execute_hybrid` (FTS + vector with RRF)
   works from Rust. The v2 read path still does its own RRF across routes
   (temporal and graph included), but Lance-native hybrid is available for
   the two-route case if profiling favors it.
6. **Recall is unmeasured here.** The seed-row spot check passed with default
   IVF_PQ parameters; real recall and parameter tuning are FNDR-Bench work
   (E05), not spike work.

## Build-system notes

- `protoc` is a build prerequisite for the lance tree (dev-setup now warns).
- The lance/datafusion dependency tree cold-builds in roughly 19 minutes on
  this machine. The spike stays a standalone crate so the product workspace
  and CI never pay that; when T-202 brings lancedb into fndr-store, expect
  the first uncached CI build to stress the 15-minute PR budget (T-102) and
  lean on rust-cache.
