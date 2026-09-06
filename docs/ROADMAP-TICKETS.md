# FNDR v2: Roadmap and Ticket Breakdown

Status: proposed 2026-08-19; revised 2026-08-20 (team pain points folded in; plan-review fixes applied per `review/REVIEW-2026-08-20.md`). Deferred by request: the review's full capacity re-cut and lane rebalance (its per-milestone load arithmetic still applies and should be revisited at sprint planning). Source documents: `PRD.md` (scope, gates), `ARCHITECTURE.md` (boundaries), `decisions/` (choices). This file is formatted for manual GitLab import; a generated `tickets.csv` (title, description with `/label` and `/milestone` quick actions) sits beside it for GitLab's CSV issue import.

## Conventions

- **Milestones:** `M1-Foundations`, `M2-Retrieval`, `M3-AgentContext`, `M4-Comprehension`, `M5-AssistantProof`, `M6-Ship`. The month-3 demo gate closes M3.
- **Labels:** `lane::ml`, `lane::backend`, `lane::frontend`, `lane::platform`; `prio::p0` (required for its milestone's goals; the M1 to M3 spine subset is additionally never cut) or `prio::p1` (cuttable under pressure); epic labels `epic::E01` ... `epic::E15`.
- **Docs move together:** any change to this file or the PRD that touches an ADR amends that ADR in the same change (root cause of the 08-20 consistency debt).
- **Ticket format:** each ticket is one list item: id, title, then backticked labels, then `deps:`. The indented line is the description; `AC:` is the acceptance criterion. Estimates are deliberately omitted; the team sizes tickets at sprint planning.
- Port tickets follow ADR-005: ported code arrives with tests and a provenance note.

## Progress ledger

Updated as tickets land; the ticket list below stays the import source of
record. Before importing `tickets.csv` into GitLab, delete the rows for
tickets listed here as done (or import everything and close them with a
comment linking the PR).

| Ticket | Status | Where |
|---|---|---|
| T-101 bootstrap monorepo + reference/v1 | Done 2026-08-20 | initial main history; reference/v1 locked read-only |
| T-102 CI test gate + make targets | Done 2026-08-20 | `.github/workflows/ci.yml`; `make bench` real since PR #6 |
| T-103 egress lint | Done 2026-08-20 | scripts/workspace-lints.sh + deny.toml (negative-tested) |
| T-104 engine-no-tauri check | Done 2026-08-20 | scripts/workspace-lints.sh (negative-tested) |
| T-105 generated TS bindings | Done 2026-08-20 | PR #3 (pins per ADR-001; sync test; raw-invoke ban) |
| T-106 skill + CONTRIBUTING | Done 2026-08-20 | `.claude/skills/fndr-v2-engineering/`, CONTRIBUTING.md |
| T-107 AGENTS.md mirror + drift check | Done 2026-08-20 | scripts/gen-agents-md.sh, CI guard |
| T-108 env bootstrap, four machines | Partial | this machine bare-to-green; scripts/dev-setup.sh for the rest |
| T-109 walking skeleton | Done 2026-08-20 | PR #4; runner: `cargo run -p fndr-mcp --example skeleton`; findings in journal |
| T-201 SQLite schema v1 + migrations | Done 2026-08-21 | PR #8; journal 2026-08-21-schema-v1 |
| T-202 Lance writer with batched flush | Done 2026-08-21 | PR #12; migration 0002; embedding contract seam in fndr-inference |
| T-203 vector/FTS/scalar indexes | Partial (audited 2026-09-05) | `fndr-store/src/lance_writer.rs`: BTree + FTS indexes ship with table creation on first flush. Missing: the vector index (explicitly deferred to T-204 maintenance in the code's own comment) and the AC's 100k-row query-plan proof. |
| T-204 compaction and version-prune scheduler | Not started (audited 2026-09-05) | No scheduler code found; `lance_writer.rs` explicitly defers vector-index and version-prune maintenance here. |
| T-205 index rebuild + crash-recovery test | Partial (audited 2026-09-05) | `LanceWriter::rebuild` in `fndr-store/src/lance_writer.rs` (labelled T-205 in its own doc comment) plus `crates/fndr-store/tests/rebuild.rs` (`rebuild_on_missing_table_is_a_fresh_build`, `crash_window_duplicates_then_rebuild_converges_to_truth`) cover the AC's mechanism and crash-recovery proof. Missing: the `fndr index rebuild` CLI surface named in the AC — only the library function exists today. |
| T-206 deletion everywhere | Not started (audited 2026-09-05) | SQLite FK cascade exists at the schema level (tested), but no public deletion API spanning SQLite + Lance by record/time/domain/all. |
| T-207 retention jobs | Not started (audited 2026-09-05) | No retention/expiry code found; depends on T-206. |
| T-208 Lance spike | Done 2026-08-21, GO | PR #10; `docs/spikes/T-208-lance-findings.md`; ADR-002 amended |
| T-301 textsignal port | Done 2026-08-21 | PR #5 (15 v1 tests) |
| T-302 ScreenCaptureKit capture provider | Done 2026-09-05 | `ScreenCaptureKitSource` in `fndr-capture/src/source.rs`: one-shot `SCScreenshotManager` per ADR-001 action item 4, `screencapturekit` pinned `=9.0.1`. Verified on a live screen end to end (real capture → Vision OCR → privacy gate → SQLite), not just compiled. Needed a new `.cargo/config.toml` rpath for the crate's Swift shim — see the journal entry. T-310's soak is still open, so this is unproven over time. |
| T-303 perceptual and semantic dedup | Done 2026-09-05 | `fndr-capture/src/dedup.rs` ports the v1 dHash/color guard, A-B-A loop detection, and bounded semantic window with tests. The real SCK source samples native RGBA into a 9×8 raster before hashing; no full-resolution PNG decode is added to the hot path. The scheduler will consume these seams in T-306. |
| T-304 admission policy port | Done 2026-09-05 | `fndr-capture/src/admission.rs` is the targeted, pure port of the v1 browser-surface policy. It classifies `Normal`, `UrlOnly`, or `SkipFrame` before capture; the scheduler remains the owner of metadata acquisition and persistence. The three v1 observed surface cases plus non-browser/no-URL boundaries pass as unit tests. |
| T-305 port OCR wrapper | Done (audited 2026-09-05) | `fndr-ocr/src/vision.rs` (857 lines): `[LOW_CONF]` convention, `min_confidence` wired, config presets, 13 passing tests including `text_volume_qualifies`/`text_volume_fails` cases. Not previously in this ledger. |
| T-401 model registry and downloader crate | Done (audited 2026-09-05) | `fndr-inference/src/registry.rs` (`MODELS`, disk-preflight `bytes_needed`) + `fndr-downloader/src/lib.rs` (`download_verified`, resume, checksum/size verify, atomic rename). AC test `checksum_mismatch_deletes_artifact_and_fails_loudly` passes; HTTP confined per the T-103 egress lint. Not previously in this ledger. |
| T-402 embedding contract v1 | Done 2026-09-05 | `fndr-inference/src/embedding.rs` (contract, asymmetry, matryoshka rule) + `src/gguf_embedder.rs` (`GgufEmbedder`, a real llama.cpp-backed `Embedder`, verified against the real downloaded model with actual Metal inference). AC's construction probe passes as an `#[ignore]`d test (needs the real model; never runs in CI). |
| T-403 model-worker priority queue | Partial 2026-09-05 | `fndr-inference/src/model_worker.rs`: `ModelWorkerHandle`/`Priority`, lazy load + idle-timeout unload, AC's contention test proves priority ordering. `scripts/check-llm-call-sites.sh` covers the "no LLM call outside the queue" AC as a standalone (not CI-wired) review-rule script. `QueuedEmbedder` adapts the queue to `&dyn Embedder`, proven both by a fast fake-embedder integration test (`fndr-store/tests/lance_flush.rs`) and a real run through the actual downloaded model (`fndr-memory/examples/end_to_end_flush.rs`). Missing: a long-lived production owner of one `ModelWorkerHandle` across many captures — that's T-306's scheduler, still blocked on ScreenCaptureKit. |
| T-404 embedding batch path | Partial 2026-09-05 | `fndr-inference/src/gguf_embedder.rs`: `memoize_by_text` (capture-burst memoization, unit-tested with a call-counting fake, no model needed) and a real-model throughput benchmark (`#[ignore]`d; recorded ~10 docs/sec and confirmed 8 duplicates add ~0ms, not another ~774ms). Missing: real multi-sequence llama.cpp batching (one `decode()` call across several sequences) — investigated (`with_n_seq_max`/`embeddings_seq_ith` exist) but not attempted; real correctness risk not worth taking without extensive empirical verification. |
| E05 head start (bench harness) | Skeleton done | PR #6: corpus format, FTS baseline route, regression gate in CI; full FNDR-Bench remains T-501+ |
| T-701 MCP transport with auth-always | Done (audited 2026-09-05) | `fndr-mcp/src/auth.rs` + `src/server.rs`: bearer (constant-time), origin/host allowlist, rate limit, owner-only discovery. Named regression tests `mcp_rejects_unauthenticated_loopback` and `mcp_rejects_web_origin_with_valid_token` both pass (`tests/auth_surface.rs`). Not previously in this ledger. |
| T-702 tools wave 1 | Partial (audited 2026-09-05) | Only 2 of the 8 named tools exist (`fndr.search`, `fndr.privacy_status`), and `server.rs`'s own header comment says they stand in for the engine API on the walking skeleton, not yet "served by the same engine functions as the UI" per the AC. context_pack, timeline, recall, project_context, active_focus, source_evidence are not started. |
| T-801 blocklist v2 | Done 2026-08-21 | PR #13; v1 false-positive classes pinned as tests |
| T-802 sensitive-context detection as data | Partial 2026-09-05 | `SensitiveContextPolicy` in `fndr-privacy/src/safety_gate.rs`; built-in lists are now owner-constructible data with parity/override tests. Still missing: an alert queue with dismissal keys, and a caller that actually loads overrides from `settings` or disk. |

---

## E01 · Repo and CI foundations (M1)

- **T-101 · Bootstrap monorepo workspace and import reference branch** `lane::platform` `prio::p0` `M1` deps: none
  Create the new repo, Cargo workspace plus `ui/` skeleton per ARCHITECTURE §3, and import the POC history as `reference/v1`. AC: workspace builds empty crates; reference branch present and documented as read-only.
- **T-102 · CI test gate on every PR** `lane::platform` `prio::p0` `M1` deps: T-101
  fmt, clippy, cargo test, vitest, tsc, on macOS runner; also creates the `make test` and `make bench` targets the engineering skill's verification commands assume. AC: a failing test blocks merge; runtime under 15 minutes (the real-model eval lane is separate, T-513).
- **T-103 · Local-only egress lint in CI** `lane::platform` `prio::p0` `M1` deps: T-101
  cargo-deny plus workspace lint banning HTTP client crates outside downloader/updater (ADR-004). AC: adding reqwest to fndr-store fails CI; allowlist constants module has its uniqueness test.
- **T-104 · Engine-independent-of-Tauri CI check** `lane::platform` `prio::p0` `M1` deps: T-101
  AC: any engine crate importing tauri fails CI.
- **T-105 · Generated TS bindings pipeline** `lane::backend` `prio::p0` `M1` deps: T-101
  specta/tauri-specta from fndr-types; hand-written IPC interfaces banned by lint. AC: a sample command's types round-trip into `ui/` at build time.
- **T-106 · Repo engineering skill and CONTRIBUTING** `lane::platform` `prio::p0` `M1` deps: T-101
  Install the v2 engineering skill (.claude/skills/fndr-v2-engineering) into the new repo; document port-provenance, module-size, and session-handoff conventions (who did what, where it stands, next steps, which agent/tool produced it), plus the incidents-and-reversals log (the failure-narrative artifact interviews run on). AC: skill loads in Claude Code; CONTRIBUTING covers the conventions including the handoff format and incidents log.
- **T-107 · Cross-tool agent conventions mirror** `lane::platform` `prio::p0` `M1` deps: T-106
  Generate AGENTS.md (read by Cursor, Codex, and other agent tools) from the fndr-v2-engineering skill so every AI tool follows one conventions source; CI drift check between the two. AC: skill and AGENTS.md agree by construction; reuse-first and port-provenance rules present in both.
- **T-108 · Environment bootstrap and dev-install on four machines** `lane::platform` `prio::p0` `M1` deps: T-101
  Per-builder setup (Rust toolchain, TCC grants for dev builds, model pulls) scripted and documented; dev-install path verified on all four machines; the bench reference machine (M1 8 GB) designated with an owner. AC: a new checkout reaches a running dev build on each machine following only the doc; dogfooding (T-512) is unblocked.
- **T-109 · Walking skeleton: thin end-to-end slice** `lane::backend` `prio::p0` `M1` deps: T-101
  By week 3, deliberately ugly: capture one screen, OCR it, store it, search it, and serve one MCP tool over it. De-risks the M1 spikes in one artifact and gives the earliest honest signal on context usefulness (v1 pain point 1). AC: the slice runs end to end on one machine; findings recorded as tickets or ADR amendments.

## E02 · Storage layer (M1, M3)

- **T-201 · SQLite schema v1 and migrations** `lane::backend` `prio::p0` `M1` deps: T-101
  Split memory domains, chunks, graph, tasks, meetings, queues, settings per ARCHITECTURE §5. AC: migration tests; FK integrity on graph edges; lifecycle enums persisted as discriminants.
- **T-202 · Lance writer with batched flush** `lane::backend` `prio::p0` `M1` deps: T-201
  Single writer module; 30 to 60 s or batch-size flush from SQLite; nothing else writes Lance. AC: flush failure leaves SQLite intact and retries; write-path test.
- **T-203 · Vector, FTS, and scalar indexes** `lane::backend` `prio::p0` `M1` deps: T-202
  Create and maintain indexes on the chunk/record tables; incremental optimize. AC: query plans hit indexes (no full scan) in a 100k-row fixture; point lookups use the scalar index.
- **T-204 · Compaction and version-prune scheduler** `lane::backend` `prio::p0` `M1` deps: T-202
  AC: a 24 h simulated write load keeps version count and small-fragment count bounded; disk returns after prune.
- **T-205 · Index rebuild command and crash-recovery test** `lane::backend` `prio::p0` `M1` deps: T-202
  `fndr index rebuild` from SQLite truth. AC: kill during flush, truth intact, rebuild converges byte-equal on retrieval results.
- **T-206 · Deletion everywhere** `lane::backend` `prio::p0` `M1` deps: T-201, T-202
  Delete by record, time range, domain, or all, across both stores and indexes (PRD P0.9). AC: post-delete search, evidence, and graph return nothing; test fixture.
- **T-207 · Retention jobs** `lane::backend` `prio::p0` `M1` deps: T-206
  Configurable retention windows executing through the deletion path (promoted to p0: the G3 storage budget depends on it). AC: expiry test with clock control.
- **T-208 · Spike: Lance FTS, prefilters, and hybrid from Rust** `lane::backend` `prio::p0` `M1` deps: T-101
  Time-boxed one week: prove BM25 FTS, metadata prefilters, RRF hybrid, and index maintenance from the Rust crate on a 100k-row fixture before T-203/T-505 assumptions harden. AC: findings note with measured behavior; go/no-go on the ADR-002 index design.
- **T-209 · Backup, export, and restore** `lane::backend` `prio::p0` `M3` deps: T-201, T-206
  `fndr backup` / `fndr export` / restore: SQLite snapshot plus config (Lance rebuilds from truth, which is the other half of the story); makes "the memory stays mine" literal and avoids the live-WAL-in-Time-Machine corruption trap. AC: backup taken under active capture restores to a working vault; export documented.

## E03 · Capture and perception (M1, M3)

- **T-301 · Port fndr-textsignal crate** `lane::backend` `prio::p0` `M1` deps: T-101
  v1 text_cleanup in full (line scoring, span salience, CUE_WORDS, noise). AC: v1 tests ported and green; provenance notes.
- **T-302 · ScreenCaptureKit capture provider** `lane::backend` `prio::p0` `M1` deps: T-101
  Replace v1's deprecated CGDisplay path; permissions health check with re-grant guidance. AC: frames on macOS 14+; revoked-permission state visible, not silent.
- **T-303 · Perceptual and semantic dedup** `lane::backend` `prio::p0` `M1` deps: T-302
  Downscale-before-hash dHash, A-B-A loop detection, semantic window; maintained hashing dependency. AC: v1 heuristic behavior preserved under test; no full-res PNG decode on the hot path.
- **T-304 · Port admission policy** `lane::backend` `prio::p0` `M1` deps: T-301
  Navigation/listing surface skips, url-only records. AC: v1 segment-list cases green.
- **T-305 · Port OCR wrapper** `lane::backend` `prio::p0` `M1` deps: T-101
  Vision via objc2; async boundary; LOW_CONF convention; min_confidence actually wired. AC: v1 observed-value tests green; no sync call on the async runtime.
- **T-306 · Staged capture pipeline and SkipReason counters** `lane::backend` `prio::p0` `M1` deps: T-302, T-303, T-304, T-305
  The ARCHITECTURE §4.1 scheduler with per-stage seams; one terminal counter per tick; durable batch on shutdown. AC: each stage testable without the loop; shutdown loses zero records.
- **T-307 · Port session identity and merge/continuity** `lane::backend` `prio::p0` `M1` deps: T-306
  Anchors, candidate scoring, merge thresholds, story merge. AC: v1 merge tests green; cross-app window rules covered.
- **T-308 · Adaptive sampling** `lane::backend` `prio::p1` `M1` deps: T-306
  Idle-blend FPS, deep-idle pause, forced interval. AC: simulated idle timeline produces expected FPS curve.
- **T-309 · Declarative gate policy and replay harness** `lane::backend` `prio::p0` `M1` deps: T-306
  Capture gates as a config-driven policy table; offline replay over recorded fixtures (the v1 stacked-gates regression is the named test). AC: gate changes show per-gate drop deltas in the replay report.
- **T-310 · Spike and soak: ScreenCaptureKit provider hardening** `lane::backend` `prio::p0` `M1` deps: T-302
  Pin the screencapturekit crate exactly; multi-day soak with an RSS trend assertion (the crate's issue history is leaks and stalled callbacks); decide periodic SCScreenshotManager captures vs a persistent SCStream for 0.5 FPS; record the fallback (objc2-screen-capture-kit or vendoring, as both shipped comparables did); document the macOS 26.1 dev-build bundle/TCC quirk. AC: soak passes or the fallback is adopted; decision note committed (ADR-001 amendment).
- **T-311 · Backfill importers (browser history, git log, shell history)** `lane::backend` `prio::p1` `M3` deps: T-201
  Deterministic, local, no new privacy surface: seed the vault on first run so the first real context pack works in minutes instead of after a day of capture (cold-start fix). AC: a fresh install plus importers yields a context pack that answers project questions; importers respect the blocklist.

## E04 · Inference and models (M1 to M3)

- **T-401 · Model registry and downloader crate** `lane::ml` `prio::p0` `M1` deps: T-101, T-103
  Pinned revisions, SHA-256, required/optional flags, resume, disk preflight (v1 ADR-012 semantics). Downloads flow through the platform-owned fndr-downloader crate. AC: checksum mismatch deletes artifact and fails loudly; HTTP confined to fndr-downloader/fndr-updater per ADR-004.
- **T-402 · Embedding contract v1** `lane::ml` `prio::p0` `M1` deps: T-401
  Qwen3-Embedding-0.6B official Q8_0 GGUF (639 MB; no official Q4 exists, avoid community sub-8-bit quants), instruction asymmetry in the contract struct, and the 768d matryoshka rule implemented app-side: take the 1024d output, truncate to 768, L2-renormalize, all inside the contract with tests. Construction probe (dimension, non-zero). AC: prefix asymmetry test; truncate+renormalize round-trip test; wrong-dimension write refused end to end.
- **T-403 · Model-worker priority queue** `lane::ml` `prio::p0` `M1` deps: T-401
  All llama.cpp work behind one queue (interactive > synthesis > review > backfill); load/unload with idle timers. AC: contention test proves priority; no LLM call outside the queue (lint or review rule).
- **T-404 · Embedding batch path** `lane::ml` `prio::p0` `M1` deps: T-402, T-403
  AC: throughput benchmark recorded; capture-burst memoization.
- **T-405 · VLM synthesis path** `lane::ml` `prio::p1` `M2` deps: T-403
  Qwen3-VL-4B with 2B tier on 8 GB; idle unload. AC: synthesis runs pressure-gated; RAM budget respected in probe.
- **T-406 · Reranker integration behind eval gate** `lane::ml` `prio::p1` `M2` deps: T-403, T-508
  Qwen3-Reranker-0.6B via the ggml-org GGUF conversion (no official Qwen GGUF exists; community conversions missing cls.output.weight are broken), SHA-256 pinned; never served from the same llama.cpp context as embeddings (upstream all-zero-output defect). AC: rerank endpoint scores sanity fixture; integrated behind a flag; the promotion decision itself is T-509's deliverable.
- **T-407 · Onboarding model provisioning** `lane::frontend` `prio::p0` `M2` deps: T-401, T-1001
  Required-model step with progress, resume, and the no-zero-vector capture block (PRD P0.2). AC: missing embedder blocks capture visibly; install-from-URL to first captured memory with real embeddings under 15 minutes on a 50 Mbps connection (PRD P0.10), timed on the clean-VM checklist.
- **T-408 · Spike: Qwen3-VL through llama-cpp-2 mtmd** `lane::ml` `prio::p1` `M1` deps: T-401
  Time-boxed one week: Qwen3-VL-2B through the published crate's experimental mtmd feature (image plus prompt to grounded JSON), building prompts via mtmd markers, not the chat-template helper (known-broken upstream). AC: go/no-go note for the F2 VLM synthesis design; failure degrades to deterministic-only synthesis, by design.
- **T-409 · Re-embedding migration design note** `lane::ml` `prio::p1` `M3` deps: T-402
  One page: what happens when the pinned embedder is superseded (the dimension guard must not brick capture); uses the model-worker backfill priority class as the queue; states cost and UX on 8 GB. AC: reviewed note in docs; answers the "what happens when the embedding model updates" question.

## E05 · Retrieval and FNDR-Bench (M1 to M5)

- **T-501 · Eval corpus v0** `lane::ml` `prio::p0` `M1` deps: none
  Synthetic capture fixtures covering the hard-case taxonomy (exact identifiers, paraphrase, time-scoped, app-scoped, cross-session) plus labelled (query, expected-record) pairs; sanitized donation protocol for real sessions; a frozen held-out test split that CI and tuning never touch (published numbers come from it). AC: 150+ labelled pairs; corpus loader; held-out split frozen and documented.
- **T-502 · Bench harness and resource probes** `lane::ml` `prio::p0` `M1` deps: T-501
  `make bench`: Recall@5, MRR@10, latency p50/p95, RSS, storage, capture CPU %, and context-pack p95; real models only (mock cannot satisfy). All latency and resource numbers come exclusively from the reference machine, never CI. AC: one command produces the metrics file on the reference machine, covering every G3/P0.8 number.
- **T-503 · Chunker v2** `lane::ml` `prio::p0` `M2` deps: T-301
  Line-kind classification rewritten with real token counts and mandatory source byte spans. AC: span integrity property tests; v1 boundary cases green.
- **T-504 · Chunk-first write path** `lane::backend` `prio::p0` `M2` deps: T-503, T-402, T-202
  Chunks embedded and indexed at capture flush; parent rollup vector. AC: idempotent re-flush; parent-child integrity test.
- **T-505 · Routes and RRF fusion** `lane::ml` `prio::p0` `M2` deps: T-504, T-203
  Ported RetrievalRoute/RouteRunner; vector, keyword (Lance FTS), temporal routes; RRF with metadata prefilters. AC: route timeouts and metrics; fusion rank tests; one stack serves search and packs.
- **T-506 · Ported ranking adjustments with attribution** `lane::ml` `prio::p0` `M2` deps: T-505
  Source-alignment ladder, diversity pass, staleness multipliers as named additive features in FusionSignals. AC: per-feature attribution visible in results; each feature has its own test and bench delta.
- **T-507 · Relevance gate, verifier, evidence pack** `lane::ml` `prio::p0` `M2` deps: T-505
  Three-state verdict, two-backer rule, evidence pack shape (ported). AC: verdict state machine tests; citations resolve to real records.
- **T-508 · FNDR-Bench v1 with baselines and CI gate** `lane::ml` `prio::p0` `M2` deps: T-502, T-505
  BM25-only, vector-only, POC-pipeline, and at least one off-the-shelf naive-RAG baseline on the same corpus (methods, not competitors); CI compares PRs to the committed baseline on the train split only; the held-out split is scored rarely and reported separately. AC: numbers committed; a deliberate regression PR is blocked; held-out numbers never appear in tuning loops.
- **T-509 · Reranker ablation** `lane::ml` `prio::p1` `M3` deps: T-406, T-508
  AC: promotion decision recorded with numbers; loser stays behind a flag.
- **T-510 · Chunk vs whole-record ablation** `lane::ml` `prio::p1` `M3` deps: T-508
  The v1 ADR-008 claim finally measured. AC: result published in the bench report.
- **T-511 · Latency instrumentation at scale** `lane::ml` `prio::p0` `M3` deps: T-505
  1M-row synthetic store; p50/p95 against PRD P0.8. AC: targets met or a remediation ticket opened before the gate.
- **T-512 · Context usefulness rubric and human review loop** `lane::ml` `prio::p0` `M2` deps: T-502
  Define the usefulness rubric for context packs and summaries (v1 pain point 1); weekly scored dogfood queries by all four builders with inter-rater agreement reported; LLM-judge harness for summary quality on real models (the v1 evals concept, done honestly). AC: rubric doc exists; first weekly scores recorded with agreement statistics; judge scores appear in the bench report; progress tracked against the G-metric of 80% rated useful-without-edits by month 6.
- **T-513 · Real-model eval CI lane** `lane::platform` `prio::p0` `M2` deps: T-502
  Per-PR quality gate on Linux runners (cached Q8_0 GGUF, small fixed corpus, Recall@5/MRR only) plus a nightly or label-triggered macOS parity lane; hosted macOS runners have no Metal and 3 vCPUs, so no latency or RAM assertion ever runs in CI (those come from the reference machine via T-502). AC: PR gate under 15 minutes; nightly lane green; a mock embedder cannot pass either lane (P0.7).
- **T-514 · Faithfulness slice: labelled unanswerable queries** `lane::ml` `prio::p0` `M2` deps: T-501
  Bench queries whose correct output is NotEnoughEvidence, so verdict overclaiming becomes a measured regression; plus a small claim-support audit in the weekly rubric (citations must support, not merely resolve). AC: verdict-accuracy metric in the bench report; an overclaiming regression blocks like any other bench regression.
- **T-515 · Visual similarity search (SigLIP 2)** `lane::ml` `prio::p1` `M5` deps: T-401
  Image-embedding similarity over captures behind the same privacy gates (PRD P1). AC: image-to-image retrieval on a fixture set; storage and RAM within budget; off by default until the bench slice exists.

## E06 · Memory synthesis and review (M2)

- **T-601 · Port deterministic insight derivation** `lane::ml` `prio::p0` `M2` deps: T-301
  Insight fields, low-RAM fusion fallback, durable-context composer. AC: v1 tests green; no-LLM path produces complete records.
- **T-602 · Port VLM synthesis prompts and grounding validation** `lane::ml` `prio::p1` `M2` deps: T-405, T-601
  MEMORY_SYNTHESIS_PROMPT, VOICE_RULES, parse clamps, grounding validator, narration filter minus the v1 self-referential regex hack. AC: prompts byte-identical to v1; validator adversarial tests.
- **T-603 · Single embedding-document composer** `lane::ml` `prio::p0` `M2` deps: T-402, T-601
  One composer for capture and review (fixes v1 drift); insight-first embedding text with provenance manifest. AC: capture and review produce identical embedding text for identical records.
- **T-604 · Review worker on durable queue** `lane::ml` `prio::p0` `M2` deps: T-201, T-403, T-603
  Attempt caps, backoff, per-record lock, pressure gating, skip-vs-fail classification. AC: deterministic-failure record retries capped; queue survives restart.
- **T-605 · Daily consolidation pass** `lane::ml` `prio::p1` `M2` deps: T-604
  Per-record locking (never batch-holding the model queue), dry-run mode, local-day handling. AC: v1 daily tests ported; capture never starved during a pass.
- **T-606 · Lifecycle end to end** `lane::frontend` `prio::p1` `M2` deps: T-604
  Enum through IPC to vault chips. AC: five states render; reviewed summary preferred in previews.

## E07 · MCP server and agent context (M2 to M5)

- **T-701 · MCP transport with auth-always** `lane::backend` `prio::p0` `M2` deps: T-101
  Official Rust SDK, streamable HTTP, bearer required in all modes, constant-time compare, origin/host allowlist, audit log, owner-only discovery file (ADR-007). AC: unauthenticated and cross-origin calls rejected by test; the two v1 audit holes are named regression tests.
- **T-702 · Tools wave 1** `lane::backend` `prio::p0` `M2` deps: T-701, T-505
  search, context_pack, timeline, recall, project_context, active_focus, privacy_status, source_evidence. AC: schema round-trip and auth-failure tests per tool; served by the same engine functions as the UI.
- **T-703 · Tools wave 2** `lane::backend` `prio::p0` `M3` deps: T-702
  delta, open_target, explain_retrieval, feedback, remember_decision (only memory-mutating tool). graph_context is split out to T-710 so a graph slip cannot block gate-critical tools. AC: same test bar; delta returns only changes since timestamp.
- **T-704 · Resources and prompt registry** `lane::backend` `prio::p1` `M3` deps: T-701
  Three fndr:// resources; prompts with real per-prompt argument schemas. AC: resources/list and prompts/get round-trip.
- **T-705 · Scoped tokens, revocation, rate limits** `lane::backend` `prio::p0` `M3` deps: T-701
  Per-client tokens, per-tool limits. AC: revoked token 401s on next call; limit test.
- **T-706 · MCP contract doc** `lane::backend` `prio::p0` `M3` deps: T-703
  docs/mcp.md v2 with per-tool schemas and the v1-to-v2 mapping. AC: doc examples execute against a dev server.
- **T-707 · Connect-your-agent onboarding** `lane::frontend` `prio::p0` `M3` deps: T-702
  Claude Desktop/Code config snippets, token issuance UI. AC: a fresh install reaches a working agent call following only the UI.
- **T-708 · Tunnel and public mode docs and tests** `lane::platform` `prio::p1` `M3` deps: T-705
  AC: non-loopback bind refuses to start without auth and TLS.
- **T-709 · Session Story tool** `lane::backend` `prio::p1` `M5` deps: T-702
  `fndr.session_story` (ratified into ADR-007 as a P1 addition): cited narrative reconstruction of a captured work session (what happened, what changed, why), exportable as markdown for demos, interviews, and standups (v1 pain point 8). Composition logic pairs with the ml lane. AC: story over a real captured session with every claim citing records; export renders standalone; ships with schema round-trip test, auth-failure test, rate limit, and docs entry per the tool-addition rule.
- **T-710 · graph_context tool** `lane::backend` `prio::p1` `M4` deps: T-702, T-1102
  Bounded typed-graph neighborhood over MCP, split from wave 2 so it rides the graph epic's schedule. AC: real data from fndr-graph; the four tool artifacts per the tool-addition rule.
- **T-711 · Grounded Q&A tool (fndr.answer)** `lane::ml` `prio::p1` `M5` deps: T-702, T-514
  Ratified into ADR-007 as a P1 addition: answers composed over full chunk text within a real token budget, per-claim citation checks, three-state verdict; the v1 answer path (1,000-char context, 4-extension validator) is the named anti-pattern. AC: faithfulness slice (T-514) covers it; the four tool artifacts.
- **T-712 · Agent field test against the dev server** `lane::backend` `prio::p0` `M2` deps: T-702
  Wire Claude Code and Claude Desktop to the dev server the week wave 1 lands; run 10 scripted agent tasks; record tool-selection quality, schema friction, and token budgets. AC: findings ticketed and feeding T-703 schemas; the first real-agent contact does not wait for the gate month.

## E08 · Privacy and safety (M1 to M3)

- **T-801 · Blocklist v2** `lane::backend` `prio::p0` `M1` deps: T-101
  Exact-token app matching and proper suffix-domain matching; v1 false-positive cases as tests. AC: short-name substring and parent-domain escalation bugs cannot recur.
- **T-802 · Sensitive-context detection as data** `lane::backend` `prio::p1` `M1` deps: T-801
  Configurable keyword/domain lists (ported v1 safety-gate data). AC: alert queue with dismissal keys; lists editable without code changes.
- **T-803 · Safety gate live on the write path** `lane::backend` `prio::p0` `M2` deps: T-306, T-801
  Allow/Redact/SkipStorage with secret-pattern redaction, on the storage path (PRD P0.4). Moved to M2 so it lands before dogfooding stores months of the team's own unredacted content; adversarial-suite hardening continues into M3 (T-806 window). AC: adversarial suite per class (secrets, password managers, banking, medical, private browsing) passes; redactions logged locally; pre-gate dogfood stores are purged (see T-905).
- **T-804 · Pause, incognito, menu-bar controls** `lane::frontend` `prio::p0` `M1` deps: T-306
  Push status events, no polling. AC: toggle reflects in UI within one tick; incognito also pauses review work.
- **T-805 · No-screenshot-persistence test** `lane::backend` `prio::p0` `M1` deps: T-306
  AC: CI test asserts no pixel bytes or paths persist from any path, including url-only and autofill flows.
- **T-806 · PRIVACY.md** `lane::backend` `prio::p0` `M3` deps: T-103, T-803
  The boundary, egress list, verification recipe (ADR-004). AC: recipe reproduces on a clean checkout.

## E09 · Shell, onboarding, release (M1 to M3, M6)

- **T-901 · Tauri shell foundation** `lane::platform` `prio::p0` `M1` deps: T-101
  Tray, autostart, single instance, main window, event bridge. AC: engine runs headless with the window closed.
- **T-902 · Permissions onboarding** `lane::platform` `prio::p0` `M2` deps: T-901, T-302
  Screen-recording flow with plain privacy story, preceded by a verify-it-yourself trust moment (live egress counter at zero, one-click audit log, pause/incognito demonstrated) before the permission prompt; health re-check on updates (TCC mitigation). AC: denial and revocation states have guidance, never silence; the trust screen renders before the prompt.
- **T-903 · Release pipeline** `lane::platform` `prio::p0` `M2` deps: T-102
  Tag builds signed DMG plus updater manifest (v1 ADR-013 carried). AC: vN to vN+1 in-place update verified.
- **T-904 · Notarization integration** `lane::platform` `prio::p1` `M6` deps: T-903
  When the Apple account exists. AC: stapled build passes spctl.
- **T-905 · Clean-VM QA checklist and demo-gate script** `lane::platform` `prio::p0` `M3` deps: T-903, T-707, T-803, T-703
  The PRD month-3 gate as an executable checklist. The demo script's spine is the counterfactual cut (the identical agent task with FNDR off, agent interrogates the user, then FNDR on, one tool call) and the live privacy negative (visit a bank and a password manager on camera, then prove absence in the vault, the pack, and privacy_status). The install step scripts the right-click Open path (ad-hoc signing; PRD open question 1 default). Includes purging pre-T-803 dogfood stores. AC: gate run recorded end to end on a clean machine.
- **T-906 · Omnibar NSPanel spike** `lane::platform` `prio::p1` `M1` deps: T-901
  Time-boxed one week (ADR-001 action). AC: non-activating panel with global hotkey demonstrated or the native-fallback note written.
- **T-907 · fndr doctor and setup diagnostics** `lane::platform` `prio::p0` `M2` deps: T-901
  One command and panel checking permissions, models, stores, indexes, sidecar, and MCP reachability with pass/fail reasons and an exportable report for debugging someone else's machine (v1 pain points 2 and 4). AC: a seeded fault matrix (revoked permission, missing model, stale index, dead sidecar) is diagnosed correctly.
- **T-908 · Month-3 gate dry-run** `lane::platform` `prio::p0` `M3` deps: T-702, T-903
  Run the full gate script two weeks before M3 close so failures are discovered with time to react instead of at the gate. AC: dry-run executed; every failure has an owner and a ticket before the real gate.

## E10 · Core UI (M1 to M4)

- **T-1001 · UI foundation** `lane::frontend` `prio::p0` `M1` deps: T-105
  Domain taxonomy, Zustand store, generated bindings, token theme base, useTauriEvent port. AC: no hand-written IPC types; no polling hooks for always-on state.
- **T-1002 · Search and result cards** `lane::frontend` `prio::p0` `M2` deps: T-1001, T-505, T-1405
  Surfacing reasons and signals visible; grounded-card prompt path with deterministic fallback. AC: card renders reason and citations; low-confidence prefix honored.
- **T-1003 · Memory vault v1** `lane::frontend` `prio::p0` `M2` deps: T-1001, T-1405
  Browse, lifecycle chips, delete flows wired to deletion-everywhere. AC: delete from vault verifiably empties search and evidence.
- **T-1004 · Pipeline health panel** `lane::frontend` `prio::p0` `M2` deps: T-1001, T-1405
  Per-stage health (ok, degraded, blocked, with reasons), SkipReason counters, model and queue status, bench snapshot, and local usage counters (queries, packs, deltas served per day) so retention is measured, not self-reported. AC: idle app performs zero status IPC; every v1 "mysteriously not working" class (silent capture stop, silent summary fallback) surfaces here with a reason; a builder diagnoses a seeded missing-capture case in under 2 minutes (PRD legibility metric).
- **T-1007 · Capture-explain** `lane::backend` `prio::p0` `M3` deps: T-306, T-309, T-1004
  Answer "why was this moment not captured" from retained gate outcomes: given a time range, return the stage and reason each frame stopped at (blocklist, dedup, admission, low-signal, pressure). AC: a seeded set of skipped moments is explained correctly from the UI.
- **T-1005 · Settings** `lane::frontend` `prio::p0` `M2` deps: T-1001, T-1405
  Blocklist, retention, models, MCP tokens and revocation. AC: every P0 privacy control reachable.
- **T-1006 · Timeline view** `lane::frontend` `prio::p1` `M4` deps: T-1002
  AC: session/day grouping consistent with fndr.timeline output.
- **T-1008 · Sample vault and empty-vault state** `lane::frontend` `prio::p0` `M2` deps: T-501, T-1003
  Loadable sample vault built from the T-501 fixtures ("explore a sample day" in onboarding, also used by the demo and by evaluators who install), plus an explicit designed empty-vault state so day-one is an experience, not a dead end (cold-start fix). AC: fresh install can explore the sample day; empty vault explains what will appear and when.
- **T-1009 · Morning digest** `lane::frontend` `prio::p1` `M4` deps: T-505
  Deterministic "yesterday" digest in the menu bar (composition over session records, no VLM required): a daily payoff that requires no invocation habit. AC: digest renders each morning from real capture; dismissible; zero LLM dependency. (Promotion to M3/P0 considered 2026-08-21; deferred to sprint planning with the capacity re-cut.)

## E11 · Knowledge graph (M4)

- **T-1101 · Entity extraction and graph store** `lane::backend` `prio::p0` `M4` deps: T-201, T-601
  UUIDv5 identity, confidence weights, 0.4 edge floor, no fabricated conflict edges; SQLite tables with indexes. AC: v1 stable-id tests green; extraction off the hot path.
- **T-1102 · Traversal and GraphPlan wiring** `lane::backend` `prio::p0` `M4` deps: T-1101
  Recursive-CTE neighborhood, paths with labeled steps, intent-to-edges table. AC: bounded traversal latency test; feeds the graph_context tool (T-710).
- **T-1103 · Real community detection** `lane::backend` `prio::p1` `M4` deps: T-1102
  Actual Louvain (modularity), community naming. AC: known-graph fixture yields expected partitions (the v1 connected-components bug is the named test).
- **T-1104 · 3D graph rebuild** `lane::frontend` `prio::p1` `M4` deps: T-1102, T-1401
  Direct typed-schema consumption, instanced nodes, batched edges, token colors, 29-to-5 relationship mapping port. AC: 2k nodes at 60 fps on the reference machine; zero hardcoded colors.
- **T-1105 · Graph-route eval experiment** `lane::ml` `prio::p1` `M4` deps: T-1102, T-508
  AC: promotion decision recorded with bench numbers (ADR-006 gate).

## E12 · Meetings (M4)

- **T-1201 · Swift sidecar** `lane::platform` `prio::p0` `M4` deps: T-403
  FluidAudio (Parakeet + pyannote), SCK system audio, versioned stdio protocol, supervised lifecycle. AC: transcription and diarization on a fixture recording; typed unavailable states.
- **T-1202 · Meeting ingestion** `lane::ml` `prio::p0` `M4` deps: T-1201, T-306
  Sessions and segments through the same write path as capture; speaker labels; links to concurrent screen context. AC: meeting content retrievable and citable like any memory.
- **T-1203 · Meetings UI** `lane::frontend` `prio::p1` `M4` deps: T-1202
  Record controls, live status, transcript view with speakers, search. AC: recording state always visible.
- **T-1204 · Meeting privacy integration** `lane::backend` `prio::p0` `M4` deps: T-1202
  Blocklist and incognito respected. AC: incognito blocks recording start; test.
- **T-1205 · Meetings consent defaults and design note** `lane::backend` `prio::p0` `M4` deps: none
  Meetings record other people, and several jurisdictions require all-party consent. Off by default; explicit per-meeting start action (never ambient); the visible recording indicator; a distinct, shorter retention default for meeting transcripts; a short consent design note in docs. AC: defaults implemented; note reviewed by all four.
- **T-1206 · Model attribution notices** `lane::platform` `prio::p1` `M4` deps: T-1201
  CC-BY-4.0 attribution for Parakeet and pyannote in the app's acknowledgements (a redistribution requirement). AC: acknowledgements surface lists all model licenses.

## E13 · Assistant surfaces (M5)

- **T-1301 · Omnibar** `lane::frontend` `prio::p0` `M5` deps: T-906, T-1002
  Ported keyboard/a11y/race-guard model; search plus ask modes. AC: v1 interaction contract preserved; component tests (v1 had zero).
- **T-1302 · Smart clipboard** `lane::frontend` `prio::p1` `M5` deps: T-803
  History with privacy guards, autofill overlay. AC: blocklisted sources never enter history.
- **T-1303 · Task extraction and panel** `lane::ml` `prio::p1` `M5` deps: T-604
  Extraction via the model queue; real task state. AC: no placeholder UI; extraction precision spot-check documented.
- **T-1304 · Focus mode** `lane::frontend` `prio::p1` `M5` deps: T-1001
  Drift detection surfaced from capture continuity signals. AC: event-driven, no polling.
- **T-1305 · Proactive resurfacing** `lane::ml` `prio::p1` `M5` deps: T-505
  Rate-limited, quiet, dismissible. AC: frequency cap tested; off by default until quality bar met.
- **T-1306 · Project context file export** `lane::ml` `prio::p0` `M3` deps: T-702
  Generate a refreshable agent warm-start file (CLAUDE.md-style) per project from context packs, regenerated daily, for agents without MCP access (v1 pain point 7 differentiator). Pulled into the spine: it improves every agent session the builders already run with zero invocation habit, which is the retention loop. AC: a fresh agent session using only the exported file answers project questions correctly; daily regeneration verified.
- **T-1307 · Claude Code session warm-start hook** `lane::backend` `prio::p1` `M3` deps: T-702, T-1306
  A repo-installable hook plus MCP flow that injects "what was I doing in this project" (from the warm-start file or a live context pack) into a fresh agent session automatically; zero-invocation daily value (owner-selected 2026-08-21). AC: a fresh Claude Code session in a captured project answers "where did I leave off" with no manual prompt; degrades silently to nothing when FNDR is not running, never blocking the session.
- **T-1308 · Result feedback capture loop** `lane::frontend` `prio::p1` `M2` deps: T-1002, T-512
  One-tap good/bad on results in the search UI plus wiring the founding `fndr.feedback` MCP tool; both export labelled pairs into the bench corpus pipeline so daily use improves retrieval quality (owner-selected 2026-08-21). AC: feedback lands as labelled pairs a bench corpus build can consume; no feedback data ever leaves the machine.

## E14 · Design system (M1 foundation, M4 to M5 completion)

- **T-1401 · Token system and palettes port** `lane::frontend` `prio::p0` `M1` deps: T-1001
  cinematic-palettes with tests, semantic tokens, dark/light. AC: v1 palette tests green; no raw hex in components (lint).
- **T-1405 · Component library** `lane::frontend` `prio::p0` `M1` deps: T-1401, T-1404
  Shared primitives (button, panel, card, input, chip, dialog) built once against the spec and tokens; feature UI may not hand-roll primitives (v1 pain point 9). AC: gallery page renders every primitive in every state; a lint or review rule blocks ad-hoc primitives.
- **T-1402 · Wallpaper system** `lane::frontend` `prio::p1` `M4` deps: T-1401
  GLSL aurora fields driven by palette triples. AC: palette switch updates wallpaper live.
- **T-1403 · Theming completeness pass** `lane::frontend` `prio::p1` `M5` deps: T-1401, T-1104
  Every surface including the 3D graph on tokens. AC: token-coverage lint clean.
- **T-1404 · Design language spec** `lane::frontend` `prio::p0` `M1` deps: none
  Type scale, spacing, color roles, elevation, motion, and component inventory, written before feature UI exists; includes the explicit anti-slop bar (no gradient soup, no boxes-in-containers, no ad-hoc buttons, consistent fonts). AC: reviewed by all four; every later UI ticket links to it.

## E15 · Proof, companion contract, ship (M5 to M6)

- **T-1501 · Benchmark page and FNDR-Bench public release** `lane::ml` `prio::p0` `M5` deps: T-508, T-511, T-514
  Published FNDR-Bench numbers from the frozen held-out split, resource budgets, methodology with number lineage (records-per-day to corpus-size derivations), reproduce instructions, plus public release of the corpus and harness with an external-submission track. Framing: the first public benchmark for screen-derived personal memory retrieval, with FNDR as the reference implementation; an honestly published loss on a slice is itself credibility. AC: numbers regenerate via make bench on the reference machine; corpus and harness public; inter-rater agreement reported.
- **T-1502 · Docs site** `lane::platform` `prio::p0` `M6` deps: T-706, T-806
  Install, privacy, MCP contract, architecture, plus a "limits and failure modes" page fed by the incidents-and-reversals log (the staff-interview artifact). Promoted to p0: two PRD goals depend on it. AC: builds from repo; linked from README.
- **T-1503 · Demo video** `lane::frontend` `prio::p0` `M6` deps: T-905
  The 3-minute story built on the counterfactual cut (same agent task, FNDR off then on) and the live privacy negative, then graph/omnibar/theming beats; show the pack artifact itself, not just the chat output. AC: final cut published.
- **T-1504 · Companion API contract v2 and relay design note** `lane::platform` `prio::p0` `M5` deps: T-702
  Spec-only: v1 contract carried with pair-start off the network, permission scopes, WS push section; relay note is design-only (ADR-004 P2). AC: contract reviewed against the engine API; no code shipped.
- **T-1505 · v1.0.0 release** `lane::platform` `prio::p0` `M6` deps: T-903, T-1501, T-1503
  Tagged release, updater manifest, announcement README rewrite. AC: clean-machine install passes the QA checklist; PRD G-metrics reviewed and recorded.
- **T-1506 · Hardening and performance pass** `lane::platform` `prio::p0` `M6` deps: T-1501
  Systematic pass against every published target (latency, RSS, CPU, storage, pack p95) plus soak and failure-injection sweeps; misses are release blockers or written exceptions. AC: all published numbers re-measured on the release candidate; exceptions documented.

---

## E16 · Codebase Memory (owner mandate 2026-08-21: top priority)

A reusable codebase-intelligence subsystem: persistent AST-derived code
knowledge graph, graph-first retrieval, and Claude Code integration,
FNDR-first but installable into arbitrary repositories. The document of
record for scope is `docs/specs/codebase-memory-brief.md`; do not re-derive
its requirements here. Keep its schemas isolated from user memory (brief
section 21). Milestone placement is decided at sprint planning against the
deferred capacity re-cut; the owner set this above existing feature work.

- **T-1601 · Codebase Memory kickoff: inspection and implementation plan** `lane::backend` `prio::p0` deps: none
  Execute brief section 28: inspect the existing CLI, crates, persistence, MCP, and agent integration surfaces, then produce the implementation plan (modules, graph schema, indexing, retrieval, Claude Code integration, storage choice, incremental strategy, testing, phase order) as an ADR plus plan doc. No code before the plan. AC: plan reviewed; phase tickets below re-cut into real tickets appended to this epic.
- **T-1602 · Phase 1: AST extraction, graph schema, persistent storage, basic CLI** `lane::backend` `prio::p0` deps: T-1601
  AC: a repository indexes locally and deterministically; the graph survives process exit; fixture-repo tests define expected nodes and edges.
- **T-1603 · Phase 2: relationship resolution, traversal, impact analysis** `lane::backend` `prio::p0` deps: T-1602
  AC: callers/dependents/path/impact queries return compact subgraphs with provenance and confidence; ambiguity preserved, never fabricated.
- **T-1604 · Phase 3: Claude Code skill, MCP interface, graph-first retrieval** `lane::backend` `prio::p0` deps: T-1603
  AC: a fresh Claude Code session discovers and queries the graph before broad exploration (graph first, source second, raw search when necessary); raw tools never blocked.
- **T-1605 · Phase 4: incremental updates, git integration, architecture summaries** `lane::backend` `prio::p0` deps: T-1604
  AC: changed files reindex incrementally (never full rebuilds); GRAPH_REPORT projection regenerates; stale-state detection via file hash, parser version, schema version, commit.
- **T-1606 · Phase 5: semantic enrichment, rationale/ADR links, visualization** `lane::backend` `prio::p1` deps: T-1605
  AC: enrichment is optional, explicit, and local-capable; extracted rationale distinguished from inferred; visualization supports filtered exploration.

---

## Import notes (GitLab)

1. Create the six milestones and the label set (Conventions above) once, manually.
2. Either import `tickets.csv` (Issues > Import CSV; descriptions carry `/label` and `/milestone` quick actions so metadata applies on creation), or create issues per epic from this file.
3. Dependencies are recorded in each description (`deps:`); GitLab linked-issues can be added lazily as work approaches.
4. Epics: if on GitLab Premium, create E01 to E16 as epics and assign; on Free, the `epic::Exx` labels serve as the grouping.
