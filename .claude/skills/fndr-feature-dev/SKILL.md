---
name: fndr-feature-dev
description: Mandatory planning workflow for any NEW feature, capability, or product change in FNDR, before implementation starts. Use this whenever the task is deciding WHAT to build or HOW to cut it (a new feature idea, a product improvement, a demo capability, scoping a ticket that does not exist yet, or re-scoping one that does), even when the user does not say "plan". It front-loads the checks that make the first implementation attempt the right one, then hands off to fndr-v2-engineering for the build. For implementing an already-scoped ticket, go straight to fndr-v2-engineering.
---

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
