# Semester review source archive

This directory preserves the three PDFs produced during the semester-start
FNDR planning session. They are primary inputs to the next product and
architecture decisions. The files are kept unchanged so that later ADRs,
implementation tickets, demos, and submission materials can trace a claim
back to the team discussion that produced it.

## Contents and integrity

| Source | Role | SHA-256 |
| --- | --- | --- |
| `01_FNDR_AI_Final_Review_Report.pdf` | External review of the product and architecture direction | `74b3b10ef7b4fcfd4ee761e8ea3445e34e08bd7d95cbab9107df3658751dcf15` |
| `02_FNDR_AI_Review_Prompts.pdf` | Exact prompts used to obtain the external review | `0870e7736ffc905c3f725ea5c6cabb8837fa49626e989456538a3d5fd639295b` |
| `03_FNDR_Team_Evaluation_of_AI_Findings.pdf` | Team decisions, accepted findings, and rejected assumptions | `f8cc2a0925f384e96c61abae87dc65db1da87c0dc1e442a3ae533ebcf8922e91` |

Regenerate the checksums after any intentional source replacement:

```sh
shasum -a 256 docs/review/semester/*.pdf
```

## How these sources govern the work

The team evaluation is the decision authority when it differs from the AI
review. The external report and its prompts are retained as evidence of the
questions asked and alternatives considered; neither overrides an accepted
ADR or a verified product constraint.

Derived work must cite this directory and distinguish among:

- a source observation;
- an accepted team decision;
- an implementation hypothesis to validate; and
- a completed, tested behavior.

Do not place real captures, private meeting content, or API credentials in
this archive. These three documents are planning artifacts only.

## Traceability queue

| Source theme | Planned destination | Required outcome before implementation |
| --- | --- | --- |
| A small, demoable local memory product | `docs/PRD.md` and `docs/ROADMAP-TICKETS.md` | Recut the three-month alpha, beta, and final scope around one end-to-end demo path. |
| Selected-context external planner option | New connected-planner ADR and an ADR-004 amendment | Define default-off mode, preview, explicit approval, redaction, audit, and a no-egress local default. |
| Action assistance and runtime skills | New connected-planner ADR and MCP contract amendment | Limit alpha to proposals and two read-only capabilities; require an allowlist and per-action approval before any execution capability. |
| Retrieval quality and evidence discipline | New evaluation ADR and benchmark tickets | Freeze a holdout set, require cited output, and block ranking changes without measured evidence. |
| Reuse of the v1 proof of concept | ADR-005 amendment and port tickets | Record the donor boundary, source commit, tests, and explicit exclusions for every port. |

No row in this table authorizes an implementation on its own. The relevant
ADR and ticket must define the contract, acceptance tests, privacy impact,
and rollback path first.
