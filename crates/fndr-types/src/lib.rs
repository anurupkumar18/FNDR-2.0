//! Shared domain types, ids, lifecycle enums, config structs, event payloads.
//!
//! Every type that crosses IPC derives `specta::Type` here and reaches the
//! frontend only through the generated bindings (T-105); hand-written TS
//! mirrors are banned.
//!
//! IPC integer convention (ADR-001): the TypeScript exporter is configured to
//! fail on i64/u64/i128/u128. Anything crossing IPC uses string ids, i32/u32
//! counts, or f64 millisecond timestamps. Widening that needs an ADR-001
//! amendment, not a local exporter setting.

mod lifecycle;

pub use lifecycle::{ReviewLifecycle, TaskStatus, UnknownDiscriminant};

use serde::Serialize;
use specta::Type;

/// Build information the shell and MCP status surfaces report. Also the
/// pipeline probe for T-105: its round-trip into `ui/` proves the generated
/// bindings work end to end.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct EngineInfo {
    pub app_version: String,
}

/// The shell-owned capture worker's current lifecycle state. This is an IPC
/// state, not a persisted record lifecycle: it deliberately says whether the
/// desktop is collecting new context right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum CaptureRuntimeState {
    Starting,
    Running,
    Paused,
    Blocked,
    Stopped,
}

/// A content-free classification of one capture opportunity. Raw OCR text,
/// window titles, URLs, and image data must never cross this status boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum CaptureTickState {
    Stored,
    UrlOnlyStored,
    Skipped,
    Failed,
}

/// The indexing side of a capture opportunity. SQLite persistence can succeed
/// while the derived Lance index is pending or failed, so the two states stay
/// separate rather than collapsing into a misleading single green state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum CaptureFlushState {
    NotDue,
    Flushed,
    Failed,
}

/// The bounded, content-free detail attached to a running capture status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct CaptureTickStatus {
    pub capture: CaptureTickState,
    pub flush: CaptureFlushState,
    /// A stable pipeline reason, never an underlying error message that might
    /// contain operating-system or captured-context details.
    pub reason: Option<String>,
}

/// Push event payload for `capture://status`. Milliseconds use `f64` because
/// IPC follows ADR-001's no-64-bit-integer convention.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
pub struct CaptureRuntimeStatus {
    pub state: CaptureRuntimeState,
    pub observed_at_ms: f64,
    pub tick: Option<CaptureTickStatus>,
    pub shutdown_flushed_chunks: Option<u32>,
    /// A stable, operator-facing startup/shutdown code. It is intentionally
    /// not an arbitrary dependency error string.
    pub reason: Option<String>,
}

/// A content-free record of one MCP call for the local owner audit viewer.
/// This mirrors only the privacy-preserving `mcp_audit` columns: no query,
/// record identifier, capture text, URL, or model output crosses IPC.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
pub struct AuditLogEntry {
    /// JavaScript-safe IPC timestamp convention: milliseconds as `f64`.
    pub at_ms: f64,
    pub tool: String,
    pub outcome: String,
    pub raw_released: bool,
}

/// The current result of the non-prompting macOS Screen Recording preflight.
/// It says nothing about capture itself and is deliberately distinct from the
/// API that requests permission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ScreenRecordingAccess {
    Granted,
    NotGranted,
}
