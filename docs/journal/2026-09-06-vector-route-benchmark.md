# First vector route and measured smoke baseline · 2026-09-06

## What landed

`fndr-retrieval::VectorRetriever` is the first read path over the existing
rebuildable Lance chunk derivative. It embeds its query through the existing
Qwen query-side instruction contract, opens the contract-matched Lance table,
and returns stable record/chunk IDs, source/time metadata, and Lance's raw
distance. It does not expose chunk text and does not fuse that distance with
keyword scores.

The route is intentionally independent from `KeywordRetriever`. ADR-006
requires rank-based fusion and a measured decision; treating the two raw score
scales as interchangeable would violate that boundary.

`fndr-bench` now has an opt-in `--route vector` mode. It requires an explicit
GGUF model path and an empty work directory. That directory receives a
disposable SQLite truth database and Lance derivative for the corpus; it is
never an application vault. The existing `make bench` command remains the
fast FTS-only CI baseline and never loads a model.

## Verified on the local reference machine

On the local M1 with `models/Qwen3-Embedding-0.6B-Q8_0.gguf`, the synthetic
12-record `bench/corpus-sample` run produced:

- Recall@5: `1.0000`
- MRR@10: `1.0000`
- query latency p50/p95: `90.43 ms` / `90.62 ms`

The run used an empty `/tmp` directory and removed its SQLite and Lance files
afterward. This is a route smoke measurement, not a committed quality baseline
or a ranking-tuning result: the corpus is deliberately too small for that.

The deterministic regression test writes chunks through `LanceWriter`, queries
the resulting derivative with `VectorRetriever`, and verifies the expected
stable record/chunk IDs and distance order. `CARGO_BUILD_JOBS=1 make test`
passed after the change.

## Still open

The vector route is not yet wired into MCP, the desktop UI, or context packs.
There is no Lance FTS, temporal route, metadata prefilter, RRF fusion,
timeout/metrics surface, or reranker. Those remain T-505 follow-through and
must be promoted only with a reviewed benchmark number.
