<!-- GENERATED FILE, DO NOT EDIT. -->
<!-- Sources: .claude/skills/fndr-v2-engineering/ and .claude/skills/fndr-feature-dev/  Regenerate: scripts/gen-agents-md.sh -->
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

- **Read `references/lessons.md` before starting work.** It is the
  append-only memory of every mistake that cost a cycle here, shared across
  all tools and people via the generated AGENTS.md. Assume at least one
  entry applies to your task.
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

Before closing the session: if any mistake or surprise cost a working cycle,
append it to `references/lessons.md` (format at the top of that file) in the
same PR. That single habit is what makes every future session, in every
tool, start smarter than this one did. Larger reversals also get an entry in
`docs/incidents.md`.

## Lane quick reference

- **ml:** `fndr-inference`, `fndr-memory`, `fndr-retrieval` (with backend), `fndr-bench`. You own the numbers; nothing you ship is done until the bench says so.
- **backend:** `fndr-types`, `fndr-textsignal`, `fndr-ocr`, `fndr-store`, `fndr-capture`, `fndr-graph`, `fndr-mcp`, `fndr-privacy` (the ownership map of record is `references/lanes.md`). You own the contracts everyone else consumes; breaking one is a cross-lane event, announce it.
- **frontend:** `ui/`, tokens/theming, the component library. You consume generated types and the engine API only; if you need data the API lacks, request the API change, do not tunnel around it. Every new surface builds from the component library and is reviewed against the design language spec; hand-rolled primitives and off-spec styling are how v1 became visually incoherent.
- **platform:** shell, sidecar, downloader/updater, release, CI. You own the machines' trust: signing, permissions, TCC re-grant flow, and the gates everyone else relies on.

Details, per-lane review checklists, and the sidecar/MCP/IPC contract rules: `references/lanes.md`.

## Working with AI agents (all tools, all lanes)

v1's codebase decayed because different agents (Cursor, Antigravity, Claude Code, Codex) each wrote from scratch instead of navigating what existed. The rules that prevent that, plus token discipline, dead-code hygiene, and the session-handoff format, live in `references/ai-collaboration.md`. Read it when starting AI-assisted work in this repo, when a session is about to end, or when you notice yourself (or another agent) about to write something the codebase may already contain. AGENTS.md in the repo root mirrors these conventions for non-Claude tools and is generated from this skill; never edit it by hand.

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

<!-- Inlined from .claude/skills/fndr-v2-engineering/references/lessons.md -->

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

<!-- Inlined from .claude/skills/fndr-feature-dev/SKILL.md -->


# FNDR feature development

The purpose of this skill is right-first-try: most wasted cycles in this
project come from starting to build before the feature is framed against the
plan of record, checked against the ADRs, and cut into verifiable slices.
The build discipline lives in `fndr-v2-engineering`; this skill covers
everything before the first line of code.

## Step 1: frame the feature

Answer in writing (a sentence each) before anything else:

1. What user problem does this solve, and for which PRD user (the builder,
   the agent, the evaluator)? Tie it to a PRD goal (G1 to G5) or pain-point
   row (PRD section 6). A feature that ties to neither is a P2 or a no.
2. Is it demo-relevant? Product capability only; staging tricks are banned
   (owner direction 2026-08-21).
3. What is the smallest version that delivers the value? Name the cut line.

## Step 2: check the plan of record

- Search `docs/ROADMAP-TICKETS.md` for an existing ticket before writing a
  new one; extend or re-scope rather than duplicate.
- Read the ADRs the feature touches. A conflict means: stop, raise it, amend
  the ADR deliberately or change the design. Never implement around a
  decision (fndr-v2-engineering invariant).
- Check `docs/specs/` for an owning brief (Codebase Memory work is governed
  by `docs/specs/codebase-memory-brief.md`, not by improvisation).

## Step 3: plan with the template

Fill `references/feature-planning.md`. Keep it under a page. The output is
either new/updated tickets in the roadmap (format per its Conventions
section) plus any PRD/ADR edits in the same change (docs move together), or
a written decision not to build, recorded where the idea came from.

