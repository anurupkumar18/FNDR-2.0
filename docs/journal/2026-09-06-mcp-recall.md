# 2026-09-06: fndr.recall, and refusing three of its four kinds

## Decision

`fndr.remember_decision` shipped write-only earlier today; that journal
entry recorded the missing reader as an open gap. `fndr.recall` closes it.
`Store::recent_decisions` reads the ledger newest first, bounded by an
inclusive `since_ms` and a limit.

ADR-007 specifies `fndr.recall` as one tool with a `kind` parameter across
decisions, errors, blockers, and todos. Only decisions have a data model.
The other three kinds return a typed `invalid_params` refusal naming the
kind, not an empty list.

## What is verified

The full loop is pinned by one test: a statement written through
`fndr.remember_decision` comes back through `fndr.recall` with its
statement and timestamp intact. A second test asserts all three unbacked
kinds refuse.

Store-level tests cover newest-first ordering, `since_ms` being inclusive
of its own instant, and `limit` clipping to the newest entries.

## Explicitly not done

Errors, blockers, and todos. The `tasks` table exists in schema v1 and
could have backed `todo` with a query, but nothing writes tasks yet, so
that query would return an empty list for every caller. That is the exact
failure this refusal is designed to avoid, so the table's existence was
not treated as a reason to wire it.

## Landmines

The reason these kinds refuse rather than return `[]` is invariant 4, not
fussiness: an agent that receives an empty list will tell the user "no
errors were recorded", which is a confident false statement about their
own memory. Whoever implements the remaining kinds should delete the
refusal arm only when there is a writer producing that kind, not when a
table merely exists to read from.
