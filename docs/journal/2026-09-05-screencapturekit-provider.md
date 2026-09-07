## Handoff: T-302 real ScreenCaptureKit provider (2026-09-05)

Done: `ScreenCaptureKitSource` (`crates/fndr-capture/src/source.rs`) — the
real SCK capture provider, replacing the `screencapture(1)` shellout the
walking skeleton used. **Verified against a real screen on real hardware,
not just compiled.**

### The proof

`cargo run -p fndr-capture --example sck_probe` (a manual probe kept in
the repo alongside `fetch_model`, same spirit):

```
captured 666171 bytes, png_magic=true, captured_at_ms=1788666398822
SCK provider works: real frame, real PNG bytes, nothing left on disk.
```

And the whole perception front-end, via the existing skeleton demo with
its live path now pointed at the new provider:

```
captured 666269 bytes of PNG
ocr: 199 chars, 5 blocks, confidence 0.50
stored 1 record; total records: 1
```

That is a real screen → real ScreenCaptureKit → real Apple Vision OCR →
privacy gate → SQLite. First time this repo has read text off an actual
screen.

### Decisions

- **One-shot `SCScreenshotManager`, not a persistent `SCStream`.** ADR-001
  action item 4 prefers this for FNDR's ~0.5 FPS model, and it sidesteps
  the upstream crate's leak/stalled-callback issue history, which is
  concentrated in the long-lived stream path. Each `grab()` is
  self-contained (enumerate content → filter to a display → capture →
  encode), so there is no background stream to supervise, leak, or stall.
  It costs some per-frame setup; at half a frame per second that is the
  right trade.
- **Kept the ADR-named `screencapturekit` crate** (pinned `=9.0.1`) rather
  than the named fallback. I checked both against their real published
  source before choosing, per the repo's own "verify against pinned
  source, not memory" lesson — and that check mattered:
  `objc2-screen-capture-kit` **0.2.2** (the version matching `fndr-ocr`'s
  pinned objc2 0.5 line) binds only `init`/`new` on `SCScreenshotManager`
  — **the actual capture methods are not bound at all**, so it could not
  do the job. Its 0.3.2 does bind them, but requires objc2 ≥0.6.2, which
  would put a second objc2 generation in the tree beside `fndr-ocr`'s
  pinned 0.5. The `screencapturekit` crate depends on apple-cf/apple-metal
  and **no objc2 at all**, so it collides with nothing, and it exposes a
  synchronous `capture_image(&filter, &config)` that maps cleanly onto the
  existing one-shot `FrameSource::grab()` seam.
- **Enabled only the `macos_14_0` feature.** That gate is what exposes the
  `screenshot_manager` module; macOS 14 is where `SCScreenshotManager`
  landed and is the floor this provider needs. Deliberately did not opt
  into `macos_15_2`/`macos_26_0`, which would raise the floor for the
  newer `capture_image_in_rect`/`capture_screenshot` APIs we do not use.
- **`TempPng` guard.** The crate encodes to a file, not to memory, so the
  frame round-trips through a temp path exactly as the `screencapture(1)`
  source did. Unlike that one, cleanup is a `Drop` impl, so the raw frame
  is removed on **every** exit path including errors — ADR-004's "no raw
  screenshot persistence" deserves better than a happy-path `remove_file`.
  Verified no `/tmp/fndr-sck-*.png` survives a run.

### The landmine that cost the most time: the Swift runtime

The pinned crate builds a **Swift shim**, so anything linking
`fndr-capture` needs the Swift runtime on its rpath. Without it the binary
compiles fine and then dies at startup:

```
dyld: Library not loaded: @rpath/libswift_Concurrency.dylib
```

Fixed with a new **`.cargo/config.toml`** adding
`-Wl,-rpath,/usr/lib/swift` for the two Apple desktop targets (matching
`deny.toml`'s `[graph] targets`). Notes for whoever touches this next:

- It has to be workspace-wide config, **not** a `build.rs` in
  `fndr-capture`: Cargo does not propagate a build script's link args to
  downstream binaries, so `fndr-shell`'s eventual app binary would still
  fail at runtime.
- `/usr/lib/swift` looks like an empty directory — the dylibs live in the
  dyld shared cache. That is expected; the rpath still resolves.
- **Adding this invalidated every cached build in the workspace.** The
  first `cargo` run after it took 6m16s for a cold rebuild of the
  lance/datafusion/llama stack. Expect CI's first run after this lands to
  do the same, once.
- Platform lane / T-310: re-check that the bundled `.app` still resolves
  this after signing and notarization. An rpath into `/usr/lib/swift` is
  standard for Swift-interop binaries, but it has not been tested through
  the packaging path yet, because there is no packaging path yet.

Ran and confirmed:
- `cargo fmt --all --check` — clean
- `cargo clippy -p fndr-capture --all-targets -- -D warnings` — clean
  (one real catch: a hand-written `Default` impl that should be derived)
- `cargo test -p fndr-capture` — 3/3 pass
- `cargo run -p fndr-capture --example sck_probe` — real capture, above
- The live skeleton demo through OCR and storage — real capture, above
- `./scripts/check-llm-call-sites.sh` — still clean
- `git diff --check` — clean
- `make test` (full sweep) — all green, on the full cold rebuild the
  `.cargo/config.toml` change forced (two complete workspace passes:
  clippy profile, then test profile)

### In flight / explicitly not done

- **No soak test (T-310).** ADR-001 asks for a multi-day soak with an RSS
  trend assertion before trusting this crate, precisely because its issue
  history is leaks. One-shot `SCScreenshotManager` is the shape least
  likely to leak, but "least likely" is not "measured". T-310 remains
  open and this provider is unproven over time.
- **No dedup, admission, or scheduler.** This is the provider only.
  T-303 (perceptual/semantic dedup), T-304 (admission policy port), and
  T-306 (the staged pipeline that would actually call this on a timer)
  are all still untouched. Nothing captures continuously yet.
- **Multi-display policy is a stub.** `display_index` defaults to 0
  (primary). Which display(s) to capture, and how to handle
  connect/disconnect, belongs to T-306's scheduler, not this seam.
- **No self-exclusion.** The content filter excludes no windows. Once
  there is an FNDR window to exclude (T-901 shell), the filter should
  exclude it — the privacy gate's `FndrSelfCapture` reason exists for
  the metadata case, but filtering at the SCK level is cleaner.
- **`ScreencaptureCliSource` is still present.** It is no longer used by
  the demo, but I left it rather than deleting it in the same change —
  it is the documented fallback if the SCK provider misbehaves before
  T-310 validates it. It should be deleted once T-310 signs off.

Produced by: Anurup + Claude Code
