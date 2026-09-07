# 2026-09-06: fndr.feedback, and saying "no" in the response body

## Decision

ADR-007 specifies `fndr.feedback` as "rate a result (logged, never silently
mutates ranking)". The parenthesis is the whole design. Migration 0006 adds
`result_feedback`; nothing reads it into a ranker, and the response carries
`ranking_changed: false` as a field rather than leaving that to be inferred
from documentation nobody reads.

Stating it in the response matters because the alternative — a bare success
— leaves a caller free to assume their thumbs-down did something. v1's
failure mode was tuning constants against nothing; this is the read-side
version of the same discipline, and any future use of this data has to
arrive through ADR-006's bench gate.

## Two deliberate departures

**It stores the query text.** The audit log refuses to, on the grounds that
a log which copies what it audits becomes a second store of the same
sensitive text. Feedback stores it anyway, because feedback without the
query it was given for cannot be replayed as an eval case, and being
replayable is the only reason to collect it at all. The owner also
initiates each row explicitly, rather than it accruing from background
capture. Different purpose, different answer — recorded here so the
inconsistency is a decision and not an oversight.

**Its foreign key is `SET NULL`, not `CASCADE`.** Every other reference to
`memory_records` cascades, because deletion-everywhere means a deleted
memory leaves nothing behind. A rating is not part of the memory, though:
it is a record of how the system behaved when that memory surfaced.
Cascading would let deletion quietly rewrite eval history. So deleting a
rated memory nulls the citation and keeps the rating, and a store test pins
exactly that.

## What is verified

A rating round-trips with its id and reports `ranking_changed: false`. An
empty query is a typed refusal, with the reason in the message. At the
store level, deleting the rated record leaves the row with a null
`record_id` and its query and rating intact.

## Explicitly not done

No reader beyond `recent_feedback`, no export into the bench corpus, and no
aggregation. Building the pipeline from ratings to eval cases before there
are any ratings would be inventing requirements; the table and the tool are
the part that has to exist first.
