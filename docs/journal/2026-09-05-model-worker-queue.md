## Handoff: T-403 model-worker priority queue (2026-09-05)

Done: `ModelWorkerHandle`/`Priority` (`crates/fndr-inference/src/model_worker.rs`),
a dedicated worker thread that every llama.cpp call is meant to go through,
in priority order (`Interactive > Synthesis > Review > Backfill`, matching
ARCHITECTURE section 2's stated order), with lazy load and idle-timeout
unload. Generic over a `loader: Fn() -> Result<Box<dyn Embedder>, EmbedError>`
closure rather than hardcoding `GgufEmbedder`, so tests inject a
deterministic fake instead of paying for a real model load per test run
(invariant 4: test embedders live in test code).

Mechanics: a `Mutex<BinaryHeap<Job>>` + `Condvar`, where `Job`'s `Ord`
sorts by priority first, then by submission sequence number (earlier
submitted wins ties) so same-priority jobs stay FIFO. The worker thread
blocks on the condvar with the idle timeout as the wait bound; on a
timeout with an empty queue and a loaded model, it drops the model
(unload) and keeps waiting. `ModelWorkerHandle::submit_embed_documents`/
`submit_embed_query` block the calling thread on an mpsc response channel
— synchronous API, matching `Embedder`'s own synchronous trait methods.

**AC: "contention test proves priority."** `higher_priority_job_is_processed_before_lower_priority_jobs_queued_earlier`
submits a Backfill job that blocks (via a gate channel) until two more
jobs — a second Backfill and an Interactive — are both queued behind it,
then releases it and asserts the actual processing order is
`[r0, r2, r1]`: the Interactive job runs before the Backfill job that was
queued earlier. This is a real ordering assertion (each job's identity
is recorded from its own payload), not just a timing/count proxy. Ran it
5 times back to back with no flakes (timing margins are generous: 150ms
initial settle, 30ms between subsequent submissions, vs near-instant
fake-embedder calls).

Two more tests: `idle_timeout_unloads_and_next_job_reloads` (a fake
loader's call count goes 1 → 2 across an idle window) and
`loader_error_is_returned_without_poisoning_future_jobs` (a failed load
returns the error to that caller without breaking the worker for the
next submission).

**AC: "no LLM call outside the queue (lint or review rule)."** Since
`GgufEmbedder`'s fields are private, `GgufEmbedder::load(...)` is the
only way to obtain a real instance — so that's the one call site that
must live only inside a `ModelWorkerHandle` loader closure (or
`fndr-inference`'s own tests/examples proving it works in isolation).
Added `scripts/check-llm-call-sites.sh`, which greps for
`GgufEmbedder::load(` outside `crates/fndr-inference/src/`, `tests/`,
and `examples/`, and fails with a clear message if found. Sanity-checked
it: dropped a real violation into a temp file, confirmed it fires and
exits 1, then removed the temp file (confirmed clean afterward).

**Deliberately NOT done:** wiring this script into
`scripts/workspace-lints.sh` / `.github/workflows/ci.yml`'s `guards` job.
That script gates every future PR for the whole team; changing it
carelessly risks false positives blocking unrelated work. It's a
standalone script for now — run manually before a PR that adds a new
`GgufEmbedder` call site — and the owner should decide separately
whether and how to wire it into CI.

Ran and confirmed:
- `cargo fmt --all --check` — clean
- `cargo test -p fndr-inference --lib` — 10 passed, 1 correctly ignored
  (the real-model construction probe, unaffected by this change)
- Ran the contention test 5x back to back — no flakes
- `cargo clippy -p fndr-inference --all-targets -- -D warnings` — clean
- `cargo clippy --workspace --all-targets -- -D warnings` — clean (fast,
  no new external dependency, cache mostly reused)
- `./scripts/check-llm-call-sites.sh` — passes on the current tree;
  sanity-checked it also correctly fails on an injected violation
- `git diff --check` — clean
- `make test` (full sweep) — all green

In flight / explicitly not done: nothing yet actually calls
`ModelWorkerHandle` in production — `LanceWriter::flush_once` still
takes a generic `&dyn Embedder` directly (pre-dating this session), and
today's `end_to_end_flush` example still constructs `GgufEmbedder`
directly rather than going through the queue (it's exempted by the
check script as an example, but wiring the real production path is a
separate future slice: either `LanceWriter` gains a
`flush_once_via_worker` variant, or the eventual capture-pipeline
scheduler (T-306) is the thing that owns a `ModelWorkerHandle` and calls
through it). T-404 (embedding batch path) and T-405/T-406 (VLM
synthesis, reranker) are the other declared consumers of this queue,
none started.

Decisions:
- Generic `loader` closure over `Box<dyn Embedder>` rather than hardcoding
  `GgufEmbedder` in the worker itself, so the same queue type will later
  serve any future `Embedder` implementation (or, with a different job
  enum, other inference types like the VLM synthesis path) without a
  rewrite.
- Synchronous, blocking `submit_*` API (not `async`) to match `Embedder`'s
  own synchronous trait methods and avoid introducing an async runtime
  requirement into `fndr-inference` itself; callers that want
  non-blocking behavior can call `submit_*` from their own spawned
  thread or async executor's blocking-task pool.
- Left the "no LLM call outside the queue" enforcement as a standalone
  script rather than a CI gate — a deliberate, conservative choice given
  the blast radius of `workspace-lints.sh`, explained above.

Landmines:
- The idle-unload path checks `queue.is_empty() && model.is_some()` only
  on a condvar timeout; a rapid-fire barrage of jobs arriving exactly at
  the timeout boundary could theoretically delay unload by one more
  timeout cycle. Not a correctness bug (the model just stays loaded
  slightly longer than the nominal timeout in that edge case), but worth
  knowing if idle-RAM behavior is ever measured precisely.
- `scripts/check-llm-call-sites.sh` is not wired into CI. If a future
  session or PR adds a real production call site for
  `GgufEmbedder::load`, this script will not catch it automatically —
  it must be run manually, or someone must decide to wire it in.

Produced by: Anurup + Claude Code
