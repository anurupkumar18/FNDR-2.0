//! Tauri shell: IPC command registration, event emission, windows, tray, permissions flow. The only crate allowed to import Tauri.

pub mod capture_adapters;
pub mod capture_scheduler;
pub mod capture_worker;
pub mod commands;

use std::error::Error;
use std::path::Path;

use specta_typescript::Typescript;
use tauri_specta::{Builder, collect_commands};

/// Every IPC command the shell registers. Adding a command here is what makes
/// it exist for the frontend; the generated bindings follow automatically.
pub fn specta_builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new().commands(collect_commands![commands::engine_info])
}

/// Export the TypeScript bindings for all registered commands and their types.
/// The exporter's default forbids i64/u64/i128/u128, which is the fndr-types
/// IPC integer convention: an i64 crossing IPC is an export error, not a
/// silent precision loss in JS. Never call
/// `dangerously_cast_bigints_to_number`; widen the convention via an ADR-001
/// amendment instead.
pub fn export_bindings(path: &Path) -> Result<(), Box<dyn Error>> {
    specta_builder().export(Typescript::default(), path)?;
    Ok(())
}
