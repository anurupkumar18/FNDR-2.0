## Handoff: codex/a006-real-store-safety-seam (2026-09-05)

Done: Alpha's runnable local spine is on this branch: fixture or screen capture is privacy-gated before OCR, OCR text is redacted before persistence, the file-backed `SkeletonStore` survives reopen, and authenticated MCP exposes `fndr.search` plus non-sensitive `fndr.privacy_status`. `make test` passed after the completed slice; targeted MCP privacy and raw-pixel persistence tests also passed. The manual demo commands and expected negative cases are in `docs/demo/ALPHA-RUNBOOK.md`.

In flight: Do not extend `SkeletonStore`. The next bounded slice is T-201/T-202 integration: add the real `fndr-memory` write seam over `fndr-store::Store`, applying the existing privacy policy before the SQLite write and testing stored, redacted, and skipped outcomes. Then replace the skeleton route only when the real write and one retrieval route are both complete; no second retrieval stack.

Decisions: `FNDR-2.0` is the canonical mainline; v1 is an alpha donor only. The current skeleton remains only as a working proof while the real store contract replaces it. Planner work remains documentation/contracts only: local default, explicit approval, and no app-owned external client or autonomous execution.

Landmines: The legacy checkout `/Users/anurupkumar/fndr` is dirty in `src-tauri/src/capture/mod.rs`; leave it untouched. `ui/node_modules` was restored with lockfile-pinned `npm ci` so the full gate can run. Do not claim the next real-store seam prevents OCR: the current runner owns the genuine pre-OCR gate; the store seam is defense in depth at persistence. Check `docs/CONTEXT.md`, ADR-004/005/006/007/008/009, and `docs/review/BASELINE-2026-09-05.md` before changing the spine.

Produced by: Anurup + Codex
