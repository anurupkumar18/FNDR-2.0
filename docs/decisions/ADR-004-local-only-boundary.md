# ADR-004: Local-only boundary: no captured data leaves the device, enforced mechanically

**Status:** Proposed
**Date:** 2026-08-19
**Deciders:** FNDR v2 team (4)

## Context

Local-only is FNDR's founding invariant and a locked product decision: capture, storage, embeddings, and any reasoning over captured data never leave the device. There is no opt-in cloud inference mode and no exception path. The POC honored this in behavior but enforced it only by convention; its docs also promised privacy properties (redaction, sensitive-context handling) that the audit found written but never wired, and its shipped network surfaces (MCP local mode, companion pairing) were open by default. v2 treats the boundary as an engineering artifact with tests, not a README promise.

## Decision

**The invariant:** no bytes derived from captured data (pixels, OCR text, embeddings, memory records, transcripts, graph, or any transformation of them) are ever transmitted off-device by FNDR. Permitted egress is exactly: (1) model artifact downloads from pinned, checksummed URLs, (2) the auto-update manifest and artifact check, both user-visible. Serving captured data to a device the user explicitly paired or an agent the user explicitly connected happens only over the authenticated MCP/companion surfaces (ADR-007), which default to loopback.

**Enforcement layers:**

1. **Dependency gate (CI):** engine crates other than the dedicated `downloader` and `updater` integration may not depend on HTTP client crates (reqwest, hyper client, ureq, curl bindings). Checked by `cargo deny` / a workspace lint in CI; violations fail the build.
2. **Egress allowlist test:** the downloader's permitted hosts and pinned URLs live in one reviewed constants module with a test asserting no other module constructs URLs.
3. **Network posture:** MCP and companion servers bind loopback by default; non-loopback binds require explicit mode plus auth (ADR-007). No telemetry, crash reporting, or analytics of any kind exists in the codebase.
4. **Data-at-rest posture:** no raw screenshot persistence (test-asserted, carried from POC ADR-004); the safety gate (allow/redact/skip) runs on the storage write path with adversarial tests (PRD P0.4); tokens and discovery files are written with owner-only permissions.
5. **Documentation:** a `PRIVACY.md` states the boundary, the exact egress list, and how to verify both from source.

## Options considered

**A (chosen): hard local-only, mechanical enforcement.** Matches the locked product decision; converts the differentiator into verifiable engineering.

**B: local-first with opt-in cloud inference.** Explicitly ruled out by discovery. Would poison the positioning (every privacy claim becomes conditional) and add a consent/redaction surface bigger than the feature.

**C: local-only by policy (POC status quo).** Rejected: the audit showed convention drifts, and reviewers cannot distinguish a promise from a property.

## Trade-off analysis

The cost of Option A is real: no cloud reranking or synthesis quality escape hatch, and the downloader/updater must be carefully carved out rather than sprinkled `reqwest` usage. The benefit is the product's entire trust story becoming checkable in CI, which is also the strongest possible portfolio claim ("the build fails if captured data could leave the device").

## Consequences

- Easier: security review, positioning, the PRD P0.1 gate.
- Harder: any future feature wanting network access must amend this ADR first (deliberate friction).
- Revisit: only if the product direction changes fundamentally; the future companion relay (P2) must be designed as end-to-end encrypted transport that the relay cannot read, and gets its own ADR when built.

## Action items

1. [ ] `cargo deny` config plus the workspace egress lint in CI (repo bootstrap, month 1).
2. [ ] Adversarial test suite for the safety gate classes (secrets, password managers, banking/medical, blocklist) (month 3).
3. [ ] `PRIVACY.md` with the verification recipe (month 3, before the demo gate).
