//! Content-free capture health aggregate.
//!
//! `pipeline::CaptureCounters` already counts every skipped or failed tick by
//! `SkipReason`. Two more signals were being computed elsewhere in the write
//! path and then discarded before this slice: how much of an OCR call's
//! cleaned text the noise/low-signal filters kept versus dropped
//! (`fndr-textsignal::cleanup::CaptureQualityStats`), and how many persisted
//! captures needed a secret redaction
//! (`fndr-memory::PersistCaptureOutcome::{Stored,Merged}::redaction_count`).
//! This module gives those two signals a typed, content-free home next to
//! `CaptureCounters` so all three combine into one queryable report: the
//! shape a future health panel (T-1004) or an `fndr.health`-style MCP tool
//! would read. Every field here is a count or a ratio; nothing here ever
//! carries captured or cleaned text, a window title, or a URL.

use crate::pipeline::CaptureCounters;

/// The content-free line counts one OCR call's cleanup pass produced.
/// Populated by the OCR boundary from its own, richer, crate-local stats
/// type; only primitive counts cross into this crate, so `fndr-capture` does
/// not need a dependency on `fndr-textsignal` just to observe them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CleanupSignal {
    pub total_lines: usize,
    pub kept_lines: usize,
    pub dropped_noise_lines: usize,
    pub dropped_low_signal_lines: usize,
    pub low_conf_lines: usize,
}

/// A rate that is either measured from at least one observation, or typed as
/// unmeasured. Per invariant 4 (no silent degradation): a health consumer
/// must never read "zero" as "measured and zero" when nothing has happened
/// yet, e.g. a fresh pipeline or an app that has captured nothing so far.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum QualityRate {
    Unmeasured,
    Measured(f64),
}

impl QualityRate {
    pub fn is_measured(&self) -> bool {
        matches!(self, QualityRate::Measured(_))
    }

    /// The measured ratio, or `None` when nothing has been observed yet.
    pub fn value(&self) -> Option<f64> {
        match self {
            QualityRate::Measured(value) => Some(*value),
            QualityRate::Unmeasured => None,
        }
    }
}

fn rate(numerator: u64, denominator: u64) -> QualityRate {
    if denominator == 0 {
        QualityRate::Unmeasured
    } else {
        QualityRate::Measured(numerator as f64 / denominator as f64)
    }
}

/// Cleanup-drop counts accumulated across every OCR call observed so far.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CleanupQuality {
    calls: u64,
    total_lines: u64,
    kept_lines: u64,
    dropped_noise_lines: u64,
    dropped_low_signal_lines: u64,
    low_conf_lines: u64,
}

impl CleanupQuality {
    /// Fold one OCR call's cleanup signal into the running aggregate.
    pub fn record(&mut self, signal: CleanupSignal) {
        self.calls += 1;
        self.total_lines += signal.total_lines as u64;
        self.kept_lines += signal.kept_lines as u64;
        self.dropped_noise_lines += signal.dropped_noise_lines as u64;
        self.dropped_low_signal_lines += signal.dropped_low_signal_lines as u64;
        self.low_conf_lines += signal.low_conf_lines as u64;
    }

    pub fn calls(&self) -> u64 {
        self.calls
    }

    pub fn total_lines(&self) -> u64 {
        self.total_lines
    }

    pub fn kept_lines(&self) -> u64 {
        self.kept_lines
    }

    pub fn dropped_noise_lines(&self) -> u64 {
        self.dropped_noise_lines
    }

    pub fn dropped_low_signal_lines(&self) -> u64 {
        self.dropped_low_signal_lines
    }

    pub fn low_conf_lines(&self) -> u64 {
        self.low_conf_lines
    }

    /// Share of observed lines dropped as noise or low signal. `Unmeasured`
    /// until at least one line has been observed.
    pub fn drop_rate(&self) -> QualityRate {
        rate(
            self.dropped_noise_lines + self.dropped_low_signal_lines,
            self.total_lines,
        )
    }

    /// Share of observed lines that carried the OCR engine's own
    /// low-confidence marker (a signal about the source image, not about
    /// what the cleanup pass chose to keep or drop).
    pub fn low_confidence_rate(&self) -> QualityRate {
        rate(self.low_conf_lines, self.total_lines)
    }
}

/// Redaction counts accumulated across every persisted capture.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RedactionQuality {
    persisted_captures: u64,
    redacted_captures: u64,
    total_redactions: u64,
}

