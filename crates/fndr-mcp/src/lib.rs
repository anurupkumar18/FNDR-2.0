//! MCP server: auth middleware, origin and host checks, rate limits, audit log, the canonical tool set (ADR-007).
//!
//! Twelve of the 14 ADR-007 tools are wired: `fndr.search`,
//! `fndr.context_pack`, `fndr.privacy_status`, `fndr.timeline`,
//! `fndr.delta`, `fndr.active_focus`, `fndr.source_evidence`,
//! `fndr.open_target`, `fndr.recall`, `fndr.explain_retrieval`,
//! `fndr.feedback`, `fndr.remember_decision`. Auth is real from this
//! first commit (invariant 2): bearer token required, constant-time compare,
//! Host allowlist, Origin allowlist, a crude global rate limit. The full
//! surface (all ADR-007 tools, scopes, audit log) is E07.

mod auth;
mod server;

pub use auth::{AuthConfig, generate_token};
pub use server::{
    ActiveFocusOutput, ActiveFocusParams, ActivityBucketOut, AppChangeOut, ChunkEvidenceOut,
    ContextPackOutput, ContextPackParams, DEFAULT_STALE_AFTER_MS, DeltaOutput, DeltaParams,
    ExplainRetrievalOutput, ExplainRetrievalParams, FeedbackOutput, FeedbackParams, FndrMcpServer,
    OpenTargetOutput, OpenTargetParams, PackedEvidence, PrivacyStatusOutput, PrivacyStatusParams,
    Rating, RecallKind, RecallOutput, RecallParams, RecalledDecision, RememberDecisionOutput,
    RememberDecisionParams, SearchHitOut, SearchOutput, SearchParams, SourceEvidenceOutput,
    SourceEvidenceParams, TimelineGrain, TimelineOutput, TimelineParams, serve_loopback,
};
