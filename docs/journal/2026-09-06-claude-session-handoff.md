# 2026-09-06: Claude Code session handoff

## Starting point and final state

- **Branch:** `codex/a006-real-store-safety-seam`
- **Started after:** `9e34e1d` (Codex's own handoff journal)
- **Canonical checkout:** `/Users/anurupkumar/FNDR-2.0`
- **User-owned file left untouched:**
  `docs/journal/2026-09-05-claude-code-handoff-prompt.md` (still untracked)

Every slice below was committed separately, with `CARGO_BUILD_JOBS=1 make
test` green before each commit and each pushed.

## What landed

**T-308 closed.** `capture_worker.rs` now consumes `SamplingPolicy` and
`MacOSInputIdle` instead of a fixed cadence: `CaptureNow` ticks and
re-checks shutdown with `try_recv`, `Wait(d)` blocks `recv_timeout(d)`,
`DeepIdle` blocks on `idle_interval`. No polling thread, no model load, the
2-second active floor still enforced before the scheduler opens.

**A pre-existing test flake, fixed.** Two `capture_scheduler` tests built
their "unique" Lance directory from pid plus a nanosecond timestamp. They
run on separate threads in one process and can read the same clock value,
colliding on the same table path. Now paired with an `AtomicU64`. Verified
with ten consecutive suite runs.

**T-702 from 2 tools to 12.** `fndr.search` moved off the walking-skeleton
`SkeletonStore` onto the durable `Store` via `KeywordRetriever`. Then, in
order: `remember_decision`, `source_evidence`, `timeline`, `recall`,
`open_target`, `delta`, `active_focus`, `context_pack`, `feedback`,
`explain_retrieval`. Each has its own journal entry with the reasoning.

**The audit log ADR-007 asked for.** Migration 0005's `mcp_audit` records
tool, outcome, and whether raw capture text was released — and nothing
else. Auditing is structural: every `#[tool]` method is a thin wrapper over
a private `_inner`, and a test asserts the audited tool set equals
`registered_tool_names()`, so a new tool cannot ship unaudited. That test
has already caught two additions.

**T-310 got an instrument.** `cargo run -p fndr-shell --example
capture_soak` is the first thing in the repo to own the real capture
worker: bounded minutes, per-outcome tick counts, shutdown drain, RSS
sampled every 15s for the AC's leak trend, non-zero exit when zero ticks
occurred. It was not run — it captures the operator's screen and this
session was unattended — so it ships verified as code and explicitly
unverified as a soak.

**Two new docs of record.** `docs/mcp.md` (per-tool contracts, required by
ADR-007's tool-addition rule, previously referenced but nonexistent) and an
ADR-007 amendment recording implementation status and open gaps.

## The through-line

Nearly every design decision this session was the same one: make the
system's limits visible rather than letting a caller infer competence from
silence. `recall` refuses unbacked kinds instead of returning `[]`.
`timeline` marks a `truncated` result. `delta`'s total counts every app
even when the app list is capped. `active_focus` reports `stale` rather
than a bare app name. `context_pack` reports `retrieval_route: keyword`,
`estimated` tokens, and `dropped_for_budget`. `feedback` says
`ranking_changed: false`. `explain_retrieval` states that privacy exclusion
happens at capture and therefore cannot appear as a retrieval drop.

Each of those is one field or one refusal standing between an agent and a
confident false statement about someone's own memory.

## Honest current boundaries

- **The capture worker still has no desktop owner.** T-901 is unstarted.
  The soak example calls `start_real_capture_worker`, but it is a CLI a
  person runs deliberately, not a lifecycle. The product still captures
  nothing in normal use.
- **Retrieval is keyword-only.** No vector, hybrid, RRF, or reranking.
  `context_pack` and `search` are honest about this in their responses.
- **Two tools blocked on data models**, not effort: `project_context` (no
  project entity) and `graph_context` (nothing writes the graph tables).
- **Audit and feedback tables have no retention and no UI.** `mcp_audit`
  grows unbounded; T-902's "one-click audit log" now has something real to
  read but nothing renders it.
- **No per-tool rate limits.** One global window, as before.
- **T-310's soak has not been run**, only made runnable. The multi-day
  run, the RSS trend call, the SCStream-versus-periodic decision, the
  fallback note, and the macOS 26.1 TCC quirk all still need a human.

## Landmines

- Preserve the untracked handoff prompt.
- `CARGO_BUILD_JOBS=1`, and check `df -h .` before a full gate; `target/`
  was ~30 GiB with ~44 GiB free at session end.
- Never pipe `make test` through `tail`: the exit code becomes `tail`'s.
  This cost a re-run and is now a `lessons.md` entry.
- The audit wrappers are load-bearing. A new tool needs its wrapper and a
  call in `every_registered_tool_writes_an_audit_entry`, or that test
  fails — which is the intended behavior, not an obstacle to route around.
- `result_feedback`'s FK is `SET NULL` on purpose so deletion cannot
  rewrite eval history. Do not "fix" it to `CASCADE` for consistency with
  its neighbors.

## Recommended next slice

Either T-901's desktop bootstrap, which is the difference between a
well-tested engine and a product that captures anything, or the first
vector route behind ADR-006's bench gate. The MCP surface is now far ahead
of both, which is itself a signal about what to do next.
