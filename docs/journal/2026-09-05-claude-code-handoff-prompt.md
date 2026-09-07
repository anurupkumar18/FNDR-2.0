# Claude Code handoff prompt — FNDR 2.0

Copy everything below into a fresh Claude Code session opened at
`/Users/anurupkumar/FNDR-2.0`.

---

You are continuing FNDR 2.0. Work carefully and incrementally; preserve the
product direction and do not replace a working vertical slice with decorative
plumbing.

## Start with this exact state

- Repository: `/Users/anurupkumar/FNDR-2.0`
- Canonical product mainline: this v2 repository. The legacy repository
  `/Users/anurupkumar/fndr` is a targeted alpha donor only. It has unrelated
  dirty work in `src-tauri/src/capture/mod.rs`; never reset, clean, edit, or
  commit it.
- Current branch: `codex/a006-real-store-safety-seam`, pushed to origin.
- Pushed baseline HEAD: `abee258` (`docs: hand off verified alpha spine`).
  The GitHub branch is safe to clone and review.
- Do not assume the local tree is clean. Before changing anything, run
  `git status --short --branch`. At handoff it contains exactly these
  uncommitted, unverified files:
  - modified `crates/fndr-memory/Cargo.toml`
  - modified `crates/fndr-memory/src/lib.rs`
  - untracked `crates/fndr-memory/src/write_path.rs`
- Those files are a proposed next slice, not a proven implementation. Inspect
  them, then either finish them with tests and QA or deliberately discard them
  only after checking with the owner. Do not silently mix them with unrelated
  changes.

## Mandatory repository instructions

Read `AGENTS.md` first. It is generated; never edit it by hand. Then read:

1. `docs/ARCHITECTURE.md`
2. `docs/decisions/ADR-004-local-only-boundary.md`
3. `docs/decisions/ADR-005-poc-reuse-policy.md`
4. `docs/decisions/ADR-006-retrieval-architecture.md`
5. `docs/decisions/ADR-007-mcp-surface.md`
6. `docs/decisions/ADR-008-connected-planner.md`
7. `docs/decisions/ADR-009-evaluation-and-trajectory.md`
8. `.claude/skills/fndr-v2-engineering/references/lessons.md`
9. `.claude/skills/fndr-v2-engineering/references/workflows.md`
10. `.claude/skills/fndr-v2-engineering/references/ai-collaboration.md`
11. `docs/CONTEXT.md`, `docs/PRD.md`, and
    `docs/review/BASELINE-2026-09-05.md`

Use the repository workflow automatically: one vertical slice at a time;
feature changes require the Feature Slice workflow; privacy or MCP changes
also require the Privacy and Surface Change workflow; a legacy port needs an
ADR-005 eligibility check, narrow provenance, tests, and an explicit defect
not carried forward. At session end, write a journal handoff with Done / In
flight / Decisions / Landmines / Produced by, then push the branch. Do not
write generated `AGENTS.md`.

Engineering invariants are non-negotiable:

- Local by default: no direct network egress of captured data, telemetry, or
  cloud credentials. Only the bounded planner-export concept in ADR-004 is
  allowed, and no provider client exists or should be added.
- Auth always: MCP is authenticated from its first commit; do not add an
  unauthenticated listener, broad CORS, or a mode bypass.
- No silent degradation: unavailable or skipped behavior is typed and visible.
- Eval-gated ranking: do not alter ranking/retrieval ordering without `make
  bench`, a real-model comparison, and documented metrics.
- No raw screenshot persistence, no real captures/databases/tokens/secrets in
  Git, and no autonomous agent execution.
- One real storage/retrieval route. `SkeletonStore` is an alpha proof only;
  do not extend it into a second production stack.

## What is already working and verified

The branch contains the Alpha runnable spine, documented in
`docs/demo/ALPHA-RUNBOOK.md`:

1. `fndr-capture::FrameSource` acquires a one-shot live screen or checked-in
   PNG fixture.
2. `fndr-privacy` runs built-in sensitive-context and owner blocklist checks
   before OCR. Password managers, financial sites, auth/private browsing, and
   self-capture skip. Secret-bearing OCR text is redacted before persistence.
3. Apple Vision OCR processes the fixture. The local file-backed
   `SkeletonStore` persists and supports FTS search across restarts.
4. Authenticated loopback MCP exposes `fndr.search` and
   `fndr.privacy_status`; the latter returns posture/counts and never lists
   blocklist entries.
5. An E2E test asserts raw fixture PNG bytes are absent from SQLite, WAL, and
   SHM artifacts.

Verified before the local uncommitted changes:

- `make test` passed: workspace lints, UI lints, generated-AGENTS check,
  `cargo fmt --check`, workspace Clippy with `-D warnings`, all Rust tests,
  TypeScript typecheck, and Vitest.
