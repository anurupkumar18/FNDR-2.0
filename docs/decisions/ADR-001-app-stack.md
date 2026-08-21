# ADR-001: Application stack: Tauri 2 shell, Rust engine, React UI, Swift ML sidecar

**Status:** Proposed
**Date:** 2026-08-19
**Deciders:** FNDR v2 team (4)

## Context

FNDR v2 is a macOS-only, local-first desktop app that must combine: always-on screen capture (ScreenCaptureKit), on-device OCR (Apple Vision), on-device embedding/VLM/LLM inference, an embedded vector plus lexical store, an MCP server for external agents, meeting audio with diarization, and a polished UI (Spotlight-style omnibar, 3D graph, heavy theming). Constraints: 4-person team (ML/infra, backend, frontend, platform), 6-month window, Apache 2.0 (all dependencies must be license-compatible), and a future thin companion/relay that must not force a rewrite.

The evaluation was run with zero anchoring on the POC's choices. Two sibling decisions constrain this one: the storage choice (ADR-002) selects LanceDB, whose native implementation is a Rust crate with no Swift binding, and the inference choice (ADR-003) selects llama.cpp, whose mature binding is the Rust crate `llama-cpp-2`. The MCP Rust SDK is Tier 2 on the current stable spec (2026-07-28) while the Swift SDK is Tier 3 on the older 2025-11-25 spec.

## Decision

- **Engine:** a headless Rust workspace of crates (capture, memory, store, retrieval, graph, inference, MCP, companion API) with no UI or Tauri dependency. This is the product; the shell is replaceable.
- **Shell:** Tauri 2 hosting the engine in-process, providing IPC, bundling, updater, tray, global shortcuts, and windows.
- **UI:** React + TypeScript (Vite), consuming generated bindings (specta/tauri-specta) rather than hand-mirrored types.
- **Native sidecar:** a small Swift helper binary owned by the platform lane for Apple-only ML that has no credible Rust path, initially the FluidAudio meeting stack (Parakeet ASR plus pyannote diarization on the Neural Engine). Vision OCR and ScreenCaptureKit are consumed from Rust via maintained objc2 bindings, as the POC proved.

## Options considered

### Option A: Tauri 2 + Rust engine + React UI + Swift sidecar (chosen)

| Dimension | Assessment |
|---|---|
| Complexity | Medium: known stack, one new boundary (Swift sidecar) |
| Team familiarity | Highest: the POC is 84k lines of Rust and 27k of React by this team's lead |
| ML access | Full: llama-cpp-2, ort, objc2-vision, screencapturekit crate; ANE audio via sidecar |
| UI polish ceiling | High for panels/theming (CSS); omnibar via NSPanel plugin (POC shipped one); WebGL 3D graph first-class |
| Distribution | Proven in POC: signed DMG, tauri-plugin-updater, one-tag releases |
| Ecosystem risk | Tauri 2 stable since 2024, v2.11.x current; screenpipe (closest shipped comparable) uses the same pattern |

