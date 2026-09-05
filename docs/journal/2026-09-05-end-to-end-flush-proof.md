## Handoff: end-to-end proof, real capture through real Lance vector (2026-09-05)

Done: `crates/fndr-memory/examples/end_to_end_flush.rs` composes the two
seams this session built: `persist_capture` (the real-store safety seam,
first commit today) writes a normal capture through the real safety gate
into real SQLite; `LanceWriter::flush_once` (already-existing, pre-dating
this session) then embeds the pending chunk with the real `GgufEmbedder`
(this session's second-to-last commit) and lands a real vector row in a
real Lance table. I ran it — not just compiled it — and it printed:

```
stored record e2e-record-1 (redactions: 0)
loading the real embedder from .../models/Qwen3-Embedding-0.6B-Q8_0.gguf
flushed 1 chunk(s) with the real embedder
end-to-end proof complete: real capture -> real safety gate -> real
SQLite -> real Lance vector, via the real GGUF embedder (no mock on the
path)
```

This is dev/demo tooling proving composition, not a production
scheduler. The real repeated-call-site for this is still T-403's
model-worker priority queue — this example loads the model once, embeds
one capture, and exits.

Ran and confirmed:
- `cargo fmt --all --check` — clean
- `cargo clippy -p fndr-memory --all-targets -- -D warnings` — clean
  (this run recompiled the lance/datafusion stack under a new
  feature-unification path since `fndr-memory` gained `fndr-inference`
  as a dev-dependency; took ~3m37s uncached)
- `cargo clippy --workspace --all-targets -- -D warnings` — clean
  (fast, cache mostly reused from the fndr-memory-scoped run)
- `cargo test -p fndr-memory` — 4/4 pass, unaffected by the new example
- `git diff --check` — clean
- `make test` (full sweep) — all green
- Manually ran `cargo run -p fndr-memory --example end_to_end_flush` and
  confirmed the printed proof above (also confirmed `models/` stayed
  untracked in `git status` throughout, same discipline as the prior two
  model-related commits)

Decisions:
- Put the example in `fndr-memory` rather than `fndr-store` or a new
  crate, since `fndr-memory` already depends on `fndr-store` and owns
  `persist_capture` — the natural place to show both seams composing.
- Added `fndr-inference` and `tokio` as `fndr-memory` dev-dependencies
  only (not regular dependencies) — `fndr-memory`'s actual library code
  has no reason to depend on the concrete GGUF embedder or an async
  runtime; only this demo binary does.
- Reused the exact seed-capture and scratch-dir pattern from
  `fndr-store/tests/lance_flush.rs` rather than inventing a different
  shape, so the example reads as an obvious real-embedder swap-in for
  that test's `TestEmbedder`.

Landmines:
- Adding a new dev-dependency edge across crates that both pull in
  `lancedb`/`lance`/`datafusion` can trigger a fresh feature-unification
  rebuild of that entire stack, even though nothing in those crates
  changed — expect this the next time a similar edge is added, and don't
  assume a long rebuild here means something is broken.

Produced by: Anurup + Claude Code
