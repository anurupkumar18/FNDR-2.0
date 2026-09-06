## Handoff: capture-stage dependencies (2026-09-05)

Done: T-304 (`6f9f495`) ports the pure browser admission-policy seam; T-303
(`f11edd3`) ports perceptual dHash/color dedup, A-B-A detection, and the
bounded semantic window. Both commits are pushed to
`codex/a006-real-store-safety-seam`, browser-reviewed with no comments, and
each passed `make test`.

In flight: T-306 is now dependency-ready but not implemented. The next slice
must compose a real, continuously driven pipeline from capture context through
pre-OCR policy, SCK, dedup, admission, OCR, semantic dedup, the real write
seam, and a durable shutdown flush. It needs an explicit owner for foreground
metadata, record/session identity, and the long-lived model-worker handle.

Decisions: T-303 feeds `img_hash` only a 9 by 8 native RGBA sample from the
ScreenCaptureKit source, not a decoded PNG; CLI and fixture sources carry no
perceptual signature. T-304 remains metadata-only; the scheduler owns URL-only
record construction and persistence. The legacy `fndr` checkout was left
untouched; FNDR-2.0 is the canonical worktree.

Landmines: do not call T-306 complete for a generic test loop or the existing
examples; neither proves continuous capture or shutdown durability. The real
write seam requires caller-supplied IDs because T-307 owns the lasting
session/continuity contract. `img_hash` added the `image 0.23` dependency tree;
the full gate is green, but account for its compile cost. An untracked
`docs/journal/2026-09-05-claude-code-handoff-prompt.md` was present before this
work and must remain unstaged unless its owner requests otherwise.

Produced by: Anurup + Codex
