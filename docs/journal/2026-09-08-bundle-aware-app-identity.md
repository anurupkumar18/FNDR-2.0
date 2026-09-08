# Handoff: app identity is typed and bundle-first (2026-09-08)

## Done

Resumed the 2026-09-07 capture-textsignal takeover and cleared its required
gates, then landed the next capture-correctness slice named in T-306.

The inherited classifier decided what an app was by substring-matching its
localized name. `AppIdentity`/`AppClass` in `fndr-textsignal` now resolve the
class from `CFBundleIdentifier` (exact, then family prefix) and fall back to
whole-token name matching only when the identifier is absent or unknown. The
`OcrRecognizer` contract carries the bundle ID beside the name, and
`MacOSForegroundContextSource::browser_kind` matches the same way.

Three independent `is_browser_app`/`is_code_app`/`is_mail_app` booleans became
one enum, so the precedence rule is explicit rather than implicit in the order
of `if` arms.

## Why this was worth a slice

Substring matching was wrong in both directions. "Search" contains `arc` and
"Knowledge Base" contains `edge`, so both classified as browsers; a renamed or
non-English Chrome, or Arc (`company.thebrowser.Browser`), classified as
nothing.

In `fndr-textsignal` that only mis-tuned line thresholds. In
`fndr-capture::foreground` the same matcher selects which browser's AppleScript
dictionary to query, so a lookalike app name would have asked a *backgrounded*
browser for its front tab and attributed that title and URL to a different
app's screenshot, through the privacy gate and into storage. That is the more
serious half of the fix and it is why the slice covers both call sites.

## Verification

At `9647537`, in the clean worktree:

- `CARGO_BUILD_JOBS=1 make test` exit 0, 44 suites green
- `CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 make bench`:
  `Recall@5=1.0000 MRR@10=1.0000`, `baseline check: ok`
- `scripts/gen-agents-md.sh --check` in sync, `git diff --check` clean

13 new tests. All 17 pre-existing cleanup tests pass unchanged, which is the
evidence that this is a tightening rather than a retune.

Bundle identifiers were read from `CFBundleIdentifier` on a real macOS install
where the app was present, rather than recalled. The table holds only
identifiers worth staking a claim on: an omission degrades to the name
fallback, but a wrong entry would produce a confidently wrong class.

## Landmines

- `make test` in a fresh worktree fails at the UI lane with
  `tsc: command not found` until `npm ci` runs in `ui/`. The whole Rust
  workspace passes first, so it reads like a regression and is not one. Now in
  `lessons.md`.
- `org.mozilla.` is not a browser prefix rule, because it also covers
  Thunderbird. Family prefixes need this check case by case.

## Known gaps

- Forward-only. Records written before this change are not reclassified or
  re-embedded.
- `is_useful_snippet_line` and `reduce_chrome_noise_for_app` still gate on byte
  length where the high-signal builder now counts characters: the same
  multilingual bias class as the 2026-09-07 correction, in the snippet path
  rather than the storage path.
- Still open on T-306: content-free cleanup-quality aggregates in capture
  health, a safe reprocessing path for older raw-OCR records, a bounded
  hardware permission run, and T-902 permission UX.

## Produced by

Claude Opus 5 (Claude Code), worktree `/tmp/fndr-round1.xDZQZ8`, branch
`codex/capture-textsignal-wiring`, PR #23.
