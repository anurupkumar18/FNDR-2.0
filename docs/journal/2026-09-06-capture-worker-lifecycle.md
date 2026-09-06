# 2026-09-06: capture-worker lifecycle

## Decision

The shell owns continuous capture in a dedicated named OS thread. The worker
constructs `RealCaptureScheduler` on that thread, ticks immediately and then
waits on a shutdown channel; it does not run synchronous ScreenCaptureKit or
Vision work on Tauri's async runtime.

## What is verified

- The worker rejects a sub-two-second fixed cadence before it initializes a
  scheduler, avoiding a busy capture loop on the reference 8 GB laptop.
- Each attempted tick emits a bounded, best-effort status event containing
  only time and terminal outcomes. A slow or absent consumer cannot block
  capture and no raw image, OCR text, URL, or model content reaches the event.
- `CaptureWorkerHandle::shutdown` joins the capture thread and calls the
  scheduler's existing SQLite-to-Lance shutdown drain. The unit test proves
  both an off-thread tick and exactly one drain.
- Model loading remains lazy in the existing `ModelWorkerHandle`; starting an
  idle worker does not load the GGUF into RAM.

## Explicitly not done

The workspace has no desktop binary or Tauri `setup`/exit lifecycle to own
this handle, so this slice exposes the real start/stop API rather than making
a false claim that capture starts automatically. It also does not add adaptive
input-idle cadence, UI health rendering, a hardware permission run, or a
ScreenCaptureKit soak.

## Landmines

- The lifecycle owner must explicitly call `shutdown` at app exit and must not
  wait for it on Tauri's async runtime.
- Do not turn the event channel into a content stream; status stays bounded
  and no-content until the health contract defines retained diagnostics.
- Keep model work behind the queue and preserve the 30-second Lance flush
  floor; capture cadence is not embedding cadence.

## Verification

`CARGO_BUILD_JOBS=1 cargo test -p fndr-shell`, followed by the serial
workspace `make test` gate.
