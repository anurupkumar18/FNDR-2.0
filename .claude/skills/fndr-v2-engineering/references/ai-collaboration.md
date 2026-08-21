# Working with AI agents: consistency, token discipline, hygiene, handoffs

v1 was built by one person driving many agents across many tools, and the audit shows exactly how that decays a codebase: three graph schemas, two retrieval stacks, duplicated scoring functions, dead modules with passing tests, and 100-plus-field structs constructed by hand in four places. None of that was one bad decision; it was hundreds of sessions that each started from scratch. These practices exist so v2's many sessions compound instead of colliding.

## One conventions source, every tool

- This skill is the source of truth. `AGENTS.md` at the repo root is generated from it (T-107) so Cursor, Codex, Antigravity, and anything else read the same rules. Never hand-edit AGENTS.md; the CI drift check will catch it.
- A session in any tool starts the same way: read the routed workflow, the lane checklist, and the touching code before writing. If a tool cannot load this skill, paste the relevant workflow section into its context.

## Navigate before you write

The single highest-leverage rule. Before creating any function, type, module, or component:

1. Search the workspace for an existing implementation or near-miss (`rg` the concept, check the owning crate per the lane map, check the component library for UI).
2. If something close exists, extend or generalize it in place; do not write a parallel version "to be safe." Parallel versions are how v1 got two rerankers that fought each other.
3. If nothing exists, check ADR-005's PORT list before writing from scratch; the tuned v1 version may already be earmarked.
4. In the PR, one sentence: what you searched and why new code was justified. This is cheap for the author and gold for the reviewer.

## Token and context discipline

- Read scoped, not whole: open the specific files and line ranges the task touches; use search to find them rather than paging through modules. Cap exploratory command output; interrupt runaway searches.
- Do not re-derive established facts: the ADRs, ARCHITECTURE.md, and this skill are the compressed context; load them instead of re-reading the codebase to rediscover decisions.
- Long sessions: when context grows stale or bloated, write the handoff note (below) and start fresh rather than pushing a degraded session to keep going. A fresh session with a good handoff outperforms a long session with a polluted context.
- Expensive model calls (bench runs, VLM experiments) get planned, not retried in a loop: state the hypothesis, run once, record the result.

## Dead-code and half-feature hygiene

- Ship whole or not at all: a feature that cannot work yet does not merge enabled, and its scaffolding does not merge at all without the ticket that finishes it. v1's stubbed graph commands and disabled chunk retrieval are the cautionary tales.
- Every merge that replaces code deletes what it replaces in the same PR. "Old path kept just in case" requires a removal ticket with a date.
- Monthly dead-code sweep (rotates across lanes): unused exports, unreferenced constants, tests asserting nothing, `#[allow(dead_code)]`. The sweep is a small PR series, not a big-bang refactor.
- Tests that exist only to make dead code look alive are worse than no tests; delete them with their subject.

## Session handoff format

Written at the end of any working session that leaves work in flight, and whenever switching agent, tool, branch, or person. Lives in the PR description, the ticket comment, or `docs/journal/` for unticketed work, whichever is closest to where the next session will look.

```
## Handoff: <branch or ticket> (<date>)
Done: <what actually works now, with evidence: tests run, bench delta>
In flight: <what is started but not done, and the exact next step>
Decisions: <choices made this session and why, one line each>
Landmines: <anything surprising the next session must know>
Produced by: <person + agent/tool, e.g. "Anurup + Claude Code">
```

Five lines is fine; the discipline is that it exists. The `Produced by` line is what makes "who did what, where" answerable across people and agents without archaeology, and it is the seed of the team work-memory feature (PRD P2).

## Explainability of your own work

The project's interview-and-demo story depends on being able to narrate what happened in a session without a scavenger hunt. Two habits feed it: handoff notes (above) capture the what-and-why at the time it is cheap, and PR descriptions state outcome, evidence, and decisions rather than restating the diff. When Session Story (T-709) ships, dogfood it: ask FNDR to reconstruct your own session and file gaps as context-quality bugs (T-512 rubric).
