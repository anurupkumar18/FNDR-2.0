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

There is no current candidate reader over the real store and no atomic
persisted merge operation, so the policy is not yet invoked by capture.
`StoreCaptureSink` continues to receive an explicit session ID from its
lifecycle owner. This avoids inventing timezone, transaction, or vector-query
semantics before their real engine boundaries exist.

## Landmines

- Candidate retrieval must remain on the one retrieval stack; do not build a
  second continuity-only store or raw-SQL search path.
- A merge is a durable write concern: when wired, it must preserve deletion,
  FTS/Lance consistency, and all record provenance atomically.
- Similarity is input to this policy, not an excuse to load a model in the
  capture thread. Keep all model work behind `ModelWorkerHandle`.

## Verification

`CARGO_BUILD_JOBS=1 cargo test -p fndr-memory`, followed by the serial
workspace `make test` gate.
