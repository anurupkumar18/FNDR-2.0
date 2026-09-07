//! The MCP surface (ADR-007). Twelve of the 14 founding tools are wired:
//! `fndr.search` (over `fndr-retrieval::KeywordRetriever`, not ADR-007's full
//! hybrid/filtered contract yet), `fndr.context_pack` (budgeted, cited
//! capture text over that same keyword route), `fndr.privacy_status`,
//! `fndr.timeline`
//! and `fndr.delta` (both counts only, never capture text),
//! `fndr.active_focus` (newest capture plus its age and a typed staleness
//! status), `fndr.source_evidence`
//! (capture text behind an explicit `include_raw` gate that defaults
//! closed), `fndr.open_target` (sanitized URL or app, else an explicit
//! unavailable state), `fndr.recall` (decisions only; unbacked kinds are refused, not
//! answered empty), `fndr.explain_retrieval` (what the index made of a
//! query, including what it structurally cannot tell you),
//! `fndr.feedback` (recorded, and the response states
//! that ranking did not change), and `fndr.remember_decision` (the only write tool: appends to
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
use fndr_store::{AuditEntry, Store, TimelineGranularity};

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

#[derive(Debug, Deserialize, schemars::JsonSchema, Default)]
pub struct ExplainRetrievalParams {
    /// The query to explain. Nothing is returned from it; this reports how
    /// the index would read it.
    pub query: String,
    /// The limit the caller would have used, so the explanation can say
    /// what that limit would drop. Defaults to 10.
    pub limit: Option<u32>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ExplainRetrievalOutput {
    pub query: String,
    pub route: String,
    /// The query as the index reads it: punctuation stripped, empties gone.
    pub terms: Vec<String>,
    pub fts_expression: Option<String>,
    /// How terms combine. `all_terms` means one unmatched word empties the
    /// result, which is the most common reason a search "finds nothing".
    pub match_mode: String,
    pub total_matches: i64,
    pub would_return: i64,
    pub dropped_by_limit: i64,
    /// The store's own ceiling, applied even above a larger requested limit.
    pub store_limit_cap: i64,
    /// Plain-language notes about this result, including the ones about what
    /// this tool structurally cannot tell you.
    pub notes: Vec<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema, Default, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Rating {
    #[default]
    Helpful,
    Unhelpful,
    Irrelevant,
}

impl Rating {
    fn as_str(self) -> &'static str {
        match self {
            Self::Helpful => "helpful",
            Self::Unhelpful => "unhelpful",
            Self::Irrelevant => "irrelevant",
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema, Default)]
pub struct FeedbackParams {
    pub rating: Rating,
    /// The query or goal the rated result was surfaced for. Stored, unlike
    /// anything in the audit log, because feedback without its query cannot
    /// be replayed as an eval case.
    pub query: String,
    pub record_id: Option<String>,
    pub chunk_id: Option<String>,
    /// Optional free-text detail from the owner.
    pub note: Option<String>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct FeedbackOutput {
    pub id: i64,
    pub rating: String,
    /// Always false. Feedback is recorded and nothing reads it into a
    /// ranker; any future use has to arrive through ADR-006's bench gate.
    /// Stated in the response so a caller is never left assuming ratings
    /// quietly retrain something.
    pub ranking_changed: bool,
}

/// Characters per estimated token. FNDR has no tokenizer on this path and
/// loading one to budget a text pack would be absurd, so the budget is an
/// honest estimate and every field carrying it says `estimated`.
const CHARS_PER_ESTIMATED_TOKEN: usize = 4;

#[derive(Debug, Deserialize, schemars::JsonSchema, Default)]
pub struct ContextPackParams {
    /// What the caller is trying to do. Used as the retrieval query.
    pub goal: String,
    /// Approximate token ceiling for the packed text (default 2000, capped
    /// at 8000). Estimated, not tokenizer-exact; see `estimated_tokens_used`.
    pub token_budget: Option<u32>,
    /// Maximum records considered before budgeting (default 20, capped 100).
    pub max_records: Option<u32>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct PackedEvidence {
    pub record_id: String,
    pub chunk_id: String,
    pub app_name: String,
    pub window_title: String,
    pub url: Option<String>,
    pub captured_at_ms: f64,
    /// The stored capture text. A context pack exists to carry this; see
    /// the tool's note about auditing it as a raw release.
    pub text: String,
    pub estimated_tokens: u32,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ContextPackOutput {
    pub goal: String,
    /// Which retrieval route produced this. Today only `keyword`: there is
    /// no vector or hybrid route yet, and a pack that hid that would let a
    /// caller assume semantic recall it did not get.
    pub retrieval_route: String,
    pub token_budget: u32,
    pub estimated_tokens_used: u32,
    pub items: Vec<PackedEvidence>,
    /// Records that matched but did not fit the budget, so a thin pack is
    /// never mistaken for a thin memory.
    pub dropped_for_budget: u32,
}

/// How old the newest capture may be before `fndr.active_focus` stops
/// calling it current. Matches `fndr-capture`'s deep-idle threshold: past
/// five minutes of no input the sampler itself stops believing the screen
/// represents what someone is doing.
pub const DEFAULT_STALE_AFTER_MS: i64 = 5 * 60 * 1_000;

#[derive(Debug, Deserialize, schemars::JsonSchema, Default)]
pub struct ActiveFocusParams {
    /// Age past which the newest capture is reported `stale` instead of
    /// `active`. Defaults to five minutes.
    pub stale_after_ms: Option<i64>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ActiveFocusOutput {
    /// `active`, `stale`, or `none`. Never a bare app name that a caller
    /// could report as "currently" true when the observation is hours old.
    pub status: String,
    pub app_name: Option<String>,
    pub window_title: Option<String>,
    pub url: Option<String>,
    pub bundle_id: Option<String>,
    pub record_id: Option<String>,
    pub captured_at_ms: Option<f64>,
    /// How old the observation is, so staleness is measurable and not just
    /// a label.
    pub age_ms: Option<f64>,
    pub stale_after_ms: f64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema, Default)]
pub struct DeltaParams {
    /// Return what was captured at or after this instant, unix ms. Pass the
    /// previous response's `newest_captured_at_ms` to continue polling.
    pub since_ms: i64,
    /// Maximum apps listed (default 10, capped at 100). The totals always
    /// count every app, listed or not.
    pub app_limit: Option<u32>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct AppChangeOut {
    pub app_name: String,
    pub record_count: i64,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct DeltaOutput {
    pub since_ms: f64,
    /// Every record captured in the window, regardless of how many apps the
    /// `apps` list was capped to.
    pub record_count: i64,
    /// Newest capture instant in the window, or absent when nothing was
    /// captured. Feed it back as the next call's `since_ms`.
    pub newest_captured_at_ms: Option<f64>,
    pub apps: Vec<AppChangeOut>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema, Default)]
pub struct OpenTargetParams {
    /// A `record_id` from a `fndr.search` hit.
    pub record_id: String,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct OpenTargetOutput {
    pub record_id: String,
    /// `url`, `app`, or `unavailable`. Never an empty string standing in for
    /// "nothing to open".
    pub kind: String,
    /// Present for `url` targets. Already sanitized on the write path:
    /// credentials, query strings, and fragments never reached storage.
    pub url: Option<String>,
    /// Present for `app` targets.
    pub bundle_id: Option<String>,
    pub app_name: String,
    pub window_title: String,
    /// Present for `unavailable`: why this memory cannot be reopened.
    pub reason: Option<String>,
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

impl FndrMcpServer {
    /// Record one tool call's outcome. Every `#[tool]` method routes its
    /// result through here, so no return path can skip the audit log: the
    /// wrappers exist for that reason and not for style.
    ///
    /// A failed audit write fails the call. For `fndr.remember_decision`
    /// that means an appended decision can be reported as an error, and a
    /// retry appends a second entry. That is deliberate: a duplicate ledger
    /// row is visible to its owner and an unaudited write is not.
    /// Every tool name this server actually exposes, read from the router
    /// rather than a hand-maintained list, so callers (and the audit
    /// coverage test) cannot drift from the real surface.
    pub fn registered_tool_names() -> Vec<String> {
        Self::tool_router()
            .list_all()
            .into_iter()
            .map(|tool| tool.name.to_string())
            .collect()
    }

    /// Read the local audit log, newest first. Not an MCP tool: the audit
    /// log is for the person who owns the machine, not for the agents being
    /// audited by it.
    pub fn recent_tool_calls(&self, limit: usize) -> Result<Vec<AuditEntry>, ErrorData> {
        let store = self
            .store
            .lock()
            .map_err(|_| ErrorData::internal_error("store lock poisoned", None))?;
        store
            .recent_tool_calls(limit)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))
    }

    fn audit<T>(
        &self,
        tool: &str,
        raw_released: bool,
        result: Result<T, ErrorData>,
    ) -> Result<T, ErrorData> {
        let raw_released = raw_released && result.is_ok();
        let outcome = if result.is_ok() { "ok" } else { "refused" };
        let store = self
            .store
            .lock()
            .map_err(|_| ErrorData::internal_error("store lock poisoned", None))?;
        store
            .record_tool_call(now_ms(), tool, outcome, raw_released)
            .map_err(|e| ErrorData::internal_error(format!("audit write failed: {e}"), None))?;
        result
    }
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
        Parameters(params): Parameters<SearchParams>,
    ) -> Result<Json<SearchOutput>, ErrorData> {
        let raw_released = false;
        let result = self.search_inner(Parameters(params));
        self.audit("fndr.search", raw_released, result)
    }

    fn search_inner(
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
        Parameters(params): Parameters<PrivacyStatusParams>,
    ) -> Result<Json<PrivacyStatusOutput>, ErrorData> {
        let raw_released = false;
        let result = self.privacy_status_inner(Parameters(params));
        self.audit("fndr.privacy_status", raw_released, result)
    }

    fn privacy_status_inner(
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
        Parameters(params): Parameters<TimelineParams>,
    ) -> Result<Json<TimelineOutput>, ErrorData> {
        let raw_released = false;
        let result = self.timeline_inner(Parameters(params));
        self.audit("fndr.timeline", raw_released, result)
    }

    fn timeline_inner(
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
        Parameters(params): Parameters<SourceEvidenceParams>,
    ) -> Result<Json<SourceEvidenceOutput>, ErrorData> {
        let raw_released = params.include_raw.unwrap_or(false);
        let result = self.source_evidence_inner(Parameters(params));
        self.audit("fndr.source_evidence", raw_released, result)
    }

    fn source_evidence_inner(
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
        name = "fndr.explain_retrieval",
        description = "Why a query returns what it does: the terms the index actually sees, how they combine, how many chunks match, and what a limit would drop. Explains only; returns no results."
    )]
    pub fn explain_retrieval(
        &self,
        Parameters(params): Parameters<ExplainRetrievalParams>,
    ) -> Result<Json<ExplainRetrievalOutput>, ErrorData> {
        let raw_released = false;
        let result = self.explain_retrieval_inner(Parameters(params));
        self.audit("fndr.explain_retrieval", raw_released, result)
    }

    fn explain_retrieval_inner(
        &self,
        Parameters(ExplainRetrievalParams { query, limit }): Parameters<ExplainRetrievalParams>,
    ) -> Result<Json<ExplainRetrievalOutput>, ErrorData> {
        let requested = limit.unwrap_or(10) as i64;
        let store = self
            .store
            .lock()
            .map_err(|_| ErrorData::internal_error("store lock poisoned", None))?;
        let explanation = store
            .explain_chunk_search(&query)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;

        let effective_limit = requested.min(explanation.store_limit_cap);
        let would_return = explanation.total_matches.min(effective_limit);
        let dropped_by_limit = explanation.total_matches - would_return;

        let mut notes = Vec::new();
        if explanation.fts_expression.is_none() {
            notes.push(
                "No usable terms survived normalization, so this query matches nothing. That is an empty result, not an error.".to_owned(),
            );
        } else if explanation.total_matches == 0 && explanation.terms.len() > 1 {
            notes.push(
                "Every term must match. One word absent from a chunk excludes it, which is the usual reason a multi-word query finds nothing.".to_owned(),
            );
        }
        if requested > explanation.store_limit_cap {
            notes.push(format!(
                "The requested limit exceeds the store's ceiling of {}, which applies regardless.",
                explanation.store_limit_cap
            ));
        }
        // The honest boundary of this tool: FNDR redacts and skips before
        // storage, so retrieval has nothing withheld to report. Saying so
        // stops "nothing was redacted" being read as "nothing was excluded".
        notes.push(
            "Privacy exclusion happens at capture, not retrieval: blocked or redacted content was never stored, so it cannot appear here as dropped.".to_owned(),
        );
        notes.push(
            "Only the keyword route exists. Nothing was ranked semantically, so a miss here does not mean a semantic search would also miss.".to_owned(),
        );

        Ok(Json(ExplainRetrievalOutput {
            query,
            route: "keyword".to_owned(),
            terms: explanation.terms,
            fts_expression: explanation.fts_expression,
            match_mode: "all_terms".to_owned(),
            total_matches: explanation.total_matches,
            would_return,
            dropped_by_limit,
            store_limit_cap: explanation.store_limit_cap,
            notes,
        }))
    }

    #[tool(
        name = "fndr.feedback",
        description = "Rate a surfaced result. The rating is recorded locally and never mutates ranking: the response says so explicitly."
    )]
    pub fn feedback(
        &self,
        Parameters(params): Parameters<FeedbackParams>,
    ) -> Result<Json<FeedbackOutput>, ErrorData> {
        let raw_released = false;
        let result = self.feedback_inner(Parameters(params));
        self.audit("fndr.feedback", raw_released, result)
    }

    fn feedback_inner(
        &self,
        Parameters(FeedbackParams {
            rating,
            query,
            record_id,
            chunk_id,
            note,
        }): Parameters<FeedbackParams>,
    ) -> Result<Json<FeedbackOutput>, ErrorData> {
        if query.trim().is_empty() {
            return Err(ErrorData::invalid_params(
                "query must not be empty: feedback without the query it was given for cannot be replayed",
                None,
            ));
        }
        let store = self
            .store
            .lock()
            .map_err(|_| ErrorData::internal_error("store lock poisoned", None))?;
        let id = store
            .record_feedback(
                now_ms(),
                record_id.as_deref(),
                chunk_id.as_deref(),
                &query,
                rating.as_str(),
                note.as_deref().unwrap_or(""),
            )
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(Json(FeedbackOutput {
            id,
            rating: rating.as_str().to_owned(),
            ranking_changed: false,
        }))
    }

    #[tool(
        name = "fndr.context_pack",
        description = "Budgeted, cited context for a goal. Returns stored capture text with a citation on every item, packed until an estimated token budget is spent. Audited as a raw-text release, because that is what it is."
    )]
    pub fn context_pack(
        &self,
        Parameters(params): Parameters<ContextPackParams>,
    ) -> Result<Json<ContextPackOutput>, ErrorData> {
        // A context pack's whole purpose is carrying capture text, so it is
        // always a raw release. `source_evidence` gates that behind
        // include_raw; this tool cannot, so the audit log must say so on
        // every call rather than only when someone opts in.
        let raw_released = true;
        let result = self.context_pack_inner(Parameters(params));
        self.audit("fndr.context_pack", raw_released, result)
    }

