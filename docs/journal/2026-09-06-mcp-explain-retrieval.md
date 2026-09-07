# 2026-09-06: fndr.explain_retrieval, including its own blind spots

## Decision

ADR-007 describes this tool as "why results surfaced, what was dropped or
redacted". Half of that is answerable today and half is not, and the design
turns on saying which is which.

What it answers: the terms the FTS index actually sees after punctuation is
stripped, the expression built from them, that they are combined with AND,
how many chunks match in total, and how many a given limit would drop —
including the store's own ceiling of 50, which applies above any larger
requested limit.

The AND semantics are the most useful thing here. `search_chunks` joins
every term with AND, so one unmatched word empties the result. That is
almost certainly the top reason a real user says "FNDR found nothing", and
until now nothing in the system could say so.

## The two notes that say what it cannot do

`notes` carries two statements about the tool's own limits, and they matter
more than the counts:

**Privacy exclusion happens at capture, not retrieval.** Blocked or
redacted content was never stored, so it cannot appear here as dropped.
ADR-007's phrasing ("what was dropped or redacted") invites the assumption
that retrieval filters for privacy and could report on it. It does not. A
tool that reported "nothing redacted" without that context would be
literally true and deeply misleading — the honest answer is that this
question belongs to the capture-explain surface (T-1007), not here.

**Only the keyword route exists.** A miss here is not evidence that a
semantic search would also miss.

## What is verified

A punctuated query reduces to its terms and matches; a three-word query
where one word is absent returns zero matches and produces the AND note; a
query of pure punctuation yields no terms, no expression, and an empty
result rather than an error. The privacy note is asserted present, because
an explanation that quietly drops its own caveat is the failure mode.

## Refactor note

`fts_query` was split into `fts_terms` plus expression building so the
explanation can show terms without duplicating normalization. Behavior is
unchanged; the store's 50-result ceiling also became the named
`SEARCH_LIMIT_CAP` rather than a literal, so the tool can report it instead
of a caller discovering it by receiving fewer rows than requested.

## Explicitly not done

No per-result scoring: this explains the query, not why one chunk outranked
another. bm25 scores exist inside the SQL and are not surfaced, because
exposing a raw score without a calibrated interpretation invites exactly
the unmeasured tuning ADR-006's bench gate is there to prevent.
