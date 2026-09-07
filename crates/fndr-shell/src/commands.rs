//! IPC command handlers. Thin: commands call the engine API and shape nothing.

use fndr_types::{AuditLogEntry, CaptureRuntimeStatus, EngineInfo, ScreenRecordingAccess};
use tauri::Manager;

use crate::app::ShellCaptureState;

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
    let data_dir = match options.inner().data_dir.clone() {
        Some(path) => path,
        None => app
            .path()
            .app_data_dir()
            .map_err(|_| "audit_log_unavailable".to_owned())?,
    };
    crate::app::recent_audit_entries(&data_dir, crate::app::AUDIT_LOG_LIMIT)
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
