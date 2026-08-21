# Contributing

The engineering conventions live in `.claude/skills/fndr-v2-engineering/` (loaded automatically by Claude Code; mirrored to `AGENTS.md` for other agent tools). This file is the short human-readable summary; the skill is the source of truth when they disagree.

## Branches and PRs

- Never commit to `main` directly. Work on short-lived branches, merge via PR.
- One repo concern per PR; keep diffs reviewable. A slice is behavior plus its tests plus its docs in one unit.
- The PR body states: what changed, tests run with results, the bench delta if the change touches ranking, and any invariant it brushed against.
- Commits are small, frequent, imperative, and concise. No generated boilerplate, no co-author trailers.

## Verification

`make test` from the repo root runs the full local gate (workspace lints, fmt, clippy, cargo test, tsc, vitest). Run the scoped versions while iterating (`cargo test -p <crate>`, `cd ui && npm test`), the full gate before requesting review. Unexpectedly empty output from a verification command is a failure to investigate, not a pass.

## Porting from v1

The v1 POC history is on the read-only `reference/v1` branch. Ports follow ADR-005: only items on the PORT list, arriving as targeted functions, constants, prompts, or contracts, with tests and a `// Ported from FNDR v1 <path>` provenance note. Prompts move byte-identical. Nothing on the DISCARD list is ever copied.

## Module size

No file over ~600 lines and no hot-path function over ~100 lines without a recorded reason in the PR. Every pipeline stage gets a seam so it is testable without the loop that drives it.

## Session handoffs

Any session that ends with work in flight, or switches agent, tool, branch, or person, writes a handoff note in the format from `.claude/skills/fndr-v2-engineering/references/ai-collaboration.md` (Done / In flight / Decisions / Landmines / Produced by). It lives wherever the next session will look: the PR description, the ticket, or `docs/journal/` for unticketed work.

## Incidents and reversals log

`docs/incidents.md` records anything that went wrong or got reversed: a bad decision undone, a regression shipped and caught, an invariant nearly violated, a tool loop that burned a day. One entry each, with root cause and lesson. This is the failure-narrative record that founder reviews and interview/demo stories run on; an empty log after a hard month means it is not being kept, not that nothing happened.

## Lessons loop

`.claude/skills/fndr-v2-engineering/references/lessons.md` is the working-level learning loop: whenever a mistake or surprise costs a cycle, append an entry (date, cost, root cause, the rule now followed) in the same PR. Every session reads it before starting, and it ships inside the generated AGENTS.md, so every tool and teammate inherits each lesson automatically. New features start from the `fndr-feature-dev` skill (planning and right-first-try preflight) before the build discipline takes over.

## Docs move together

A change that touches the PRD or the roadmap amends any ADR it touches in the same PR. Counts and contracts live in exactly one document of record (for example, ADR-007 owns the MCP tool inventory) and are referenced elsewhere, never repeated.

## House style

No em dashes anywhere: docs, comments, commit messages, UI copy. Use commas, colons, parentheses, or separate sentences.