**Pros:** keeps every Rust-first dependency native (LanceDB, llama-cpp-2, MCP rust-sdk); team productive from week 1; free IPC/updater/bundling; the engine/UI boundary is enforced by the workspace, not aspiration.
**Cons (named, with mitigations):** macOS screen-recording TCC permission can reset across updates (upstream issue closed as not-planned; mitigation: consistent signing identity, a first-tick permission health check that re-prompts with guidance, and QA on every update path); non-activating omnibar panels need the tauri-nspanel route with known quirks (mitigation: the POC's working omnibar window is the reference, and the platform lane owns a native fallback); WKWebView is 60 fps-capped before macOS 26 (acceptable; cap removed on Tahoe).

### Option B: Native Swift (AppKit/SwiftUI shell + Swift engine, WKWebView island for the 3D graph)

| Dimension | Assessment |
|---|---|
| Complexity | High for this team: full rewrite of skills as well as code |
| Team familiarity | Low: one Swift-strong lane out of four |
| ML access | Best-in-class (MLX, Foundation Models, FluidAudio native) |
| Storage | Blocking: no LanceDB; falls back to sqlite-vec, whose stable releases are brute-force scan (ADR-002) |
| MCP | Swift SDK is Tier 3 on an older spec |

**Pros:** strongest always-on power story (ANE), gold-standard omnibar and TCC behavior, Sparkle updater maturity.
**Cons:** forfeits the chosen store and inference bindings; three of four teammates start over; 6-month window absorbs a language migration instead of product. Rejected for this team and window, not on merit in the abstract.

### Option C: Swift shell + Rust engine over UniFFI + web UI (the Raycast 2.0 pattern)

**Pros:** native shell polish plus the Rust engine; proven by Raycast in 2026.
**Cons:** hand-rolled JS bridge, Sparkle integration, and bundling replace what Tauri gives for free; two FFI boundaries (Swift-Rust, Swift-JS) for a 4-person team. **Held as the designated escape hatch:** because the engine is shell-agnostic by construction, migrating to this shape later is a shell swap, not a rewrite. Revisit if Tauri TCC/panel pain exceeds one sprint of cumulative cost.

### Option D: Electron

**Cons:** 80 to 150 MB installers plus a Chromium runtime whose baseline overhead is spent on rendering rather than the product's own models; ML still requires native modules. For an always-on agent whose RAM budget should buy inference, that overhead is indefensible. Rejected.

### Option E: Pure-Rust GUI (GPUI, Dioxus, Slint)

Rejected: GPUI has an open GPL-3.0 transitive-dependency contamination issue (incompatible with Apache 2.0 distribution); none reach the required polish ceiling.

## Trade-off analysis

The decisive chain: LanceDB and llama-cpp-2 and the Tier-2 MCP SDK make the engine Rust; the team's skills and the POC's proven release pipeline make the shell Tauri; Apple-only ML that Rust cannot reach (ANE audio) is isolated in one Swift sidecar owned by the lane that will later build iOS. Option B optimizes the shell at the cost of the engine and the team; Option A optimizes the engine and the team at the cost of accepting documented, mitigable shell friction, while keeping Option C open.

## Consequences

- Easier: reuse of ported POC heuristics (same language), CI on Linux for engine crates (everything except capture/OCR mocks cleanly), future relay reuses engine crates.
- Harder: the team owns the Tauri TCC/panel mitigations; the Swift sidecar adds a process boundary (JSON over stdio, supervised).
- Revisit: shell choice (Option C) if Tauri friction exceeds budget; MLX via sidecar if llama.cpp falls behind on Apple Silicon.

## Action items

1. [ ] Scaffold the Cargo workspace with an `engine-must-not-depend-on-tauri` CI check.
2. [ ] Wire specta/tauri-specta type generation before the first IPC command lands, with exact `=rc` version pins for specta, specta-typescript, and tauri-specta (breaking changes land between RCs), and the i64/id export convention defined in `fndr-types` first. Binding-generator upgrades are scheduled maintenance PRs, never drive-by bumps.
3. [ ] Platform lane: spike the omnibar NSPanel on Tauri 2.11 and document the TCC re-grant flow (time-boxed, one week, in month 1).
4. [ ] Capture provider hardening (T-310): pin the screencapturekit crate exactly and soak-test it in month 1 (its issue history is leaks and stalled callbacks; both shipped comparables vendored their own bindings); prefer periodic SCScreenshotManager captures over a persistent SCStream at 0.5 FPS; named fallback: objc2-screen-capture-kit or vendoring. Document the macOS 26.1 dev-build bundle requirement for the Screen Recording pane.

## Amendment (2026-08-20, plan review)

The technology due-diligence review (docs/review/technical-verification.md) confirmed the stack but added the mitigations now in action items 2 and 4: tauri-specta is permanently release-candidate and requires pinning discipline, and the crates.io screencapturekit crate is a single-maintainer bet requiring a soak test and a named fallback.
