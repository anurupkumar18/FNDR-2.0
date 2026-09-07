//! The MCP surface (ADR-007). Six of the 14 founding tools are wired:
//! `fndr.search` (over `fndr-retrieval::KeywordRetriever`, not ADR-007's full
//! hybrid/filtered contract yet), `fndr.privacy_status`, `fndr.timeline`
//! (activity counts only, never capture text), `fndr.source_evidence`
//! (capture text behind an explicit `include_raw` gate that defaults
//! closed), `fndr.recall` (decisions only; unbacked kinds are refused, not
//! answered empty), and `fndr.remember_decision` (the only write tool: appends to
//! `fndr-store::Store::remember_decision`'s append-only ledger, never edits
//! or removes). `fndr-store::Store` replaced the walking-skeleton
//! `SkeletonStore` stand-in as of T-702.
//!
//! ADR-007's flexible `time_window` shorthand is not implemented; tools that
//! take a window take explicit unix-ms bounds until a second caller needs
//! the shared parser.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::body::Body;
use axum::http::Request;
use axum::middleware::{self, Next};
use axum::response::Response;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ErrorData, tool, tool_router};
use serde::{Deserialize, Serialize};

use fndr_privacy::Blocklist;
use fndr_retrieval::KeywordRetriever;
use fndr_store::{Store, TimelineGranularity};

use crate::auth::{AuthConfig, RateWindow, check_request};