    fn context_pack_inner(
        &self,
        Parameters(ContextPackParams {
            goal,
            token_budget,
            max_records,
        }): Parameters<ContextPackParams>,
    ) -> Result<Json<ContextPackOutput>, ErrorData> {
        if goal.trim().is_empty() {
            return Err(ErrorData::invalid_params("goal must not be empty", None));
        }
        let token_budget = token_budget.unwrap_or(2_000).min(8_000);
        let max_records = max_records.unwrap_or(20).min(100) as usize;
        let budget_chars = token_budget as usize * CHARS_PER_ESTIMATED_TOKEN;

        let store = self
            .store
            .lock()
            .map_err(|_| ErrorData::internal_error("store lock poisoned", None))?;
        let hits = KeywordRetriever::new(&store)
            .search(&goal, max_records)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;

        let mut items = Vec::new();
        let mut used_chars = 0usize;
        let mut dropped_for_budget = 0u32;
        for hit in hits {
            let Some(evidence) = store
                .record_evidence(&hit.record_id)
                .map_err(|e| ErrorData::internal_error(e.to_string(), None))?
            else {
                // Deleted between retrieval and packing; a citation to a
                // record that no longer exists must not reach the caller.
                continue;
            };
            let Some(chunk) = evidence
                .chunks
                .into_iter()
                .find(|chunk| chunk.chunk_id == hit.chunk_id)
            else {
                continue;
            };
            if used_chars + chunk.text.len() > budget_chars {
                dropped_for_budget += 1;
                continue;
            }
            used_chars += chunk.text.len();
            items.push(PackedEvidence {
                record_id: evidence.record_id,
                chunk_id: chunk.chunk_id,
                app_name: evidence.app_name,
                window_title: evidence.window_title,
                url: evidence.url,
                captured_at_ms: evidence.captured_at_ms as f64,
                estimated_tokens: chunk.text.len().div_ceil(CHARS_PER_ESTIMATED_TOKEN) as u32,
                text: chunk.text,
            });
        }

        Ok(Json(ContextPackOutput {
            goal,
            retrieval_route: "keyword".to_owned(),
            token_budget,
            estimated_tokens_used: used_chars.div_ceil(CHARS_PER_ESTIMATED_TOKEN) as u32,
            items,
            dropped_for_budget,
        }))
    }

