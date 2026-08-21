# FNDR v2: Product Requirements Document

Status: Approved 2026-08-20 (drafted 2026-08-19; revised 2026-08-20: team pain points folded in, plan-review fixes applied per `review/REVIEW-2026-08-20.md`; owner defaults recorded in section 13). Implementation started 2026-08-20; progress ledger in `docs/ROADMAP-TICKETS.md`.

**Priority insertion (2026-08-21, owner mandate):** Codebase Memory, a reusable codebase-intelligence subsystem (persistent AST-derived code knowledge graph with graph-first retrieval and Claude Code integration, installable into arbitrary repositories), is the top-priority feature going forward. Scope of record: `docs/specs/codebase-memory-brief.md`. Roadmap: epic E16 (kickoff T-1601 produces the implementation plan before any code). Its schemas stay isolated from the user-memory system per the brief. Scheduling against existing milestones is resolved at sprint planning.

Produced from the v2 discovery brief, a full code audit of the v1 proof of concept, and fresh technology research. Companion documents: `docs/ARCHITECTURE.md`, `docs/decisions/ADR-001` through `ADR-007`, `docs/ROADMAP-TICKETS.md`.

---

## 1. Problem statement

Everyone who works on a computer accumulates context (what they read, decided, debugged, wrote, and abandoned) that evaporates between sessions. When they later use an LLM or agent, they must re-explain who they are, what they were doing, and why, every single time. Existing memory tools either upload raw personal activity to a cloud service (unacceptable for private data) or stop at keyword search over screenshots (useless as agent input).

FNDR makes captured desktop context automatically available to any LLM or agent, locally. It watches the screen, distills non-sensitive activity into structured memory, and serves that memory over MCP so agents can pull real, cited, time-aware context instead of asking the user to paste it.

The v1 proof of concept (5.5 months, solo, ~111k lines, shipped v0.3.0) proved the pipeline end to end but the code audit found the core promise unvalidated: retrieval quality was never measured against real models, the flagship chunk-RAG read path never shipped, no table has any index, and the shipped defaults expose the memory store to any local web page. v2 is a from-scratch rebuild that keeps the product vision and the POC's hard-won heuristics, and replaces its architecture.

## 2. Product identity

**Headline capability: injectable agent context via MCP.** "Give any LLM or agent your real context without manual copy-paste." Every other feature exists to make that context richer, safer, or more legible.

Positioning pillars (the story every surface tells):

1. **Agent-native.** MCP is the primary interface. The desktop UI consumes the same engine API the agents do.
2. **Local-only, provably.** Capture, storage, embeddings, and all reasoning over captured data never leave the device. Enforced mechanically (dependency gates in CI), not just promised.
3. **Shows its work.** Every retrieved memory carries a surfacing reason. Every context pack carries citations. Every answer carries a grounded / partial / not-enough-evidence verdict. No silent failure anywhere in the pipeline.
4. **Measured.** Published retrieval-quality, latency, RAM, and storage numbers from a reproducible benchmark (FNDR-Bench). If a ranking heuristic has no eval number, it does not merge.

## 3. Users

- **The owner (primary human).** A technical professional on an Apple Silicon Mac who works across editor, terminal, browser, chat, and meetings, and uses LLMs/agents daily. Privacy-sensitive: would never install a cloud screen recorder.
- **The agent (primary machine user).** Any MCP client (Claude Code, Claude Desktop, IDEs, custom agents) that needs the owner's working context. Agents are a first-class persona: tool schemas, token budgets, and delta updates are designed for them.
- **The evaluator.** A hiring manager, collaborator, or open-source user assessing the project from its demo, benchmark page, and codebase. Served by the engineering-rigor and polish goals.
- **The teammate.** The four builders. Served by the architecture's layer boundaries and the repo's engineering skill.

Selected user stories (full decomposition in `ROADMAP-TICKETS.md`):

- As an owner, I want capture to run all day at negligible cost so that I never think about it.
- As an owner, I want blocked apps, sites, and sensitive contexts to never enter storage so that I can trust what the system remembers.
- As an agent, I want a context pack for "what was the user doing on X" with citations and a token budget so that I can resume the user's work without interrogating them.
- As an agent, I want a delta since my last query so that repeated calls stay cheap.
- As an owner, I want to search memories by meaning, keyword, time, or app in under half a second so that recall feels instant.
- As an owner, I want to see why any result surfaced and delete anything permanently so that the memory stays mine.
- As an owner, I want meetings transcribed and diarized into the same memory so that spoken context is retrievable too.
- As an evaluator, I want a benchmark page with reproducible numbers so that the quality claims are verifiable.