- The manual fixture flow, persistent store flow, password-manager skip, and
  owner-domain blocklist skip were exercised as in the alpha runbook.

Important locations:

- Capture: `crates/fndr-capture/src/source.rs`
- Privacy policy: `crates/fndr-privacy/src/safety_gate.rs` and `blocklist.rs`
- OCR: `crates/fndr-ocr/src/vision.rs`
- Real SQLite truth: `crates/fndr-store/src/store.rs`
- Temporary alpha FTS proof: `crates/fndr-store/src/skeleton.rs`
- MCP: `crates/fndr-mcp/src/server.rs`
- Current state map: `docs/CONTEXT.md`

## Semester plan and current phase

The source review documents are archived under `docs/review/semester/`; the
reconciled baseline is `docs/review/BASELINE-2026-09-05.md`; the detailed
recut is in the PRD and ADR amendments.

### Alpha (now: working/demoable in 2–3 weeks)

The goal is a credible local capture-to-memory-to-authenticated-MCP demo with
visible privacy exclusions. The Alpha proof already works through the
temporary skeleton. Complete it by replacing, not expanding, that temporary
write/read route only when the real `Store` write path and one retrieval route
are both real and test-covered. Rehearse the alpha runbook against a clean
temporary database. Preserve raw-pixel absence, pre-OCR skips, redaction,
blocklist controls, MCP auth, and a clear demo narrative.

### Beta

Build the single eval-gated retrieval path and citation/context-pack behavior.
Implement only the planner draft/preview/immutable approval/audit contract
ratified by ADR-004 and ADR-008. FNDR must not own an external provider client
or run actions. Add data-minimization, cancellation, digest-mismatch,
one-time approval, and audit-deletion tests before any planner surface is
claimed.

### Final (three-month demonstration/submission)

Deliver the PRD month-3 gate: clean-machine installation; a normal workday
captured locally; Claude Code/Desktop resuming work through FNDR MCP with a
cited context pack; and a live negative proof that bank/password-manager
content is absent from vault, context pack, and `privacy_status`. The demo
must include the counterfactual (same task with FNDR off versus one MCP call
with FNDR on), a dry run two weeks beforehand, and evidence rather than
unverified quality claims.

## Exact next slice: real-store safety seam

The in-progress local files begin a narrowly scoped `fndr-memory` API over
the existing `fndr-store::Store`. This is useful only if it is a genuine,
tested persistence boundary, not an unused abstraction.

Desired contract:

- Accept an already assembled capture with supplied record/session/chunk IDs,
  source/app/title/optional bundle ID and URL/OCR text/timestamps.
- Recheck `fndr-privacy::evaluate` immediately before the real SQLite write;
  the scheduler still owns the genuine pre-OCR gate, so do not claim this
  seam prevents OCR.
- Return a typed outcome: stored record with redaction count, or skipped with
  the exact `SafetyReason`; never silently drop a capture.
- For secret-bearing OCR text, call `redact_secret_lines` before
  `Store::insert_capture`, and prove the secret cannot be read through
  `pending_chunks`.
- Use `Store::insert_capture` so record and chunk remain atomic; do not add a
  new database, schema, or retrieval route.
- Add stable unit tests for normal storage, secret redaction, password-manager
  skip, and owner domain-blocklist skip. Run the existing adversarial privacy
  tests and named MCP auth tests as part of verification.
- Update `docs/CONTEXT.md` only after the seam is implemented and accurately
  state that it is a persistence seam, not a full scheduler/retrieval swap.

The existing candidate `write_path.rs` appears aligned with this contract but
has not been formatted, compiled, or tested. Review its API first. If it is
kept, run at least:

```sh
cargo fmt --all --check
cargo test -p fndr-memory
cargo test -p fndr-privacy
cargo test -p fndr-mcp
cargo clippy -p fndr-memory --all-targets -- -D warnings
make test
git diff --check
```

Only commit/push after the owner authorizes it or your working agreement
permits normal implementation commits. The pushed branch remains the stable
handoff point until then.

## Handoff and collaboration expectations

- Start by reporting current branch, status, and whether you will finish or
  set aside the three local files.
- Keep changes narrow and reviewable. Do not perform broad refactors, reset
  branches, delete build trees, or touch the dirty legacy checkout.
- Explain what current code actually proves versus what remains future work.
- For any privacy/surface change, include a one-paragraph threat model in the
  PR/hand-off: who can reach it, credentials, and data exposed.
- Before ending, write `docs/journal/YYYY-MM-DD-<slug>.md` using the required
  format and push the branch so no work exists only locally.

---

End of prompt.
