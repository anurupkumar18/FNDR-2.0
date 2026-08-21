//! IPC command handlers. Thin: commands call the engine API and shape nothing.

use fndr_types::EngineInfo;

#[tauri::command]
#[specta::specta]
pub fn engine_info() -> EngineInfo {
    EngineInfo {
        app_version: env!("CARGO_PKG_VERSION").to_string(),
    }
}
