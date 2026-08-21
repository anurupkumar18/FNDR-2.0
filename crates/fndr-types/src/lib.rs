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

use serde::Serialize;
use specta::Type;

/// Build information the shell and MCP status surfaces report. Also the
/// pipeline probe for T-105: its round-trip into `ui/` proves the generated
/// bindings work end to end.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct EngineInfo {
    pub app_version: String,
}
