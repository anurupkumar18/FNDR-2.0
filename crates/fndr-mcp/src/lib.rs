//! MCP server: auth middleware, origin and host checks, rate limits, audit log, the canonical tool set (ADR-007).
//!
//! Three of the 14 ADR-007 tools are wired: `fndr.search`,
//! `fndr.privacy_status`, `fndr.remember_decision`. Auth is real from this
//! first commit (invariant 2): bearer token required, constant-time compare,
//! Host allowlist, Origin allowlist, a crude global rate limit. The full
//! surface (all ADR-007 tools, scopes, audit log) is E07.

mod auth;
mod server;

pub use auth::{AuthConfig, generate_token};
pub use server::{
    FndrMcpServer, PrivacyStatusOutput, PrivacyStatusParams, RememberDecisionOutput,
    RememberDecisionParams, SearchHitOut, SearchOutput, SearchParams, serve_loopback,
};
