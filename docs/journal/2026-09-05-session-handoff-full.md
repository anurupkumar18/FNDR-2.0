## Handoff: full session, codex/a006-real-store-safety-seam (2026-09-05)

Twelve commits landed and pushed this session, from `abee258` (the pushed
baseline this session started from) to `fa80956` (HEAD). All verified
green with `make test` before each commit; nothing unpushed, nothing left
uncommitted except the pre-existing, untouched
`docs/journal/2026-09-05-claude-code-handoff-prompt.md` (a saved copy of
the handoff prompt itself, not a journal entry — deliberately left alone
all session).

The arc: a privacy-safe write seam, then the real embedder that gives it
something to store, then the queue that makes the embedder safe to share,
then the wiring proving all of it composes end to end against a real
model. Items 1–6 below were the first half; 7–9 (this roll-up's original
end) and 10–12 the second.

### Done, in commit order

1. **`04e6d40` — real-store persistence seam.** Reviewed and finished the
   candidate `fndr-memory::persist_capture` left at handoff: rechecks
   `fndr-privacy::evaluate` immediately before the real SQLite write,
   redacts secret-bearing OCR text, returns a typed
   `Stored`/`Skipped` outcome. This is T-803's write-path down payment,
   not the full ticket (scheduler wiring is separate, larger work).
2. **`bbede19` — T-802 sensitive-context policy (partial).** Made the
   built-in password-manager/financial/medical/auth/secret-pattern lists
   in `fndr-privacy` owner-constructible data (`SensitiveContextPolicy`)
   instead of hardcoded consts, with zero behavior/perf change to
   existing callers (proven by a parity test) and a second test proving
   custom policies replace rather than union with the defaults. The
   "alert queue with dismissal keys" half of T-802's AC is explicitly
   not done — flagged as cross-lane, needs a UI/event surface.
3. **`f325c7e`, `70712bc`, `0f6f614` — roadmap ledger audit.** Found and
   corrected several stale/missing rows in `docs/ROADMAP-TICKETS.md`'s
   progress ledger by reading the actual code and tests rather than
   trusting the table: T-203/T-204/T-205/T-206/T-207 (storage layer),
   T-305/T-401/T-402 (OCR port, model registry+downloader, embedding
   contract), T-701/T-702 (MCP transport, tools wave 1). Several tickets
   with real, tested work were entirely absent from the ledger before
   this (T-305, T-401, T-701); others were more done or less done than
   implied. Docs-only, zero code risk.
4. **`aeacfa9` — embedding contract asymmetry + matryoshka rule.** Added
   `query_embedding_text`/`QUERY_INSTRUCTION` (index/query asymmetry) and
   `truncate_and_renormalize` (the matryoshka rule) to
   `fndr-inference::embedding`, both pure and fully unit-tested. Also
   downloaded the real pinned Qwen3-Embedding-0.6B-Q8_0.gguf (639MB) into
   `models/` (gitignored) via the actual production
   `download_verified` path — first time that function ran against a
   real artifact, not a test fixture.
5. **`e8cc2ac` — concrete GGUF embedder (T-402 core).** `GgufEmbedder`,
   a real `Embedder` backed by llama.cpp via `llama-cpp-2` (new pinned
   dependency, `=0.1.156`, API verified against real source). Ran the
   `#[ignore]`d construction-probe test against the actual model: real
   Metal-backed inference produced a genuine 768-dim, unit-norm,
   non-zero embedding. `cargo deny check` confirmed bans/licenses/sources
   all pass for the new dependency tree (a pre-existing, unrelated yanked
   `chacha20` advisory failure was found and flagged separately, not
   caused by this change — verified via `git diff Cargo.lock`).
6. **`a3718f3` — end-to-end composition proof.** Ran (not just compiled)
   a real capture through `persist_capture` → real SQLite →
   `LanceWriter::flush_once` with the real `GgufEmbedder` → a real vector
   row in a real Lance table. No mock anywhere on the path. Printed and
   confirmed proof output is in the commit body and
   `docs/journal/2026-09-05-end-to-end-flush-proof.md`.

