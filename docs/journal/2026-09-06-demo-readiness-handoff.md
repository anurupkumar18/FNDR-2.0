# Demo-readiness handoff · 2026-09-06

## Verified in this workspace

- Normal FNDR launch is non-capturing. The desktop starts in `not_started`;
  **Start capture** is the only normal action that creates the worker path.
  A live process smoke check stayed alive with an explicit vault path absent,
  then exited cleanly.
- The trust window shows content-free lifecycle state, current non-prompting
  Screen Recording preflight, an explicit start action, pause/resume, and a
  bounded read-only owner audit viewer.
- `fndr-shell --doctor` is non-capturing: it checks only demo paths/model and
  Core Graphics' preflight. Its successful local check reported `granted` and
  did not create the temporary vault.
- A capture-boundary failure maps to
  `screen_recording_or_capture_unavailable`; the UI gives System Settings
  guidance without exposing the underlying error or claiming denial versus
  revocation.
- Existing private/incognito **title cues** map to a distinct content-free
  pre-pixel `private_browsing` skip. This is not browser-native detection.
- The audit viewer reads only an existing SQLite vault and returns time, tool,
  outcome, and raw-release flag. It cannot create a missing vault and carries
  no query, record ID, URL, or capture content.
- The retrieval bench has a real optional vector smoke route, but the demo
  MCP/UI route remains keyword FTS. Do not claim hybrid, RRF, reranking, or
  MCP-served vector retrieval.

## Evidence

- `CARGO_BUILD_JOBS=1 make test` passed after the Screen Recording preflight
  IPC/UI change: workspace lints, formatting, clippy, all Rust tests,
  generated binding sync, TypeScript typecheck, and Vitest.
- `make bench` passed: FTS baseline Recall@5/MRR@10 `1.0000/1.0000`, p50/p95
  `0.20/0.34 ms` in the last run before this handoff.
- `git diff --check` passes.

## Human-only demo checks

1. Render and inspect the desktop window during the presenter rehearsal.
2. Deliberately authorize (or deny/revoke) Screen Recording and observe the
   status behavior on the target macOS build.
3. Perform the private-window, menu-bar handoff, pause/resume, audit-viewer,
   and clean-drain demonstration in a real GUI session.
4. Run the bounded capture soak only with a human at the keyboard.

## Current machine constraint

The build directory is approximately 72 GiB and the data volume has about
4 GiB free after repeated native workspace gates. Do not run more heavy Cargo
builds or clean `target` without an explicit storage decision. The source,
model, and user-owned `docs/journal/2026-09-05-claude-code-handoff-prompt.md`
remain protected.

## Presenter materials

- `docs/demo/PRESENTER-CARD.md`
- `docs/demo/DEMO-READINESS.md`
- `docs/demo/ALPHA-RUNBOOK.md`
