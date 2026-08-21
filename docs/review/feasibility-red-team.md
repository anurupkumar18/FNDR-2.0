# Reviewer report: feasibility red team

Independent fresh-context agent, 2026-08-20. Brief: skeptical principal engineer red-teaming capacity, critical path, sequencing, integration, estimation, and process risk across all of docs. Synthesis: [REVIEW-2026-08-20.md](REVIEW-2026-08-20.md).

**Ticket count by milestone and lane (census from ROADMAP-TICKETS.md):**

| | M1 | M2 | M3 | M4 | M5 | M6 | Total |
|---|---|---|---|---|---|---|---|
| backend | 20 | 3 | 9 | 2 | 0 | 0 | 34 |
| ml | 6 | 13 | 3 | 2 | 5 | 0 | 29 |
| frontend | 4 | 7 | 1 | 4 | 4 | 1 | 21 |
| platform | 8 | 3 | 2 | 1 | 1 | 3 | 18 |
| **Total** | **38** | **26** | **15** | **9** | **10** | **4** | **102** |

Backend M1 detail: T-105, T-201..207, T-301..309, T-801, T-802, T-805 (17 of the 20 are p0).

## Findings, ranked most severe first

**1. Month 1 is not achievable: the backend lane alone carries roughly two months of work.**
38 of 102 tickets (37%) sit in M1; 20 of those are one person's. At ~21 working days that is 1.05 days per ticket, and at least ten of the twenty have multi-day acceptance criteria by inspection: T-201 (full split schema plus migration tests), T-202 (Lance single-writer with crash semantics), T-203 (indexes proven on a 100k fixture), T-204 (24h simulated write load), T-205 (kill-during-flush recovery), T-206 (deletion across both stores), T-302 (new ScreenCaptureKit provider), T-306 (staged pipeline scheduler), T-307 (session identity/merge port), T-309 (replay harness). At an optimistic 3 days each, those ten alone are 30 days before the other ten tickets, reviews, or environment setup. Estimates were "deliberately omitted" (ROADMAP Conventions), so milestone load was never checked against capacity.
Fix: re-cut M1. Move T-204, T-205, T-207, T-307, T-308, T-309, T-802 to M2 (none gate the M2 read path; T-204/205 need real write load anyway) and reassign T-105 to platform. Target backend M1 at 10 to 11 tickets.

**2. The month-3 gate critical path has zero slack, and M3 carries 9 backend tickets the gate does not need.**
The gate (PRD §10) needs T-101 → T-201 → T-202 → T-203, joined by T-402 and T-503 into T-504 → T-505 → T-702 → T-707 → T-905, plus T-903 and T-803: eight-plus serial p0 tickets crossing three lanes. M3 also contains T-1101, T-1102, T-703, T-704, T-706, and T-703 deps on T-1102, so the wave-2 tool chain is blocked behind the knowledge graph, which the demo script does not require. The only scheduled buffer (month 6, 4 tickets) sits after the gate. Single failures that blow the gate: the backend builder out two weeks in M1 or M3; Lance FTS/index behavior not matching T-203/T-505 assumptions (the ADR-002 fallback, sqlite-vec, fails P0.8 at scale); llama-cpp-2 not cleanly serving Qwen3-Embedding; the Tier-2 MCP SDK lacking pieces T-701 assumes.
Fix: move E11 and T-704/T-706 to M4; split graph_context out of T-703; add a gate dry-run ticket two weeks before M3 close; schedule M1 spikes for the three load-bearing dependency assumptions (Lance FTS+prefilter, Qwen3 embedding via llama-cpp-2, rmcp streamable HTTP auth).

**3. The backend lane is a single point of failure by construction, and the lane rules prevent rescue.**
Backend owns 34 of 102 tickets including fndr-types (everything consumes it), 20/38 of M1 and 9/15 of the gate month. T-105 (backend) blocks T-1001, which blocks T-1401, which blocks the frontend lane; T-804 (frontend M1) deps T-306 (backend, late M1). lanes.md says lanes "own disjoint crates" and cross-lane PRs need the owning lane's review, so there is exactly one qualified reviewer per contract and no rebalancing mechanism. Frontend is simultaneously starved (4 M1 tickets, 3 dep-blocked; 1 ticket in M6).
Fix: name a secondary owner per backend crate now (frontend for fndr-types/bindings, platform for fndr-store ops tickets, ml for capture dedup) and write the pairing expectation into lanes.md.

