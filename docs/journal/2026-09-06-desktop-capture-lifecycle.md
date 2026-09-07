# Desktop capture lifecycle (T-901 partial) · 2026-09-06

## Shipped

`fndr-shell` now has a headless-first Tauri host that owns the real capture
worker for the active capture lifetime. A normal launch first renders a
non-capturing trust/status state; only the explicit **Start capture** action
starts the existing `RealCaptureWorker`. It forwards only generated and
content-free `capture://status` payloads, makes a missing model or other
startup problem a visible `blocked` state, and joins the worker's final
SQLite-to-Lance drain on `ExitRequested`.

The host now has a status-only main window and a menu-bar icon. The window
uses the generated IPC command/event contract and therefore never receives
screen content; closing it hides it while the engine keeps running in the menu
bar. **Show FNDR** restores the window and **Quit FNDR** follows the same
`ExitRequested` drain path, so a presenter does not need to force-quit a
headless process. The explicit window and tray pause controls wait for the
worker's acknowledgement before publishing `paused`, which prevents new
capture opportunities without discarding an in-flight write. The generated
Tauri capability schemas are ignored as build products; the checked-in source
of truth is the Tauri configuration and Rust host.

The status command gives a just-opened UI its initial state; the UI then
subscribes to the push event. It must not poll. The generated binding and its
sync test enforce the Rust-to-TypeScript contract.

`fndr-shell --doctor` is the first T-907 diagnostic seam. It takes the same
explicit `--data-dir`/`--model` paths as the demo host, reports model and
data-path readiness plus the intentionally untested Screen Recording state,
and exits before Tauri or capture construction. A missing model is a typed
`blocked` exit with code 3; doctor never creates the named data directory.

The desktop host also retains a non-blocking advisory lock in its chosen data
directory for its entire lifetime. This is the first single-writer boundary:
a second host using the same vault fails at startup rather than opening a
second SQLite/Lance writer. The kernel releases the lock after process exit;
the harmless lock file may remain. The supported Tauri single-instance plugin
now intercepts a second graphical launch before that host starts, and asks the
first host to show and focus its existing main window. The plugin integration
is compile-verified; a live graphical handoff still needs the deliberate
operator demo run below.

## Deliberate demo command

On a machine where Screen Recording permission is deliberately granted and the
registered embedding model is already present, a bounded run is:

```sh
cargo run -p fndr-shell --bin fndr-shell -- \
  --data-dir /tmp/fndr-demo \
  --model models/Qwen3-Embedding-0.6B-Q8_0.gguf \
  --run-seconds 60
```

`--run-seconds` requests the normal Tauri exit path, so the run exercises the
capture worker's shutdown drain rather than teaching the demo operator to
force-quit a headless process. The command intentionally creates demo data
only under the named directory.

## Verified

- `CARGO_BUILD_JOBS=1 cargo test -p fndr-shell` — 16 unit/integration tests
  plus bindings sync passed.
- `CARGO_BUILD_JOBS=1 cargo test -p fndr-mcp --bin fndr-mcp` — the durable
  local MCP launcher parses its required SQLite store and bounded port.
- `CARGO_BUILD_JOBS=1 make test` — workspace lints, generated-files check,
  formatting, Clippy, Rust tests, UI typecheck, and Vitest all passed.
- The lifecycle's regression test proves a private pipeline error and a
  private flush error cannot appear in a serialized status payload.
- `target/debug/fndr-shell --doctor --data-dir <missing-dir> --model
  <missing-model>` — reports `will_create`/`missing`, exits 3, and leaves the
  named data directory absent.
- A normal `target/debug/fndr-shell --data-dir <missing-dir> --model
  <missing-model>` launch remained alive for the controlled probe and left the
  named data directory absent before and after exit. Since capture startup
  creates that directory before constructing ScreenCaptureKit, this proves the
  normal launch did not enter the capture path.

## Still honest about the boundary

This is not yet the complete T-901/T-902 shell: login autostart, a live egress
counter, audit-log viewer, and permission/revocation guidance remain open. No
unattended hardware capture was run for this journal; a person must
deliberately run the bounded command above.
