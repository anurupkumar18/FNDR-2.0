# Lessons: the cross-session learning loop

Append-only. Every mistake or surprise that cost a working cycle becomes an
entry here, at the moment it is understood. Every session, in every tool
(Claude Code, Codex, Cursor, a teammate's editor), reads this file before
starting work: it ships inside the generated AGENTS.md, so the whole team
and every agent inherit each lesson automatically. Larger reversals also go
to `docs/incidents.md`; this file is for the working-level rules.

Entry format:

```
## <date> · <one-line title>
Cost: <what it burned: a red CI run, a debugging hour, a wrong design>
Root cause: <the actual mechanism, not the symptom>
Rule: <the behavior now followed instead>
```

---

## 2026-08-21 · Migration file written but never registered
Cost: three failing tests and a diagnosis pass.
Root cause: migrations are registered by a const array in
`crates/fndr-store/src/migrations.rs`; creating the SQL file does nothing by
itself. The runner's list is the source of truth.
Rule: after creating any registered-by-list artifact (migration, CI step,
binding, workspace member), grep for the registry and confirm membership
before running tests.

## 2026-08-21 · Edition 2024 makes unsafe-in-unsafe-fn a hard gate
Cost: a green `cargo test` followed by a red `make test` (clippy).
Root cause: code ported from edition 2021 relies on implicit unsafe blocks
inside unsafe fns; edition 2024 warns and `-D warnings` promotes it.
Rule: ports from v1 get explicit `unsafe {}` blocks at each operation during
the port, and `make test` (not bare `cargo test`) is the local gate.

## 2026-08-21 · Header names are lowercase on the wire (http crate)
Cost: a failing resume test blamed on the wrong component.
Root cause: ureq/reqwest normalize header names to lowercase per the http
crate; a hand-rolled test server matching "Range:" case-sensitively never
saw the header.
Rule: any hand-rolled HTTP parsing matches headers case-insensitively.

## 2026-08-21 · Lance default prune reclaims nothing for our write pattern
Cost: would have shipped a maintenance scheduler that never freed disk.
Root cause: prune keeps versions inside a retention window and refuses files
newer than 7 days unless `delete_unverified` is set; our versions are always
younger than that.
Rule: measured behavior beats documented behavior; spike the maintenance
path of any storage engine before designing its scheduler (T-208 pattern).

## 2026-08-21 · Release-candidate crates drift between rc versions
Cost: a compile failure on the first specta-typescript API use.
Root cause: the specta family is permanently rc and renames APIs between
rc releases; remembered API shapes are unreliable.
Rule: before coding against specta/tauri-specta/lancedb/rmcp, read the
pinned version's source in `~/.cargo/registry` (or fetch the crate), and pin
exactly (`=x.y.z-rc.n`).

## 2026-08-21 · Transitive dependencies can violate our own bans
Cost: a red cargo-deny lane after adding lancedb.
Root cause: lance core hard-embeds a catalog REST client (reqwest) that no
feature flag removes; tauri pulls reqwest for iOS/Android targets only.
Rule: after adding a heavy dependency, run `cargo deny check` locally and
trace hits with `cargo tree -i <crate>` before pushing; scope any exception
to the exact parent crate and amend ADR-004 in the same PR.

## 2026-08-21 · The guard hook reads the session cwd's branch
Cost: a blocked push and a confusing denial while working on a second repo.
Root cause: the personal block-main hook resolves the current branch from
the directory the session started in, not from the repo the git command
targets.
Rule: open sessions inside the repository being changed; never bypass the
hook, restructure the work instead.

## 2026-08-21 · The first CI run after a heavy dependency is the budget test
Cost: a 17m24s rust lane (budget: 15m) on the lance PR.
Root cause: rust-cache has no cache for a new dependency tree; the first
uncached run pays full compile.
Rule: when adding a heavy dependency, say so in the PR body, expect the
first run to bust the budget once, and verify the cached follow-up run
returns under it.

## 2026-09-06 · Clamp before casting, and prove the test can fail
Cost: a self-review catch, not a production bug — but only because the
review happened. Extracting a `50` literal into a named `SEARCH_LIMIT_CAP`
turned `limit.min(50) as i64` into `(limit as i64).min(CAP)`. A `usize`
above `i64::MAX` casts to `-1`, and SQLite reads `LIMIT -1` as *no limit*,
so the "safety" cap silently became unbounded.
Root cause: reordering a clamp and a cast looks like a formatting change and
is a semantic one. The first regression test written for it also passed
against the bug, because the fixture held fewer rows than the cap.
Rule: clamp in the target domain before casting (`limit.min(CAP as usize) as
i64`). And when a test exists to catch a specific regression, reintroduce
the bug once and watch it fail — a test whose fixture is too small to
distinguish the two behaviors is theater.

## 2026-09-06 · `make test | tail` reports the pipe's exit code, not make's
Cost: a full gate re-run, and a few minutes believing a green gate that had
not been verified.
Root cause: `make test 2>&1 | tail -150` exits with `tail`'s status, which is
0 whether or not `make` failed. The truncation also cut the failing crate's
output out of the saved log, so neither the exit code nor the text showed
the failure.
Rule: run the gate as `make test > /tmp/gate.log 2>&1; echo "EXIT=$?"` and
grep the full log, rather than piping it through `tail`/`head`. Beware the
mirror-image trap when checking: a trailing `grep -c FAILED` that finds
nothing exits 1 and makes a green run look failed. An exit code you did not
actually read is not a verification.

## 2026-09-06 · Nanosecond timestamps are not a per-thread unique ID
Cost: an intermittent `make test` failure (`Lance(TableAlreadyExists)`)
across two unrelated `capture_scheduler` tests, misdiagnosed at first as
caused by an unrelated same-session change to a different crate.
Root cause: a test helper built a "unique" Lance directory from
`process::id()` + `SystemTime::now()` nanos only. `cargo test` runs tests in
one process on separate threads; two threads can read the same clock value,
so two tests collided on the same Lance table path.
Rule: never rely on a raw timestamp alone for per-test-run uniqueness inside
one process; pair it with a process-wide `AtomicU64` counter (or a crate
like `tempfile` that guarantees this). A flaky failure that reproduces at a
different assertion/line on retry, in a file the current diff never touched,
is a signal to check test isolation before assuming the diff is at fault.
