# 2026-09-06: fndr.source_evidence and its include_raw gate

## Decision

`fndr.search` returns snippets and stable IDs; nothing could turn one of
those IDs back into the evidence behind it. `fndr.source_evidence` closes
that loop through one new engine read, `Store::record_evidence`, which
returns a record's retained metadata plus its chunks' stored text in capture
order. The storage boundary hands back the text; the MCP surface decides
whether it may leave.

## What is verified

The gate defaults closed. Without `include_raw: true`, a caller gets
metadata, chunk ids, ords, and each chunk's `text_len`, and no `text` field
at all; the test asserts the stored string never appears in that response.
With the flag explicitly true, the text is returned and `raw_included` says
so. `raw_included` is always present precisely so a caller never has to
infer the gate's state from a missing field, which is the shape that lets a
silent redaction pass for an empty record. An unknown `record_id` is a typed
`invalid_params` refusal, not an empty success.

`fndr-store` covers the read directly: chunks come back ordered by `ord`
regardless of insertion order, and an unknown record is `None` rather than
an error.

## Explicitly not done

No time-window or app filters, no evidence for a "card" (there are no cards
yet), and no audit-log entry when raw text is released. That last one
matters: ADR-007 wants an audit log of tool calls, and a raw-text release is
exactly the event worth recording. It is unstarted for every tool, not just
this one, and stays with E07.

## Landmines

`include_raw` is the only thing standing between an agent and stored capture
text, so it must stay explicit and default-false at every future layer that
wraps it (`fndr.context_pack`'s planner export already has the same gate in
ADR-008). Never widen it to "true when a scope allows it"; the scope and the
per-call flag are separate on purpose.
