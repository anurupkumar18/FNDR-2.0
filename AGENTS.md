<!-- GENERATED FILE, DO NOT EDIT. -->
<!-- Source: .claude/skills/fndr-v2-engineering/  Regenerate: scripts/gen-agents-md.sh -->
<!-- References below are inlined; a pointer to references/<name>.md resolves to the matching section. -->

# FNDR v2 engineering

FNDR v2 is a local-first memory engine whose credibility rests on five enforced invariants and a strict layer split. This skill exists because the v1 POC failed in specific, audited ways (silent degradation, unmeasured ranking, unauthenticated surfaces, monster modules, dead plumbing), and every rule here traces to one of those failures. Read the rule's "why" before deciding it does not apply to your task.

Authoritative context, in reading order for anything non-trivial:
1. `docs/ARCHITECTURE.md` (crate map, data flow, lane boundaries)
2. `docs/decisions/` ADR-001..007 (the choices; do not re-litigate silently)
3. `docs/PRD.md` (scope, P0s, the month-3 demo gate)

## Step 1: classify the task, load the matching workflow

| Task looks like | Workflow (read `references/workflows.md` section) |
|---|---|
| New feature or change in engine/UI | Feature slice |
| Carrying anything from the v1 POC | Port from v1 |
| Retrieval, ranking, embedding, chunking, fusion, reranker | Eval-gated ranking change |
| Capture, privacy, blocklist, redaction, MCP/companion, tokens, any network listener | Privacy and surface change |
| Bug or regression | Diagnose before edit |
| New UI surface or component | Feature slice, plus the design gate in `references/lanes.md` (frontend) |
| Ending a session, switching agent/tool/branch/person | Session handoff |
| Monthly or milestone checkpoint on the plan itself | Founder review |
| Tag, release, updater | Release gate |

If the task spans lanes (ml/backend/frontend/platform), check `references/lanes.md` for the contract at the boundary you are crossing; cross-lane changes go contract-first (types and schema before implementation).

## Step 2: the five invariants (checked on every change)

Full checklists and rationale in `references/invariants.md`. The short form:

1. **Local-only.** No HTTP client outside `fndr-downloader`/`fndr-updater`; no new egress ever. CI enforces; do not fight the lint, amend ADR-004 first if you genuinely need network.
2. **Auth-always.** Any listener authenticates from its first commit; no "add auth later". The v1 audit found the memory store readable by any web page because auth was a follow-up.
3. **Eval-gated ranking.** No ranking constant, weight, stage, or model change merges without `make bench` numbers against the committed baseline. Mock embedders never satisfy the gate. v1 tuned ~30 constants against nothing; that is the anti-pattern.
4. **No silent degradation.** Missing model, failed embed, gated capture, unavailable sidecar: all are visible typed states, never zero-vectors, mocks, or quiet skips.
5. **Port provenance.** Code from v1 arrives as a targeted function/constant/prompt with tests and a `// Ported from FNDR v1 <path>` note. Wholesale copies are banned (ADR-005 lists what is portable).

## Step 3: build discipline

- **One vertical slice at a time.** Behavior change plus its tests plus its docs in one reviewable unit; no drive-by refactors.
- **Anti-bloat gate before adding code:** can deleting, reusing, tightening an interface, or renaming solve it instead? The v1 repo grew three graph schemas and two retrieval stacks by skipping this question.
- **Size rules:** file over ~600 lines or hot-path function over ~100 lines needs a recorded reason in the PR. Pipeline stages get seams (testable without the loop that drives them).
- **Types at boundaries:** IPC types are generated (specta), never hand-mirrored; lifecycle states are enums with persisted discriminants, never strings.
- **Config, not literals:** tunables live in named config structs; capture gates live in the declarative policy table with replay coverage.
- **Docs move together:** a change that touches the PRD or tickets amends any ADR it touches in the same PR; counts and contracts live in exactly one document of record and are referenced, not repeated, elsewhere. (The 08-20 review traced 40+ consistency defects to skipping this.)

