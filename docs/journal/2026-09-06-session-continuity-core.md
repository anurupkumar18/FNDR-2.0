# 2026-09-06: session continuity core

## Decision

T-307 begins with the reusable, deterministic v1 policy rather than copying
the old capture-loop and store monolith. `fndr-memory::continuity` accepts
safe capture facts plus an already-computed vector similarity; it has no model,
SQLite, or screen dependency.

## What is verified

- The 30-minute local session-ID format and context key are ported with the
  caller supplying local date/minute, keeping platform timezone conversion out
  of the engine.
- URL anchors use host plus at most three path segments and omit credentials,
  queries, and fragments.
- Candidate score feature weights, thresholds, eligibility rule, and the
  45-minute cross-app policy are the reference values. Matching URLs may cross
  apps only inside the window; different domains do not.
- Story text merge is bounded, deterministic, and overlap-aware; it performs
  no synthesis or model call.

## Explicitly not done

The real write seam now uses the policy only against recent **unflushed**
SQLite candidates and atomically updates the original record/chunk, so FTS and
the eventual Lance flush observe exactly one row. Indexed records are still
not candidates: mutating one without a Lance-safe replacement protocol could
leave a stale vector. `StoreCaptureSink` continues to receive an explicit
session ID from its lifecycle owner, avoiding invented timezone semantics.

## Landmines

- Candidate retrieval must remain on the one retrieval stack. The narrow
  unflushed queue read is a write-path invariant, not a second search surface.
- Do not extend merges to indexed records until Lance replacement/deletion and
  SQLite mutation can be verified as one recoverable operation.
- Similarity is input to this policy, not an excuse to load a model in the
  capture thread. Keep all model work behind `ModelWorkerHandle`.

## Verification

`CARGO_BUILD_JOBS=1 cargo test -p fndr-memory`, followed by the serial
workspace `make test` gate.
