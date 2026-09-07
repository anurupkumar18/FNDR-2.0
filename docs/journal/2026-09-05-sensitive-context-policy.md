## Handoff: T-802 sensitive-context detection as data (2026-09-05)

Done: Converted `fndr-privacy`'s built-in sensitive-context lists (password
manager names, financial domains, medical domain markers, auth indicators,
secret patterns) from compiled-in `const` arrays into an owner-constructible
`SensitiveContextPolicy` (`crates/fndr-privacy/src/safety_gate.rs`). Added
`evaluate_with_policy` and `redact_secret_lines_with_policy` alongside the
existing `evaluate`/`redact_secret_lines`, which now delegate to a shared
generic `evaluate_core`/`redact_secret_lines_matching` so there is exactly
one matching implementation, not two copies to drift. `evaluate`/
`redact_secret_lines` still run directly against the original `const`
arrays (zero allocation, zero behavior change) — `SensitiveContextPolicy`
is strictly additive. `SensitiveContextPolicy::default()` normalizes the
same lists the same way (domains through `Blocklist`'s own
`normalize_domain`, so a custom domain list gets the same suffix-spoof
protection T-801 established) and is proven behavior-identical to the
built-in path by a new parity test running both through every sensitive
case. A second new test proves a custom policy *replaces* the built-in
lists rather than unioning with them (mirrors `Blocklist`'s contract).

Ran and confirmed:
- `cargo fmt --all --check` — clean
- `cargo test -p fndr-privacy` — 18/18 pass (16 original + 2 new: default-parity, override-not-union)
- `cargo test -p fndr-memory -p fndr-mcp` — unaffected, all pass (proves the
  default `evaluate()` path is unchanged for existing callers)
- `cargo clippy -p fndr-privacy -p fndr-memory -p fndr-mcp --all-targets -- -D warnings` — clean
  (one type-complexity lint hit in a new test's tuple array annotation, fixed by
  dropping the redundant explicit type and letting inference handle it)
- `git diff --check` — clean
- `make test` (full sweep) — all green

Updated `docs/ROADMAP-TICKETS.md`'s progress ledger with a `Partial`
T-802 row (not `Done` — see below) and left `docs/CONTEXT.md` alone since
no code path summarized there is affected yet (no caller uses the new
policy API outside tests).

In flight / explicitly not done: T-802's full acceptance criteria also
call for "alert queue with dismissal keys," which needs a delivery/UI
surface (push events per ADR's no-polling rule, a dismissal-key persistence
scheme) that doesn't exist yet — genuinely cross-lane (backend event +
frontend) and out of scope for this slice. Also not done: any real caller
that loads a `SensitiveContextPolicy` from `settings` or a config file —
`fndr-privacy` deliberately does no I/O itself (matches its ADR-004 "no
I/O" posture and `Blocklist`'s existing precedent), so a future slice in
whichever crate owns `settings` (`fndr-store` per the crate table) does the
loading and hands parsed values here. This journal entry and the ledger
both call the ticket "Partial," not "Done," so this isn't miscounted later.

Decisions:
- Kept `evaluate()`/`redact_secret_lines()` on the exact original code path
  (same consts, no allocation) rather than routing them through
  `SensitiveContextPolicy::default()`, to guarantee zero performance or
  behavior risk to every existing caller — verified via the parity test
  instead of by inspection alone.
- Made `SensitiveContextPolicy::new` fully replace rather than merge with
  the built-in lists, matching `Blocklist::new`'s existing contract in the
  same crate, rather than inventing a different merge semantics.
- Did not add a `toml`/config-file dependency to `fndr-privacy` to "load"
  the policy — that would be a new dependency needing its own cargo-deny
  and CI-budget review, and would put file I/O in a crate that currently
  has none. Left loading as the future caller's responsibility.

Landmines: none new beyond what's already noted in
`2026-09-05-real-store-safety-seam.md` (lancedb's first-compile cost,
fndr repo cwd reset).

Produced by: Anurup + Claude Code
