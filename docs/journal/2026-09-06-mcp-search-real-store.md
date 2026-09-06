# 2026-09-06: fndr.search moves off the walking-skeleton store

## Decision

`fndr-mcp::FndrMcpServer` no longer holds a `fndr_store::SkeletonStore`. It
holds the durable `fndr_store::Store` and serves `fndr.search` through
`fndr_retrieval::KeywordRetriever`, the same FTS5 keyword route T-505 built
for the rest of the engine. `SkeletonStore` itself is untouched (`fndr-store`
still ships it; it dies only when every consumer moves off it, per its own
header comment) and remains the backing store for its own T-109 OCR
round-trip regression test and for `fndr-bench`'s FTS baseline corpus.

## What is verified

`SearchHitOut` gained a `chunk_id` field and `record_id` changed from `i64`
to `String` to match `KeywordRetriever::search`'s durable, stable IDs. All
three call sites that constructed `FndrMcpServer` with a store were updated:
`tests/auth_surface.rs` (trivial swap, auth behavior unaffected),
`tests/skeleton_e2e.rs` (the OCR round-trip test now inserts through
`Store::insert_capture`/`NewRecord`/`NewChunk` and searches through
`KeywordRetriever` before handing the same store to the tool), and
`examples/skeleton.rs` (same insert/search swap, plus reusing
`fndr_privacy::sanitize_url_for_storage` for the `--url` flag and
`Store::record_ids_for_delete(&DeleteScope::All)` for the record count,
rather than inventing new store accessors). `cargo test -p fndr-mcp` (12
tests, including the 3 named auth regression tests and the real-Vision-OCR
round trip) and a manual run of the `skeleton` example against a real PNG
fixture both pass.

## Explicitly not done

This does not implement ADR-007's full `fndr.search` contract (hybrid
ranking, time-window/app filters, surfacing reasons) or any of the other
named tools. It also does not retire `SkeletonStore` from the codebase —
only from the MCP surface.

## Landmines

Any future route added to `fndr.search` (vector, hybrid, temporal) is a
ranking change and needs `make bench` numbers per the eval-gate invariant,
even though this slice's route swap did not (no ranking constant, weight,
or algorithm changed; `KeywordRetriever` itself was untouched).
