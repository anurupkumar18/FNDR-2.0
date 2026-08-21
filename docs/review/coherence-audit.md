# Reviewer report: cross-document coherence audit

Independent fresh-context agent, 2026-08-20. Brief: exhaustive consistency and completeness checks across PRD, ARCHITECTURE, ADR-001..007, ROADMAP-TICKETS, tickets.csv, README, and the skill. Ticket metadata, the dependency graph, and CSV/Markdown equivalence were checked programmatically over all 102 tickets, not sampled. Line numbers reference the 2026-08-20 state of the files. Synthesis: [REVIEW-2026-08-20.md](REVIEW-2026-08-20.md).

## Check 1: P0.1 to P0.11 ticket mapping

Clean: no ticket cites an undefined P0; P0.2 (T-407), P0.4 (T-803), P0.8 (T-511), P0.9 (T-206) map correctly.

- **1.1 P0.5 cannot pass as written (no companion code exists in v2).** PRD P0.5 requires "any MCP or companion endpoint" to reject unauthenticated calls with failure-closed tests, but T-1504 is "spec-only ... no code shipped" and ARCHITECTURE marks `fndr-companion` spec-only until mobile starts. The only auth ticket (T-701) covers MCP. Half of a P0 has no implementing ticket and no testable subject.
- **1.2 P0.10 has no ticket with a matching acceptance criterion.** The 15-minute install number appears in no ticket; T-407 says "under target" (unnamed); no ticket asserts "installs from a URL"; no ticket cites P0.10.
- **1.3 P0.7's CI requirement is not fundable under its CI ticket.** P0.7 requires real-model bench in CI; T-102 caps CI at 15 minutes; ADR-006 names the unresolved prerequisite ("macOS runner or cached local run protocol"); no ticket creates the real-model lane.
- **1.4 P0.3 is covered but never cross-referenced** (T-805 does not cite P0.3 while sibling tickets cite theirs): inconsistent traceability convention.

## Check 2: ADR action items to tickets

Mapped and clean for ADR-001, -002, -004, -005, -006, -007 items, except:

- **2.1 ADR-003 item 3 unscheduled in its window:** the FluidAudio sidecar spike is specified "month 3 to 4, ahead of meetings," but the only ticket (T-1201) is M4, the same milestone as its consumer.
- **2.2 ADR-007's posture split across milestones:** "per-tool rate limits ... per-client scoped tokens" are part of "Security posture, all modes," but T-701/T-702 ship the server and 8 tools in M2 while rate limits and scopes land in T-705 (M3). The skill's invariant ("new route carries a permission scope and a rate limit") is unachievable for a full milestone.
- **2.3 ADR-002 item 3 has no ordering mechanism:** "deletion-everywhere before any UI exists" is not expressed as a dependency (T-206 and T-1001 are both M1, unordered).
- **2.4 ADR-003 lineup rows with no ticket and no explicit deferral:** Qwen3-4B Q&A, Foundation Models backend, SigLIP 2 image embedding, SpeechAnalyzer fallback.

## Check 3: dependency integrity

Graph-level clean across all 102 tickets: zero dangling IDs, zero cycles, zero backwards cross-milestone deps. Logical defects:

- **3.1 Mutual gate between T-406 and T-509:** T-406's AC says "promotion blocked on T-509" while T-509 deps on T-406; reword T-406's done-state to "integrated behind flag."
- **3.2 T-905 (gate script) missing deps on T-803 (safety gate) and T-703** although the gate must exercise both ("sensitive content verifiably absent").
- **3.3 M2 feature UI (T-1002/1003/1004/1005) does not depend on T-1405 (component library)** it is required to use: the dependency enforcing the fix for v1 pain point 9 is absent.
- **3.4 T-1007 (capture-explain) omits T-309**, the ticket that produces the retained gate outcomes it reads.
- **3.5 T-407 (frontend) lacks the T-1001 dep** every other frontend ticket carries.
- **3.6 T-703 couples five non-graph tools to graph work** via its T-1102 dep; a graph slip blocks delta, explain_retrieval, open_target, feedback, and the only write tool, all needed by the gate month. Split the graph tool out.

## Check 4: milestone consistency

