# FNDR current context

Use this file to orient a new contributor or coding agent. It distinguishes
verified current behavior from approved future direction so a partial alpha
does not become an accidental product claim.

## Product boundary

- **Canonical mainline:** FNDR-2.0. The legacy FNDR repository is a targeted
  donor only; see ADR-005 and `docs/review/BASELINE-2026-09-05.md`.
- **Local default:** capture, storage, OCR, retrieval, and FNDR's own runtime
  have no direct captured-data egress. The future Connected Planner is a
  default-off, user-reviewed external-client export mode defined by ADR-004
  and ADR-008. It is not implemented yet.
- **No autonomous execution:** alpha does not execute planner actions. The
  only future proposal capability identifiers are `memory.open_target` and
  `git.status.short`, each behind a separate approval contract.

## Verified alpha behavior

The walking skeleton is intentionally small but real:

1. A live screen or checked-in PNG fixture is acquired through
   `fndr-capture::FrameSource`.
2. `fndr-privacy` evaluates built-in sensitive context and an owner-provided
   app/domain blocklist before OCR. Password-manager, financial, auth, and
   private-browsing cases visibly skip; secret-bearing OCR text redacts before
   the store call.
3. The checked-in fixture path persists OCR through the local WAL-backed
   skeleton proof, while the real scheduler writes durable `Store` records
   through the privacy-aware write seam; both stay local.
4. Authenticated loopback MCP serves twelve audited durable-store tools,
   including keyword search, cited context packs, source evidence, timeline,
   focus, recall, and privacy posture. It does not expose the sensitive
   blocklist entries.
5. The end-to-end fixture test proves raw PNG bytes are absent from the
   SQLite, WAL, and SHM artifacts after OCR text is stored.

Run [`docs/demo/ALPHA-RUNBOOK.md`](demo/ALPHA-RUNBOOK.md) for exact commands
and boundaries. The final alpha verification command is `make test`.

## Important code locations

| Area | Current owner and entry point | State |
| --- | --- | --- |
| Capture seam | `crates/fndr-capture/src/` and `fndr-shell/src/capture_worker.rs` | Working: the real one-shot ScreenCaptureKit provider, compact native-pixel perceptual signature, A-B-A/semantic dedup, browser admission, and the stage pipeline with concrete privacy/Vision/SQLite adapters. `RealCaptureScheduler` owns one queued model worker, a `LanceWriter`, cadence-limited flush, and a draining shutdown flush. The Tauri `CaptureLifecycle` owns the worker on an explicit **Start capture** action and forwards bounded no-content status events. Missing: a human-authorized hardware permission run and T-310 soak. |
| Privacy | `crates/fndr-privacy/src/safety_gate.rs` | Working deterministic policy and redaction seam; real pipeline/store integration continues in T-803. |
| OCR | `crates/fndr-ocr/src/vision.rs` | Working Apple Vision wrapper. |
| Alpha store | `crates/fndr-store/src/skeleton.rs` | Working local FTS proof; deliberately replaced by the real schema/read path, not extended into a second retrieval stack. |
| Real-store write seam | `crates/fndr-memory/src/write_path.rs` | Working: `persist_capture` rechecks `fndr-privacy::evaluate` immediately before writing an already-assembled capture through `Store::insert_capture`, redacting secret-bearing OCR text first and returning a typed `Stored`/`Skipped` outcome. It retains bundle ID plus a structurally sanitized HTTP(S) URL (no credentials, query, or fragment), never pixels. This is a persistence boundary only; it does not replace the capture scheduler's pre-OCR gate or add a retrieval route. |
| Session continuity | `crates/fndr-memory/src/continuity.rs` | Working: local-day/30-minute session IDs, context keys, safe URL/title anchors, candidate scoring, strict cross-app merge guard, and bounded deterministic story merge. The real write seam queries only its own unflushed SQLite candidates and atomically merges a safe burst before Lance observes it, preserving one record/chunk and FTS row. It is model-free: callers supply similarity. Missing: a Lance-safe indexed-record merge/update path and lifecycle-owned session-ID derivation; the scheduler still receives its session ID explicitly. |
| Retrieval | `crates/fndr-retrieval/src/lib.rs` | Working low-RAM first route: `KeywordRetriever` searches a SQLite FTS5 index transactionally maintained beside durable `chunks`, returning stable IDs and snippets. `VectorRetriever` can query the same flushed Lance derivative through an explicit local-model benchmark route, returning IDs/metadata/distance only. Vector is not MCP/UI/fusion behavior; Lance-FTS, temporal, hybrid/RRF, ranking, and shared UI/MCP routing remain open. |
| MCP | `crates/fndr-mcp/src/server.rs` | Working authenticated local MCP server with twelve durable-store tools, structural audit logging, and a durable-store launcher. The desktop owner audit viewer is read-only and content-free. Retrieval remains keyword FTS on this surface. |
| Planner | ADR-008 and ADR-009 | Contract and evaluation only; no runtime implementation or provider integration. |

## Work rules that matter next

1. Read the generated `AGENTS.md`, its routed workflow, `docs/ARCHITECTURE.md`,
   the touching ADR, and `references/lessons.md` before editing.
2. Keep one vertical slice per change. Extend the engine API shared by UI and
   MCP instead of introducing a surface-specific retrieval or privacy path.
3. A port from legacy needs ADR-005 eligibility, a narrow provenance note,
   tests, and an explicit defect not carried forward.
4. Ranking changes require `make bench`; privacy or MCP changes require the
   relevant adversarial suite and named auth tests. Run `make test` before a
   behavior-changing PR.
5. Never place real captures, databases, generated bearer tokens, model files,
   or credentials in version control.

## Near-term order of operations

1. Rehearse the alpha runbook against a clean temporary database.
2. Run the human-authorized Screen Recording and clean-drain rehearsal, then
   the bounded T-310 soak; do not replace either with an unattended run.
3. Promote another retrieval route only with an ADR-006 evaluation and a
   clear shared-engine integration plan; do not expose the benchmark-only
   vector route decoratively.
4. Add only the reviewable Connected Planner draft/approval contract required
   for beta. Do not add an outbound provider client or executor.
