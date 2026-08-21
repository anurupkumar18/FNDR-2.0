# ADR-006: Retrieval architecture: one stack, chunk-first RAG, eval-gated ranking

**Status:** Proposed
**Date:** 2026-08-19
**Deciders:** FNDR v2 team (4)

## Context

Retrieval quality is the product: agents only prefer FNDR context if recall is high and precision is trustworthy. The POC audit found the retrieval layer structurally compromised: two parallel stacks served different answers for the same query (the card path discarded the planner's routes; a second reranker overwrote the first's output); the flagship parent-child chunk design was written but disabled in production; the live query path applied an instruction prefix its index never used; fusion summed unnormalized weights so five weak matches beat one perfect one; the rerank was ~30 chained multiplicative heuristics never validated against real models; and both graph routes provably returned zero results. Every relevance eval ran on a mock embedder.

## Decision

1. **One retrieval stack.** A single route-based pipeline (ported `RetrievalRoute`/`RouteRunner` skeleton) serves the UI, MCP, Q&A, and the future companion. There is exactly one reranking stage. Any surface-specific shaping happens in composition, never in scoring.
2. **Parent-child chunk RAG from day 1.** Chunks (with source byte spans) are written at capture time and are the primary vector search target; matches resolve to parent records for composition. One embedding contract (Qwen3-Embedding-0.6B, 768d matryoshka, instruction asymmetry captured in the contract and tested). No dual-contract transition period: the v2 store is born on the final contract.
3. **Fusion is rank-based.** Vector and BM25 branches (Lance-native FTS) merge via reciprocal rank fusion with metadata prefilters (time, app, source, project). No unnormalized weighted sums. Route weights, where needed, are declarative config with per-feature attribution in the result's `FusionSignals`.
4. **Reranking is a model, not a heuristic chain.** Qwen3-Reranker-0.6B over the fused top-k, promoted into the default pipeline only on a measured FNDR-Bench win (P1). The small set of product-specific adjustments that survive (source alignment ladder, diversity pass, staleness multipliers) are ported as named, individually-tested, additively-combined features with attribution, never an opaque multiplier chain.
5. **Eval-first rule.** No ranking heuristic, weight, threshold, or stage merges without a benchmark number. FNDR-Bench: a labelled corpus (synthetic capture fixtures plus sanitized donated sessions) with a **frozen held-out test split that tuning and CI never touch** (published numbers come from it; CI gates on the train split), plus a **faithfulness slice** of labelled unanswerable queries where the correct output is NotEnoughEvidence, so verdict overclaiming is a measured regression. Real models only (mock embedders cannot satisfy the gate); metrics Recall@5, MRR@10, verdict accuracy, latency (latency from the reference machine only). CI design: per-PR quality gate on Linux runners with the cached real embedder; nightly macOS parity lane; hosted runners never produce latency or RAM numbers (no Metal, 3 vCPUs). Baselines: BM25-only, vector-only, at least one off-the-shelf naive-RAG pipeline (methods, not competitors), and the POC pipeline re-run on the same corpus. Human usefulness scores report inter-rater agreement.
6. **Graph-aware retrieval is gated.** The typed graph ships for context and visualization first (ADR-006 does not block it); graph and entity routes enter the default retrieval pipeline only when they beat the hybrid baseline on the bench. No decorative plumbing: a route that cannot return results does not ship enabled.
7. **Explainability is first-class.** Every result carries `SurfacingReason` and per-signal attribution; every context pack carries citations that resolve to real records; grounded answering returns the three-state verdict (Grounded / PartialAnswer / NotEnoughEvidence) with the two-distinct-backers rule. Answering composes over full chunk text within a real token budget, with per-claim citation checks (the POC's 1,000-char, 4-extension validator is the named anti-pattern).

## Options considered

**A (chosen): single hybrid stack, chunk-first, model reranker, eval-gated.** Standard modern retrieval shape, differentiated by the screen-memory-specific ported features and the benchmark.

**B: port the POC's heuristic ranking and tune it.** Rejected: 30 multiplicative constants with unknown interactions and no eval history is unfalsifiable; the audit called it unmaintainable, and tuning it would consume the window.

**C: LLM-as-ranker (listwise rerank by the local LLM).** Interesting but wrong cost profile for interactive search on 8 GB machines (seconds per query). May be revisited for offline re-scoring during review; not the interactive path.

**D: graph-RAG-first (traversal as the primary retrieval).** The POC aspired to this and shipped zero-result routes. Rejected as a default until the bench proves it; the GraphPlan intent-to-edges table is ported so the experiment is cheap to run.

## Trade-off analysis

The chosen shape trades novelty-for-novelty's-sake for measurability: the research contribution is not an exotic ranking algorithm but (1) the benchmark itself (no public eval exists for screen-derived personal memory retrieval), (2) the ablations it enables (insight-first embedding text vs raw OCR, chunk-first vs whole-record, graph-augmented vs hybrid, reranker vs RRF-only), and (3) the explainability contract. This satisfies the "novel research" resume pillar with published, reproducible evidence rather than claims.

## Consequences

- Easier: every ranking argument ends with a number; regressions are visible in CI; the demo's quality claims are defensible.
- Harder: building the corpus is real month-1 work; CI needs a real-model lane (macOS runner or cached local run protocol).
- Revisit: reranker promotion (P1 gate), graph-route promotion. The matryoshka dimension is decided once, before the first durable data is written, informed by the month-1 synthetic latency probe and an early 768-vs-1024 ablation; after that there is no transition (consistent with "born on the final contract").

## Action items

1. [ ] Eval corpus v0 (synthetic fixtures spanning the POC's known hard cases: identifiers, paraphrase, time-scoped, app-scoped queries) plus harness skeleton (month 1).
2. [ ] FNDR-Bench v1 with real-model baseline numbers (month 2).
3. [ ] Reranker and chunk-vs-whole-record ablations (month 2 to 3).
4. [ ] Publish the bench and per-release numbers (month 5), including public release of the corpus and harness with an external-submission track; framing: the first public benchmark for screen-derived personal memory retrieval, with FNDR as the reference implementation.

## Amendment (2026-08-20, plan review)

The review found the original design tuning on its own test set and measuring retrieval but not faithfulness; decision point 5 was rewritten accordingly (frozen held-out split, faithfulness slice, naive-RAG baseline, agreement statistics, split CI lanes). The +15 Recall@5 headline moved to a stretch figure in the PRD until first real numbers exist.