## Step 4: verify before claiming done

Run the cheapest relevant checks and state what you ran:

- Rust: `cargo test -p <crate>` for the touched crates, full `cargo test` before PR.
- UI: `npm run typecheck && npm test` (scoped test file is fine for small edits, say so).
- Ranking-touching: `make bench` and paste the delta table into the PR.
- Privacy/surface-touching: the adversarial suite for the class you touched, plus the named regression tests (open-MCP-loopback, LAN-pairing-mint).
- Full sweep: `make test` from repo root.

A PR states: what changed, tests run with results, bench delta if applicable, and any invariant it brushed against. Unexpectedly empty output from a verification command is a failure to investigate, not a pass.

## Lane quick reference

- **ml:** `fndr-inference`, `fndr-memory`, `fndr-retrieval` (with backend), `fndr-bench`. You own the numbers; nothing you ship is done until the bench says so.
- **backend:** `fndr-types`, `fndr-textsignal`, `fndr-ocr`, `fndr-store`, `fndr-capture`, `fndr-graph`, `fndr-mcp`, `fndr-privacy` (the ownership map of record is `references/lanes.md`). You own the contracts everyone else consumes; breaking one is a cross-lane event, announce it.
- **frontend:** `ui/`, tokens/theming, the component library. You consume generated types and the engine API only; if you need data the API lacks, request the API change, do not tunnel around it. Every new surface builds from the component library and is reviewed against the design language spec; hand-rolled primitives and off-spec styling are how v1 became visually incoherent.
- **platform:** shell, sidecar, downloader/updater, release, CI. You own the machines' trust: signing, permissions, TCC re-grant flow, and the gates everyone else relies on.

Details, per-lane review checklists, and the sidecar/MCP/IPC contract rules: `references/lanes.md`.

## Working with AI agents (all tools, all lanes)

v1's codebase decayed because different agents (Cursor, Antigravity, Claude Code, Codex) each wrote from scratch instead of navigating what existed. The rules that prevent that, plus token discipline, dead-code hygiene, and the session-handoff format, live in `references/ai-collaboration.md`. Read it when starting AI-assisted work in this repo, when a session is about to end, or when you notice yourself (or another agent) about to write something the codebase may already contain. AGENTS.md in the repo root mirrors these conventions for non-Claude tools and is generated from this skill; never edit it by hand.

---

<!-- Inlined from .claude/skills/fndr-v2-engineering/references/invariants.md -->

# FNDR v2 invariants: checklists and rationale

Each invariant exists because the v1 audit found the opposite shipped. The v1 failure is named so reviewers can recognize the pattern returning.

## 1. Local-only (ADR-004)

v1 failure mode: local-only by convention, with privacy features documented but never wired.

Before merging, confirm:
- [ ] No new dependency on reqwest/hyper-client/ureq/curl outside `fndr-downloader` and `fndr-updater` (CI lint enforces; if it fires, the answer is a design change, not an exemption).
- [ ] No new URL construction outside the reviewed egress-constants module (its uniqueness test must still pass).
- [ ] Nothing derived from captured data (pixels, OCR text, embeddings, records, transcripts, graph) crosses a process or network boundary except the authenticated MCP/companion surfaces.
- [ ] No telemetry, crash reporting, or analytics dependency, ever.

## 2. Auth-always surfaces (ADR-007)

v1 failure mode: default-local MCP served the whole memory store to any web origin (auth off, CORS `Any`); the companion API let any LAN host mint a full-permission token via unauthenticated pair-start.

Before merging any listener or route change, confirm:
- [ ] Bearer auth required in every mode; comparison is constant-time.
- [ ] Origin and Host validated against an explicit allowlist; no wildcard CORS.
- [ ] Non-loopback binds require TLS; loopback may omit it (ADR-007).
- [ ] New route carries a permission scope and a rate limit; auth-failure test exists.
- [ ] Tokens and discovery files written owner-only; no secret ever logged.
- [ ] The two named regression tests still pass: `mcp_rejects_unauthenticated_loopback`, `companion_pair_start_not_network_reachable`.

