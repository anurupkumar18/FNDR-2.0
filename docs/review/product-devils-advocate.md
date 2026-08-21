# Reviewer report: product devil's advocate

Independent fresh-context agent, 2026-08-20. Brief: attack the product thesis within the locked constraints (local-only, macOS-only, keep all features, MCP spine, no competitor benchmarking), against the three goals: daily personal use, portfolio centerpiece, future B2B/B2C credibility. Synthesis: [REVIEW-2026-08-20.md](REVIEW-2026-08-20.md).

## Ranked objections

**1. The plan never addresses time-to-first-value; it encodes the cold start as a precondition instead of a problem.**
P0.10 measures install-to-first-captured-memory (15 minutes). First value is different: a context pack an agent can use, which needs hours to days of capture. The month-3 gate literally begins "work normally for a day," baking the 24-hour dead zone into the flagship demo. No ticket covers empty-vault UX, sample data, or backfill. Connect-your-agent onboarding (T-707) wires an agent to a vault that will honestly answer NotEnoughEvidence to everything, so the verdict system makes the first impression worse. Threatens all three goals: dogfooders beyond the four churn on day 1; an evaluator who installs gets nothing; a structural day-one activation cliff is the first thing a buyer finds.
Cheapest fix: ship the T-501 synthetic fixtures as a loadable sample vault with an "explore a sample day" onboarding step; add one deterministic backfill importer (browser history, git log, shell history: local, no new privacy surface) so the first real context pack works in minutes; add an explicit empty-vault state to T-1003.

**2. There is no daily payoff loop until month 5, so week-6 retention rests on willpower, measured by self-report.**
Through month 2 the builders get capture plus search. Invocable payoffs arrive late: packs M3, omnibar M5, Session Story M5 (P1), resurfacing M5 (P1, first on the cut list). Passive-capture tools churn because cost is constant (RAM, battery, screen-recording unease) while value is episodic. T-512's mandatory weekly queries manufacture usage and mask the churn signal. If builders quietly stop running it in month 2, the donated-session corpus starves and the gate is run by people who no longer work normally with it on.
Cheapest fix: pull T-1306 (warm-start file export) from M5/P1 to M3/P0 and regenerate it daily: FNDR then improves every agent session the builders already run, with zero invocation habit required. Add a deterministic morning "yesterday" digest in the menu bar (composition only, no VLM), and a local usage counter (queries and packs served per day) so retention is measured, not self-reported.

**3. FNDR-Bench as designed is tuning on its own test set and grading its own homework; the "measured" pillar will not survive expert inspection.**
T-508 runs the bench in CI against every ranking PR, and the same corpus produces the published numbers: optimizing on the test set by construction. The corpus is authored by the team building the retriever. The headline target (+15 Recall@5 over BM25-only on noisy OCR text) is a weak-baseline victory, and "beats the POC pipeline" is beating your own discarded code. Usefulness evidence is four authors rating their own product plus an LLM judge with known biases, and no agreement statistics.
Cheapest fix, inside ADR-006's framing and the no-competitor ruling: freeze a held-out test split in T-501 that CI never touches; add one or two off-the-shelf-pipeline baselines (a default naive-RAG stack, an alternative open embedder): methods, not competitors; report inter-rater agreement for the rubric; publish corpus and harness with an external-submission track. Reframe the deliverable as "the first public benchmark for screen-derived personal memory retrieval" with FNDR as the reference implementation. An honestly published loss is itself credibility.

**4. "Citations resolve" is the wrong target: resolution is not support, and nothing measures verdict accuracy.**
Section 9 targets 100% of citations resolving to real records. A citation can resolve and still mislead: OCR fragments of a page the user merely read become "you decided X," and the two-distinct-backers rule is satisfied by two captures of the same misleading screen. The bench measures retrieval, not answer faithfulness, and no metric scores the Grounded/Partial/NotEnoughEvidence verdict itself. A product whose pillar is "shows its work" is defined by its confidently wrong verdicts; a plausible surfacing reason attached to a wrong result is worse than silence because it miscalibrates trust.
Cheapest fix: add a faithfulness slice to the bench with labelled unanswerable queries where the correct output is NotEnoughEvidence, so overclaiming becomes a measured regression; add a small claim-support audit to the weekly rubric; add a read-versus-authored provenance bit on records (derivable from app category and focus/input signals) so composition can distinguish "saw" from "did."