7. **`27598fb`** — this roll-up's first version (docs only).
8. **`0ff85ab` — T-403 model-worker priority queue.**
   `ModelWorkerHandle`/`Priority`: one dedicated worker thread, priority
   ordering, lazy load, idle-timeout unload. The AC's contention test is
   a real ordering assertion (a blocking Backfill job holds the worker
   while a Backfill and an Interactive job queue behind it; released,
   Interactive runs first despite being submitted last) — ran it 5x with
   no flakes. The "no LLM call outside the queue" AC is covered by
   `scripts/check-llm-call-sites.sh`, deliberately left standalone
   rather than wired into the CI `guards` job (that script gates every
   future PR for the whole team; wiring it in deserves its own review).
9. **`f80c419` — `QueuedEmbedder`, the queue's first real consumer.**
   A thin `Embedder` adapter over `Arc<ModelWorkerHandle>`; because
   `LanceWriter::flush_once` only needs `&dyn Embedder`, the queue slots
   in with zero changes to `fndr-store`. Proven by a fast fake-embedder
   integration test *and* a real run through the actual model. The
   `end_to_end_flush` example now flushes via the queue, so it no longer
   calls `GgufEmbedder::load` from application logic at all.
10. **`fa80956` — T-404 capture-burst memoization + throughput
    benchmark.** `memoize_by_text` computes each distinct text once per
    batch (unit-tested with a call-counting fake — an exact count, not a
    timing proxy). Real-model benchmark recorded ~10 docs/sec, with 8
    exact duplicates adding ~0ms instead of doubling the time. Real
    multi-sequence llama.cpp batching was investigated and deliberately
    not attempted (see that entry for why).

Per-commit detail, decisions, and landmines are each in their own journal
entry dated today; this entry is the roll-up index, not a replacement for
them:
- `docs/journal/2026-09-05-real-store-safety-seam.md`
- `docs/journal/2026-09-05-sensitive-context-policy.md`
- `docs/journal/2026-09-05-embedding-contract-asymmetry.md`
- `docs/journal/2026-09-05-gguf-embedder.md`
- `docs/journal/2026-09-05-end-to-end-flush-proof.md`
- `docs/journal/2026-09-05-model-worker-queue.md`
- `docs/journal/2026-09-05-queued-embedder-wiring.md`
- `docs/journal/2026-09-05-embedding-batch-memoization.md`

### In flight / explicitly not done

Nothing is half-finished or merged-but-disabled. What's next, in the
order a future session would naturally hit it:

1. **A long-lived owner of one `ModelWorkerHandle`.** T-403's queue and
   its `QueuedEmbedder` adapter both exist and are proven, but nothing
   holds a worker across many captures yet — the example spawns one per
   run (and therefore reloads the model each time), correct for a demo,
   not the final shape. The real owner is T-306's scheduler, which is
   blocked as below.
2. **Real multi-sequence batching for T-404.** The memoization and
   throughput halves landed; batching several texts into one `decode()`
   call did not. `with_n_seq_max`/`embeddings_seq_ith` exist in the
   pinned API, so the path is known — the risk is that per-sequence
   position/KV-cache/pooling mistakes corrupt embeddings *silently*
   rather than failing loudly, so it needs real empirical verification,
   not a confident-looking diff.
3. **T-306 staged capture pipeline.** Still blocked on T-302
   (ScreenCaptureKit provider) and T-303 (dedup) — neither exists. This
   is real hardware/permission work I flagged earlier this session as
   not safely completable or verifiable headlessly; it wasn't attempted.
4. **T-802's remaining half** (alert queue with dismissal keys) and
   **a real caller** that loads a `SensitiveContextPolicy` from
   `settings` or disk — `fndr-privacy` deliberately does no I/O itself.
