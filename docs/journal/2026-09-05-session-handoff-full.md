## Handoff: full session, codex/a006-real-store-safety-seam (2026-09-05)

Fourteen commits landed and pushed this session, from `abee258` (the
pushed baseline this session started from) to `ab56409` (HEAD). All
verified green with `make test` before each commit; nothing unpushed,
nothing left uncommitted except the pre-existing, untouched
`docs/journal/2026-09-05-claude-code-handoff-prompt.md` (a saved copy of
the handoff prompt itself, not a journal entry — deliberately left alone
all session).

The arc: a privacy-safe write seam, then the real embedder that gives it
something to store, then the queue that makes the embedder safe to share,
then the wiring proving all of it composes end to end against a real
model — and finally the real ScreenCaptureKit provider, which is the
first time this repo has read text off an actual screen.

**The headline, for anyone reading only this paragraph:** as of `ab56409`
FNDR-2.0 can capture a real screen (ScreenCaptureKit), OCR it (Vision),
gate it for privacy, store it in SQLite, embed it with a real local model
through a priority queue, and land a real vector in Lance. Every one of
those was verified running against real hardware and a real model, not
just compiled. What it still cannot do is any of that *continuously*
(no scheduler), *search* it meaningfully (retrieval is a stub), or
*show* it (no UI, no shell).

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

11. **`55a9acd`** — this roll-up's second version (docs only).
12. **`ab56409` — T-302 real ScreenCaptureKit provider.**
    `ScreenCaptureKitSource`, one-shot `SCScreenshotManager` per ADR-001
    action item 4, `screencapturekit` pinned `=9.0.1`. Verified against a
    live screen end to end (666KB real PNG → Vision OCR read 199 chars →
    privacy gate → SQLite). Chose the ADR-named crate after checking both
    candidates against real published source — `objc2-screen-capture-kit`
    0.2.2 does not bind the capture methods at all. Needed a new
    workspace `.cargo/config.toml` rpath for the crate's Swift shim,
    which invalidated every cached build once.

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
- `docs/journal/2026-09-05-screencapturekit-provider.md`

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
3. **T-306 staged capture pipeline — now the single biggest blocker.**
   T-302 (its hardware-dependent dep) landed this session and T-305 was
   already done, so T-306 is now gated only on **T-303** (perceptual/
   semantic dedup) and **T-304** (admission policy port, which ADR-005
   lists as a near-verbatim PORT item). Neither needs hardware. Once
   those land, T-306's scheduler is what finally makes capture
   *continuous* rather than one-shot — and it is the natural owner of a
   long-lived `ModelWorkerHandle` (item 1). This is the shortest
   remaining path to dogfooding.
4. **T-310 soak for the capture provider.** ADR-001 wants a multi-day
   soak with an RSS trend assertion before trusting the pinned
   `screencapturekit` crate, whose issue history is leaks. One-shot
   `SCScreenshotManager` is the shape least likely to leak, but that is
   an argument, not a measurement. Until this runs, T-302 is working but
   unproven over time.
5. **T-802's remaining half** (alert queue with dismissal keys) and
   **a real caller** that loads a `SensitiveContextPolicy` from
   `settings` or disk — `fndr-privacy` deliberately does no I/O itself.
6. **The M2 retrieval stack** (`fndr-retrieval`) is still a one-line
   stub. Building it requires the eval corpus/FNDR-Bench infrastructure
   ADR-006 gates ranking changes behind, which doesn't exist yet. Not
   attempted, correctly out of scope for a single session.
7. A **pre-existing, unrelated `cargo-deny` advisories failure** (yanked
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
- **`.cargo/config.toml` now exists and is load-bearing.** Without its
  `-Wl,-rpath,/usr/lib/swift`, anything linking `fndr-capture` compiles
  fine and then dies at startup on `@rpath/libswift_Concurrency.dylib`.
  Do not "clean up" that file. It cannot move into a `build.rs` — Cargo
  does not propagate build-script link args to downstream binaries. Also
  note it invalidated every cached build once; CI's first run after
  `ab56409` will be a full cold rebuild.
- **Capture is one-shot, not continuous.** `ScreenCaptureKitSource::grab()`
  takes a single frame. Nothing calls it on a timer yet. If someone
  expects FNDR to be recording in the background right now, it isn't —
  that's T-306.
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
