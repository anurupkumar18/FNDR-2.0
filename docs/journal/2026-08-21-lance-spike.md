# Handoff: T-208 Lance spike (2026-08-21)

Done (PR #10): the spike harness (`spikes/lance-spike`, standalone crate outside the workspace) measured ingest, all four index types, prefiltered ANN, BM25 FTS, native hybrid RRF, point lookups, compaction, and prune on a 100k-row 768d fixture from Rust. Verdict GO on ADR-002; full numbers in `docs/spikes/T-208-lance-findings.md`; the binding corrections are amended into ADR-002.

The three numbers that shape T-202/T-203/T-204: every batch commit is a Lance version (batch the flush, prune on a schedule), default prune reclaims nothing under our write pattern (explicit `older_than=0` + `delete_unverified=true` reclaimed 331 MB in 57 ms, safe under the instance lock), and IVF_PQ builds in ~27 s at 100k rows (background maintenance; BTree and FTS are effectively free at table creation).

In flight: nothing broken. Next: T-202 flush writer designed directly from the findings, T-401 model registry, T-302 SCK provider (spike T-310 pending).

Decisions: spike code merged for reproducibility but stays out of the workspace so the ~19 minute lance/datafusion cold build never taxes product CI; protoc documented as a prerequisite in dev-setup.

Landmines: when T-202 brings lancedb into fndr-store the first uncached CI build will stress the 15-minute PR budget; watch rust-cache behavior on that PR. The spike's `delete_unverified` prune is only safe single-process.

Produced by: Anurup + Claude Code