**4. "Install from a URL" at month 3 has an unacknowledged hard dependency: Developer ID signing.**
PRD §13 calls the Apple Developer account "notarization timing only. Non-blocking," but T-903 (M2) promises a "signed DMG" and the gate requires install on a clean machine from a URL. Without a paid account there is no Developer ID certificate; an ad-hoc build downloaded from a URL is quarantined by Gatekeeper on macOS 14+, and the 15-minute onboarding target dies at "unidentified developer." The ADR-001 TCC mitigation also depends on a consistent signing identity.
Fix: reclassify the account as a month-1 blocking acquisition with an owner and date, or rewrite the gate script now to include the right-click path and accept the demo-credibility cost explicitly.

**5. ML month 2 is the second overload: 13 tickets including the retrieval core, the synthesis epic, and the eval program that gates them.**
M2 ml = T-405, T-406, T-503, T-505, T-506, T-507, T-508, T-512, T-601, T-602, T-603, T-604, T-605. T-505 alone is the retrieval engine, built concurrently with the CI bench gate (T-508) that is supposed to gate it. Even at 1.5 days per ticket this is ~20 working days with zero slack.
Fix: move T-602, T-605, T-405, T-406 to M3 (T-509 is already M3, so T-406 in M2 buys nothing) and hand T-503 (chunker) to backend, whose M2 has 3 tickets.

**6. Eval-first discipline structurally cannot hold in month 2, exactly when it matters most.**
Invariant 3 forbids merging ranking code without bench numbers against "the committed baseline," but the gate and baselines are T-508, landing in the same month as T-505/506/507 by the same overloaded person. The largest ranking drop of the project merges before the gate exists. T-501 wants 150+ labelled pairs in M1 from the ml lane while it also builds T-401..404 and T-502; rushed labels make the gate green-but-meaningless. ADR-006 admits the real-model CI lane is unresolved, and T-102 caps CI at 15 minutes, which a real-model bench on a hosted macOS runner will not fit. Predicted slip order under pressure: bench-in-CI first, then the weekly rubric, then port provenance during the M1 crunch, which is how the v1 liabilities re-enter.
Fix: sequencing rule "bench gate merges before any ranking PR"; resolve the CI lane now (recommend nightly bench on a self-hosted Mac, not per-PR); cut the rubric to biweekly with a named owner.

**7. The first contact between a real agent and the MCP surface is in the gate month itself.**
T-702 (M2) is tested by schema round-trip and auth-failure tests; no ticket exercises Claude Code/Desktop against the server until T-707 (M3, assigned to lane::frontend although its failure modes are protocol-level). Tool ergonomics for agents are the headline claim and are unvalidated until weeks before the gate.
Fix: add a small M2 ticket "wire Claude Code to the dev server and run 10 scripted agent tasks" the week T-702 lands, owned by backend, findings feeding T-703.

**8. The safety gate goes live two months after capture starts, and lands in the gate month with zero soak time.**
T-803 is M3; capture ships in M1 and dogfooding must start by M2 (T-512). The team's own machines capture unredacted secrets/banking/medical content for up to two months, and the gate's "sensitive content verifiably absent" requirement rests on a days-old suite. ADR-004's adversarial-suite action item is also month 3.
Fix: move T-803 to M2 (its deps are both M1); keep only adversarial-suite hardening in M3; add "purge pre-gate dogfood stores" to T-905.

