# Lessons: the cross-session learning loop

Append-only. Every mistake or surprise that cost a working cycle becomes an
entry here, at the moment it is understood. Every session, in every tool
(Claude Code, Codex, Cursor, a teammate's editor), reads this file before
starting work: it ships inside the generated AGENTS.md, so the whole team
and every agent inherit each lesson automatically. Larger reversals also go
to `docs/incidents.md`; this file is for the working-level rules.

Entry format:

```
## <date> · <one-line title>
Cost: <what it burned: a red CI run, a debugging hour, a wrong design>
Root cause: <the actual mechanism, not the symptom>
Rule: <the behavior now followed instead>
```

---

## 2026-08-21 · Migration file written but never registered
Cost: three failing tests and a diagnosis pass.
Root cause: migrations are registered by a const array in
`crates/fndr-store/src/migrations.rs`; creating the SQL file does nothing by
itself. The runner's list is the source of truth.
Rule: after creating any registered-by-list artifact (migration, CI step,
binding, workspace member), grep for the registry and confirm membership
before running tests.

## 2026-08-21 · Edition 2024 makes unsafe-in-unsafe-fn a hard gate
Cost: a green `cargo test` followed by a red `make test` (clippy).
Root cause: code ported from edition 2021 relies on implicit unsafe blocks
inside unsafe fns; edition 2024 warns and `-D warnings` promotes it.
Rule: ports from v1 get explicit `unsafe {}` blocks at each operation during
the port, and `make test` (not bare `cargo test`) is the local gate.

## 2026-08-21 · Header names are lowercase on the wire (http crate)
Cost: a failing resume test blamed on the wrong component.
Root cause: ureq/reqwest normalize header names to lowercase per the http
crate; a hand-rolled test server matching "Range:" case-sensitively never
saw the header.
Rule: any hand-rolled HTTP parsing matches headers case-insensitively.

## 2026-08-21 · Lance default prune reclaims nothing for our write pattern
Cost: would have shipped a maintenance scheduler that never freed disk.
Root cause: prune keeps versions inside a retention window and refuses files
newer than 7 days unless `delete_unverified` is set; our versions are always
younger than that.
Rule: measured behavior beats documented behavior; spike the maintenance
path of any storage engine before designing its scheduler (T-208 pattern).

## 2026-08-21 · Release-candidate crates drift between rc versions
Cost: a compile failure on the first specta-typescript API use.
Root cause: the specta family is permanently rc and renames APIs between
rc releases; remembered API shapes are unreliable.
Rule: before coding against specta/tauri-specta/lancedb/rmcp, read the
pinned version's source in `~/.cargo/registry` (or fetch the crate), and pin
exactly (`=x.y.z-rc.n`).

## 2026-08-21 · Transitive dependencies can violate our own bans
Cost: a red cargo-deny lane after adding lancedb.
Root cause: lance core hard-embeds a catalog REST client (reqwest) that no
feature flag removes; tauri pulls reqwest for iOS/Android targets only.
Rule: after adding a heavy dependency, run `cargo deny check` locally and
trace hits with `cargo tree -i <crate>` before pushing; scope any exception
to the exact parent crate and amend ADR-004 in the same PR.

## 2026-08-21 · The guard hook reads the session cwd's branch
Cost: a blocked push and a confusing denial while working on a second repo.
Root cause: the personal block-main hook resolves the current branch from
the directory the session started in, not from the repo the git command
targets.
Rule: open sessions inside the repository being changed; never bypass the
hook, restructure the work instead.

## 2026-08-21 · The first CI run after a heavy dependency is the budget test
Cost: a 17m24s rust lane (budget: 15m) on the lance PR.
Root cause: rust-cache has no cache for a new dependency tree; the first
uncached run pays full compile.
Rule: when adding a heavy dependency, say so in the PR body, expect the
first run to bust the budget once, and verify the cached follow-up run
returns under it.

## 2026-09-06 · Clamp before casting, and prove the test can fail
Cost: a self-review catch, not a production bug — but only because the
review happened. Extracting a `50` literal into a named `SEARCH_LIMIT_CAP`
turned `limit.min(50) as i64` into `(limit as i64).min(CAP)`. A `usize`
above `i64::MAX` casts to `-1`, and SQLite reads `LIMIT -1` as *no limit*,
so the "safety" cap silently became unbounded.
Root cause: reordering a clamp and a cast looks like a formatting change and
is a semantic one. The first regression test written for it also passed
against the bug, because the fixture held fewer rows than the cap.
Rule: clamp in the target domain before casting (`limit.min(CAP as usize) as
i64`). And when a test exists to catch a specific regression, reintroduce
the bug once and watch it fail — a test whose fixture is too small to
distinguish the two behaviors is theater.

## 2026-09-06 · `make test | tail` reports the pipe's exit code, not make's
Cost: a full gate re-run, and a few minutes believing a green gate that had
not been verified.
Root cause: `make test 2>&1 | tail -150` exits with `tail`'s status, which is
0 whether or not `make` failed. The truncation also cut the failing crate's
output out of the saved log, so neither the exit code nor the text showed
the failure.
Rule: run the gate as `make test > /tmp/gate.log 2>&1; echo "EXIT=$?"` and
grep the full log, rather than piping it through `tail`/`head`. Beware the
mirror-image trap when checking: a trailing `grep -c FAILED` that finds
nothing exits 1 and makes a green run look failed. An exit code you did not
actually read is not a verification.

## 2026-09-06 · Nanosecond timestamps are not a per-thread unique ID
Cost: an intermittent `make test` failure (`Lance(TableAlreadyExists)`)
across two unrelated `capture_scheduler` tests, misdiagnosed at first as
caused by an unrelated same-session change to a different crate.
Root cause: a test helper built a "unique" Lance directory from
`process::id()` + `SystemTime::now()` nanos only. `cargo test` runs tests in
one process on separate threads; two threads can read the same clock value,
so two tests collided on the same Lance table path.
Rule: never rely on a raw timestamp alone for per-test-run uniqueness inside
one process; pair it with a process-wide `AtomicU64` counter (or a crate
like `tempfile` that guarantees this). A flaky failure that reproduces at a
different assertion/line on retry, in a file the current diff never touched,
is a signal to check test isolation before assuming the diff is at fault.

## 2026-09-06 · A Tauri app binary needs its build context and icon from day one
Cost: several compile cycles while turning a library-only shell into a runnable
desktop host.
Root cause: `tauri::generate_context!` needs a build-script `OUT_DIR`, while
`tauri::tauri_build_context!` needs `tauri-build` code generation explicitly
enabled; the generated desktop context also requires a real PNG icon even when
the first host creates no window.
Rule: when adding the first runnable Tauri binary, add the pinned
`tauri-build` build dependency, `build.rs` with `CodegenContext`,
`tauri.conf.json`, and the product icon in the same slice; compile the binary
as part of the focused gate before claiming the shell is runnable.

## 2026-09-06 · `target/` grew to 72 GiB and nearly exhausted the disk
Cost: a session paused at the demo-readiness gate because the machine had 4
GiB free, blocking any further native build or the human rehearsal.
Root cause: repeated full-workspace `CARGO_BUILD_JOBS=1` rebuilds (forced by
low-memory/low-core gates) plus multiple binaries (shell, sidecar, mcp, bench)
each accumulate their own incremental artifacts under `target/debug`; nothing
in this repo ever pruned it, so it grew unbounded across sessions.
Rule: check `du -sh target` before starting a build-heavy session; run `make
clean` (now wraps `cargo clean`) when it passes a few GiB, since debug build
output is fully regenerable and never worth protecting. Don't let a low-disk
warning block work silently -- surface it and clean instead of routing around
it with partial builds.

## 2026-09-08 · A fresh worktree fails `make test` at the UI lane
Cost: a red full gate that looked like a regression and was `tsc: command
not found`; the whole Rust workspace had already passed.
Root cause: `git worktree add` gives a new tree without `ui/node_modules`,
and `make test`'s `test-ui` target runs `tsc`/`vitest` directly rather than
installing first. `make bootstrap` (scripts/dev-setup.sh) is what installs.
Rule: run `npm ci` in `ui/` (or `make bootstrap`) as the first command in a
new worktree, before reading a `make test` failure as a code problem.

## 2026-09-08 · Substring app matching misroutes AppleScript, not just cleanup
Cost: would have shipped a metadata source that attributes one app's URL to
another app's screenshot.
Root cause: the v1 classifier identified apps with `name.contains(...)`.
"Search" contains `arc` and "Knowledge Base" contains `edge`. In
`fndr-textsignal` that only mis-tuned line thresholds, but the same pattern
in `fndr-capture::foreground` selects which browser's AppleScript dictionary
to query, and a backgrounded browser answers with its own front tab.
Rule: identify apps by `CFBundleIdentifier` first (exact, then family
prefix) and treat the localized name as a whole-token fallback; never
substring-match an app name. Mozilla is the worked example for why family
prefixes need care: `org.mozilla.` covers Thunderbird too.

## 2026-09-09 · A finished agent is not a committed agent
Cost: a near-total loss of several thousand lines of real, working code
(a full keyword+vector search engine, a declarative gate-policy table with
a passing named regression test, a Lance compaction scheduler, and more)
across nine separate worktrees, discovered only because each one was
opened and its `git status` read before being deleted. A tenth agent, given
an explicit "open a draft PR" instruction, still stopped after a passing
`make test` without committing.
Root cause: a background agent finishing (or a session crashing) says
nothing about whether the agent's edits are in git history. Two different
worktree directories with real diffs and zero commits looked, from the
outside (a `git branch -a` in the main repo), identical to worktrees that
had done nothing — the branch pointer is only informative if the agent
committed to it. "The task notification says completed" and "the work is
safe" are unrelated facts.
Rule: before deleting, reusing, or otherwise treating any agent worktree as
finished (success, failure, or crash alike), `cd` into it and read `git
status --short` and `git diff --stat` directly — never infer completeness
from a branch's committed history in the main repo, a task notification's
`status` field, or an agent's own prose summary. If there is any uncommitted
diff, commit and push it before doing anything else, even if the work looks
unfinished or of unknown quality; a `wip:` checkpoint costs nothing and a
deleted worktree is not recoverable. When briefing an agent whose run might
be long or might be interrupted, tell it explicitly to commit incrementally
as it goes, not only once at the very end.

## 2026-09-09 · `vitest.config.ts`'s `globals: false` breaks RTL's automatic cleanup
Cost: a "prove it fails" cycle diagnosing an intermittent-looking
`getByRole` "found multiple elements" failure in the first React component
test written in `ui/`.
Root cause: `@testing-library/react`'s automatic per-test DOM cleanup only
self-registers when it detects a global `afterEach` (`typeof afterEach ===
"function"`). This project's `vite.config.ts` sets `test.globals: false`
(explicit imports required, matching the rest of the codebase's style), so
that global never exists and cleanup silently never runs — multiple tests'
rendered trees pile up in the same document, and a later `getByRole` query
for a name used in more than one test matches more than once.
Rule: any Vitest project with `globals: false` needs an explicit
`afterEach(() => cleanup())` in a shared `vitest.setup.ts`, not per test
file. Add it in the same slice as the first component test, not after the
first mysterious multi-element failure.

## 2026-09-09 · A controlled-component unit test needs a real controller, not a no-op mock
Cost: a wrong first fix (adding internal `useState`/`useEffect` buffering to
a presentational `SearchField` component) that was caught and reverted
before landing, plus a second diagnosis cycle.
Root cause: a test rendered `<SearchField value="" onChange={vi.fn()} />`
and typed into it, expecting `onChange` to report the accumulated string
("rust"). But a no-op mock never feeds the typed value back into the
`value` prop, so React's controlled-input reconciliation resets the DOM
value to the fixed prop after each keystroke's event is processed — each
`onChange` call ends up reporting only the single most-recently-typed
character. This looks exactly like a component bug (and was first
misdiagnosed as one) but is a test-authoring bug: no real caller of a
controlled input behaves like a static value plus a no-op callback.
Rule: when a controlled-component test needs to verify typed input
accumulates correctly, wrap the component in a small local stateful helper
that mimics its real caller (state + a handler that feeds the new value
back into the prop), not a bare `vi.fn()`. If a test for a controlled
component fails in a way that looks like "only the last character sticks,"
suspect the test's mock before the component.

## 2026-09-09 · Vitest fake timers need a `jest` global shim for `waitFor` to work
Cost: five async tests hanging until Vitest's real outer timeout (~5s each)
before being diagnosed.
Root cause: `@testing-library/dom`'s `waitFor` only takes its fake-timer-aware
polling path when it detects a global `jest` object with specific properties
(`jestFakeTimersAreEnabled()` checks `typeof jest !== "undefined"` first).
Vitest's `vi.useFakeTimers()` never defines a `jest` global, so `waitFor`
always falls back to real-timer polling (`setInterval`) — which is itself
faked and never fires, so the awaited assertion is never rechecked.
Rule: any Vitest suite combining `vi.useFakeTimers()` with
`@testing-library/dom`'s `waitFor` (directly or via RTL) needs
`globalThis.jest = { advanceTimersByTime: vi.advanceTimersByTime }` in
`vitest.setup.ts`. Confirmed by reading `@testing-library/dom`'s own
`helpers.js`/`wait-for.js` source, not just by trying things until tests
passed.

## 2026-09-09 · A debounced effect must invalidate stale requests on every run, not only on dispatch
Cost: a code-review-caught, then independently-reproduced Critical bug in a
newly-written screen, requiring a second implementation pass and a new
regression test.
Root cause: a request-id ref (`requestIdRef.current`) was only incremented
inside the debounced `setTimeout` callback, right before a new request
dispatched. Clearing the input (an early-return branch that never dispatches
anything) never bumped the ref, so a slow response from an already-abandoned
query could still pass the `requestId === requestIdRef.current` staleness
check and silently overwrite the idle view with stale results or an error
the person had already moved past.
Rule: in a debounced-request `useEffect` using a ref-based staleness guard,
bump the ref unconditionally at the top of every effect run (before any
early return), capture it once into a local, and only ever compare against
it afterward (in the timeout callback and in `.then`/`.catch`) — never
re-increment anywhere but that one place. Write the regression test as
"dispatch, then abandon before the response arrives, then let the response
land late" — the version of the race that a scenario like "type fast, retype
before the first response" won't exercise.

## 2026-09-09 · A Vite/React app that passes every automated check can still render as a blank window in Tauri
Cost: a fully "done" plan (13 tasks, 20 passing tests, clean `vite build`,
clean `vite preview` over real HTTP) that rendered as a completely blank
window the first time it was actually launched as the real native app —
caught only because the manual verification step in the plan was actually
run rather than trusted-on-paper, after a first pass had reported it
"could not verify" the launch and moved on.
Root cause, two independent bugs neither test nor build caught: (1) Vite's
default absolute asset base (`/assets/...`) resolves against the webview's
origin root, not the subfolder (`workspace/`) this window's HTML is served
from under Tauri's shared `frontendDist` — silently 404s, no visible error.
(2) Vite's default `crossorigin` attribute on the module script/link tags
forces a CORS-mode fetch; Tauri's custom asset protocol doesn't return
`Access-Control-Allow-Origin`, so WKWebView silently discards the script —
the page loads, CSS applies (the background color visibly changes), but the
JS module never executes and `<div id="root">` stays empty forever. Neither
symptom throws anywhere reachable from Rust-side logs, and neither
reproduces via `vite preview` over real HTTP, since that's genuinely
same-origin. The only way either bug was visible at all was a screenshot of
the actual running app window.
Rule: "the build succeeded and the tests pass" is not evidence a Tauri
frontend actually renders — a plan's manual-launch verification step is not
optional busywork, it is the only check that exercises the real asset
protocol. When an agent reports an environment limitation ("no display,
can't verify") for a step that claims to test a real GUI launch, don't
accept the claim at face value if the environment might actually support
it (check `who`/for an active console session before believing "headless").
Fixed here with `base: "./"` plus a `transformIndexHtml` plugin that strips
`crossorigin`; also added `scripts/check-tauri-build-output.sh` (wired into
CI) so this exact regression class fails a build step instead of only being
visible in a screenshot.