## Step 4: right-first-try preflight

Run `references/right-first-try.md` before the first commit of the
implementation. It is distilled from this repo's actual failure modes; every
item earned its place. Read
`../fndr-v2-engineering/references/lessons.md` as part of it; that file is
where past sessions' mistakes become your head start.

## Step 5: hand off

Implementation follows `fndr-v2-engineering` (routing, invariants, lanes,
verification). When the feature lands, close the loop: ledger row in the
roadmap, journal or PR handoff note, and a lesson appended to `lessons.md`
if anything cost a cycle.

<!-- Inlined from .claude/skills/fndr-feature-dev/references/right-first-try.md -->

# Right-first-try preflight

Run before the first implementation commit of a feature. Each item exists
because skipping it has already cost a cycle in this repository.

1. **Read the lessons file.**
   `.claude/skills/fndr-v2-engineering/references/lessons.md` is the distilled
   memory of every mistake that cost time. Assume at least one applies to you.
2. **Navigate before you write.** Search the workspace for an existing
   implementation or near-miss; check the ADR-005 PORT list before writing
   from scratch. Parallel versions are how v1 got two rerankers.
3. **Name the tests before the code.** Write down the test names (including
   the negative and failure-path tests) the slice must ship with. If a test
   is hard to name, the seam is wrong.
4. **Enumerate the gates this change must pass.** fmt and clippy under
   `-D warnings` (edition 2024 rules included), workspace lints (egress,
   no-tauri), cargo-deny (a new dependency can fail bans or licenses
   transitively), AGENTS.md drift, bench baseline if ranking-adjacent, the
   bindings sync test if IPC-adjacent. Run the relevant ones locally before
   pushing, not in CI roulette.
5. **Verify third-party API shapes against the pinned source, not memory.**
   Crates in this stack (specta rc line, lancedb, rmcp, ureq) drift between
   versions; read the vendored source in `~/.cargo/registry` or fetch the
   crate before coding against a remembered API.
6. **Register every artifact where its runner expects it.** A migration file
   is not a migration until the runner's array includes it; a binding is not
   generated until the builder collects it; a CI step is not a gate until the
   workflow names it. After creating any registered-by-list artifact, grep
   for the list and confirm membership.
7. **State the typed failure path.** What does the user or caller see when
   this feature's dependency is missing or its operation fails? "Nothing"
   is the v1 answer and it is banned (invariant 4).
8. **Check the docs blast radius.** Which of PRD, roadmap, ADRs, ARCHITECTURE
   move together with this change? List them in the plan so the PR carries
   them, not a follow-up.
9. **Estimate the CI cost.** A heavy new dependency changes every future CI
   run (lance added ~17 minutes uncached). Know before you add.
10. **Confirm the branch and repo context.** Work happens in FNDR-2.0 on a
    feature branch; the guard hook resolves branches from the session cwd,
    so open sessions inside the repo you are changing.

<!-- Inlined from .claude/skills/fndr-feature-dev/references/feature-planning.md -->

# Feature plan template

Keep the whole plan under a page. Delete sections that honestly do not apply
rather than padding them.

```
## <feature name>

Problem: <one sentence: who hurts, when, and how this fixes it>
PRD tie: <goal G1..G5 or pain-point row; or "none" and stop>
User: <builder / agent / evaluator>
Smallest valuable version: <the cut line>
Demo relevance: <none | which beat, product capability only>

ADR touchpoints: <ADRs read; conflicts and how resolved; amendments needed>
Existing code/tickets reused: <what was found in the navigate-before-write search>

Slices (each = behavior + tests + docs, one reviewable unit):
1. <slice, with its named tests>
2. ...

Bench/eval impact: <none | which metric could move; baseline plan>
Typed failure path: <what shows when dependencies are missing or calls fail>
Docs moving together: <PRD / roadmap / ADR / ARCHITECTURE edits in the same PR>
Gates: <the preflight item-4 list relevant to this change>

Tickets: <new or re-scoped ticket lines in roadmap format>
```