#[derive(Debug, thiserror::Error)]
pub enum ServeError {
    #[error("bind failed: {0}")]
    Bind(#[from] std::io::Error),
}

#[derive(Debug, Deserialize, schemars::JsonSchema, Default)]
pub struct SearchParams {
    /// FTS query over everything FNDR has remembered.
    pub query: String,
    /// Maximum results (default 10, capped at 50).
    pub limit: Option<u32>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct SearchHitOut {
    pub record_id: String,
    pub chunk_id: String,
    pub source: String,
    pub captured_at_ms: f64,
    pub snippet: String,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct SearchOutput {
    pub hits: Vec<SearchHitOut>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema, Default)]
pub struct PrivacyStatusParams {}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct PrivacyStatusOutput {
    pub local_default: bool,
    pub planner_enabled: bool,
    pub configured_blocked_apps: u32,
    pub configured_blocked_domains: u32,
    pub raw_pixels_persisted: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema, Default, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TimelineGrain {
    Hour,
    #[default]
    Day,
}

#[derive(Debug, Deserialize, schemars::JsonSchema, Default)]
pub struct TimelineParams {
    /// Start of the window, unix ms, inclusive.
    pub from_ms: i64,
    /// End of the window, unix ms, inclusive.
    pub to_ms: i64,
    /// Bucket width. Defaults to `day`.
    pub granularity: Option<TimelineGrain>,
    /// Minutes east of UTC, so buckets land on the caller's local
    /// day/hour boundaries. Defaults to 0 (UTC).
    pub utc_offset_minutes: Option<i64>,
    /// Maximum buckets returned (default 200, capped at 1000).
    pub limit: Option<u32>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ActivityBucketOut {
    pub bucket_start_ms: f64,
    pub app_name: String,
    pub record_count: i64,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct TimelineOutput {
    pub from_ms: f64,
    pub to_ms: f64,
    pub granularity: String,
    pub buckets: Vec<ActivityBucketOut>,
    /// True when `limit` truncated the result, so a caller never reads a
    /// clipped timeline as a complete one.
    pub truncated: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema, Default)]
pub struct SourceEvidenceParams {
    /// A `record_id` from a `fndr.search` hit.
    pub record_id: String,
    /// Opt in to the stored capture text itself. Off by default: metadata
    /// and chunk shape answer "what is this?" without moving the content.
    pub include_raw: Option<bool>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ChunkEvidenceOut {
    pub chunk_id: String,
    pub ord: i64,
    /// Length of the stored text, always present, so a caller can judge a
    /// record's substance without reading it.
    pub text_len: u32,
    /// Present only when `include_raw` was explicitly true.
    pub text: Option<String>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct SourceEvidenceOutput {
    pub record_id: String,
    pub session_id: String,
    pub source: String,
    pub app_name: String,
    pub bundle_id: Option<String>,
    pub url: Option<String>,
    pub window_title: String,
    pub captured_at_ms: f64,
    pub chunks: Vec<ChunkEvidenceOut>,
    /// Echoes whether raw text was included, so a caller never has to infer
    /// the gate's state from an absent field.
    pub raw_included: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema, Default, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RecallKind {
    #[default]
    Decision,
    Error,
    Blocker,
    Todo,
}

#[derive(Debug, Deserialize, schemars::JsonSchema, Default)]
pub struct RecallParams {
    /// What to recall. Only `decision` is backed by data today; the others
    /// are refused rather than answered with an empty list.
    pub kind: RecallKind,
    /// Only entries at or after this instant, unix ms.
    pub since_ms: Option<i64>,
    /// Maximum entries (default 20, capped at 200).
    pub limit: Option<u32>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct RecalledDecision {
    pub id: i64,
    pub decided_at_ms: f64,
    pub statement: String,
    pub record_id: Option<String>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct RecallOutput {
    pub kind: String,
    pub decisions: Vec<RecalledDecision>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema, Default)]
pub struct RememberDecisionParams {
    /// The decision, in the user's or agent's own words.
    pub statement: String,
    /// Optional record this decision was made about or in the context of.
    pub record_id: Option<String>,
    /// Unix ms the decision was made; defaults to now.
    pub decided_at_ms: Option<i64>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct RememberDecisionOutput {
    pub id: i64,
    pub decided_at_ms: i64,
}

#[derive(Clone)]
pub struct FndrMcpServer {
    // Mutex because rusqlite's Connection is Send but not Sync. The real
    // engine gets a proper connection strategy with T-201.
    store: Arc<Mutex<Store>>,
    blocklist: Blocklist,
}

#[tool_router(server_handler)]
impl FndrMcpServer {
    pub fn new(store: Store) -> Self {
        Self::with_blocklist(store, Blocklist::default())
    }

    pub fn with_blocklist(store: Store, blocklist: Blocklist) -> Self {
        Self {
            store: Arc::new(Mutex::new(store)),
            blocklist,
        }
    }

    #[tool(
        name = "fndr.search",
        description = "Full-text search over the local FNDR screen memory. Returns matching records with highlighted snippets, most relevant first."
    )]
    pub fn search(
        &self,
        Parameters(SearchParams { query, limit }): Parameters<SearchParams>,
    ) -> Result<Json<SearchOutput>, ErrorData> {
        let limit = limit.unwrap_or(10).min(50) as usize;
        let store = self
            .store
            .lock()
            .map_err(|_| ErrorData::internal_error("store lock poisoned", None))?;
        let hits = KeywordRetriever::new(&store)
            .search(&query, limit)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(Json(SearchOutput {
            hits: hits
                .into_iter()
                .map(|h| SearchHitOut {
                    record_id: h.record_id,
                    chunk_id: h.chunk_id,
                    source: h.source,
                    captured_at_ms: h.captured_at_ms as f64,
                    snippet: h.snippet,
                })
                .collect(),
        }))
    }

    #[tool(
        name = "fndr.privacy_status",
        description = "Reports FNDR's active local privacy posture and configured blocklist counts without exposing blocklist entries."
    )]
    pub fn privacy_status(
        &self,
        Parameters(PrivacyStatusParams {}): Parameters<PrivacyStatusParams>,
    ) -> Result<Json<PrivacyStatusOutput>, ErrorData> {
        Ok(Json(PrivacyStatusOutput {
            local_default: true,
            planner_enabled: false,
            configured_blocked_apps: self.blocklist.app_count(),
            configured_blocked_domains: self.blocklist.domain_count(),
            raw_pixels_persisted: false,
        }))
    }

    #[tool(
        name = "fndr.timeline",
        description = "Grouped chronological activity: which apps were active, in which time buckets, over a window. Returns counts only, never capture text."
    )]
    pub fn timeline(
        &self,
        Parameters(TimelineParams {
            from_ms,
            to_ms,
            granularity,
            utc_offset_minutes,
            limit,
        }): Parameters<TimelineParams>,
    ) -> Result<Json<TimelineOutput>, ErrorData> {
        if to_ms < from_ms {
            return Err(ErrorData::invalid_params(
                "to_ms must not precede from_ms",
                None,
            ));
        }
        let utc_offset_minutes = utc_offset_minutes.unwrap_or(0);
        if !(-(12 * 60)..=(14 * 60)).contains(&utc_offset_minutes) {
            return Err(ErrorData::invalid_params(
                "utc_offset_minutes must be within -720..=840",
                None,
            ));
        }
        let grain = granularity.unwrap_or_default();
        let limit = limit.unwrap_or(200).min(1_000) as usize;
        let store = self
            .store
            .lock()
            .map_err(|_| ErrorData::internal_error("store lock poisoned", None))?;
        let buckets = store
            .activity_buckets(
                from_ms,
                to_ms,
                match grain {
                    TimelineGrain::Hour => TimelineGranularity::Hour,
                    TimelineGrain::Day => TimelineGranularity::Day,
                },
                utc_offset_minutes,
                limit,
            )
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(Json(TimelineOutput {
            from_ms: from_ms as f64,
            to_ms: to_ms as f64,
            granularity: match grain {
                TimelineGrain::Hour => "hour".to_owned(),
                TimelineGrain::Day => "day".to_owned(),
            },
            truncated: buckets.len() == limit,
            buckets: buckets
                .into_iter()
                .map(|bucket| ActivityBucketOut {
                    bucket_start_ms: bucket.bucket_start_ms as f64,
                    app_name: bucket.app_name,
                    record_count: bucket.record_count,
                })
                .collect(),
        }))
    }

