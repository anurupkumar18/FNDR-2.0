# Handoff: T-301 port and fndr-bench skeleton (2026-08-21)

Done: fndr-textsignal carries v1's text_cleanup in full (line scoring, span salience, CUE_WORDS, noise estimation) with all 15 v1 tests green (PR #5). fndr-bench is real (PR #6): corpus format defined (records.jsonl plus queries.jsonl), Recall@5 / MRR@10 / latency percentiles, committed-baseline comparison that fails on quality regression, and the honest FTS baseline route running the same store search the fndr.search tool serves. `make bench` runs it locally and in CI on the sample corpus.

The bench paid for itself on day one: the sample exposed that unstemmed FTS misses morphological variants ("index" vs "indexes" scored 0 recall), so the store's FTS table now uses the porter tokenizer and the committed baseline is 1.0/1.0 on the sample. That is the eval-gated loop working as designed.

In flight: nothing broken. Next: T-201 real schema and migrations (design-heavy, deserves a fresh session), T-208 Lance spike, T-401 model registry. The textsignal functions are not yet wired into the capture path; that happens with the real pipeline stages (T-303+), not the skeleton.

Decisions: the sample corpus is a format fixture, never an eval instrument (bench/README.md says so; never tune against it). Latency is recorded but never baseline-compared; published numbers stay reference-machine-only (PRD P0.7). Baseline refreshes ride the PR that legitimately improves the numbers.

Landmines: fndr-bench's baseline gate is exact (epsilon 1e-9) because FTS is deterministic; embedding routes will need a tolerance policy when E05 lands. insert_record_with_id exists on SkeletonStore for corpus loading; the real T-201 store should decide id strategy deliberately.

Produced by: Anurup + Claude Code
