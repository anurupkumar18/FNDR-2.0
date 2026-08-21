# FNDR v2 invariants: checklists and rationale

Each invariant exists because the v1 audit found the opposite shipped. The v1 failure is named so reviewers can recognize the pattern returning.

## 1. Local-only (ADR-004)

v1 failure mode: local-only by convention, with privacy features documented but never wired.

Before merging, confirm:
- [ ] No new dependency on reqwest/hyper-client/ureq/curl outside `fndr-downloader` and `fndr-updater` (CI lint enforces; if it fires, the answer is a design change, not an exemption).
- [ ] No new URL construction outside the reviewed egress-constants module (its uniqueness test must still pass).
- [ ] Nothing derived from captured data (pixels, OCR text, embeddings, records, transcripts, graph) crosses a process or network boundary except the authenticated MCP/companion surfaces.
- [ ] No telemetry, crash reporting, or analytics dependency, ever.

## 2. Auth-always surfaces (ADR-007)

v1 failure mode: default-local MCP served the whole memory store to any web origin (auth off, CORS `Any`); the companion API let any LAN host mint a full-permission token via unauthenticated pair-start.

Before merging any listener or route change, confirm:
- [ ] Bearer auth required in every mode; comparison is constant-time.
- [ ] Origin and Host validated against an explicit allowlist; no wildcard CORS.
- [ ] Non-loopback binds require TLS; loopback may omit it (ADR-007).
- [ ] New route carries a permission scope and a rate limit; auth-failure test exists.
- [ ] Tokens and discovery files written owner-only; no secret ever logged.
- [ ] The two named regression tests still pass: `mcp_rejects_unauthenticated_loopback`, `companion_pair_start_not_network_reachable`.

## 3. Eval-gated ranking (ADR-006)

v1 failure mode: ~30 multiplicative rerank constants and multiple per-intent fusion-weight sets never measured against a real model; relevance evals ran a mock embedder; the flagship chunk path shipped disabled.

Before merging anything that can change result ordering, confirm:
- [ ] `make bench` run on real models; the delta table (Recall@5, MRR@10, latency) is in the PR description.
- [ ] Regressions are either justified in writing or the change is reworked; CI blocks silent regressions.
- [ ] New heuristics are named additive features with per-result attribution in `FusionSignals`, never anonymous multipliers.
- [ ] A new route or stage that cannot yet return real results does not ship enabled (no decorative plumbing).

## 4. No silent degradation (v1 ADR-012 carried forward)

v1 failure mode: missing embedder wrote zero-vector rows for weeks; mock embedder leaked into production paths and every eval; two LLM calls ran outside the model lock; `Embedder::new()` was constructed per query and per worker tick.

Before merging, confirm:
- [ ] Unavailability of a model, sidecar, or dependency surfaces as a typed state with a user-visible reason and, where applicable, a skip counter.
- [ ] No mock or fallback implementation is reachable in a production path.
- [ ] All llama.cpp work goes through the model-worker queue with a priority; no direct session use.
- [ ] Expensive resources (sessions, embedders) are constructed once and shared, never per-call.

## 5. Port provenance (ADR-005)

v1 failure mode: not v1's failure but this rebuild's biggest risk: wholesale copying would inherit the audited structural liabilities (monster modules, dual stacks, dead schemas).

Before merging ported code, confirm:
- [ ] The port is on the ADR-005 PORT list, or the PR argues for moving it there with audit/eval justification.
- [ ] It arrives as a targeted function, constant set, prompt, schema, or contract, with tests (v1's tests ported or new ones).
- [ ] It carries `// Ported from FNDR v1 <path>`; prompts are byte-identical unless a change is called out.
- [ ] Nothing on the DISCARD list is copied; consult it on the `reference/v1` branch only to understand history.

## Cross-cutting: the demo-gate priority rule

When any of these invariants conflicts with a deadline, the invariant wins and the scope moves. The PRD's pre-agreed cut lines exist precisely so schedule pressure never argues against an invariant. The spine (capture to retrieval to MCP) is never cut.
