//! Blocklist, sensitive-context detection, the safety gate enforced at the storage write path.

mod blocklist;
mod safety_gate;

pub use blocklist::Blocklist;
pub use safety_gate::{
    SafetyContext, SafetyDecision, SafetyReason, SensitiveContextPolicy, evaluate,
    evaluate_with_policy, redact_secret_lines, redact_secret_lines_with_policy,
};