## 4. Goals

| # | Goal | Measure at month 6 |
|---|---|---|
| G1 | The MCP spine is the best local agent-context source available | Month-3 demo gate passed (see §10); 14 founding MCP tools plus two ratified P1 additions, contract-documented; context pack p95 latency under 2 s |
| G2 | Retrieval quality is measured and beats baselines | FNDR-Bench v1 published from a frozen held-out split; hybrid pipeline beats BM25-only, a naive-RAG baseline, and the POC pipeline re-run on the same corpus (+15 points Recall@5 over BM25-only as the stretch figure); verdict faithfulness measured on the unanswerable slice; numbers on a public page |
| G3 | Resource footprint fits an 8 GB Mac | Idle RSS under 400 MB, active capture under 900 MB, capture CPU under 5% average; storage under 2 GB per active month at default retention (all published) |
| G4 | Privacy is enforceable, not aspirational | Local-only egress lint green in CI; redaction/safety gate on the storage write path with adversarial tests; zero raw pixels persisted; every network surface authenticated unconditionally |
| G5 | The project is a credible portfolio centerpiece | 3-minute demo (capture day, then agent resumes work via MCP); polished UI including graph, omnibar, theming; docs site; reproducible benchmark |

Budget notes for G3: idle RSS counts resident pages (mmap-mapped model weights are evictable and mostly paged out at idle); the session-scoped meeting stack (roughly 1 to 1.5 GB in the sidecar process) is budgeted separately and released when the meeting ends. Latency and resource numbers come exclusively from `make bench` on the reference machine, never from CI runners.

## 5. Non-goals (v2, 6-month window)

- **No mobile app engineering.** The iOS/Watch companion is deferred. We ship a versioned companion API contract (spec only, month 5) so a companion can attach later. Rationale: the spine must be excellent before satellites.
- **No cloud or relay component, no opt-in cloud inference.** Local-only is a hard invariant (ADR-004). The engine's API layer is designed so a future self-hosted relay does not force a rewrite, but none is built.
- **No Windows/Linux.** macOS 14+ only, Apple Silicon primary (an x86_64 target is a P1 stretch). The perception layer is deliberately Apple-native.
- **No competitor benchmarking in this PRD.** Per discovery ruling, the document argues from the technical vision only.
- **No autonomous agent execution.** FNDR provides context to agents; it does not run its own tool-executing agent. The POC's agent-runner surface is dropped. Agents get one memory-mutating tool (the decision ledger); feedback is logged append-only and never mutates memories or ranking.
- **No general chatbot.** Grounded Q&A exists as a thin layer over context packs, but the product optimizes for feeding other agents, not for being the chat interface.
- **No telemetry of any kind.** All quality signals stay local; benchmark numbers come from opt-in local runs.

## 6. Lessons taken from the POC (what "rebuild properly" means)

Keep (port or re-implement faithfully; full inventory in ADR-005):

- The tuned perception heuristics: OCR cleanup and salience scoring, capture admission policy, merge/continuity scoring, grounding validators, session identity.
- The contract discipline: embedding contracts (model, file, dimension, table move together), dimension guards, pinned checksummed model downloads, required-model gating.
- The explainability primitives: surfacing reasons, verifier state machine, grounded snippet citations, memory review lifecycle.
- The product vocabulary: graph node/edge taxonomy, MCP tool concepts, memory synthesis prompt voice rules, companion API contract.

