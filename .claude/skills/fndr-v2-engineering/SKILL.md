---
name: fndr-v2-engineering
description: Mandatory engineering workflow for the FNDR v2 repository (Rust engine workspace + Tauri shell + React UI + Swift sidecar, 4-person lane split). Use this for ANY task in the v2 repo: implementing or reviewing a feature, porting code from the v1 reference branch, changing retrieval or ranking, touching capture/privacy/network surfaces, building UI, fixing bugs, ending or handing off a work session, running a product/plan review, or preparing a release, even when the user does not name a workflow. It routes the task to the right lane, contract, invariant checklist, AI-collaboration practice, and verification commands.
---

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