impl RedactionQuality {
    /// Fold one persisted capture's redaction count into the running
    /// aggregate. Only captures that actually reached storage (`Stored` or
    /// `Merged`) count toward the denominator; a capture the safety gate
    /// skipped entirely already has its own `SkipReason` counter and must
    /// not also move this rate.
    pub fn record(&mut self, redaction_count: usize) {
        self.persisted_captures += 1;
        if redaction_count > 0 {
            self.redacted_captures += 1;
        }
        self.total_redactions += redaction_count as u64;
    }

    pub fn persisted_captures(&self) -> u64 {
        self.persisted_captures
    }

    pub fn redacted_captures(&self) -> u64 {
        self.redacted_captures
    }

    pub fn total_redactions(&self) -> u64 {
        self.total_redactions
    }

    /// Share of persisted captures that needed at least one secret
    /// redaction. `Unmeasured` until at least one capture has persisted.
    pub fn redaction_rate(&self) -> QualityRate {
        rate(self.redacted_captures, self.persisted_captures)
    }
}

/// The queryable, content-free capture-health snapshot: per-`SkipReason`
/// skip counts (the pipeline's own `CaptureCounters`, not a second map),
/// redaction rate, and cleanup-drop rate. This is the shape a future health
/// panel or `fndr.health`-style MCP tool would serialize.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CaptureHealthReport {
    pub skips: CaptureCounters,
    pub redaction: RedactionQuality,
    pub cleanup: CleanupQuality,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::SkipReason;

    #[test]
    fn cleanup_quality_is_unmeasured_before_any_call() {
        let quality = CleanupQuality::default();
        assert_eq!(quality.drop_rate(), QualityRate::Unmeasured);
        assert_eq!(quality.low_confidence_rate(), QualityRate::Unmeasured);
        assert_eq!(quality.calls(), 0);
    }

    #[test]
    fn cleanup_quality_accumulates_across_calls() {
        let mut quality = CleanupQuality::default();
        quality.record(CleanupSignal {
            total_lines: 10,
            kept_lines: 6,
            dropped_noise_lines: 3,
            dropped_low_signal_lines: 1,
            low_conf_lines: 2,
        });
        quality.record(CleanupSignal {
            total_lines: 10,
            kept_lines: 10,
            dropped_noise_lines: 0,
            dropped_low_signal_lines: 0,
            low_conf_lines: 0,
        });

        assert_eq!(quality.calls(), 2);
        assert_eq!(quality.total_lines(), 20);
        assert_eq!(quality.kept_lines(), 16);
        assert_eq!(quality.drop_rate(), QualityRate::Measured(4.0 / 20.0));
        assert_eq!(quality.low_confidence_rate(), QualityRate::Measured(2.0 / 20.0));
    }

    #[test]
    fn cleanup_quality_with_only_empty_calls_stays_unmeasured() {
        // An OCR call whose text was already empty (e.g. a call that never
        // produced a line) must not manufacture a zero drop rate: zero
        // observed lines is "not measured", not "measured at zero".
        let mut quality = CleanupQuality::default();
        quality.record(CleanupSignal::default());
        assert_eq!(quality.calls(), 1);
        assert_eq!(quality.drop_rate(), QualityRate::Unmeasured);
    }

    #[test]
    fn redaction_quality_is_unmeasured_before_any_persisted_capture() {
        let quality = RedactionQuality::default();
        assert_eq!(quality.redaction_rate(), QualityRate::Unmeasured);
    }

    #[test]
    fn redaction_quality_counts_captures_not_just_redactions() {
        let mut quality = RedactionQuality::default();
        quality.record(0);
        quality.record(2);
        quality.record(0);

        assert_eq!(quality.persisted_captures(), 3);
        assert_eq!(quality.redacted_captures(), 1);
        assert_eq!(quality.total_redactions(), 2);
        assert_eq!(quality.redaction_rate(), QualityRate::Measured(1.0 / 3.0));
    }

    #[test]
    fn report_reuses_capture_counters_rather_than_a_second_map() {
        let skips = CaptureCounters {
            skipped: std::collections::HashMap::from([(SkipReason::LowSignal, 4)]),
            ..CaptureCounters::default()
        };
        let report = CaptureHealthReport {
            skips,
            redaction: RedactionQuality::default(),
            cleanup: CleanupQuality::default(),
        };

        assert_eq!(report.skips.skip_count(SkipReason::LowSignal), 4);
    }
}
