## Handoff: codex/a006-real-store-safety-seam (2026-09-05)

Done: Reviewed and finished the in-progress `fndr-memory` write-path slice left
at handoff. `persist_capture` (`crates/fndr-memory/src/write_path.rs`) accepts
an already-assembled capture, reruns `fndr-privacy::evaluate` immediately
before the real SQLite write, redacts secret-bearing OCR text via
`redact_secret_lines` before `Store::insert_capture`, and returns a typed
`PersistCaptureOutcome::Stored { record_id, redaction_count }` or
`Skipped { reason: SafetyReason }` — never a silent drop. Verified the
candidate code against the real `fndr-privacy` and `fndr-store` public APIs
(not from memory) before trusting it. Ran and confirmed:
- `cargo fmt --all --check` (applied one formatting fix, then clean)
- `cargo test -p fndr-memory` — 4/4 pass (normal storage, secret redaction,
  password-manager skip, owner domain-blocklist skip — the exact mandated set)
- `cargo test -p fndr-privacy` — 16/16 pass (adversarial safety-gate + blocklist suite)
- `cargo test -p fndr-mcp` — 12/12 pass, including named regression tests
  `mcp_rejects_unauthenticated_loopback` and `mcp_rejects_web_origin_with_valid_token`,
  and the raw-pixel-absence proof
- `cargo clippy -p fndr-memory --all-targets -- -D warnings` — clean
- `git diff --check` — clean
- `make test` (full sweep: workspace lints, ui lints, AGENTS.md drift check,
  `cargo fmt --check`, workspace clippy `-D warnings`, all Rust unit/integration/doc
  tests, UI `tsc --noEmit`, Vitest) — all green

Updated `docs/CONTEXT.md` with an accurate row for the new seam, stating
explicitly that it is a persistence boundary only and does not replace the
capture scheduler's pre-OCR gate or add a retrieval route.

In flight: Nothing left in flight on this slice. Next natural step per
`docs/CONTEXT.md`'s near-term order of operations is wiring the actual
capture pipeline (scheduler) to call `persist_capture` instead of the
`SkeletonStore`, which is a separate, larger vertical slice (replacing the
alpha skeleton write/read path) and was intentionally not started here to
keep this change narrow and reviewable.

Decisions:
- Kept the candidate `write_path.rs` as-is after review rather than rewriting
  it — its shape, test coverage, and API usage already matched the ADR-005/
  fndr-v2-engineering contract exactly.
- Did not touch `crates/fndr-store/src/skeleton.rs` or add any new schema/
  retrieval route, per the explicit instruction not to extend the temporary
  skeleton into a second production stack.
- Left the second `evaluate()` call for metadata-only pre-check (with
  `ocr_text: None`) as a deliberate redundant recheck rather than optimizing
  it away — matches the "recheck immediately before write" contract even
  though metadata fields don't change between the two calls in this seam.

Landmines:
- `cargo clippy -p fndr-memory` triggers a full `lancedb`/`datafusion`/`lance`
  compile the first time in a session (transitively via `fndr-store`) — this
  took ~2m43s uncached in this session. Expect the same on a clean CI cache.
- The pre-existing untracked file `docs/journal/2026-09-05-claude-code-handoff-prompt.md`
  is just a saved copy of the handoff prompt text itself, not a journal entry;
  left untouched.
- Working directory `/Users/anurupkumar/fndr` (legacy repo, no `-2.0` suffix)
  is a separate checkout with unrelated dirty work in
  `src-tauri/src/capture/mod.rs`; this session's shell cwd kept resetting to
  it between commands, so every command explicitly `cd`'d into
  `/Users/anurupkumar/FNDR-2.0` rather than relying on persisted cwd. Confirm
  the same in any follow-up session.

Produced by: Anurup + Claude Code