- **4.1 Four epic headings contradict their own tickets:** E05 "(M2 to M3)" contains M1 tickets; E07 "(M2 to M3)" contains an M5 ticket; E09 "(M1, M6)" contains M2/M3 tickets; E10 "(M2 to M4)" contains an M1 ticket.
- **4.2 Six PRD feature-phase labels contradict the tickets:** F1 "(phase 1)" includes the safety gate (M3); F2 "(phases 1 to 2)" is all M2; F4 "(phase 2)" spans M1 to M3; F5 "(phases 2 to 3)" includes an M5 tool; F8 "(phases 4 to 5)" is all M5; F10 "(phases 1, 6)" has its work in M2/M3.
- **4.3 Context packs are month 3 in the PRD and month 2 everywhere else** (T-702, ARCHITECTURE, ADR-007). The headline feature's milestone is ambiguous.
- **4.4 Theming: month 4 in the PRD, M5 in the roadmap (T-1403).**
- **4.5 Three PRD roadmap deliverables have no ticket:** month-6 "hardening and performance passes," month-3 "hardening," and month-6 "FNDR-Bench public release" (the benchmark page ticket is M5).
- **4.6 T-903 (release pipeline, M2) appears in no PRD month description.**
- **4.7 `prio::p0` means both "spine, never cut" (its definition) and "must-ship"** (it is applied to meetings, omnibar, and month-6 proof tickets that the PRD pre-agrees can degrade or that sit outside the spine). Pick one meaning.

## Check 5: lane consistency

