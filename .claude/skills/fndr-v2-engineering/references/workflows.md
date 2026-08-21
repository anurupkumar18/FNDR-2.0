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