## 3. Eval-gated ranking (ADR-006)

v1 failure mode: ~30 multiplicative rerank constants and multiple per-intent fusion-weight sets never measured against a real model; relevance evals ran a mock embedder; the flagship chunk path shipped disabled.

Before merging anything that can change result ordering, confirm:
- [ ] `make bench` run on real models; the delta table (Recall@5, MRR@10, latency) is in the PR description.
- [ ] Regressions are either justified in writing or the change is reworked; CI blocks silent regressions.
- [ ] New heuristics are named additive features with per-result attribution in `FusionSignals`, never anonymous multipliers.
- [ ] A new route or stage that cannot yet return real results does not ship enabled (no decorative plumbing).

## 4. No silent degradation (v1 ADR-012 carried forward)

v1 failure mode: missing embedder wrote zero-vector rows for weeks; mock embedder leaked into production paths and every eval; two LLM calls ran outside the model lock; `Embedder::new()` was constructed per query and per worker tick.

Before merging, confirm:
- [ ] Unavailability of a model, sidecar, or dependency surfaces as a typed state with a user-visible reason and, where applicable, a skip counter.
- [ ] No mock or fallback implementation is reachable in a production path.
- [ ] All llama.cpp work goes through the model-worker queue with a priority; no direct session use.
- [ ] Expensive resources (sessions, embedders) are constructed once and shared, never per-call.

## 5. Port provenance (ADR-005)

v1 failure mode: not v1's failure but this rebuild's biggest risk: wholesale copying would inherit the audited structural liabilities (monster modules, dual stacks, dead schemas).

