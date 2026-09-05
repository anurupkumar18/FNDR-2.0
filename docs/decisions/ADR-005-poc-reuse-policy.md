# ADR-005: POC code reuse policy: new repo, targeted ports, no wholesale inheritance

**Status:** Proposed
**Date:** 2026-08-19
**Deciders:** FNDR v2 team (4)

## Context

The v1 POC is ~84k lines of Rust and ~27k of TypeScript built through heavy AI-agent iteration. A four-module code audit (capture/privacy, storage/retrieval, graph/MCP/companion, frontend) produced a precise inventory of what is load-bearing versus structural liability. Locked decisions: new repo starts clean; the old repo's git history is imported as a reference branch, not the mainline. Open question resolved here: what gets ported.

The audit's headline: the POC's **tuned constants, prompts, contracts, and small pure functions encode months of real-world iteration and are the most valuable artifacts in the codebase**, while its **large modules, query layers, and wiring are the liability** (1,900-line capture loop, 5,200-line MCP file, dual retrieval stacks, three graph schemas, no indexes, dead privacy logic, evals on mock embedders).

## Decision

**Ports are targeted, test-covered, and provenance-noted.** A port is a specific function, constant set, prompt, schema, or contract, arriving with tests and a `// Ported from FNDR v1 <path>` note. Wholesale module or file copies are prohibited. Prompts port verbatim. Discarded areas may be consulted on the reference branch but never copied.

### PORT (carry near-verbatim)

Perception and capture heuristics:
- `capture/text_cleanup.rs` in full (line scoring, span salience, CUE_WORDS, noise estimation) as its own crate; it had four consumers in v1.
- `capture/admission.rs` surface policy (navigation/listing skip lists) in full.
- The merge/continuity block (`continuity_anchor`, candidate scoring, merge thresholds, cross-app rules, `merge_story_text`).
- `validate_structured_memory_extraction` grounding validator and constants; `build_durable_memory_context` composer; `build_low_ram_semantic_fusion` no-LLM fallback; `VisualNoveltyTracker`; session key/id scheme; `weighted_primary_embedding` weights.
- `ocr/vision.rs` largely as-is: `text_volume_qualifies`, `preprocess_ocr_for_qwen` with the `[LOW_CONF]` convention, `OcrAggregateStats`, config defaults with their tuning comments (make the call async at the boundary; wire `min_confidence` for real).
- `capture/entity_extractor.rs` (UUIDv5 stable node identity, confidence weighting, 0.4 edge floor) **minus** the fabricated Decision-Contradicts-Error pair.

Contracts and storage:
- `inference/model_config.rs` contract system (model, file, tokenizer, dimension, table move together; pinned SHA-256 URLs) as the first thing written.
- The embedder construction-time dimension and non-zero probe; the never-fall-back-across-dimensions guards.
- `memory_chunk_schema` (with source byte spans added); `lexical_keyword_score` field weights; `vector_distance_to_similarity` with its bug-history comment; `estimate_signal_strength` weights (deduplicated to one home).

Retrieval and explainability:
- `RetrievalRoute` trait, `RouteCtx`, `RouteRunner` two-wave dispatch, per-route metrics.
- `SurfacingReason` and `FusionSignals` types; surfacing-reason headline generation.
- `GraphPlan::from_intent` (intent to hops/seed-kinds/allowed-edges table).
- Verifier state machine shape (Grounded / PartialAnswer / NotEnoughEvidence, two-backer rule); `EvidencePack` shape.
- `embedding_retrieval_adjustment` staleness multipliers with the lexical-route exemption.
- `query_source_alignment` ladder (1.0 / 0.85 / 0.45); `diversify_results` (app/source/time-bucket novelty, top-3 preserved); `collect_grounded_snippets` citation labelling.
- BGE-style prefix discipline generalized: instruction/prefix is part of the embedding contract, tested for index/query asymmetry.

Prompts (verbatim):
- `MEMORY_SYNTHESIS_PROMPT` plus `parse_synthesis_json` clamps; `VOICE_RULES`; `synthesize_memory_card` prompt; the memory-review prompt with SAME_DAY_CANDIDATES grounding and single-repair-retry; the two agent guardrail strings.

Lifecycle and review:
- `ReviewProvider`/`ReviewInput`/`ReviewWriteMode`/outcome types; `validate_review` and evidence-blob grounding checks; skip-vs-fail classification; dry-run contract; `parse_day_range_local`. Fixes on the way in: durable queue, attempt caps and backoff, per-record locking, one embedding composer.

