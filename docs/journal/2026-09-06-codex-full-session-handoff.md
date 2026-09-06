# Codex full-session handoff — 2026-09-06

## Starting point and final state

- **Branch:** `codex/a006-real-store-safety-seam`
- **Started after:** `1243a30`
- **Final pushed commit:** `25f09b6`
- **Canonical checkout:** `/Users/anurupkumar/FNDR-2.0`
- **User-owned file left untouched:**
  `docs/journal/2026-09-05-claude-code-handoff-prompt.md` (untracked)

Every behavior slice below was committed and pushed. Its serial
`CARGO_BUILD_JOBS=1 make test` gate was green before commit. The last observed
resource state was about 50 GiB free and a 24 GiB `target/`; do not launch
parallel full Cargo gates on this 8 GB laptop.

## Delivered after the prior handoff

1. **Capture scheduler composition** — `e6efb24`
   `RealCaptureScheduler` composes real ScreenCaptureKit foreground/capture,
   privacy, Vision OCR, SQLite persistence, one queued model worker, Lance
   flushing, and shutdown drain. It is still not started by a desktop binary.

2. **Deletion everywhere** — `f15cd19`
   Owner record/time/domain/all deletion removes derived Lance rows first,
   then SQLite truth. Domain matching is label-safe (`bank.com` does not match
   `burbank.com`).

3. **Durable keyword route** — `0b954cd`
   SQLite FTS5 migration 0004 trigger-maintains chunk search with Porter
   stemming. `KeywordRetriever` returns durable record/chunk IDs and snippets;
   deleted data no longer appears in that route.

4. **Continuous-worker lifecycle seam** — `fabdc30`
   A named capture thread owns the real scheduler, emits bounded no-content
   events, and explicitly joins through shutdown draining. There is no Tauri
   app bootstrap to invoke it yet.

5. **Session continuity** — `bf84573`, `b817718`
   Ported session identity, safe anchors, scoring, strict cross-app policy,
   and deterministic story merge. The real write seam merges only recent,
   unflushed SQLite captures atomically, preserving one FTS/Lance-bound chunk.
   Indexed rows are intentionally never edited because Lance replacement is
   not yet safe.

6. **Adaptive sampling foundations** — `2065abd`, `25f09b6`
   `SamplingPolicy` is pure and tested (2 s active, 15 s idle after one
   minute, deep idle at five minutes, forced capture at two minutes).
   `MacOSInputIdle` is a one-shot CoreGraphics boundary. Neither is wired into
   the worker yet, so current worker cadence remains fixed and safe.

## Earlier work retained and verified

The pre-existing session roll-up plus journal slices document the real-store
privacy seam, T-802 owner policy, ledger audit, real GGUF embedder/Metal
probe, end-to-end composition proof, model worker and queued embedder,
capture-burst memoization, and hardware-verified ScreenCaptureKit provider.
Do not recreate those seams or revive the removed legacy checkout.

## Honest current boundaries

- Real capture-to-OCR-to-privacy-to-SQLite-to-Lance composition exists, but a
  desktop app entry point does not yet own `start_real_capture_worker`.
- Keyword search is real; vector/Lance FTS, temporal, hybrid/RRF, ranking,
  packs, MCP/UI engine wiring are still open.
- In-flight continuity is safe; indexed-record merging is intentionally open.
- Adaptive policy and idle probe are present but the worker has not adopted
  them.
- T-310 long-running ScreenCaptureKit soak and fresh permission run remain
  unverified.

## Recommended next slice

Wire `SamplingPolicy` and `MacOSInputIdle` into `capture_worker.rs` while
keeping the shutdown channel responsive: use policy-selected `recv_timeout`
waits, treat deep idle as a wait rather than a busy loop, retain the 2-second
active floor, and test the fake scheduler timeline. Do not load a model to
make this decision. Then update the T-308 ledger/journal and run the serial
full gate.

## Landmines

- Preserve the untracked handoff prompt.
- Use `CARGO_BUILD_JOBS=1`; inspect `df -h .` and `du -sh target` before a
  new full gate. Do not delete source, Git data, or models as cleanup.
- Never merge indexed records without a Lance-safe replacement/deletion path.
- Do not claim auto-capture until a real desktop lifecycle calls the worker.
- Browser-hosted review was unavailable in this task; local tests and pushes
  are the verification evidence for these commits.
