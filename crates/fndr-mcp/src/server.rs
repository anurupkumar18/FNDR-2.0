//! The MCP surface. One tool for the walking skeleton: `fndr.search`.
//! The same engine function will later serve UI and MCP alike (ARCHITECTURE
//! section 6); for the skeleton the store call stands in for the engine API.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

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

use fndr_store::SkeletonStore;

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
    pub record_id: i64,
    pub source: String,
    pub captured_at_ms: f64,
    pub snippet: String,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct SearchOutput {
    pub hits: Vec<SearchHitOut>,
}

#[derive(Clone)]
pub struct FndrMcpServer {
    // Mutex because rusqlite's Connection is Send but not Sync. The real
    // engine gets a proper connection strategy with T-201.
    store: Arc<Mutex<SkeletonStore>>,
}

#[tool_router(server_handler)]
impl FndrMcpServer {
    pub fn new(store: SkeletonStore) -> Self {
        Self {
            store: Arc::new(Mutex::new(store)),
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
        let hits = store
            .search(&query, limit)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(Json(SearchOutput {
            hits: hits
                .into_iter()
                .map(|h| SearchHitOut {
                    record_id: h.record_id,
                    source: h.source,
                    captured_at_ms: h.captured_at_ms as f64,
                    snippet: h.snippet,
                })
                .collect(),
        }))
    }
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
