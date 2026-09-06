## Handoff: QueuedEmbedder — the queue's first real consumer (2026-09-05)

Done: closed the gap flagged as "not done" in the T-403 model-worker
journal entry — nothing routed through the queue yet. Added
`QueuedEmbedder` (`crates/fndr-inference/src/model_worker.rs`), a thin
adapter implementing the `Embedder` trait over an `Arc<ModelWorkerHandle>`
at a fixed `Priority`. Because `LanceWriter::flush_once` already only
needs `&dyn Embedder`, this required zero changes to `fndr-store` itself
— the queue slots in as a drop-in replacement for a directly-constructed
embedder.

Proved it two ways:
1. **Fast, default-running integration test**:
   `flush_once_works_through_the_model_worker_queue` in
   `crates/fndr-store/tests/lance_flush.rs`, using a fake `TestEmbedder`
   behind `ModelWorkerHandle` — no real model needed, runs in every
   `cargo test`/CI pass.
2. **Real end-to-end run**: updated
   `crates/fndr-memory/examples/end_to_end_flush.rs` to spawn a real
   `ModelWorkerHandle` (loader closure calls `GgufEmbedder::load`) and
   flush through `QueuedEmbedder` instead of constructing `GgufEmbedder`
   directly. Ran it against the real downloaded model; printed and
   confirmed:
   ```
   flushed 1 chunk(s) via the model-worker queue
   end-to-end proof complete: real capture -> real safety gate -> real
   SQLite -> model-worker queue -> real GGUF embedder -> real Lance
   vector (no mock on the path)
   ```
   This also means the example itself no longer calls `GgufEmbedder::load`
   from application logic — only from inside the loader closure, which is
   exactly the sanctioned pattern `scripts/check-llm-call-sites.sh`
   expects. Re-ran that script: still clean.

Ran and confirmed:
- `cargo fmt --all --check` — clean
- `cargo test -p fndr-inference --lib` — 10 passed, 1 correctly ignored
  (unaffected)
- `cargo test -p fndr-store --test lance_flush` — 4/4 pass, including the
  new queue-integration test
- Manually ran `cargo run -p fndr-memory --example end_to_end_flush` —
  confirmed the updated proof output above
- `./scripts/check-llm-call-sites.sh` — still clean
- `cargo clippy -p fndr-inference -p fndr-store -p fndr-memory
  --all-targets -- -D warnings` — clean
- `git diff --check` — clean
- `make test` (full sweep) — all green

Updated `docs/ROADMAP-TICKETS.md`'s T-403 row: no longer says "nothing in
production routes through the queue" — the caveat now names what's
actually still missing (T-306's real capture scheduler being the thing
that owns a long-lived `ModelWorkerHandle` across many captures, versus
today's example spawning one per run).

In flight / explicitly not done: this is still proven at the
`LanceWriter::flush_once` call level, not wired into any long-running
process. The real production shape is a scheduler (T-306, still blocked
on ScreenCaptureKit) that owns one `ModelWorkerHandle` for the app's
whole lifetime and calls `flush_once` with a `QueuedEmbedder` repeatedly
as captures accumulate — today's example constructs a fresh
`ModelWorkerHandle` (and therefore reloads the model) on every run,
which is correct for a one-shot demo but not the final shape.

Decisions:
- Put `QueuedEmbedder` in `model_worker.rs` rather than a new file —
  it's a small adapter tightly coupled to `ModelWorkerHandle`'s exact
  API, not an independent concept.
- Chose a fixed `Priority` per `QueuedEmbedder` instance rather than a
  per-call priority parameter on `embed_documents`/`embed_query` — the
  `Embedder` trait's signature is fixed (it's implemented by
  `GgufEmbedder` too, which has no concept of priority), so the priority
  has to be decided at construction time; a caller needing different
  priorities for different calls constructs two `QueuedEmbedder`s
  sharing the same `Arc<ModelWorkerHandle>`.

Landmines: none new beyond what's already documented in the T-403
journal entry.

Produced by: Anurup + Claude Code
