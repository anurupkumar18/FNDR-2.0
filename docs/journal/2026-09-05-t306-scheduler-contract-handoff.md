# T-306 staged scheduler contract handoff

## Shipped on `codex/a006-real-store-safety-seam`

- `02cf133 feat(capture): add staged pipeline contract`
- `5143a26 fix(capture): preserve engine-owned OCR signal rules`
- `b656c3e feat(capture): fail closed on missing foreground metadata`

`make test` passed after each behavioral change. The three commits are pushed
to `origin/codex/a006-real-store-safety-seam` and their hosted diffs were
reviewed on GitHub.

## Decisions

- `CapturePipeline` is a synchronous, generic stage contract owned by
  `fndr-capture`. The shell must run it on a dedicated capture worker, never
  on Tauri's async runtime.
- Each tick terminates exactly once as stored, URL-only stored, a counted skip,
  or a typed failed stage. It never substitutes an empty frame or a PNG-derived
  perceptual hash when a native signature is unavailable.
- OCR's existing `RecognizedText::is_low_signal` rule remains authoritative.
  The eventual Vision adapter supplies that result instead of reimplementing a
  weaker signal rule in the scheduler.
- `MacOSForegroundContextSource` reads the frontmost app from AppKit. For a
  supported browser it requires both a current title and an HTTP(S) URL from
  the browser's AppleScript dictionary; unavailable, unsupported, or denied
  metadata fails the tick before pixels are captured. Generic apps likewise
  require a readable front-window title.

## Explicitly not done

- T-306 is **not complete**. There is no production loop, lifecycle command,
  scheduler-owned model worker, periodic Lance flush, or durable shutdown
  flush yet.
- The foreground provider was compiled and unit-tested but deliberately not
  invoked on hardware in this slice. Invocation can cause macOS
  Automation/Accessibility consent prompts.
- `CaptureSink`, `PreCaptureGate`, and `OcrRecognizer` have no concrete shell
  adapters. URL-only persistence also remains blocked on adding durable URL
  and bundle metadata to the real store schema.
- T-307 session identity is not implemented. Do not quietly invent a
  long-lived session contract while wiring the sink; any temporary IDs must be
  isolated and documented as temporary.

## Next vertical slice

1. Add a shell-owned privacy gate backed by `fndr_privacy::evaluate` and a
   Vision adapter backed by `OcrEngine::recognize_with_metadata`.
2. Extend the real-store record contract and migration for bundle/URL metadata
   so `UrlOnly` does not become a lossy no-op.
3. Build the concrete `CaptureSink` around `persist_capture`, then introduce a
   dedicated scheduler worker which owns one `ModelWorkerHandle`, queued
   embedder, `LanceWriter`, and explicit shutdown flush.
4. Add a deterministic composition test with fakes before requesting the
   macOS permissions needed for a real, bounded hardware run.

## Landmines

- Do not port v1's `capture/mod.rs` loop wholesale; ADR-005 allows targeted
  policy provenance but requires this loop to be a V2 rewrite.
- A browser with no URL must not fall through to normal pixel capture. This is
  a privacy boundary, not an availability optimization.
- The foreground provider's AppleScript calls must stay off the Tauri async
  runtime. Expect first-run macOS Automation/Accessibility consent and ensure
  the eventual app bundle declares any required usage descriptions.
- `docs/journal/2026-09-05-claude-code-handoff-prompt.md` is a user-owned,
  untracked file. It remains untouched and unstaged.