Fix by construction (the audit's structural findings, addressed in architecture):

- One retrieval stack, not two divergent ones. One graph schema, not three. One event pattern, not polling plus push.
- Real indexes (vector, FTS, scalar) from day 1; SQLite as durable truth with LanceDB as a rebuildable derived index.
- Eval-first ranking: no heuristic constants without a benchmark number; evals run real models, never mocks.
- Auth-always on every network surface, from the first commit.
- No monster modules: the POC's 1,900-line capture loop, 5,200-line MCP file, and 104-field table are explicitly anti-patterns; the architecture doc sets size and boundary rules.
- Features ship whole or not at all: no decorative plumbing (graph routes that return zero results), no silently disabled flagship paths.

### Pain points from lived v1 use (added 2026-08-20)

The team's ten first-hand v1 pain points, each with its v2 answer and where it lands:

| # | v1 pain | v2 answer |
|---|---|---|
| 1 | Context not useful enough for human summary or agent use (make-or-break) | Context-quality program in F4: FNDR-Bench for retrieval plus a human usefulness rubric with weekly scored dogfood queries and an LLM-judge harness for summary quality (T-501, T-502, T-512); the month-3 gate demos usefulness, not plumbing |
| 2 | Mysterious silent failures, especially capture and summary | "No silent degradation" invariant plus pipeline legibility as a P0 feature: health panel, capture-explain, `fndr doctor` (P0.11, T-907, T-1004, T-1007) |
| 3 | Timeline and knowledge graph unusable | Full rebuilds with usability acceptance criteria, not visual ones (F6, T-1006, T-1104); graph retrieval eval-gated so it must earn its place |
| 4 | Setup on devices other than ours is a blackbox | Installability P0.10 plus `fndr doctor` with an exportable diagnostic report for debugging someone else's machine (T-907); clean-VM QA every release (T-905) |
| 5 | Inconsistent code written from scratch across Cursor, Antigravity, Claude Code, Codex | One conventions source every agent tool reads: the engineering skill mirrored into AGENTS.md with a CI drift check (T-106, T-107); reuse-first and port-provenance rules |
| 6 | No practices for token/AI-usage discipline or codebase hygiene | The engineering skill gains an AI-collaboration reference: context discipline, session handoffs, scheduled dead-code sweeps, ship-whole-or-not-at-all |
| 7 | Plan never challenged or improved; AI should think like a founder | Founder-review workflow in the skill (monthly, milestone-triggered) plus three differentiator bets added below (Session Story, warm-start file export, team work-memory) |
| 8 | Coding sessions unexplainable for interviews and demos without a scavenger hunt | Session Story: cited narrative reconstruction of any captured work session, as an MCP tool and exportable document (T-709) |
| 9 | UI reads as AI slop: bad fonts, gradients, boxes in containers, inconsistent primitives | Design language spec moves to month 1 and a shared component library to month 2, before feature UI multiplies (T-1404, T-1405); token lint; design review in the frontend lane checklist |
| 10 | No shared memory of who did what, where, across branches, agents, people | Near term: session-handoff convention and board discipline in the skill and CONTRIBUTING (T-106); long term: team work-memory is FNDR's own thesis applied to teams, tracked as a P2 differentiator |

## 7. Feature scope

All POC feature areas are kept and rebuilt. Each has a phase, an owner lane, and a quality bar.

### F1. Capture and privacy (phases 1 to 2)
Continuous foreground capture via ScreenCaptureKit, perceptual and semantic dedup, admission policy (navigation/listing surface skips), Apple Vision OCR with app-aware cleanup, adaptive sampling with idle detection. Privacy: blocklist (exact-token and suffix-domain matching, fixing POC false positives), sensitive-context detection, and the safety gate (allow / redact / skip-storage with secret-pattern redaction) live on the storage write path with adversarial tests. No raw screenshot persistence, verified by test. Pause, incognito, and per-app exclusions surfaced in UI and menu bar.

### F2. Memory synthesis and review (phase 2)
Structured memory records from OCR plus metadata; deterministic insight derivation always; optional local VLM synthesis (first-person narrative voice, grounding-validated, narration-filtered). Post-capture review worker (queue durable in SQLite, backoff and attempt caps, pressure-gated) and daily consolidation pass. Lifecycle states surfaced on every card.

### F3. Storage and indexing (phase 1)
SQLite (WAL) as the system of record: memory records (split schema, not 104 columns), tasks, meetings, graph, review queue, config. LanceDB as the derived retrieval index: text and image vectors, native BM25 FTS, metadata prefilters, IVF/scalar indexes, batched writes, scheduled compaction and version pruning. The index is rebuildable from SQLite at any time (crash-safety strategy). Retention and permanent deletion operate on both stores.

### F4. Retrieval and evals (phases 1 to 3, the quality core)
Parent-child chunk RAG as the primary design (chunks written and searched from day 1). Hybrid retrieval: vector plus BM25 with RRF fusion, metadata filters, then an optional model reranker stage promoted into the default pipeline only on a measured bench win. Route-based pipeline (vector, keyword, temporal, graph, entity) behind one stack with per-route timeouts and metrics. Explainability: surfacing reasons and per-result signals. FNDR-Bench: a labelled eval corpus (synthetic capture fixtures plus donated real sessions) with a frozen held-out test split that tuning and CI never touch, a faithfulness slice of labelled unanswerable queries (the correct output is NotEnoughEvidence, so overclaiming is a measured regression), real models only, published metrics. Graph-aware retrieval ships only if it beats the hybrid baseline on the bench. Retrieval metrics alone do not prove usefulness, so F4 also owns the context-quality program: a human usefulness rubric scored weekly by all four builders on real queries, and an LLM-judge harness for summary quality; both feed the bench report.

### F5. MCP server and agent context (phases 2 to 3 core, P1 tools phase 5, the headline)
Rust MCP SDK, streamable HTTP, current spec. 14 founding tools in one namespace covering: search, context pack (with depth and token budget), timeline, active focus, project context, recall (decisions, errors, blockers, todos), source evidence (raw text gated), graph context, delta-since-timestamp, open target, retrieval explanation, feedback, privacy status, plus one memory-write tool (remember decision); two ratified P1 additions (session story, grounded answer) follow ADR-007's tool-addition rule. Resources and prompt templates carried over from the POC concepts. Auth required in every mode, strict origin and host checks, constant-time token compare, rate limits, audit log. Deployment modes local/tunnel/public with hardened defaults. A first-run "connect your agent" flow (Claude Desktop and Claude Code config snippets) treats agent connection as onboarding, not an advanced feature. P1 addition: `fndr.session_story`, a cited narrative reconstruction of a captured work session (what happened, what changed, why), exportable as a document for demos, interviews, and standups.

### F6. Knowledge graph (phases 3 to 4)
Deterministic entity extraction from finalized memory fields (stable UUIDv5 identities, confidence-weighted, no fabricated conflict edges), typed schema carried from the POC (14 node types, 29 edge types), stored in SQLite with real traversal queries. Graph context exposed over MCP. 3D visualization rebuilt against the real schema: instanced rendering, real community detection (actual Louvain), token-driven colors, direct consumption of typed nodes/edges. The 3D graph is a demo and comprehension surface; retrieval usage is gated on eval wins.

### F7. Meetings (phase 4)
Swift sidecar using FluidAudio (Parakeet ASR plus pyannote community diarization on the Neural Engine); ScreenCaptureKit system audio capture (no ffmpeg dependency). Transcripts with speaker labels ingested as first-class memories (searchable, citable, linked to concurrent screen context). Recording status always visible; meetings respect blocklist and incognito. Consent is a design surface, not copy: recording is off by default, starts only from an explicit per-meeting action (never ambient), and meeting transcripts carry a distinct, shorter retention default, because several jurisdictions require all-party consent.

### F8. Assistant surfaces (phase 5; warm-start export phase 3, morning digest phase 4)
Omnibar (global hotkey, non-activating panel, search plus ask, keyboard-first, carrying over the POC's race-guard and accessibility work), smart clipboard with privacy guards, focus mode and task panel (task extraction from memories with real state, not placeholders), proactive resurfacing (quiet, rate-limited).

### F9. Design system and theming (design language phase 1, full system phases 2 to 5)
The v1 UI's inconsistency is treated as a build-order bug, not a taste problem: the design language spec (type scale, spacing, color roles, elevation, motion, component inventory, and the explicit anti-slop bar) is written in month 1, and a shared component library (button, panel, card, input, chip, dialog) lands in month 2, before feature UI multiplies. Semantic token system and cinematic palette architecture carried from the POC (its strongest UI subsystem), self-hosted fonts, dark/light, GLSL wallpaper system. Every surface, including the 3D graph, consumes tokens and the component library; new primitives require a design review against the spec; no hardcoded colors (linted).

### F10. Shell, onboarding, release (phases 1 to 3, 6)
Tauri 2 shell with menu-bar presence, autostart, single-instance. Onboarding: a verify-it-yourself trust moment before the screen-recording prompt (live egress counter at zero, one-click audit log), permissions with plain privacy story, required-model download (pinned, checksummed, resumable), sample-vault exploration and a designed empty-vault state (day one is an experience, not a dead end), connect-your-agent step, and `fndr doctor` diagnostics with an exportable report. Release: CI test gate on every PR, tagged releases building signed DMG plus auto-updater, notarization when an Apple Developer account lands. Benchmarks and docs site published from the repo.

## 8. Requirements

### Must-have (P0)
The feature cannot ship without these; each maps to tickets in `ROADMAP-TICKETS.md`.

- **P0.1 Local-only enforcement.** Given any engine crate, when CI runs, then a dependency and egress lint proves no network calls exist outside the model-download and update allowlist. No captured data ever leaves the device.
- **P0.2 No zero-vector or unindexed writes.** Given a missing embedder, when capture runs, then frames are visibly blocked, never stored degraded (carries POC ADR-012 forward).
- **P0.3 No raw pixel persistence.** Given any capture path, when a record is stored, then no screenshot bytes or paths are persisted; asserted by test.
- **P0.4 Safety gate on the write path.** Given a secret pattern, password manager, banking/medical context, or blocklisted source, when storage is attempted, then the record is redacted or skipped per policy, with adversarial tests for each class.
- **P0.5 Authenticated surfaces.** Given any MCP endpoint in any mode, when called without a valid token, then the request is rejected; origin and host are validated; tests assert the failure closed. (Companion endpoints ship no code in v2; their auth conformance requirements are part of the T-1504 contract spec review instead.)
- **P0.6 The month-3 demo gate** (see §10) passes end to end.
- **P0.7 Eval harness with real models.** Given a ranking or retrieval change, when the per-PR Linux eval lane runs (cached real embedder, fixed corpus), then Recall@5 and MRR@10 are reported against the previous baseline and regressions block; a nightly macOS lane checks parity; latency and resource numbers come from the reference machine, never CI. Mock embedders cannot satisfy any lane.
- **P0.8 Hybrid retrieval at target latency.** Given 1M memory records with indexes, when a search runs, then p50 is under 150 ms and p95 under 500 ms on an M1 8 GB reference machine.
- **P0.9 Permanent deletion.** Given a delete (record, time range, domain, or everything), when it completes, then the data is gone from both stores and all indexes, verified by test.
- **P0.10 Installable.** Given a clean macOS 14+ machine, when a user installs from a URL, then onboarding reaches first captured memory with real embeddings in under 15 minutes including model download, and the app auto-updates thereafter.
- **P0.11 Pipeline legibility.** Given any stage of capture, synthesis, indexing, or review, when it skips, defers, or fails, then the health panel shows the stage, reason, and count; a user can answer "why was this moment not captured" from the UI (capture-explain); and `fndr doctor` produces an exportable diagnostic report for someone else's machine.

### Nice-to-have (P1)
- Reranker stage beating RRF-only on the bench (ship only on a win).
- Image-embedding visual similarity search (SigLIP 2), behind the same privacy gates.
- Proactive resurfacing toasts; clipboard autofill overlay.
- Grounded Q&A (`fndr.answer`) tool over context packs with per-claim citation checks (T-711).
- Session Story (`fndr.session_story`): cited narrative of a captured work session, exportable for demos, interviews, and standups (T-709).
- x86_64 build target.

(Project context file export was promoted into the month-3 spine as P0, T-1306: it is the daily retention loop.)

### Future considerations (P2, architectural insurance only)
- iOS/Watch companion consuming the v2 companion contract; minimal self-hosted relay for off-LAN pairing.
- Multi-monitor capture; browser-extension capture assist.
- Graph-augmented retrieval promoted into the default pipeline (currently gated on eval wins).
- Plugin/extractor system for third-party context sources.
- Team shared work-memory: who did what, where, on which branch, across people and agents, as a product surface. The single-user session-handoff convention and Session Story are its precursors; this is FNDR's own thesis applied to teams and the most credible B2B wedge.
- Apple Foundation Models as an optional Q&A/synthesis backend where available (OS-gated; design keeps the backend swappable).
- Multi-Mac: not in v2. Local-only permits a future LAN-sync design without a rewrite; until then `fndr backup`/`export` (T-209) is the portability answer, and the stance is documented so it reads as a decision, not an omission.

## 9. Success metrics

Leading (checked continuously from month 2):
- FNDR-Bench Recall@5 and MRR@10 vs the BM25-only baseline and vs the POC pipeline (target: +15 points Recall@5 over BM25-only; strictly better than POC on the same corpus).
- Search p50/p95 (target in P0.8); context pack p95 (target in G1); capture CPU %; idle/active RSS; storage per month.
- Grounding quality: share of context packs with all citations resolving to real records (target: 100%; failures are bugs).
- Context usefulness: weekly dogfood rubric (each builder scores five real context packs and summaries); target: 80% rated useful-without-edits by month 6.
- Legibility: time for a builder to diagnose a missing capture via the health panel; target under 2 minutes.
- Faithfulness: verdict accuracy on the labelled unanswerable-query slice; overclaiming regressions block like any other bench regression.
- Usage, measured not self-reported: queries, packs, and deltas served per builder per day, from the health panel's local counters.
- CI health: PR gate green rate, eval-regression blocks actually blocking.

Lagging (evaluated at months 3 and 6):
- Month-3 gate passed on the first attempt or with at most one week slip.
- Dogfood retention: all four builders running FNDR daily and using it through their own agents (self-reported weekly; the founding use case).
- Portfolio artifacts shipped: demo video, benchmark page, docs site, tagged v1.0.0 release with updater.
- External signal: stars/installs/issues from strangers (directional, no target).

Measurement method: the bench and resource numbers come from a reproducible `make bench` on a reference M1 8 GB machine and are committed to the repo with each release; no telemetry.

## 10. Phased roadmap

Two macro-phases with a hard gate between them. Detailed epics and tickets in `ROADMAP-TICKETS.md`.

### Months 1 to 3: the spine

- **Month 1, foundations.** Repo, CI (test gate, local-only lint), crate workspace skeleton, storage layer (SQLite schema, LanceDB index tables, batched flush, compaction), capture v1 (ScreenCaptureKit, dedup, admission policy ported), OCR wrapper ported, embedding contract and model registry (Qwen3-Embedding-0.6B), minimal shell with pause/blocklist/status, the design language spec and component library, the three de-risking spikes (Lance hybrid from Rust, Qwen3-VL through the bindings, capture-provider soak), the walking skeleton (a deliberately ugly capture-to-MCP slice by week 3), environment bootstrap and dev-install on all four machines, eval corpus v0 with its frozen held-out split, and the harness skeleton.
- **Month 2, retrieval.** Chunk-first write and read paths, hybrid search (Lance FTS plus vector, RRF), reranker experiment, memory synthesis (deterministic always, VLM optional), review worker, search UI and vault v1 on the component library, FNDR-Bench v1 with first real-model numbers plus the usefulness rubric and faithfulness slice, the real-model CI lane, the safety gate live on the write path, the sample vault and empty-vault state, `fndr doctor`, MCP server v1 (auth-always, 8 core tools including context packs), a first agent field test against the dev server, and the release pipeline (signed DMG, updater).
- **Month 3, agent context.** Context-pack quality hardening (budgets, citations), delta tool, retrieval explanation, remaining canonical tools, the project warm-start file export (the daily retention loop), backup/export/restore, backfill importers, capture-explain, onboarding with connect-your-agent, and the gate dry-run two weeks before the gate closes.

**Month-3 demo gate (P0.6).** On a clean machine: install from a URL, work normally for a day, connect Claude Code or Claude Desktop to FNDR via MCP, ask the agent to resume yesterday's work; the agent produces a correct, cited context pack; blocklisted and sensitive content from the day is verifiably absent, shown live (visit a bank and a password manager on camera, then prove absence in the vault, the pack, and privacy_status). The script is staged as a counterfactual cut: the same agent task with FNDR off (the agent interrogates the user), then on (one tool call). The whole flow is captured as the demo video draft, and a dry-run of the script executes two weeks before the gate closes. **If the gate fails, months 4 to 6 start by fixing the spine, not by adding features.**

### Months 4 to 6: the surround

- **Month 4, comprehension surfaces.** Graph extraction and storage plus the 3D graph rebuilt on the real schema (entering at view-only scope), timeline, the morning digest, meetings v1 (FluidAudio sidecar, diarized transcripts into memory, consent defaults), wallpaper and theming build-out.
- **Month 5, assistant surfaces and proof.** Omnibar, smart clipboard, focus/task panels, proactive resurfacing, Session Story, grounded Q&A, visual similarity, theming completeness, benchmark page and FNDR-Bench public release (corpus, harness, external submissions), companion API contract v2 (spec only), relay design note (build-nothing).
- **Month 6, ship quality.** Hardening and performance passes against the published targets, notarized release pipeline if account available, docs site, final demo video, FNDR-Bench public release, buffer for the gate's discoveries.

Cut lines, pre-agreed: proactive resurfacing, clipboard autofill, and visual similarity are the first cuts; meetings diarization can degrade to transcription-only; the 3D graph can ship view-only (no editing affordances). The spine is never cut.

### Team lanes
- **ML/infra:** inference crate, model registry, eval harness, FNDR-Bench, reranker/VLM experiments, performance budgets.
- **Backend:** storage, capture, retrieval, graph, MCP server.
- **Frontend:** shell UI, vault, omnibar, timeline, 3D graph, theming.
- **Platform (mobile lane, deferred mobile):** Swift sidecar (FluidAudio, SCK audio), macOS shell integrations (panel, tray, permissions), release engineering (signing, updater, CI), companion API contract authorship.

## 11. Mobile deferral note

The POC's iOS/Watch companion (pairing, ask, capture, memories) validated demand and produced a clean, versioned API contract; the audit rated the companion module the best-engineered part of the POC. v2 deliberately builds no mobile software in this window. The commitment instead: the engine keeps a UI-agnostic API layer, the month-5 companion contract is written against it, and the pairing trust model (short-lived code, pinned cert, revocable opaque tokens) carries forward with the pair-start endpoint moved off the network surface (audit finding). Mobile work, when it starts, consumes the contract without engine changes.

## 12. Risks

| Risk | Mitigation |
|---|---|
| Retrieval quality on real models disappoints (the riskiest assumption) | Eval corpus in month 1; bench gates from month 2; reranker and chunk ablations run early; the month-3 gate forces the truth before the surround is built |
| Tauri macOS friction (TCC permission resets, panel quirks) | Known issues catalogued in ADR-001 with mitigations; engine is shell-agnostic, so a native Swift shell remains an escape hatch without engine rewrite |
| 8 GB machines vs model ambitions | Tiered model policy (2B VLM, matryoshka dims, on-demand load/unload); resource budgets measured continuously from month 2 via `make bench` on the reference machine |
| LanceDB version sprawl on desktop write patterns | SQLite is truth; Lance is rebuildable; batched flush plus scheduled compact/prune (ADR-002) |
| Four-person coordination on a monorepo | Crate boundaries per lane, contract-first interfaces, the repo engineering skill encodes the workflow |
| Scope gravity from ten feature areas | Month-3 gate, pre-agreed cut lines, "ship whole or not at all" rule |

## 13. Open questions

1. ~~**Apple Developer account**~~ Dropped 2026-08-20 (owner ruling: proceed without blocking). Default: ad-hoc signing with the right-click Open path scripted into the gate and demo (the v1 ADR-013 approach); Developer ID signing and notarization are added to the same pipeline whenever an account lands, without redesign. Gatekeeper friction on install is an accepted, documented cost until then.
2. ~~**Repo host**~~ Dropped 2026-08-20. Default: the POC's status quo, GitHub for code, CI, and releases (the updater manifest convention already assumes it), GitLab board for work tracking. Revisit only if the split causes real friction.
3. ~~**DRIs and cadence**~~ Dropped 2026-08-20. Default: decided at team kickoff alongside sprint planning; until then the lane owner is the DRI for their crates and the PRD owner is the product tie-breaker.
4. **Eval corpus seeding** (team): how much real (donated, sanitized) session data can the four builders contribute vs synthetic fixtures? Affects FNDR-Bench credibility. Resolve in month 1. Non-blocking to start.
5. **GitLab board conventions** (owner): label and milestone scheme is proposed in `ROADMAP-TICKETS.md`; confirm before import. Non-blocking.