**9. Worst five one-line icebergs.**
T-505 (an entire hybrid retrieval engine); T-903 (release engineering is never a one-liner; see finding 4); T-302 (replacing the capture substrate: SCK permission model, display reconfig, sleep/wake, multi-display); T-701 (Tier-2 SDK plus auth middleware, allowlists, audit log; the v1 equivalent was 5,197 lines); T-501 (150+ labelled pairs plus a donation protocol before capture exists to donate from). Honorable mention: T-205's AC ("rebuild converges byte-equal") is likely unsatisfiable: IVF training is not deterministic across rebuilds; either it silently forces exact search or the AC gets waived, and a waived AC on the crash-recovery ticket leaves rebuildability (the entire ADR-002 crash strategy) unverified.
Fix: split T-505 (routes / fusion / serving) and T-903 (signing / updater / CI wiring); respecify T-205 to "recall parity within epsilon on the bench corpus"; pre-spike T-701 and T-302 in week 1.

**10. The component library lands in the same month as four of its consumers, recreating the exact v1 failure the PRD names.**
T-1405 (M2) is concurrent with T-1002/1003/1004/1005 (all M2); PRD F9 says the library must exist "before feature UI multiplies." Under pressure, feature UI will hand-roll primitives and the lint arrives too late.
Fix: unblock frontend in M1 by stubbing T-105 early (platform) and pull T-1405 into M1 against the T-1404 spec.

**11. Milestone loads whipsaw because tickets were assigned by feature phase, not person capacity.**
Backend 20-3-9-2-0-0; ml 6-13-3-2-5-0; frontend has 1 ticket in M3; platform has 1 in each of M4/M5. Whole lanes idle while adjacent lanes run 2x over.
Fix: rebalance per findings 1, 2, 5, 10; enforce at import: no lane exceeds ~8 tickets per milestone without written justification.

**12. Months 4 and 5 land on the December/January holidays, where the plan schedules its only four-lane simultaneous integration (meetings).**
A September start puts the M3 gate at end of November (US Thanksgiving week) and M4 in December. E12 requires platform (T-1201, a brand-new process boundary), ml, frontend, and backend to integrate in that month. ADR-003 calls for a sidecar spike in month 3 to 4 but no spike ticket exists.
Fix: add a T-1201 spike in M3 (platform's M3 load is 2 tickets); accept transcription-only as the M4 target; write the holiday assumption into the roadmap.

**13. Dogfooding is a hidden month-2 dependency with unbudgeted cost.**
T-512 requires weekly scored queries from all four builders, which requires capture+store+embed on four personal machines by early M2, before onboarding (T-407, M2) or fndr doctor (T-907, M2) exist. Nothing verifies all four builders have Apple Silicon macOS 14+ daily drivers, and the reference machine has no named owner.
Fix: add an M1 ticket "dev-install path on all four machines plus reference-machine designation"; gate T-512's start on it.

**14. The plan is silent on working-time assumptions, review latency, and environment setup.**
"Self-paced" (the actual constraint) appears in no document; the skill assumes monthly founder reviews, monthly dead-code sweeps, per-session handoffs, and per-lane review of every cross-lane PR. No ticket covers per-builder environment bootstrap, which reliably eats the first two weeks. Single-reviewer lanes make review latency additive to every critical-path ticket.
Fix: state per-person hours/week in the roadmap header and re-derive loads; name fallback reviewers; add an environment-bootstrap ticket; move part of the M6 buffer in front of the gate (a deliberately light second half of M3).

**15. Two quality targets are bets made before any measurement, placed where failure is discovered latest.**
G2 commits to "+15 points Recall@5 over BM25-only" while the corpus deliberately includes exact-identifier queries, the case BM25 wins; a miss at month 5 dies publicly. T-511 proves P0.8 latency at 1M rows only in M3, and its AC explicitly permits entering the gate failing P0.8; a miss can force IVF re-parameterization or a dimension change, colliding with ADR-006's "no dual-contract transition."
Fix: run a cheap 1M-row synthetic latency probe in M1 before the embedding contract is final; soften G2's +15 to a stretch figure until first real numbers exist.

**Top three if you make only three changes:** re-cut backend M1 (finding 1), pull graph out of M3 (finding 2), resolve the signing-account question this week (finding 4).
