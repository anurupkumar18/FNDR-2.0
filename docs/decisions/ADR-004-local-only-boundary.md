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

## Amendment (2026-08-21, lance transitive HTTP client)

Bringing lancedb into fndr-store (T-202) surfaced one unavoidable transitive HTTP client: lance core hard-depends on lance-namespace, whose REST catalog client embeds reqwest. FNDR never constructs a remote namespace (tables open via local directory URIs only), no feature flag removes it, and forking is not worth the maintenance. Ruling: the cargo-deny ban carries a wrapper scoped to exactly `lance-namespace-reqwest-client`; reqwest under any other parent still fails CI, our own crates remain forbidden direct HTTP by the workspace lint, and the runtime egress posture is unchanged. Revisit at each lancedb upgrade PR; if lance ever feature-gates the namespace client, drop the wrapper.

## Amendment (2026-09-05, controlled external-planner export)

The semester team review introduces a narrowly scoped experimental use case:
the owner may choose to use an external planner, such as ChatGPT, for a
specific task after reviewing the exact FNDR context that would be shared.
This is not opt-in cloud capture, inference, storage, indexing, ranking, or
background synchronization. FNDR remains a local memory engine by default.

**Revised boundary:** FNDR itself still makes no network request using
captured data and still owns no cloud credentials. It may construct a local
`PlannerExport` only after explicit user approval, then expose that export to
a user-configured external planner through the authenticated local MCP
surface. The independently configured planner client, not FNDR, is
responsible for any later network transmission. This distinction keeps the
existing direct-egress lint meaningful while making the user-visible export
boundary honest rather than implicit.

**Alpha policy:** the mode is off by default. Before every export, FNDR must
show a stable preview containing the destination client, task, selected
record identifiers, field-level content, redactions, estimated token count,
and the fact that the external client may transmit the approved payload. The
owner approves or cancels that exact immutable payload. No auto-send, queued
retry, background refresh, clipboard injection, or reuse of a previous
approval is permitted. `include_raw` remains separately gated and raw pixel
data is never exportable.

**Data minimization and audit:** the export is assembled from the existing
local evidence pipeline after blocklist, sensitive-context, and redaction
policy. Its local audit event records payload digest, record identifiers,
redaction count, destination label, approval time, and outcome; it never
records a second copy of raw payload text. Owners can inspect and delete the
local audit history. Disabling the mode immediately prevents new exports and
invalidates any pending approval.

**Execution boundary:** an external planner may return a proposal only. It
cannot execute an action through FNDR in alpha. Any later action capability
must be separately allowed by ADR-008, use a named local capability, present
a per-action approval, and write an audit event before and after execution.

**Claims and verification:** product copy must say "local by default" and
name the controlled export mode when it is enabled. The existing CI egress
lint proves that FNDR has no direct captured-data egress; it does not prove
that an owner-approved external planner will keep a payload local. Tests for
this amendment must cover disabled mode, cancel, preview/payload digest
mismatch, sensitive-content exclusion, redaction visibility, one-time
approval, audit deletion, and refusal of every unapproved action request.

**Threat model:** a local attacker, a compromised planner client, or an
over-broad prompt can attempt to obtain more context than the owner intended.
The mitigations are a default-off mode, explicit preview of a frozen minimal
payload, the existing capture/redaction policy before export, authenticated
loopback MCP, one-time approvals, and a local audit trail. FNDR cannot make
an external provider retain or delete data differently; that residual risk is
shown at approval time rather than hidden by a local-only claim.

**Implementation prerequisites:** ADR-008 must define the planner and action
contracts; `docs/mcp.md` must document the export capability and scopes; a
privacy test suite must prove the policy above; and the alpha demo must show
both the local-default path and a visible approved-export path. Until these
artifacts and tests exist, no runtime external-planner mode ships.
