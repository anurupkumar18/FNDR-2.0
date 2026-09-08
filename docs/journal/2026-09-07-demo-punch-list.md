# Demo readiness punch list (2026-09-07)

Written after a hands-on session: the vector-search wiring landed
(PR #19), a trust-window design pass landed (PR #20), and the app was run
for real (desktop shell + MCP server, real capture, real OCR, real model).
This is the honest gap analysis against the PRD's own month-3 demo gate
(§10) and its pre-agreed cut lines (§10, §5), not a vibe.

## Where things actually stand

Verified this session by running the app, not by reading tickets:

- Real capture (ScreenCaptureKit) -> real OCR (Apple Vision) -> real local
  storage -> real search works end to end. `fndr.search` now merges
  keyword and vector routes (PR #19).
- The trust/status window has a real design system (tokens + components,
  lint-enforced) as of PR #20.
- **No product UI exists beyond that one status screen.** The root `ui/`
  (the React app ADR-001 planned) is empty. There is no way for a person
  to search or browse their own memories except through an MCP-connected
  agent or raw HTTP - confirmed by driving the running app directly.
- `fndr.context_pack` (the tool the PRD calls "the headline") is still
  keyword-only; only `fndr.search` got the vector route this session.
- Capture reliability against real target content (code, terminal, docs)
  is unmeasured - the only real capture observed this session was one
  video-content frame, with OCR quality issues inherent to stylized
  thumbnail text, not a pipeline bug.
- Onboarding, an installer, backup/export, and the warm-start file export
  do not exist yet.

## Punch list, ordered by leverage

### Phase 1 - the two gaps that matter most
1. **Minimal vault UI**: search + browse inside the app itself (not just
   via MCP). Reuses the PR #20 design tokens/components. Needs a new
   Tauri IPC command backed by the same `fndr-retrieval` routes
   `fndr.search` already uses (shared, not duplicated - see
   ARCHITECTURE.md section 4.2: "the same function serves Tauri IPC, MCP
   tools, and future companion routes").
2. **Capture reliability, measured for real**: run continuously for a
   real workday against real target content; check whether the
   `low_signal` quality gate is well-calibrated or too aggressive.

### Phase 2 - search completeness
3. Wire the vector route into `fndr.context_pack`, not just `fndr.search`.
4. Surface result quality signals (route, surfacing reason, citation) in
   the new UI, not bare text.

### Phase 3 - demo-gate requirements that are pure gaps today
5. `fndr doctor` / onboarding basics (permission flow, model download
   progress, connect-your-agent screen).
6. Backup/export (T-209) and the warm-start file export (T-1306, already
   PRD-promoted to P0 as "the daily retention loop").
7. Rehearse the privacy live-verification beat (bank/password-manager
   visit, prove absence) - likely already works mechanically (blocklist +
   safety gate exist); needs scripting, not building.

### Phase 4 - polish and rehearsal
8. Extend the PR #20 design system to whatever Phase 1 adds.
9. Dry-run the exact demo script twice on a clean state, per the PRD's own
   gate process.

### Explicitly cut for this demo (PRD's own pre-agreed cut lines)
Graph/3D view, meetings, omnibar, clipboard, proactive resurfacing, visual
similarity search, RRF/hybrid fusion, reranking, a real installer/DMG
(ad-hoc "right-click Open" is the accepted default per PRD section 13).

## Estimate

Phase 1 is substantial on its own (new UI surface + a real reliability
measurement pass): roughly 1-2 weeks focused. Phases 2-3 another 1-2 weeks.
Phase 4 is rehearsal, not new building. Honest estimate: **3-5 weeks** for
a demo a human can drive end to end, not just an agent - faster with more
than one person on Phase 1 and Phase 3 in parallel, since they don't block
each other.

Produced by: Anurup + Claude Code
