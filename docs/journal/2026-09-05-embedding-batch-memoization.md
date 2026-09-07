## Handoff: T-404 embedding batch path (partial: memoization + throughput) (2026-09-05)

Done: scoped T-404 down to its two AC bullets that were safely
deliverable without touching llama.cpp's multi-sequence batching
internals (real batching across sequences in one `LlamaBatch`/`decode`
call is real, meaningfully risky native-API work I didn't attempt this
session — see "Explicitly not done" below).

**"Capture-burst memoization"**: `memoize_by_text` in
`crates/fndr-inference/src/gguf_embedder.rs` — a small, pure, generic
helper (`Fn(&str) -> Result<Vec<f32>, EmbedError>`) that computes each
distinct text in a batch exactly once and reuses the result for later
duplicates, preserving the caller's original order. `GgufEmbedder::
embed_documents` now calls through it instead of looping directly.
Motivation: capture-burst chunk batches often repeat boilerplate (nav
chrome, headers) verbatim across chunks in the same tick.

Made this independently unit-testable without a real model by keeping
`compute` generic: `memoize_by_text_computes_each_distinct_text_exactly_once`
proves exactly one call per distinct text via an atomic counter (not an
inference cost proxy — an actual call count), and
`memoize_by_text_propagates_the_first_error_and_stops` proves a failure
mid-batch doesn't call `compute` for texts after the failing one.

**"Throughput benchmark recorded"**: `throughput_benchmark_and_memoization_win`
(`#[ignore]`d, needs the real model — same CI-safety discipline as the
other real-model tests). Ran it against the actual downloaded model:

```
memoization: 16 texts (8 unique, 8 duplicate) in 723.885875ms;
unique-only baseline was 773.658709ms
```

8 unique documents embedded in ~774ms (≈10 docs/sec on this machine,
Metal-accelerated). 16 texts with 8 exact duplicates finished in
~724ms — not roughly double the baseline, which is what memoization is
for. This is a recorded number for a human to read, explicitly not a
pass/fail perf gate (real-hardware timing varies too much for a hard
threshold) — the test's own assertion is a generous `< 2x` sanity bound,
not a benchmark-quality claim. `make bench`/FNDR-Bench remains the
eval-gated path for anything that would change ranking (ADR-006); this
is throughput only and doesn't touch retrieval quality.

Ran and confirmed:
- `cargo fmt --all --check` — clean
- `cargo test -p fndr-inference --lib` — 12 passed, 2 correctly ignored
  (construction probe + the new throughput benchmark)
- `cargo test -p fndr-inference --lib throughput_benchmark -- --ignored
  --nocapture` — ran against the real model, printed the numbers above,
  assertion passed
- `cargo clippy -p fndr-inference --all-targets -- -D warnings` — clean
- `cargo clippy --workspace --all-targets -- -D warnings` — clean
- `git diff --check` — clean
- `make test` (full sweep) — all green

Explicitly not done (real multi-sequence batching): the AC's deeper
implication — genuinely batching multiple texts into one `LlamaBatch`
across several sequences in a single `decode()` call, rather than a
fresh `LlamaContext` per text — was investigated (`llama-cpp-2` does
expose `with_n_seq_max`/`embeddings_seq_ith` for this) but not
attempted. Getting KV-cache sizing, per-sequence position resets, and
pooling-type isolation right across concurrent sequences is real
correctness risk I can't fully validate without extensive empirical
testing against real inference (each real-model test run takes real
wall-clock time), and getting it subtly wrong would silently corrupt
embeddings rather than fail loudly. Left as a clearly separate future
slice; `docs/ROADMAP-TICKETS.md`'s T-404 row is marked Partial, not
Done, naming exactly this gap.

Decisions:
- Extracted `memoize_by_text` as a free function taking a generic
  closure specifically so it could be unit-tested without any llama.cpp
  involvement — testing memoization correctness by counting real
  inference calls would have been both slow and a weaker proof (timing
  variance vs. an exact count).
- Kept the throughput test separate from the construction-probe test
  (both `#[ignore]`d, same reason) rather than merging them, so a future
  session running just the construction probe isn't forced to also pay
  for the throughput run's extra embedding calls.

Landmines: none new. The usual "never remove `#[ignore]` from a
real-model test" caution applies to `throughput_benchmark_and_memoization_win`
too.

Produced by: Anurup + Claude Code
