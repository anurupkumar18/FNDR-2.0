//! MCP server: auth middleware, origin and host checks, rate limits, audit log, the canonical tool set (ADR-007).
//!
//! Nine of the 14 ADR-007 tools are wired: `fndr.search`,
//! `fndr.privacy_status`, `fndr.timeline`, `fndr.delta`,
//! `fndr.active_focus`, `fndr.source_evidence`, `fndr.open_target`,
//! `fndr.recall`, `fndr.remember_decision`. Auth is real from this
//! first commit (invariant 2): bearer token required, constant-time compare,
//! Host allowlist, Origin allowlist, a crude global rate limit. The full
//! surface (all ADR-007 tools, scopes, audit log) is E07.

mod auth;
mod server;

pub use auth::{AuthConfig, generate_token};
pub use server::{
    ActiveFocusOutput, ActiveFocusParams, ActivityBucketOut, AppChangeOut, ChunkEvidenceOut,
    DEFAULT_STALE_AFTER_MS, DeltaOutput, DeltaParams, FndrMcpServer, OpenTargetOutput,
    OpenTargetParams, PrivacyStatusOutput, PrivacyStatusParams, RecallKind, RecallOutput,
    RecallParams, RecalledDecision, RememberDecisionOutput, RememberDecisionParams, SearchHitOut,
    SearchOutput, SearchParams, SourceEvidenceOutput, SourceEvidenceParams, TimelineGrain,
    TimelineOutput, TimelineParams, serve_loopback,
};
