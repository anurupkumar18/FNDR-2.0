//! Declarative gate policy table (T-309).
//!
//! `pipeline::run_tick_inner` used to decide whether a capture-volume gate
//! (admission surface policy, perceptual dedup, OCR low-signal, semantic
//! dedup) applied with a bare inline `if <condition> { return Skipped(...) }`,
//! in a fixed hardcoded order baked into the function body. That is exactly
//! the anti-pattern this ticket exists to close: FNDR v1's capture loop
//! stacked overlapping inline gate conditions in its main loop until one of
//! them silently absorbed every LLM-touched frame (see the comment at
//! `reference/v1` `src-tauri/src/capture/mod.rs` around the
//! `stacked_critical_extraction_issues` gate, and the regression test in
//! `crates/fndr-capture/tests/stacked_gates_regression.rs`), and nothing
//! short of reading a diff months later could tell an operator which check
//! was responsible.
//!
//! This module names each tunable gate by the `SkipReason` it reports,
//! fixes their canonical pipeline order, and lets a caller enable or
//! disable one without adding another inline `if` to `pipeline.rs`.
//! `pipeline.rs` still owns *when* each check runs, because the checks have
//! real sequential data dependencies (OCR needs a captured frame; semantic
//! dedup needs OCR text) that a generic rule engine would only obscure.
//! This table owns *whether* a firing check is allowed to drop the tick, so
//! `crates/fndr-capture/src/replay.rs` can attribute every drop to exactly
//! one named, orderable policy entry and a config change becomes a visible
//! per-gate delta instead of a silent behavior change.
//!
//! Privacy-critical checks (`PreCapturePrivacy`, `PrivateBrowsing`,
//! `FinalPrivacy`) and the structural `MissingPerceptualSignature` failure
//! are deliberately not in this table: they are unconditional safety and
//! data-integrity checks, never a capture-volume tuning knob, and making
//! them disableable would weaken ADR-004's no-silent-degradation guarantee
//! for no product benefit. `set_enabled` refuses to touch them.

use crate::pipeline::SkipReason;

/// Canonical pipeline order for the gates this table governs. Keep this in
/// sync with the sequence of checks in `pipeline.rs::run_tick_inner`;
/// `gate_policy::tests::default_table_enables_every_gate_in_canonical_order`
/// is the seam test that pins the order this module claims.
pub const GATE_ORDER: [SkipReason; 4] = [
    SkipReason::AdmissionPolicy,
    SkipReason::PerceptualDuplicate,
    SkipReason::LowSignal,
    SkipReason::SemanticDuplicate,
];

/// One entry in the ordered, declarative gate table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GateEntry {
    /// The `SkipReason` this gate reports when it fires and is enabled.
    pub reason: SkipReason,
    pub enabled: bool,
}

/// The config-driven policy for every tunable capture-volume gate, in the
/// fixed order `pipeline::run_tick_inner` evaluates them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatePolicyTable {
    entries: Vec<GateEntry>,
}

impl GatePolicyTable {
    /// Every gate enabled, in canonical order. This is the table that
    /// reproduces T-306's original inline-if behavior exactly: T-309 is a
    /// refactor of how that behavior is expressed, not a retune of it.
    pub fn all_enabled() -> Self {
        Self {
            entries: GATE_ORDER
                .iter()
                .map(|&reason| GateEntry {
                    reason,
                    enabled: true,
                })
                .collect(),
        }
    }

    /// True if `reason`'s gate is allowed to drop the current tick. Reasons
    /// outside this table (the privacy/structural checks named in the
    /// module doc) always return `true`: they are not policy-tunable, so
    /// `pipeline.rs` never consults this table for them; treating an
    /// untracked reason as "on" keeps this function total instead of
    /// requiring pipeline.rs to special-case reasons the table does not
    /// own.
    pub fn is_enabled(&self, reason: SkipReason) -> bool {
        self.entries
            .iter()
            .find(|entry| entry.reason == reason)
            .is_none_or(|entry| entry.enabled)
    }

    /// Enable or disable one gate by the `SkipReason` it reports. Returns
    /// `false` (no-op) if `reason` does not name an entry in this table —
    /// either a typo or one of the privacy-critical/structural reasons that
    /// are deliberately not configurable — so a caller cannot silently
    /// believe it disabled a gate that this table does not govern.
    pub fn set_enabled(&mut self, reason: SkipReason, enabled: bool) -> bool {
        match self.entries.iter_mut().find(|entry| entry.reason == reason) {
            Some(entry) => {
                entry.enabled = enabled;
                true
            }
            None => false,
        }
    }

    /// The table in canonical pipeline order, for reporting (health panel,
    /// replay harness, `fndr doctor`).
    pub fn entries(&self) -> &[GateEntry] {
        &self.entries
    }
}

impl Default for GatePolicyTable {
    fn default() -> Self {
        Self::all_enabled()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_table_enables_every_gate_in_canonical_order() {
        let table = GatePolicyTable::default();
        let reasons: Vec<_> = table.entries().iter().map(|entry| entry.reason).collect();
        assert_eq!(reasons, GATE_ORDER.to_vec());
        assert!(table.entries().iter().all(|entry| entry.enabled));
    }

    #[test]
    fn disabling_a_gate_only_affects_that_reason() {
        let mut table = GatePolicyTable::default();
        assert!(table.set_enabled(SkipReason::LowSignal, false));
        assert!(!table.is_enabled(SkipReason::LowSignal));
        assert!(table.is_enabled(SkipReason::SemanticDuplicate));
        assert!(table.is_enabled(SkipReason::PerceptualDuplicate));
        assert!(table.is_enabled(SkipReason::AdmissionPolicy));
    }

    #[test]
    fn reenabling_a_gate_restores_default_behavior() {
        let mut table = GatePolicyTable::default();
        table.set_enabled(SkipReason::SemanticDuplicate, false);
        table.set_enabled(SkipReason::SemanticDuplicate, true);
        assert!(table.is_enabled(SkipReason::SemanticDuplicate));
    }

    #[test]
    fn privacy_critical_and_structural_reasons_are_not_configurable() {
        let mut table = GatePolicyTable::default();
        assert!(!table.set_enabled(SkipReason::FinalPrivacy, false));
        assert!(!table.set_enabled(SkipReason::PreCapturePrivacy, false));
        assert!(!table.set_enabled(SkipReason::PrivateBrowsing, false));
        assert!(!table.set_enabled(SkipReason::MissingPerceptualSignature, false));
        // Untracked reasons are always reported as "enabled" so pipeline.rs
        // never needs a special case for them.
        assert!(table.is_enabled(SkipReason::FinalPrivacy));
        assert!(table.is_enabled(SkipReason::MissingPerceptualSignature));
    }
}
