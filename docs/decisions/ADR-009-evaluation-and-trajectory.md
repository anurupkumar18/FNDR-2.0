# ADR-009: Evaluation and trajectory: evidence, planner usefulness, and safety are separate gates

**Status:** Proposed
**Date:** 2026-09-05
**Deciders:** FNDR v2 team (4)

## Context

ADR-006 already prevents unmeasured retrieval ranking from shipping. The
semester plan adds a connected planner and reusable runtime skills, which
create two additional failure modes that retrieval metrics cannot detect:
the planner can produce a plausible but unsupported recommendation, or it can
propose an unsafe capability use. Treating either result as a successful
search would repeat the v1 pattern of claiming quality from an indirect test.

The product also needs a three-month trajectory that protects the alpha demo
without confusing a working fixture, a planner proposal, and a final
submission claim. This ADR makes the evidence needed at each milestone
explicit.

## Decision

### 1. Keep five evaluation surfaces distinct

| Surface | Question | Required evidence |
| --- | --- | --- |
| Retrieval | Did FNDR find the right local records? | ADR-006 FNDR-Bench Recall@5, MRR@10, latency, and frozen holdout reporting. |
| Evidence fidelity | Do displayed claims and citations actually support one another? | Citation-resolution and claim-support annotations, including unanswerable queries. |
| Planner usefulness | Does an approved export produce a plan that helps the owner resume work? | Blinded human rubric over cited proposals, with the no-planner baseline kept visible. |
| Capability safety | Can a proposal or malformed input bypass approval or expand its scope? | Adversarial approval, argument, path, expiry, and audit tests. |
| Runtime-skill reliability | Does a reviewed skill stay within its declared input and capability contract? | Schema validation, fixture replay, and revision-invalidates-approval tests. |

A green score on one surface never substitutes for a missing score on another.
In particular, a planner cannot use a high retrieval score to claim that its
steps are supported, and an approved export cannot count as an approved
action.

### 2. Use fixture tiers and protect the holdout

Evaluation data is stored locally and classified as:

- **Unit fixtures:** small synthetic records for policy, storage, and contract
  tests. Safe to run in every PR.
- **Development scenarios:** labelled retrieval, export, and proposal
  scenarios used to develop behavior. They must never be presented as final
  quality results.
- **Frozen holdout:** separately stored, access-controlled scenarios for
  milestone and final reporting. It is never used to tune ranking, prompts,
  policy lists, or skill text.
- **Dogfood samples:** voluntarily donated, sanitized examples used for the
  weekly usefulness rubric. They remain local unless separately approved for
  benchmark publication.

Every scenario records its source tier, expected citations, expected verdict,
and whether it contains a forbidden export/action attempt. A scenario with
private captures is not committed to the repository.

### 3. Planner evaluation is citation-first and comparative

For each planner scenario, reviewers independently score: correct task
understanding, citation support, actionability, bounded uncertainty, and
privacy/approval compliance. The report shows both the local FNDR-context
planner result and the same planner task without FNDR context. It does not
claim causal improvement from a single demo. Unsupported claims, missing
citations, and a proposal that asks for more context than approved are
failures even if the prose sounds useful.

Planner output is evaluated as untrusted text. No production capability is
enabled because a rubric score is high; capability promotion also requires the
safety suite below.

### 4. Safety is a blocking gate, not a weighted metric

The following failures block alpha, beta, and final milestones regardless of
quality scores:

1. a sensitive, blocklisted, or redacted value is present in an export;
2. a changed payload, destination, policy version, or expired approval is
   accepted;
3. a proposal executes without a second per-action approval;
4. an unsupported capability, shell metacharacter, unapproved path, network
   command, or recursive traversal is accepted; or
5. an audit record omits the required outcome or includes raw payload text.

The test name and fixture for each discovered bypass become a permanent
regression test. Safety pass rates are reported separately; they are never
averaged into a quality score.

### 5. Milestone evidence rises by phase

| Phase | Product claim allowed | Required evidence |
| --- | --- | --- |
| Alpha | A local, fixture-backed capture-to-search/MCP spine can demonstrate cited memory and visible privacy gating. | Deterministic fixture replay, auth regression tests, policy tests, one end-to-end smoke run, and a short no-planner versus approved-context comparison labelled as a demo. |
| Beta | A small dogfood cohort can use persisted local memory, reviewed planner exports, and two narrow proposal capabilities. | Development-scenario report, weekly usefulness rubric, capability adversarial suite, and a recorded dry-run with known failures ticketed. |
| Final | The submission demonstrates reproducible quality and an auditable end-to-end user journey. | Frozen-holdout report, benchmark methodology, final privacy/capability report, clean-machine demo checklist, and retained evidence for every stated metric. |

If an alpha evaluation fails, scope is cut back to the last demonstrated
local spine. If beta planner evidence fails, the planner stays experimental or
is removed from the final demo rather than papered over with scripted output.

## Options considered

**A (chosen): separate quality, fidelity, safety, and milestone evidence.**
It produces honest claims and catches the failure modes the new planner adds.

**B: use a single LLM judge score.** Rejected. It cannot prove citation
support, consent correctness, or capability containment, and it obscures
reviewer disagreement.

**C: wait for a large real-user corpus before testing.** Rejected. It delays
the alpha learning loop and pressures the team to collect sensitive data.

## Consequences

- Easier: each failure has an owning test class and a visible decision about
  whether to fix, cut, or defer it.
- Harder: evaluation artifacts must be maintained alongside feature work;
  claims that cannot be reproduced cannot appear in the submission.
- Deferred: automated tuning from planner feedback. It requires a separate
  privacy, consent, and leakage decision and is not implied by this ADR.

## Action items

1. Extend FNDR-Bench fixtures with export/proposal metadata without placing
   private captures in version control.
2. Define the planner usefulness rubric and the alpha comparison script.
3. Add approval/capability adversarial tests before enabling Connected Planner.
4. Add a milestone evidence report template and link it from the final demo
   runbook.