    #[tool(
        name = "fndr.active_focus",
        description = "What the newest capture says someone was doing, with its age and whether it is recent enough to still call current. Returns status 'none' when nothing has been captured."
    )]
    pub fn active_focus(
        &self,
        Parameters(params): Parameters<ActiveFocusParams>,
    ) -> Result<Json<ActiveFocusOutput>, ErrorData> {
        let raw_released = false;
        let result = self.active_focus_inner(Parameters(params));
        self.audit("fndr.active_focus", raw_released, result)
    }

    fn active_focus_inner(
        &self,
        Parameters(ActiveFocusParams { stale_after_ms }): Parameters<ActiveFocusParams>,
    ) -> Result<Json<ActiveFocusOutput>, ErrorData> {
        let stale_after_ms = stale_after_ms.unwrap_or(DEFAULT_STALE_AFTER_MS);
        if stale_after_ms < 0 {
            return Err(ErrorData::invalid_params(
                "stale_after_ms must not be negative",
                None,
            ));
        }
        let store = self
            .store
            .lock()
            .map_err(|_| ErrorData::internal_error("store lock poisoned", None))?;
        let latest = store
            .latest_record_id()
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        let evidence = match latest {
            Some(record_id) => store
                .record_evidence(&record_id)
                .map_err(|e| ErrorData::internal_error(e.to_string(), None))?,
            None => None,
        };
        let Some(evidence) = evidence else {
            return Ok(Json(ActiveFocusOutput {
                status: "none".to_owned(),
                app_name: None,
                window_title: None,
                url: None,
                bundle_id: None,
                record_id: None,
                captured_at_ms: None,
                age_ms: None,
                stale_after_ms: stale_after_ms as f64,
            }));
        };

        // A clock that has moved backwards must not read as a fresh capture.
        let age_ms = now_ms().saturating_sub(evidence.captured_at_ms).max(0);
        Ok(Json(ActiveFocusOutput {
            status: if age_ms > stale_after_ms {
                "stale".to_owned()
            } else {
                "active".to_owned()
            },
            app_name: Some(evidence.app_name),
            window_title: Some(evidence.window_title),
            url: evidence.url,
            bundle_id: evidence.bundle_id,
            record_id: Some(evidence.record_id),
            captured_at_ms: Some(evidence.captured_at_ms as f64),
            age_ms: Some(age_ms as f64),
            stale_after_ms: stale_after_ms as f64,
        }))
    }

    #[tool(
        name = "fndr.delta",
        description = "What was captured since an instant: totals and the busiest apps, never capture text. Built for cheap repeated polling; feed newest_captured_at_ms back as the next since_ms."
    )]
    pub fn delta(
        &self,
        Parameters(params): Parameters<DeltaParams>,
    ) -> Result<Json<DeltaOutput>, ErrorData> {
        let raw_released = false;
        let result = self.delta_inner(Parameters(params));
        self.audit("fndr.delta", raw_released, result)
    }

    fn delta_inner(
        &self,
        Parameters(DeltaParams {
            since_ms,
            app_limit,
        }): Parameters<DeltaParams>,
    ) -> Result<Json<DeltaOutput>, ErrorData> {
        let app_limit = app_limit.unwrap_or(10).min(100) as usize;
        let store = self
            .store
            .lock()
            .map_err(|_| ErrorData::internal_error("store lock poisoned", None))?;
        let summary = store
            .changes_since(since_ms, app_limit)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(Json(DeltaOutput {
            since_ms: since_ms as f64,
            record_count: summary.record_count,
            newest_captured_at_ms: summary.newest_captured_at_ms.map(|ms| ms as f64),
            apps: summary
                .apps
                .into_iter()
                .map(|app| AppChangeOut {
                    app_name: app.app_name,
                    record_count: app.record_count,
                })
                .collect(),
        }))
    }

    #[tool(
        name = "fndr.open_target",
        description = "Resolve one memory to something reopenable: the page's sanitized URL, or the app it was captured from. A memory with neither returns an explicit unavailable state with a reason."
    )]
    pub fn open_target(
        &self,
        Parameters(params): Parameters<OpenTargetParams>,
    ) -> Result<Json<OpenTargetOutput>, ErrorData> {
        let raw_released = false;
        let result = self.open_target_inner(Parameters(params));
        self.audit("fndr.open_target", raw_released, result)
    }

    fn open_target_inner(
        &self,
        Parameters(OpenTargetParams { record_id }): Parameters<OpenTargetParams>,
    ) -> Result<Json<OpenTargetOutput>, ErrorData> {
        let store = self
            .store
            .lock()
            .map_err(|_| ErrorData::internal_error("store lock poisoned", None))?;
        let evidence = store
            .record_evidence(&record_id)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?
            .ok_or_else(|| ErrorData::invalid_params("unknown record_id", None))?;

        let (kind, url, bundle_id, reason) = match (&evidence.url, &evidence.bundle_id) {
            (Some(url), _) => ("url", Some(url.clone()), evidence.bundle_id.clone(), None),
            (None, Some(bundle_id)) => ("app", None, Some(bundle_id.clone()), None),
            (None, None) => (
                "unavailable",
                None,
                None,
                Some("this memory retained no URL and no bundle identifier".to_owned()),
            ),
        };
        Ok(Json(OpenTargetOutput {
            record_id: evidence.record_id,
            kind: kind.to_owned(),
            url,
            bundle_id,
            app_name: evidence.app_name,
            window_title: evidence.window_title,
            reason,
        }))
    }

    #[tool(
        name = "fndr.recall",
        description = "Recall decisions, errors, blockers, or todos. Only kind=decision has a data model today; the other kinds are refused explicitly rather than returning an empty list that reads as 'you have none'."
    )]
    pub fn recall(
        &self,
        Parameters(params): Parameters<RecallParams>,
    ) -> Result<Json<RecallOutput>, ErrorData> {
        let raw_released = false;
        let result = self.recall_inner(Parameters(params));
        self.audit("fndr.recall", raw_released, result)
    }

    fn recall_inner(
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
        Parameters(params): Parameters<RememberDecisionParams>,
    ) -> Result<Json<RememberDecisionOutput>, ErrorData> {
        let raw_released = false;
        let result = self.remember_decision_inner(Parameters(params));
        self.audit("fndr.remember_decision", raw_released, result)
    }

    fn remember_decision_inner(
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