    #[tool(
        name = "fndr.source_evidence",
        description = "The evidence behind one memory: its capture metadata and chunk shape. The stored capture text is returned only when include_raw is explicitly true."
    )]
    pub fn source_evidence(
        &self,
        Parameters(SourceEvidenceParams {
            record_id,
            include_raw,
        }): Parameters<SourceEvidenceParams>,
    ) -> Result<Json<SourceEvidenceOutput>, ErrorData> {
        let include_raw = include_raw.unwrap_or(false);
        let store = self
            .store
            .lock()
            .map_err(|_| ErrorData::internal_error("store lock poisoned", None))?;
        let evidence = store
            .record_evidence(&record_id)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?
            .ok_or_else(|| ErrorData::invalid_params("unknown record_id", None))?;
        Ok(Json(SourceEvidenceOutput {
            record_id: evidence.record_id,
            session_id: evidence.session_id,
            source: evidence.source,
            app_name: evidence.app_name,
            bundle_id: evidence.bundle_id,
            url: evidence.url,
            window_title: evidence.window_title,
            captured_at_ms: evidence.captured_at_ms as f64,
            chunks: evidence
                .chunks
                .into_iter()
                .map(|chunk| ChunkEvidenceOut {
                    chunk_id: chunk.chunk_id,
                    ord: chunk.ord,
                    text_len: chunk.text.len() as u32,
                    text: include_raw.then_some(chunk.text),
                })
                .collect(),
            raw_included: include_raw,
        }))
    }

    #[tool(
        name = "fndr.recall",
        description = "Recall decisions, errors, blockers, or todos. Only kind=decision has a data model today; the other kinds are refused explicitly rather than returning an empty list that reads as 'you have none'."
    )]
    pub fn recall(
        &self,
        Parameters(RecallParams {
            kind,
            since_ms,
            limit,
        }): Parameters<RecallParams>,
    ) -> Result<Json<RecallOutput>, ErrorData> {
        // Invariant 4: an unbacked kind is a visible refusal, never an empty
        // success that an agent would report as "no errors were recorded".
        let unbacked = match kind {
            RecallKind::Decision => None,
            RecallKind::Error => Some("error"),
            RecallKind::Blocker => Some("blocker"),
            RecallKind::Todo => Some("todo"),
        };
        if let Some(kind) = unbacked {
            return Err(ErrorData::invalid_params(
                format!("recall kind '{kind}' has no data model yet; only 'decision' is recorded"),
                None,
            ));
        }

        let limit = limit.unwrap_or(20).min(200) as usize;
        let store = self
            .store
            .lock()
            .map_err(|_| ErrorData::internal_error("store lock poisoned", None))?;
        let decisions = store
            .recent_decisions(since_ms, limit)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(Json(RecallOutput {
            kind: "decision".to_owned(),
            decisions: decisions
                .into_iter()
                .map(|decision| RecalledDecision {
                    id: decision.id,
                    decided_at_ms: decision.decided_at_ms as f64,
                    statement: decision.statement,
                    record_id: decision.record_id,
                })
                .collect(),
        }))
    }

    #[tool(
        name = "fndr.remember_decision",
        description = "The only write tool: appends one entry to the local, append-only decision ledger. Never edits or removes prior entries and never mutates ranking."
    )]
    pub fn remember_decision(
        &self,
        Parameters(RememberDecisionParams {
            statement,
            record_id,
            decided_at_ms,
        }): Parameters<RememberDecisionParams>,
    ) -> Result<Json<RememberDecisionOutput>, ErrorData> {
        if statement.trim().is_empty() {
            return Err(ErrorData::invalid_params(
                "statement must not be empty",
                None,
            ));
        }
        let decided_at_ms = decided_at_ms.unwrap_or_else(now_ms);
        let store = self
            .store
            .lock()
            .map_err(|_| ErrorData::internal_error("store lock poisoned", None))?;
        let id = store
            .remember_decision(decided_at_ms, &statement, record_id.as_deref())
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(Json(RememberDecisionOutput { id, decided_at_ms }))
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

struct AuthState {
    config: AuthConfig,
    rate: RateWindow,
}

async fn auth_middleware(
    state: axum::extract::State<Arc<AuthState>>,
    req: Request<Body>,
    next: Next,
) -> Response {
    match check_request(&state.config, &state.rate, &req) {
        Ok(()) => next.run(req).await,
        Err(denied) => *denied,
    }
}

/// Bind the MCP surface on loopback and serve until the task is aborted.
/// `port` 0 picks an ephemeral port; the actual address is returned. The
/// bearer token is required on every request from the first byte served
/// (invariant 2: no "add auth later").
pub async fn serve_loopback(
    server: FndrMcpServer,
    token: String,
    port: u16,
) -> Result<(SocketAddr, tokio::task::JoinHandle<()>), ServeError> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    let addr = listener.local_addr()?;

    let auth = Arc::new(AuthState {
        config: AuthConfig::loopback(token, addr.port()),
        rate: RateWindow::new(),
    });

    let mcp_service = StreamableHttpService::new(
        move || Ok(server.clone()),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default(),
    );

    let router = Router::new()
        .nest_service("/mcp", mcp_service)
        .layer(middleware::from_fn_with_state(
            auth.clone(),
            auth_middleware,
        ))
        .with_state(auth);

    let handle = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, router).await {
            tracing::error!("mcp server exited: {e}");
        }
    });
    Ok((addr, handle))
}