5. **The M2 retrieval stack** (`fndr-retrieval`) is still a one-line
   stub. Building it requires the eval corpus/FNDR-Bench infrastructure
   ADR-006 gates ranking changes behind, which doesn't exist yet. Not
   attempted, correctly out of scope for a single session.
6. A **pre-existing, unrelated `cargo-deny` advisories failure** (yanked
   `chacha20` via `rand v0.10.2`, pulled by `rmcp`/`lance-core`) was
   found and flagged as a spawned background task (`task_2b06824b`), not
   fixed here — it predates this session's changes.

### Decisions (session-wide, beyond each commit's own notes)

- Treated the fndr repo (`/Users/anurupkumar/fndr`, no `-2.0` suffix) as
  strictly off-limits per the handoff's explicit warning — never
  touched, never reset, never read for anything but the CLAUDE.md at the
  very start of the session.
- Chose not to attempt any change that would need CI infrastructure I
  can't verify from here (GitHub Actions runners) or hardware I can't
  drive headlessly (ScreenCaptureKit) — every commit this session is
  something I could fully verify locally before claiming it works.
- Every commit was proposed and confirmed with the owner before pushing,
  per the handoff's explicit "commit/push only after the owner
  authorizes it" instruction and this session's own working agreement.

### Landmines for the next session

- **Never remove the `#[ignore]`** on either real-model test in
  `fndr-inference::gguf_embedder::tests` —
  `construction_probe_dimension_and_non_zero` or
  `throughput_benchmark_and_memoization_win`. `models/*.gguf` is
  gitignored and will never exist on a fresh CI checkout; an un-ignored
  version of either fails every future PR, forever.
- **`scripts/check-llm-call-sites.sh` is not a CI gate.** It enforces
  T-403's "no LLM call outside the queue" rule only when someone runs it.
  If it should become automatic, that's a deliberate change to
  `scripts/workspace-lints.sh` / the `guards` CI job, with its own review
  — not a drive-by edit, since it would gate every future PR.
- **`models/` at the repo root now has a real 639MB file in it** (not
  committed, confirmed gitignored and untracked at every commit this
  session). If setting up a fresh clone, run
  `cargo run -p fndr-downloader --example fetch_model` before trying
  `cargo test -p fndr-inference -- --ignored` or
  `cargo run -p fndr-memory --example end_to_end_flush`.
- **`llama-cpp-2` is a new heavy native dependency** (pulls in
  `bindgen`/`clang-sys`/`cmake` and compiles the full llama.cpp C++
  tree). The first CI run after this branch's changes land will almost
  certainly bust the 15-minute PR budget once; expect it, per the
  repo's own dependency-budget lesson. Metal is auto-enabled on Apple
  Silicon macOS via a target-cfg override in `llama-cpp-2`'s own
  Cargo.toml, independent of any feature flag we set.
- **`cargo-deny` was not installed on this machine before this
  session**; a future session may need `cargo install cargo-deny
  --locked` again if it's not already present, and should expect the
  pre-existing `chacha20` advisories failure until `task_2b06824b` (or
  equivalent) is resolved — don't assume that failure means something
  in a new change broke.
- **This session's shell kept resetting cwd to `/Users/anurupkumar/fndr`**
  (the legacy repo) between Bash tool calls; every command in this
  session explicitly `cd`'d into `/Users/anurupkumar/FNDR-2.0` rather
  than relying on persisted cwd. Confirm the same behavior in any
  follow-up session before assuming a bare command runs in the right
  repo.
- Several `make test`/`cargo clippy --workspace` runs this session took
  3+ minutes from a cold cache whenever a new dependency edge touched
  the lance/datafusion/llama-cpp-2 stack (feature-unification triggers a
  surprisingly wide rebuild). Not a hang; budget the wait.

Produced by: Anurup + Claude Code
