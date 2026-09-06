//! The MCP surface (ADR-007). Three of the 14 founding tools are wired:
//! `fndr.search` (over `fndr-retrieval::KeywordRetriever`, not ADR-007's full
//! hybrid/filtered contract yet), `fndr.privacy_status`, and
//! `fndr.remember_decision` (the only write tool: appends to
//! `fndr-store::Store::remember_decision`'s append-only ledger, never edits
//! or removes). `fndr-store::Store` replaced the walking-skeleton
//! `SkeletonStore` stand-in as of T-702.

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
use fndr_store::Store;

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