Before merging ported code, confirm:
- [ ] The port is on the ADR-005 PORT list, or the PR argues for moving it there with audit/eval justification.
- [ ] It arrives as a targeted function, constant set, prompt, schema, or contract, with tests (v1's tests ported or new ones).
- [ ] It carries `// Ported from FNDR v1 <path>`; prompts are byte-identical unless a change is called out.
- [ ] Nothing on the DISCARD list is copied; consult it on the `reference/v1` branch only to understand history.

## Cross-cutting: the demo-gate priority rule

When any of these invariants conflicts with a deadline, the invariant wins and the scope moves. The PRD's pre-agreed cut lines exist precisely so schedule pressure never argues against an invariant. The spine (capture to retrieval to MCP) is never cut.

---

<!-- Inlined from .claude/skills/fndr-v2-engineering/references/lanes.md -->

# Lanes, boundaries, and contracts

Four lanes own disjoint crates; contracts are the only crossing points. When a task spans lanes, change the contract first (types, schema, tool definition), get the owning lane's review, then implement both sides.

## Crate ownership map

| Lane | Owns | Consumes |
|---|---|---|
| ml | `fndr-inference`, `fndr-memory`, `fndr-retrieval` (co-owned with backend), `fndr-bench` | store API, types |
| backend | `fndr-types`, `fndr-textsignal`, `fndr-capture`, `fndr-ocr`, `fndr-privacy`, `fndr-store`, `fndr-graph`, `fndr-mcp` | inference API (model queue), types |
| frontend | `ui/` | generated IPC types, engine API via shell commands, tokens |
| platform | `fndr-shell`, `apps/helper` (Swift), `fndr-companion` (contract), `fndr-downloader`, `fndr-updater`, CI, release | everything, at arm's length |

## The contracts

### Engine API (backend owns)
The Rust functions `fndr-shell` and `fndr-mcp` call. Typed errors; no stringly results. Rule: the UI and MCP must be servable by the same function; surface-specific shaping happens in composition, not scoring or storage.

### Generated IPC types (backend produces, frontend consumes)
specta/tauri-specta generation runs at build time. Hand-written TS mirrors are banned by lint. If the frontend needs a field, the change starts in `fndr-types`.

### Events (backend to frontend)
Push-only for always-on state: emit on change with fingerprint suppression. Adding a poller for always-on state is a review-blocking defect (v1 ADR-011's half-finished migration is the cautionary tale). Rarely-running progress states may poll while visible.

### MCP contract (backend owns, ml feeds quality)
The canonical tool set lives in ADR-007's table, the single inventory of record (14 founding tools plus ratified P1 additions). Adding a tool needs: a use case no existing tool covers, an ADR-007 amendment, a schema round-trip test, an auth-failure test, a rate limit, and a `docs/mcp.md` entry. Removing or renaming bumps the manifest version.

### Sidecar protocol (platform owns)
Versioned JSON over stdio to `apps/helper`. Engine treats the sidecar as optional: absence is a typed unavailable state (meetings degrade visibly, nothing else is affected). Protocol changes need fixture-replay tests on both sides.

### Bench interface (ml owns)
`make bench` on the corpus dir produces the metrics file. Baselines are committed; CI compares. Anyone may run it; only ml changes what it measures, and such changes are announced (they re-baseline everyone).

## Per-lane review checklists

**ml reviews ask:** is the change measured (bench delta present)? Does every heuristic carry attribution? Is model residency budgeted (load/unload, priority)? Is the prompt ported byte-identical or the change called out? Is anything trained/tuned on the eval set (leakage)?

**backend reviews ask:** does SQLite stay the only truth (Lance written solely via the flush writer)? Are deletes routed through deletion-everywhere? Are new columns in the migration with tests? Do errors stay typed end to end? Is the capture hot path free of blocking syscalls and direct LLM calls?

**frontend reviews ask:** generated types only? Tokens only (no raw hex; the lint should catch it)? Built from the component library, with no hand-rolled buttons, panels, or inputs, and reviewed against the design language spec (the anti-slop bar: consistent type scale and spacing, no gradient soup, no boxes-in-containers)? Push events, not polls? Does the component consume the engine API rather than duplicating derivation logic client-side? Are the v1-ported interaction contracts (omnibar keyboard model, lifecycle chips) preserved?

**platform reviews ask:** does the change alter the signing identity, entitlements, or permission surface (TCC re-grant risk)? Do CI gates still cover it? Is the sidecar lifecycle supervised (restart, health)? Does the release pipeline remain one-tag-push?

## Cross-lane change protocol

1. Open the contract change (types/schema/tool def) as its own commit or PR; tag the owning lane.
2. Land both sides behind the contract within the same milestone; a contract with only one side implemented does not ship enabled (no decorative plumbing).
3. If the contract change is breaking, the PR names every consumer and updates them in the same change or links the tickets that will.

---

<!-- Inlined from .claude/skills/fndr-v2-engineering/references/workflows.md -->

# Task workflows

Pick the section Step 1 of SKILL.md routed you to. Each workflow ends at the SKILL.md Step 4 verification bar.

## Feature slice

1. Read the touching code, its tests, and the relevant ADR before editing. If the feature contradicts an ADR, stop and raise it; do not implement around a decision.
2. Write the slice contract-first if it crosses a lane (see `lanes.md`).
3. Anti-bloat gate: prefer deleting, reusing, or tightening over adding. New module or layer needs a sentence of justification in the PR.
4. Tests land with the change at stable boundaries (pipeline stage seams, engine API, contract round-trips). A slice without tests is not done.
5. Wire observability if the slice adds a skip/failure path: typed state plus counter, never a silent branch.

## Port from v1

1. Confirm the item is on ADR-005's PORT list (or argue the move in the PR).
2. Fetch it from the `reference/v1` branch; bring its tests. If v1 had no tests for it, write the tests first from the v1 behavior you intend to keep.
3. Add the provenance note. Prompts move byte-identical; heuristic constants move exactly with their tuning comments (those comments encode the why).
4. Explicitly check the item's known-defect list in the audit (for example: entity extractor ports WITHOUT the fabricated conflict-edge pair; blocklist matching is rewritten, not ported).
5. Never port from the DISCARD list. If you find yourself reading the v1 capture loop, MCP mod.rs, or dual rerankers for anything but history, stop.

## Eval-gated ranking change

1. Before touching code, run `make bench` to capture the pre-change baseline on your machine (CI has the committed one; local delta is your fast loop).
2. Make the change as a named feature with attribution (`FusionSignals`), behind config where promotion is uncertain.
3. Run the bench; put the delta table in the PR. Improvements land; regressions need written justification or rework.
4. Ablations for new stages (reranker, graph route) follow the promotion rule: enabled by default only on a measured win; otherwise it stays behind a flag with the numbers recorded.
5. Never tune against the eval set itself (leakage); hard cases discovered in use become new labelled pairs first.

## Privacy and surface change

1. Re-read ADR-004 and the invariant checklists 1 and 2 before designing.
2. Threat-model the change in one paragraph in the PR: who can reach this surface, with what credentials, and what do they get.
3. Adversarial tests for the class you touched (secrets, password managers, banking/medical, private browsing, blocklist) and the named regression tests must pass.
4. Any new gate or redaction is a declarative policy entry with replay coverage, not an inline `continue` (v1's long chain of sequential inline gates once silently dropped every LLM-touched frame; the replay harness exists so tuning is visible).
5. Capture-path changes state their effect on the SkipReason counters (which reasons can now fire, which disappear).

## Diagnose before edit

1. Reproduce first; capture the repro as a failing test when feasible.
2. Narrow with evidence (logs, counters, bisect), not guesses. The observability contract (typed states, per-stage counters) exists to make this cheap; if it did not help, improving it is part of the fix.
3. Fix the root cause. Banned classes of "fix": `async: false`-style serialization of a racy test, widening a timeout without understanding the wait, catching-and-ignoring a typed error, re-enabling a mock path.
4. Add the regression test named after the failure.

## Session handoff

Runs whenever a working session ends with work in flight, or when switching agent, tool, branch, or person (the v1 failure this prevents: work scattered across tools with no record of who did what, where, or why).

1. Write the handoff note in the format from `ai-collaboration.md` (Done / In flight / Decisions / Landmines / Produced by), placed where the next session will look: PR description, ticket comment, or `docs/journal/`.
2. Push the branch; never leave the only copy of in-flight work in a local tree or an agent's context.
3. Update the board: ticket status reflects reality (in progress, blocked with reason, in review).
4. If the session discovered scope change, file the ticket now; do not carry it in memory.

## Founder review

Runs monthly and at each milestone boundary. The v1 failure this prevents: a plan that was never challenged, features that were never questioned, and no mechanism for the AI to think like a founder rather than a ticket-taker.

1. Re-read PRD goals, the current bench and rubric numbers, and the last month of handoff notes.
2. Answer in writing, one paragraph each: What is the weakest part of the product story right now? What did we learn that the plan does not reflect? What would we cut if we lost a month? What is one differentiator candidate worth a spike, and what is the cheapest test of it?
3. Challenge one standing decision on merits (an ADR, a phase boundary, a feature's existence). Either reaffirm it in a sentence or open the amendment PR.
4. Output: a short review note in `docs/journal/`, PRD/roadmap edits if warranted, and at most one new spike ticket. The review proposes; the month-3 gate and cut-line rules still decide.

## Release gate

1. `make test` green; `make bench` shows no unexplained regression; the adversarial and named-regression suites pass.
2. The clean-VM QA checklist runs on the release candidate (install, onboarding, permission grant and revoke, update from previous tag, degraded states).
3. Resource budgets spot-checked against the PRD targets (idle RSS, capture CPU); a miss is a release blocker or a written exception.
4. Tag; the pipeline does the rest (one tag push, no manual signing steps). Verify the updater applies vN to vN+1 on a real install.
5. Update the benchmark page numbers for the release.

---

<!-- Inlined from .claude/skills/fndr-v2-engineering/references/ai-collaboration.md -->

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
