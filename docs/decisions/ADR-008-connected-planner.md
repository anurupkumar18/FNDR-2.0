# ADR-008: Connected Planner: local evidence, controlled export, proposal-only actions

**Status:** Proposed
**Date:** 2026-09-05
**Deciders:** FNDR v2 team (4)

## Context

FNDR's main value is a trustworthy local memory layer, while the semester
review identifies a complementary workflow: an owner may want an external
planner to turn selected, cited context into a next-step proposal. The v1 POC
already contains an agent-runner execution surface, but ADR-005 explicitly
places that surface on the discard list because it combined orchestration,
tools, and approval behavior in a large, difficult-to-audit module.

The product must not turn a planner into an ambient exfiltration channel or an
autonomous actor. The local default and the controlled-export amendment to
ADR-004 remain binding. This ADR defines the narrow contract that lets the
alpha demo prove useful planning without importing the v1 agent runner or
weakening those boundaries.

## Decision

### 1. Planner mode is a separately enabled local capability

Connected Planner is disabled by default. Enabling it exposes no captured
data and executes nothing. It only permits an owner to create an
approval-bound `PlannerExportDraft` for a named task and a named
user-configured planner client. FNDR does not own a provider API key, make an
outbound planner request, retry delivery, or retain provider responses as
memory by default.

### 2. Context is assembled locally, cited, minimal, and frozen before approval

The draft is produced by the same local retrieval and evidence-composition
engine that serves the UI and MCP. It must include only the records selected
by the owner or justified by the cited retrieval result, after all blocklist,
sensitive-context, redaction, token-budget, and raw-text gates have run.

Before export, FNDR renders an immutable preview with:

- destination label and task;
- each source record and citation;
- every included field and every redaction;
- estimated token count and a payload digest;
- a plain-language statement that the destination client may send the
  approved payload to an external provider; and
- cancel and approve controls with no preselected approval.

Approval creates a one-time `ExportApproval` bound to that digest, destination,
and expiry. A changed task, destination, payload, record lifecycle, or policy
version invalidates the approval. A planner cannot request a wider export by
modifying its own response.

### 3. Planner output is untrusted and proposal-shaped

The external planner may return a `PlannerProposal`: structured next steps,
citations to the export evidence, and zero or more `ActionProposal` items. A
proposal is display data, not an instruction to execute. It is stored locally
only when the owner explicitly saves it, and its provenance states the planner
label, export digest, and creation time. Unsourced claims display as
unverified; an unavailable planner or invalid response becomes a visible typed
state, never an empty successful plan.

### 4. Alpha supports proposals and two narrow capabilities only

The alpha implementation may recognize only these proposal capabilities:

| Capability | Effect | Alpha rule |
| --- | --- | --- |
| `memory.open_target` | Resolve a cited memory's reopen target for display | Never opens an application automatically; the owner chooses any subsequent open action. |
| `git.status.short` | Read the short status of an owner-selected local repository | Requires an exact approved repository path; no shell interpolation, mutation, network command, or recursive traversal. |

Neither capability is available to a planner until the owner previews and
approves the exact proposal. They are not generalized shell access, browser
automation, file writes, clipboard access, or task execution. All other
capability identifiers are rejected with a typed `CapabilityUnavailable`
state and an audit event.

### 5. Every later action is allowlisted, approval-gated, and auditable

Any post-alpha capability must be declared in one static allowlist with a
machine-readable identifier, argument schema, risk label, timeout, and
rollback or irreversibility note. Execution requires a new per-action preview
after the planner response arrives; export approval never authorizes an
action. FNDR records proposed, approved, started, succeeded, failed, denied,
expired, and cancelled outcomes locally. A capability may not self-escalate,
chain into another capability, or execute after a restart without renewed
approval.

### 6. Runtime skills are local review artifacts, not executable plugins

Reusable runtime skills live in `runtime-skills/<skill-id>/SKILL.md` with
front matter for id, version, allowed capability ids, input schema reference,
and owner. They are exposed to connected planners as read-only MCP resources
only after local validation. A skill cannot embed credentials, invoke shell
commands, download code, add a capability, or bypass the policy table. Skill
edits invalidate their prior review state and are visible in the local audit.

## Contract shapes

The first implementation must define generated Rust/IPC types for the
following boundary objects before wiring UI or MCP behavior:

| Type | Required fields |
| --- | --- |
| `PlannerExportDraft` | `id`, `task`, `destination`, `policy_version`, `evidence`, `redactions`, `token_estimate`, `payload_digest`, `expires_at` |
| `ExportApproval` | `draft_id`, `payload_digest`, `destination`, `approved_at`, `expires_at`, `status` |
| `PlannerProposal` | `id`, `export_digest`, `planner_label`, `steps`, `citations`, `actions`, `verification_state` |
| `ActionProposal` | `id`, `capability_id`, `arguments`, `risk_label`, `rationale`, `evidence_citations` |
| `ActionApproval` | `action_id`, `argument_digest`, `approved_at`, `expires_at`, `status` |
| `ActionAuditEvent` | `action_id`, `capability_id`, `status`, `occurred_at`, `reason`, `result_summary` |

IDs are opaque and non-guessable. Digests cover a canonical serialization, so
field ordering cannot create approval mismatches. Payload text is not copied
into audit events. All statuses are persisted enums, not free-form strings.

## Options considered

**A (chosen): local evidence plus controlled export and proposal-only
capabilities.** It makes the external planner useful while preserving a
verifiable local default and a small auditable alpha surface.

**B: app-owned provider integration with an opt-in API key.** Rejected for
alpha. It would add a new egress client, credential storage, retry semantics,
provider-specific privacy behavior, and a substantially larger review surface.

**C: reuse the v1 agent runner.** Rejected by ADR-005. Its structure is the
exact coupling that the new product is avoiding.

**D: planner with unrestricted shell or browser tools.** Rejected. It cannot
produce an understandable consent boundary or a credible demo safety story.

## Consequences

- Easier: the alpha demo can show a real plan grounded in local evidence while
  maintaining a strict no-autonomy story.
- Harder: every planner and capability change has both a privacy and a
  product-contract cost; proposal quality must be evaluated separately from
  retrieval quality.
- Deferred: app-owned provider integrations, arbitrary tool execution,
  browser control, and plugin marketplaces. They need separate ADRs and are
  not implied by this decision.

## Required tests and demo evidence

1. Disabled mode cannot create an export draft or expose planner resources.
2. A payload changes after preview, an expired approval, and a destination
   change each prevent export.
3. Blocklisted and sensitive-context records are absent; redactions are
   visible in the preview and represented in the digest.
4. A proposal cannot execute a capability; every action needs an independent,
   one-time approval after the proposal arrives.
5. Unknown capability ids, malformed arguments, non-allowlisted paths, and
   shell metacharacters are refused with typed outcomes and audit events.
6. `memory.open_target` and `git.status.short` work only with their narrow
   alpha contract and leave no mutation or network side effect.
7. The alpha demo shows the local-default path, approved context preview,
   cited proposal, cancelled action, and the local audit trail.

## Action items

1. Amend ADR-007 with the resource and tool additions, including schema,
   auth-failure, scope, rate-limit, and documentation requirements.
2. Add a privacy/surface test suite for drafts, approvals, and action refusal.
3. Add runtime-skill validation and two reviewed alpha skill files.
4. Add an eval plan for proposal usefulness and citation support before
   promoting planner output beyond the alpha demonstration.