Surfaces and product vocabulary:
- Graph node/edge taxonomy (14 node types, 29 edge types) and `graph-schema.md` as the v2 spec; `graphRelationshipResolver.ts` 29-to-5 UI mapping.
- Companion `dto.rs` wire contract, route/permission table, pairing state machine, revocation semantics, `api-contract.md` (with pair-start moved off the network surface).
- MCP tool vocabulary deduplicated to the canonical set (ADR-007); flexible time-window schema; content plus structuredContent envelope.
- `OmnibarApp.tsx` (race guard, mode union, keyboard and a11y model); `cinematic-palettes.ts` and token system with tests; domain folder taxonomy; `useTauriEvent`; `displayTitle.ts` label cascade.
- Safety-gate keyword/pattern lists (password managers, banking, medical, auth, secrets) as data, with the three-way Allow/Redact/SkipStorage model, wired live this time.
- `SkipReason` enum and per-tick counter observability contract; ADRs 002/004/007/010/012 concepts carried into v2 docs.

### REFERENCE (rewrite, keep the idea)

Chunker line classification (rewrite with real token counts and byte spans), relevance-gate coverage floors, intent taxonomy (rewrite as scored rules; fix ResumeWork-before-Debug ordering), session grouping and impure-group splitting, temporal half-life routing, keyword variant budgeting, token-budgeted pack trimming, 2D graph canvas, capture loop shape (rewrite as a staged pipeline with seams), blocklist matching (exact-token and suffix-domain, fixing substring false positives), event-driven status pattern.

### DISCARD (do not consult except for post-mortems)

The 1,913-line capture loop; the 5,197-line MCP module and its mode matrix; the dual retrieval stacks (Stack A route-discard and the second reranker that overwrote the first); the ~30-multiplier rerank chain; candidate-set-local BM25 IDF; unnormalized summed fusion weights; the three-schema graph situation (projection layer, legacy graph, lossy 3D normalizer); the 104-field table and JSON-blob side tables; the 5-minute-bucket content hash; MockEmbedder and every mock-based eval; the Python sidecars; polling remnants; `compose_answer`'s 1,000-char context and 4-extension citation validator; the agent-runner execution surface.

## Options considered

**A (chosen): targeted ports with provenance.** Preserves tuning capital, prevents architecture inheritance.
**B: fork and refactor the POC.** Rejected: the audit shows the liabilities are structural (module boundaries, schemas, wiring), which refactoring inherits by default; the team would spend the window fighting the old shape.
**C: pure clean-room, read-only inspiration.** Rejected: throws away validated constants and prompts that took months to tune, and re-learning them would consume the exact time the 6-month plan does not have.

## Consequences

- Easier: v2 starts with proven heuristics and a precise map of what not to rebuild.
- Harder: port discipline requires review attention (the provenance note and test rule).
- Revisit: items move between lists only with an eval or audit justification recorded in the PR.

## Action items

1. [ ] Import the POC repo as `reference/v1` branch in the new repo at bootstrap.
2. [ ] Create the port checklist as tracked tickets (`ROADMAP-TICKETS.md` carries them per epic).
3. [ ] Add the provenance-note convention to the repo engineering skill.

## Amendment (2026-09-05, alpha donor matrix)

The semester execution plan narrows the early reuse decision further. The
legacy repository is a reference donor for the alpha only; it is never a
fallback mainline. Each candidate below requires its own targeted port commit,
tests, and provenance note before it can be considered shipped in v2.

| Legacy asset | Decision | Alpha handling | Explicit exclusion |
| --- | --- | --- | --- |
| Pre-OCR sensitive-context guard in `src-tauri/src/capture/mod.rs` | Reference | Preserved independently at `codex/p002-sensitive-preocr` / `4f48ae46`; use its regression cases to shape v2 policy replay. | Do not transplant the capture loop or its `SkipReason` plumbing. |
| `privacy/safety_gate.rs` keyword/domain/pattern data | Targeted data port | Use the lists only, with v2 exact-token and suffix-domain matching, typed reasons, and redaction tests. | Do not carry v1 raw substring matching or its unstructured decision result. |
| OCR fixture flow and Vision wrapper behavior | Reuse at seam | Keep v2's small `FrameSource` and `OcrEngine` boundaries; compare fixture behavior during alpha. | Do not copy v1 capture orchestration or synchronous hot-path wiring. |
| `agent/actions.rs`, `approvals.rs`, `audit.rs`, `execution.rs`, and agent panel | Reference only | Use only as product vocabulary and scenario evidence for ADR-008. | No agent runner, shell executor, approval-file format, or UI module port. |
| `mcp/mod.rs` and its tool set | Vocabulary only | Preserve only canonical tool concepts already ratified in ADR-007. | No server module, transport mode matrix, auth wiring, or duplicate tools. |
| Existing persistent store/search code | Reimplement on v2 contracts | Use the walking-skeleton FTS path to prove the alpha spine while the real SQLite/Lance contracts replace it. | No old Lance schema, direct query path, or dual retrieval stack. |

For an alpha port review, the author records the legacy path and commit, the
v2 target path, the test evidence, the defect intentionally not carried over,
and the deletion or replacement ticket for any temporary skeleton. A legacy
behavior that cannot meet ADR-004, ADR-006, ADR-007, or ADR-008 is not
eligible merely because it makes the demo faster.