- **5.1 `fndr-companion` has three owners:** platform (ARCHITECTURE, T-1504), backend (skill lanes.md), and both inside PRD §10 (backend "companion contract" and platform "companion API contract authorship").
- **5.2 `fndr-shell` co-ownership (PL+FE) exists only in ARCHITECTURE;** the skill gives it to platform alone; the PRD gives "shell UI" to frontend.
- **5.3 The skill contradicts itself on the backend crate set** (SKILL.md's list omits `fndr-textsignal`, `fndr-ocr`, `fndr-companion`, which lanes.md includes).
- **5.4 T-401 assigns the platform-owned downloader crate to lane::ml and its AC says "only crate with HTTP,"** which would fail the updater (ARCHITECTURE and ADR-004 allow two crates).
- **5.5 T-709 adds an MCP tool from lane::ml** while the skill says the MCP contract is backend-owned, and its AC omits all four required tool artifacts (schema round-trip test, auth-failure test, rate limit, docs entry).
- **5.6 Other label conflicts:** T-804 (menu-bar controls) is frontend but tray is platform's crate; T-1007 is backend inside the Core UI epic with a UI-facing AC and no frontend counterpart; T-1202 is ml but writes through the backend-owned write path; T-407 splits registry code and UI across lanes without a contract note.

## Check 6: terminology drift

- **6.1 MCP tool count: three numbers.** ADR-007, ARCHITECTURE (three places), and the skill say 14; the PRD says "12 to 15"; the PRD adds session_story and answer, neither in ADR-007's table; T-709 ships a 15th tool with no ADR amendment.
- **6.2 "Telemetry" means opposite things:** "No telemetry of any kind" (PRD, invariants) vs "`logs/` (JSONL quality/telemetry, local only)" (ARCHITECTURE).
- **6.3 The local-only gate has four names** (local-only CI gate, dependency gate, egress allowlist test, local-only egress lint); G4 counts "the gate" being green, so which artifact is the gate matters.
- **6.4 Auth phrasing weakens in the PRD:** "authenticated by default" vs ADR-007's "required unconditionally" and the skill's "auth-always." "By default" implies a supported off switch.
- **6.5 Safety-gate verb set varies** (skip-storage vs SkipStorage vs skip).
- **6.6 Milestone tokens vs labels:** the backticked `M1` tokens on ticket lines are visually indistinguishable from labels; the CSV expands them correctly, but import-time ambiguity exists.
- **6.7 `fndr.recall` is never named in the PRD** (called "decisions/errors/todos").
- **6.8 / 6.9 Descriptor drift:** the audit is "full" / "four-module" / "four-agent" depending on the document; POC line counts are rounded in one doc and exact in another.
- **6.10 Lance table names pin 768d** (`chunks_v1_qwen768`) while ADR-006 keeps a 768-vs-512 revisit open and simultaneously forbids a dual-contract transition; the revisit is exactly a dual-contract migration.
- **6.11 SQLite table naming mixes singular and plural** in one line (`chunk` beside `tasks`).

## Check 7: PRD internal contradictions and coverage

- **7.1 Broken metric pointer:** "targets in G1/G3" for search latency, but G3 has no latency target (it lives in P0.8).
- **7.2 The reranker is simultaneously core (F4 prose) and conditional (P1, ship-only-on-a-win).**
- **7.3 x86_64 (P1) contradicts the "Apple Silicon only" non-goal and has no ticket.**
- **7.4 P1 features with no ticket:** visual similarity (SigLIP 2), Foundation Models backend, the grounded Q&A tool. The cut lines assume visual similarity is scheduled ("the first cuts"), but it was never planned, so it cannot be cut.
- **7.5 Goal/metric numbers absent from every AC:** capture CPU under 5% (not in T-502's metric list), context-pack p95 under 2 s (measured by no ticket), 80% usefulness (not in T-512), 2-minute legibility (not in T-1004).
- **7.6 Two goals depend on cuttable P1 tickets:** G3's storage-at-default-retention depends on T-207 (p1); G5 and the lagging metrics require the docs site, T-1502 (p1).
- **7.7 Mobile is both deferred and staffed:** "no mobile app engineering" vs the team section's "iOS groundwork from month 6" (no ticket).
- **7.8 "One write tool" vs fndr.feedback,** which persists logged feedback: state the exception or rename the claim.

## Check 8: conflicting claims of fact

- **8.1 RAM: the PRD's 400 MB idle budget exceeds the 150 to 300 MB range ADR-001 calls "indefensible" when rejecting Electron;** ADR-003 keeps the ~400 MB embedder always-on, which alone equals the entire idle budget.
- **8.2 A companion HTTP server both runs (ARCHITECTURE §2 process table) and does not exist (spec-only until mobile).**
- **8.3 "Resource budgets are P0 metrics" (PRD §12) but no resource requirement is in the P0 list** (they live in G3, a month-6 goal).
- **8.4 Meeting residency (1 to 1.5 GB session-scoped) exceeds the 900 MB active-capture budget** with no stated carve-out for the sidecar process.
- **8.5 "Only crate with HTTP" (T-401) vs two allowed crates (ARCHITECTURE, ADR-004).**
- **8.6 T-102's 15-minute CI cap vs P0.7's in-CI real-model bench.**
- **8.8 Stale status dates:** PRD/ROADMAP/ARCHITECTURE say 2026-08-19 while carrying 08-20 content; only README records the revision; the ADRs were never amended, which is the root cause of 6.1.

## Check 9: tickets.csv vs ROADMAP-TICKETS.md

Clean, verified exhaustively: 102 rows, identical ID sets, zero field differences across title, description, AC, deps, labels, milestones; quoting well-formed. Observations: late-insertion ordering anomalies (T-1007 between T-1004/1005; T-1405 between T-1401/1402) will produce confusing import order; epic membership exists as a heading in one file and a label in the other with nothing keeping them aligned after edits.

## Check 10: skill vs ADRs

- **10.1 Crate ownership conflict** (fndr-companion): see 5.1.
- **10.2 The rate-limit invariant is unachievable for M2:** see 2.2.
- **10.3 TLS missing from the listener checklist** although ADR-007 requires it for non-loopback.
- **10.4 Two different lane maps inside the skill:** see 5.3.
- **10.5 Unsourced figures in the rulebook:** "7 fusion-weight sets" and "fifteen sequential inline gates" appear in no ADR or audit record; unsourced numbers in a mandatory document invite the re-litigation the skill forbids.
- **10.6 Verification commands are not the CI commands:** the skill requires `make test`/`make bench`; T-102's CI is "fmt, clippy, cargo test, vitest, tsc"; no ticket creates the Makefile targets.
- **10.7 The tool-addition rule hard-codes "14 canonical tools"** and will be stale the day T-709 lands.

Verified clean and worth noting: the two named v1 security regression tests match across all documents; port-provenance and DISCARD rules match ADR-005 exactly; eval-gate metrics match ADR-006; all v1 ADR cross-references resolve to real files.

## The five defects most worth fixing first

1. **P0.5 is unsatisfiable as written (1.1):** narrow it to MCP surfaces and move companion auth into T-1504's spec review, or fund a conformance test.
2. **The MCP tool contract has no single number or inventory (6.1, 8.2):** amend ADR-007, restate one count, add the new tools to T-706.
3. **Auth-always is planned as auth-mostly (2.2, 10.2):** pull the rate-limit and scope skeleton into T-701, leaving revocation UI in T-705.
4. **Scheduling metadata contradicts itself in ten places (4.1, 4.2, 4.3, 4.7):** the layer the team plans against, and the cheapest to fix.
5. **Several P0 and goal numbers are unmeasurable as ticketed (1.2, 1.3, 7.5):** add the numbers to T-407, T-502, T-511, T-512, T-1004; open the real-model CI lane ticket ADR-006 already flagged.
