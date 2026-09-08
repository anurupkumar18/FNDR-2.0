# Handoff: app-aware OCR evidence reaches durable memory (2026-09-07)

## Outcome

The real desktop capture path now applies the already-ported
`fndr-textsignal::build_high_signal_text_for_app` policy at the
`VisionOcrAdapter` boundary. Semantic deduplication, final privacy evaluation,
SQLite/FTS storage, and later Lance indexing therefore receive the same cleaned
text rather than marker-bearing OCR output. This is a forward-only quality
improvement: captures written before this change are not rewritten or
re-embedded.

The OCR contract now receives the foreground app name explicitly because the
cleanup policy is app-aware. The pipeline remains policy-agnostic and still has
one OCR boundary; no retrieval or storage stack was added.

## User and demo value

Fresh captures are less likely to return browser navigation labels or internal
`[LOW_CONF]` annotations when a user searches FNDR or asks a connected agent
for context. Browser, terminal, mail, multilingual, and literal-marker fixtures
cover common dogfooding surfaces, and a composition test proves that the
cleaned browser evidence—not the raw fixture—is what durable FTS can retrieve.

This does not prove a live-screen experience or improve already-stored records.
For an honest demo, use a fresh consented desktop capture after installing this
build. The alpha runbook's deterministic MCP skeleton bypasses this shell
adapter, so its fixture does not demonstrate the new wiring. Screen Recording
permission, live content choice, and long-running capture quality remain
operator-only checks.

## Donor provenance and intentional corrections

The cleanup policy was ported from
`reference/v1:src-tauri/src/capture/text_cleanup.rs` under ADR-005. Wiring it
exposed two defects that are intentionally not carried forward:

- `[LOW_CONF]` is recognized only as the OCR engine's exact line prefix, so a
  literal token inside captured source code remains evidence.
- admission and line-quality math use Unicode character counts rather than
  UTF-8 byte counts, avoiding a systematic multilingual scoring bias.

## Verification

Passed locally with single-job builds and debug symbols disabled:

- `cargo test -p fndr-ocr -p fndr-capture -p fndr-shell --lib` (75 tests)
- `cargo test -p fndr-textsignal` (17 tests plus doc-tests)
- `cargo fmt --all --check` and `git diff --check`

`make bench` was started but deliberately interrupted when the operator asked
to stop for the usage-limit handoff. It remains the first required command in
the next session, followed by the full serial gate and CI. The committed sample
benchmark is an invariant/format gate; it does not contain raw OCR fixtures and
must not be presented as evidence that cleanup improved retrieval quality.

## Known limitations and next slice

- Safe aggregate cleanup statistics are not yet propagated into capture-health
  telemetry, so the UI cannot explain how many lines cleanup discarded.
- The inherited app classifier uses fuzzy name matching and does not yet use
  the known bundle ID; bundle-aware typed app identity should be the next
  capture-correctness slice.
- Existing SQLite chunks and Lance vectors need a separate, transactional
  reprocessing/reindex design; this branch deliberately does not mutate them.
