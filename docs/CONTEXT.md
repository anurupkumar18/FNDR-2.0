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
3. Apple Vision OCR produces text that is stored in the local WAL-backed
   `SkeletonStore`; `--store` proves persistence across process restarts.
4. Authenticated loopback MCP serves `fndr.search` and
   `fndr.privacy_status`. The latter exposes posture and counts, not the
   sensitive blocklist entries.
5. The end-to-end fixture test proves raw PNG bytes are absent from the
   SQLite, WAL, and SHM artifacts after OCR text is stored.

Run [`docs/demo/ALPHA-RUNBOOK.md`](demo/ALPHA-RUNBOOK.md) for exact commands
and boundaries. The final alpha verification command is `make test`.

## Important code locations

| Area | Current owner and entry point | State |
| --- | --- | --- |
| Capture seam | `crates/fndr-capture/src/` and `fndr-shell/src/capture_scheduler.rs` | Working: the real one-shot ScreenCaptureKit provider (T-302, verified against a live screen through OCR); T-303's compact native-pixel perceptual signature, A-B-A deduper, and semantic window; T-304's pure browser admission policy; and T-306's stage pipeline plus concrete privacy/Vision/SQLite adapters. `RealCaptureScheduler` owns the real pipeline, one queued model worker, a `LanceWriter`, cadence-limited flush, and a draining shutdown flush. Its composition tests cover SQLite-to-Lance through the queue. Still missing: the Tauri lifecycle/capture worker that invokes it continuously, and a hardware permission run. T-310's soak is also open. |
| Privacy | `crates/fndr-privacy/src/safety_gate.rs` | Working deterministic policy and redaction seam; real pipeline/store integration continues in T-803. |
| OCR | `crates/fndr-ocr/src/vision.rs` | Working Apple Vision wrapper. |
| Alpha store | `crates/fndr-store/src/skeleton.rs` | Working local FTS proof; deliberately replaced by the real schema/read path, not extended into a second retrieval stack. |
| Real-store write seam | `crates/fndr-memory/src/write_path.rs` | Working: `persist_capture` rechecks `fndr-privacy::evaluate` immediately before writing an already-assembled capture through `Store::insert_capture`, redacting secret-bearing OCR text first and returning a typed `Stored`/`Skipped` outcome. It retains bundle ID plus a structurally sanitized HTTP(S) URL (no credentials, query, or fragment), never pixels. This is a persistence boundary only; it does not replace the capture scheduler's pre-OCR gate or add a retrieval route. |
| Keyword retrieval | `crates/fndr-retrieval/src/lib.rs` | Working low-RAM first route: `KeywordRetriever` searches a SQLite FTS5 index transactionally maintained beside durable `chunks`, returning stable record/chunk IDs and snippets. Porter stemming covers index/indexes. Vector, Lance-FTS, temporal, hybrid/RRF, ranking, context packs, and MCP/UI wiring remain open. |
| MCP | `crates/fndr-mcp/src/server.rs` | Working authenticated streamable-HTTP skeleton with search and privacy status. |
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
2. Replace the skeleton write/read path with the real capture-to-store engine
   seam while preserving the policy and MCP tests.
3. Add only the reviewable Connected Planner draft/approval contract required
   for beta. Do not add an outbound provider client or executor.
4. Promote a feature into the final demo only with the ADR-009 evidence for
   its retrieval, citation, safety, and usefulness claims.