**5. The month-3 demo is a softball that hides the hard parts and outsources its wow to the agent's eloquence.**
"Agent resumes yesterday's work with citations" can be simulated with a pasted text file; nothing on screen proves capture discipline, privacy gating, latency, or grounding, and the visible quality is Claude's prose, not FNDR's pack. Stronger demos with the same engineering: (a) the counterfactual cut: the identical agent task with FNDR off (agent interrogates the user, user pastes context) then on (one tool call), showing the pain being removed; (b) make the negative demo the spine: visit a bank and a password manager on camera, then show the vault, the pack, and privacy_status proving absence; (c) one unscripted cross-app multi-day question with citations opened live via fndr.open_target; (d) show the pack artifact itself, not just the chat output.
Cheapest fix: rewrite the T-905/T-1503 script around (a) and (b). Pure staging, zero new engineering. Optionally pull a minimal session_story into the gate; it is the most visceral artifact for all three audiences.

**6. Long-horizon ownership is unplanned: no backup or restore, no vault export, embedding migration designed out, retention at P1, multi-Mac absent.**
"The memory stays mine" is contradicted by the absence of any export or backup surface. Time Machine copying a live WAL database is a corruption story awaiting its first victim. ADR-006's "born on the final contract, no dual-contract transition" is a red-flag sentence: when the pinned embedder is superseded, the dimension guard bricks capture until a full re-embed with no ticket, no UX, and multi-hour cost on 8 GB. Storage grows ~2 GB per month while retention jobs sit at P1. The target user often owns two Macs; the plan gives them two amnesiac half-memories with no design stance.
Cheapest fix: one `fndr export` / `fndr backup` command (SQLite snapshot plus config; Lance already rebuilds, the free half of the story); promote T-207 to P0; a one-page re-embedding migration note (the backfill priority class in T-403 is the queue it needs); a multi-Mac stance paragraph in the PRD even if the stance is "not in v2, and here is why it forces no rewrite."

**7. Meetings capture records other people, and the entire consent treatment is one line of UI copy.**
Diarized, retained transcripts of colleagues are their data too, and several jurisdictions (California included) require all-party consent. T-1204's "explicit consent framing in UI copy" survives neither a B2B legal review nor an ethics probe, and one "FNDR recorded my 1:1" incident poisons the trust positioning.
Cheapest fix: keep the feature, add a consent design note: off by default, explicit per-meeting start action (never ambient), the visible indicator, and a distinct shorter retention default for meeting transcripts. Copy, defaults, and a short doc; no new engineering.

**8. The plan generates design docs and numbers but not the failure narrative a staff interview actually runs on.**
The POC audit is the best interview artifact this team owns, and v2 has no mechanism to produce its own equivalent: no incident log, no postmortem convention, no record of reversed decisions. Interviewers probe wrong turns and published constants ("1M records is how many months of capture? why 15 points?"), and those derivations are written nowhere.
Cheapest fix: extend T-106's handoff convention with an incidents-and-reversals log; add a "limits and failure modes" page to the docs site; write the records-per-day to corpus-size derivation into the bench methodology so every published number has a lineage.

**9. The 3D graph is the worst value-to-effort area kept, and it lands in the highest-leverage month.**
Its retrieval value is eval-gated and, on the POC's own evidence, likely to lose; that leaves "demo and comprehension surface," where 3D force-graphs are look-once features and, to senior evaluators, a recognized portfolio-slop marker the project's own anti-slop bar should catch. It consumes the M4 frontend lane exactly when retention surfaces are missing.
Cheapest fix within the keep-everything ruling: enter at the pre-agreed cut-line scope (view-only) from the start; move T-1104 polish into the M6 buffer; let T-1105's eval result gate further visualization investment. Spend the freed M4 frontend weeks on timeline, empty-vault, and the daily digest.

**10. The scariest permission on macOS gets a paragraph of copy instead of a trust moment.**
Onboarding asks for screen recording with a "plain privacy story"; the verifiable proof (PRIVACY.md recipe, M3) is developer-grade and lands after onboarding ships. Trust is won or lost at that dialog, and "local-only, provably" is currently proven only to people who read CI configs.
Cheapest fix: a "verify it yourself" onboarding screen before the permission prompt: live egress counter at zero, the CI local-only badge, one-click audit log, and pause/incognito demonstrated. All existing surfaces, resequenced.

## What I would add

1. Backfill importers (browser history, git log, shell history) as a first-run memory seed: deterministic, local, no new privacy surface, kills the cold start.
2. A loadable sample vault built from the T-501 fixtures, shared by onboarding, the demo, and any evaluator who installs.
3. `fndr export` and `fndr backup`/`restore`, making "the memory stays mine" literal.
4. A frozen held-out FNDR-Bench test split plus public corpus and harness release with an external-submission track.
5. Local usage instrumentation (queries, packs, deltas served per builder per day) surfaced in the health panel, so the founding retention metric is measured rather than self-reported.
