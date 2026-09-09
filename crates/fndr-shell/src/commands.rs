//! IPC command handlers. Thin: commands call the engine API and shape nothing.

use std::path::PathBuf;

use fndr_inference::Embedder;
use fndr_types::{
    AuditLogEntry, CaptureRuntimeStatus, EngineInfo, MemorySearchResults, ScreenRecordingAccess,
};
use tauri::Manager;

use crate::app::ShellCaptureState;
use crate::search::{DEFAULT_SEARCH_LIMIT, MemorySearchPaths, SEARCH_LIMIT_CAP, ShellSearchModel};

/// The data directory this run reads and writes: the explicit launch override,
/// otherwise the app-managed location. Every owner-facing read path resolves
/// it the same way, so the audit viewer and search never look at two different
/// vaults.
fn resolved_data_dir(
    app: &tauri::AppHandle,
    options: &crate::app::CaptureLaunchOptions,
) -> Option<PathBuf> {
    options
        .data_dir
        .clone()
        .or_else(|| app.path().app_data_dir().ok())
}

#[tauri::command]
#[specta::specta]
pub fn engine_info() -> EngineInfo {
    EngineInfo {
        app_version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

/// The latest pushed capture state. The UI should subscribe to
/// `capture://status` after this initial read; it must not poll this command.
#[tauri::command]
#[specta::specta]
pub fn capture_status(state: tauri::State<'_, ShellCaptureState>) -> CaptureRuntimeStatus {
    state.status()
}

/// Read the current macOS Screen Recording preflight without requesting it.
/// This is intentionally available before `start_capture`, so the trust
/// window can render the status before a person chooses to begin capture.
#[tauri::command]
#[specta::specta]
pub fn screen_recording_preflight() -> ScreenRecordingAccess {
    crate::app::screen_recording_preflight()
}

/// Starts capture only after an explicit action in the desktop trust screen.
/// The configured paths come from the local launch command; they are not
/// surfaced to the webview or accepted from IPC.
#[tauri::command]
#[specta::specta]
pub fn start_capture(
    app: tauri::AppHandle,
    state: tauri::State<'_, ShellCaptureState>,
    options: tauri::State<'_, crate::app::CaptureLaunchOptions>,
) -> Result<CaptureRuntimeStatus, String> {
    state
        .start(&app, options.inner().clone())
        .map_err(|_| "capture_start_unavailable".to_owned())?;
    Ok(state.status())
}

/// Show the machine owner's local MCP audit ledger. This command is bounded
/// and read-only; it deliberately returns no capture content or query text.
#[tauri::command]
#[specta::specta]
pub fn recent_audit_entries(
    app: tauri::AppHandle,
    options: tauri::State<'_, crate::app::CaptureLaunchOptions>,
) -> Result<Vec<AuditLogEntry>, String> {
    let data_dir =
        resolved_data_dir(&app, options.inner()).ok_or_else(|| "audit_log_unavailable".to_owned())?;
    crate::app::recent_audit_entries(&data_dir, crate::app::AUDIT_LOG_LIMIT)
}

/// Search the machine owner's own local memory.
///
/// This is the person-facing counterpart to the `fndr.search` MCP tool, over
/// the same `fndr-retrieval` routes: until it existed, an agent could query
/// this vault and its owner could not.
///
/// Read-only and independent of capture: the launch options are managed
/// state, registered whether or not capture was ever started, so a person can
/// search what FNDR already remembers without turning capture on first.
///
/// `MemorySearchResults` reports whether the semantic route ran (and if not,
/// why), so a keyword-only answer is never presented as the whole answer.
#[tauri::command]
#[specta::specta]
pub async fn search_memories(
    query: String,
    limit: Option<u32>,
    app: tauri::AppHandle,
    options: tauri::State<'_, crate::app::CaptureLaunchOptions>,
    model: tauri::State<'_, ShellSearchModel>,
) -> Result<MemorySearchResults, String> {
    let data_dir = resolved_data_dir(&app, options.inner())
        .ok_or_else(|| "memory_search_unavailable".to_owned())?;
    let paths = MemorySearchPaths::resolve(&data_dir, options.inner().model_path.as_deref());
    let limit = limit.unwrap_or(DEFAULT_SEARCH_LIMIT).min(SEARCH_LIMIT_CAP) as usize;

    // The embedder is resolved (and the model worker's mutex released) before
    // the await: no lock is ever held across the model round trip.
    let embedder = model.embedder(&paths.model_path);
    crate::search::search_memories(
        &paths,
        embedder.as_ref().map(|embedder| embedder as &dyn Embedder),
        &query,
        limit,
    )
    .await
}

/// Explicitly pauses or resumes new capture opportunities. Failures are
/// stable operator codes rather than dependency messages, which could expose
/// environment details through the desktop bridge.
#[tauri::command]
#[specta::specta]
pub fn set_capture_paused(
    paused: bool,
    state: tauri::State<'_, ShellCaptureState>,
) -> Result<CaptureRuntimeStatus, String> {
    state.set_paused(paused).map_err(|error| match error {
        crate::capture_lifecycle::CaptureLifecycleControlError::NotRunning => {
            "capture_not_running".into()
        }
        crate::capture_lifecycle::CaptureLifecycleControlError::Worker(_) => {
            "capture_control_unavailable".into()
        }
    })
}
