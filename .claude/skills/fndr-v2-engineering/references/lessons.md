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
