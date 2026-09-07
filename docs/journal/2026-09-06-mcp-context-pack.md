# 2026-09-06: fndr.context_pack, and three fields that limit the claim

## Decision

ADR-007 calls `fndr.context_pack` the headline, and the PRD's month-3 demo
turns on it: the identical agent task with FNDR off (the agent interrogates
the user) and with FNDR on (one tool call). This is the tool that has to
make that cut real.

v1 runs the keyword route for the goal, then packs stored capture text until
an estimated token budget is spent, citing record, chunk, app, window title,
URL, and capture time on every item.

## Three fields that exist to limit the claim

A context pack is the tool most likely to be over-trusted, because its
output looks like an answer. So three fields bound what it is asserting:

- `retrieval_route` says `keyword`. There is no vector or hybrid route yet.
  Without this, a caller getting a thin pack would reasonably assume
  semantic recall had been tried and found nothing, when in fact only
  literal terms were matched.
- `estimated_tokens_used` says *estimated*, at four characters per token.
  There is no tokenizer on this path and loading one to budget a text pack
  would be absurd, so the number is honest about being approximate instead
  of dressing a heuristic as an exact count.
- `dropped_for_budget` counts records that matched but did not fit. Without
  it, a thin pack and a thin memory look identical.

## The audit consequence

`fndr.source_evidence` puts capture text behind an `include_raw` gate that
defaults closed. `fndr.context_pack` cannot have that gate: carrying capture
text is the entire point of the tool. So it passes `raw_released: true`
unconditionally to the audit log, and a test asserts the audit entry says
so. The gate and the log are separate mechanisms, and the tool that cannot
have the gate is exactly the one that most needs the log.

## What is verified

A packed item carries the text with its record and chunk citation and the
capturing app. A budget of one estimated token drops the chunk and reports
`dropped_for_budget: 1` with an empty item list, rather than silently
returning nothing. An empty goal is a typed refusal. The audit entry for a
pack is flagged as a raw release.

A record deleted between retrieval and packing is skipped rather than cited,
so a pack never hands back a citation to something that no longer exists.

## Explicitly not done

ADR-007's `depth` parameter, relevance gating, and diversity are unbuilt:
items come back in the keyword route's order, and a low-scoring match is
packed the same as a strong one. No re-ranking, no dedup across chunks of
the same record, no time or app filters. Those are ranking behaviors and
belong behind ADR-006's bench gates, not slipped in under a packing tool.
