# Real capture scheduler handoff

## Shipped on `codex/a006-real-store-safety-seam`

- Concrete `PrivacyGate`, `VisionOcrAdapter`, and `StoreCaptureSink` live in
  `fndr-shell/src/capture_adapters.rs`.
- `CaptureScheduler` owns the already-stage-tested capture pipeline, a
  `LanceWriter`, and a `QueuedEmbedder`. It flushes only at the configured
  cadence and retries visibly when indexing fails; SQLite remains truth.
- `RealCaptureScheduler::open` assembles the real macOS foreground source,
  ScreenCaptureKit source, Vision OCR, privacy policy, WAL-backed store,
  one long-lived `ModelWorkerHandle`, and the real GGUF loader. A missing
  model is a typed startup failure, never a fallback.
- `flush_on_shutdown` drains every pending SQLite batch through the model
  queue before returning. A failed flush returns its typed error and leaves
  the durable rows pending for retry after restart.

## Decisions

- The scheduler remains a synchronous, dedicated-worker component. It does
  not run capture syscalls on Tauri's async runtime; Tauri's runtime is used
  only to await Lance's async writer after the capture is already in SQLite.
- The normal flush minimum is 30 seconds, preventing one Lance commit per
  frame. Shutdown deliberately bypasses that cadence to preserve durable
  batches.
- The real scheduler requires explicit database, index, and model paths.
  It does not silently choose an application-data directory before the shell
  has an owner-visible onboarding and permission lifecycle.

## Verification

- `cargo test -p fndr-shell`: 5 tests passed, including SQLite -> queued
  embedder -> Lance and shutdown draining without a real model.
- `CARGO_BUILD_JOBS=1 make test`: passed (workspace lint, generated AGENTS
  check, Clippy, Rust tests, TypeScript typecheck, and Vitest). The serial
  limit intentionally kept an 8 GB development machine responsive while a
  trimmed Cargo cache rebuilt.

## Explicitly not done

- There is not yet a Tauri startup/lifecycle owner that starts the dedicated
  capture worker, emits scheduler status, and invokes `tick` continuously.
- No hardware permission run was requested for this composition slice; do
  not claim this new scheduler has captured a live screen.
- T-307 session continuity, T-308 adaptive sampling, T-804 pause controls,
  T-805 no-pixel persistence coverage, deletion-everywhere, and retrieval
  remain separate work.

## Landmines

- Keep all capture and foreground-metadata syscalls off Tauri's async runtime.
- Do not replace the queue with a direct GGUF call: the worker is what limits
  RAM to one loaded model and releases it after idle timeout.
- `target/` grew from roughly 1 GB to roughly 7 GB after rebuilding removed
  Cargo artifacts. It is rebuildable; preserve it while iterating, then clean
  it only when disk pressure makes the cache less valuable than free space.
