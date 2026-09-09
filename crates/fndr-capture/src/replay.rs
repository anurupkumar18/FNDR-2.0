//! Offline replay harness for the declarative gate policy table (T-309).
//!
//! Runs the exact same `CapturePipeline` gate sequence used at runtime over
//! a fixed, ordered set of recorded capture fixtures, with no
//! ScreenCaptureKit, Vision, or SQLite involved, so a gate policy change
//! (enable/disable a gate, retune a threshold) can be diffed before it
//! ships. This is what ADR-005 / the fndr-v2-engineering skill mean by "a
//! declarative policy entry with replay coverage, not an inline
//! `continue`": FNDR v1's capture loop had no such harness, so a change to
//! one inline gate condition could — and once did — silently swallow every
//! frame another gate was supposed to catch, discovered only by reading a
//! diff months later (`crates/fndr-capture/tests/stacked_gates_regression.rs`
//! ports that defect forward as a concrete test against this harness).
//!
//! Fixture files are JSON Lines (one [`ReplayRecord`] object per non-empty,
//! non-`#` line), the same convention `bench/corpus-sample` documents for
//! retrieval fixtures. Scope: this harness replays the capture-volume gate
//! table (admission, perceptual dedup, low signal, semantic dedup). It does
//! not exercise the storage-layer final privacy recheck or SQLite/Lance
//! persistence — those are `fndr-store`'s and T-306's own boundary and are
//! already covered there; `FixtureSink` always reports `Stored`.

use std::cell::RefCell;
use std::collections::{BTreeSet, VecDeque};
use std::path::Path;
use std::rc::Rc;

use serde::Deserialize;

use crate::{
    CaptureContext, CaptureContextSource, CaptureCounters, CaptureError, CapturePipeline,
    CapturePipelineConfig, CaptureSink, CaptureStage, Frame, FrameSource, GateDecision,
    GatePolicyTable, OcrOutput, OcrRecognizer, PerceptualSignature, PersistenceOutcome,
    PipelineError, PreCaptureGate, SkipReason,
};

/// One recorded capture opportunity, as read from a replay fixture file.
///
/// Fields map onto the pipeline boundaries a real tick would populate:
/// `pixel_rgb` stands in for a captured frame (perceptual dedup only ever
/// sees a 9x8 downscaled raster, never full pixels, so a flat colour is a
/// faithful substitute), and `ocr_text`/`ocr_confidence`/`ocr_block_count`
/// stand in for the Vision adapter's output. `low_signal` is deliberately
/// NOT a fixture field: the harness derives it from `ocr_text` against the
/// replayed config's `min_ocr_chars`, the same way the real OCR boundary
/// derives it from its own threshold, so a fixture set actually exercises
/// the config-driven `LowSignal` gate instead of hand-authoring its
/// verdict.
#[derive(Debug, Clone, Deserialize)]
pub struct ReplayRecord {
    pub app_name: String,
    #[serde(default)]
    pub bundle_id: Option<String>,
    #[serde(default)]
    pub window_title: String,
    #[serde(default)]
    pub url: Option<String>,
    pub observed_at_ms: u64,
    pub captured_at_ms: u64,
    /// Colour standing in for a downscaled native frame; see the struct doc.
    pub pixel_rgb: [u8; 3],
    #[serde(default)]
    pub ocr_text: String,
    #[serde(default)]
    pub ocr_confidence: f32,
    #[serde(default)]
    pub ocr_block_count: usize,
    /// True if this capture opportunity should be blocked by the
    /// pre-capture privacy gate before any pixel is read (the fixture
    /// analogue of pause/incognito/blocklist). Not part of the declarative
    /// gate table (`gate_policy.rs`): privacy gates are unconditional, so
    /// this field has no enable/disable counterpart.
    #[serde(default)]
    pub private: bool,
}

impl ReplayRecord {
    fn context(&self) -> CaptureContext {
        CaptureContext {
            app_name: self.app_name.clone(),
            bundle_id: self.bundle_id.clone(),
            window_title: self.window_title.clone(),
            url: self.url.clone(),
            observed_at_ms: self.observed_at_ms,
        }
    }

    fn frame(&self) -> Frame {
        Frame {
            png: Vec::new(),
            captured_at_ms: self.captured_at_ms,
            perceptual_signature: Some(flat_signature(self.pixel_rgb)),
        }
    }
}

/// Build the same kind of 9x8 flat-colour native signature the pipeline
/// tests use, so replay fixtures exercise the real `PerceptualDeduper`
/// without decoding any PNG.
fn flat_signature(rgb: [u8; 3]) -> PerceptualSignature {
    let mut rgba = [0_u8; 9 * 8 * 4];
    for pixel in rgba.as_chunks_mut::<4>().0 {
        pixel[..3].copy_from_slice(&rgb);
        pixel[3] = u8::MAX;
    }
    PerceptualSignature::from_downscaled_rgba(rgba)
}

/// Failures loading a replay fixture file.
#[derive(Debug, thiserror::Error)]
pub enum ReplayFixtureError {
    #[error("failed to read fixture file {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("fixture file {path} line {line}: {source}")]
    Parse {
        path: String,
        line: usize,
        #[source]
        source: serde_json::Error,
    },
}

/// Load a JSON Lines fixture file in file order. Blank lines and lines
/// starting with `#` are skipped so a fixture file can carry a short header
/// comment.
pub fn load_fixtures(path: impl AsRef<Path>) -> Result<Vec<ReplayRecord>, ReplayFixtureError> {
    let path_ref = path.as_ref();
    let contents =
        std::fs::read_to_string(path_ref).map_err(|source| ReplayFixtureError::Io {
            path: path_ref.display().to_string(),
            source,
        })?;
    contents
        .lines()
        .enumerate()
        .filter(|(_, line)| {
            let trimmed = line.trim();
            !trimmed.is_empty() && !trimmed.starts_with('#')
        })
        .map(|(index, line)| {
            serde_json::from_str(line).map_err(|source| ReplayFixtureError::Parse {
                path: path_ref.display().to_string(),
                line: index + 1,
                source,
            })
        })
        .collect()
}

/// The fixture-backed capture boundary. A single `Rc`-shared instance plays
/// all four pipeline seams (`CaptureContextSource`, `PreCaptureGate`,
/// `FrameSource`, `OcrRecognizer`) so one popped record supplies every stage
/// of the tick it belongs to, without four independently-advancing queues
/// that could drift out of alignment.
#[derive(Clone)]
struct FixtureSource(Rc<FixtureState>);

struct FixtureState {
    queue: RefCell<VecDeque<ReplayRecord>>,
    current: RefCell<Option<ReplayRecord>>,
}

impl FixtureSource {
    fn new(records: Vec<ReplayRecord>) -> Self {
        Self(Rc::new(FixtureState {
            queue: RefCell::new(records.into()),
            current: RefCell::new(None),
        }))
    }
}

impl CaptureContextSource for FixtureSource {
    fn current_context(&self) -> Result<CaptureContext, PipelineError> {
        let record = self.0.queue.borrow_mut().pop_front().ok_or_else(|| {
            PipelineError::new(CaptureStage::Metadata, "replay fixture queue exhausted")
        })?;
        let context = record.context();
        *self.0.current.borrow_mut() = Some(record);
        Ok(context)
    }
}

impl PreCaptureGate for FixtureSource {
    fn evaluate(&self, _context: &CaptureContext) -> GateDecision {
        match self.0.current.borrow().as_ref() {
            Some(record) if record.private => GateDecision::Skip(SkipReason::PreCapturePrivacy),
            _ => GateDecision::Allow,
        }
    }
}

impl FrameSource for FixtureSource {
    fn grab(&self) -> Result<Frame, CaptureError> {
        self.0
            .current
            .borrow()
            .as_ref()
            .map(ReplayRecord::frame)
            .ok_or(CaptureError::Empty)
    }
}

impl OcrRecognizer for FixtureSource {
    fn recognize(
        &self,
        _png: &[u8],
        _app_name: &str,
        _bundle_id: Option<&str>,
        min_chars: usize,
    ) -> Result<OcrOutput, PipelineError> {
        let current = self.0.current.borrow();
        let record = current
            .as_ref()
            .ok_or_else(|| PipelineError::new(CaptureStage::Ocr, "no current replay record"))?;
        Ok(OcrOutput {
            text: record.ocr_text.clone(),
            confidence: record.ocr_confidence,
            block_count: record.ocr_block_count,
            low_signal: record.ocr_text.chars().count() < min_chars,
        })
    }
}

/// Always accepts. Final-privacy behavior is `fndr-store`'s boundary and
/// out of scope for this harness (see the module doc).
struct FixtureSink;

impl CaptureSink for FixtureSink {
    fn persist_capture(
        &mut self,
        _context: &CaptureContext,
        _frame: &Frame,
        _ocr: &OcrOutput,
    ) -> Result<PersistenceOutcome, PipelineError> {
        Ok(PersistenceOutcome::Stored)
    }

    fn persist_url_only(
        &mut self,
        _context: &CaptureContext,
    ) -> Result<PersistenceOutcome, PipelineError> {
        Ok(PersistenceOutcome::Stored)
    }
}

/// Run the declarative gate table over `records`, in order, and report
/// per-gate drop counts. Pure and synchronous: the same fixture set and
/// config always produce the same report, so a gate policy change can be
/// diffed in a unit test or a standalone binary
/// (`examples/gate_replay.rs`) without the real capture loop.
pub fn run_replay(records: Vec<ReplayRecord>, config: CapturePipelineConfig) -> ReplayReport {
    let total = records.len();
    let gates = config.gates.clone();
    let source = FixtureSource::new(records);
    let mut pipeline = CapturePipeline::new(
        source.clone(),
        source.clone(),
        source.clone(),
        source,
        FixtureSink,
        config,
    );
    for _ in 0..total {
        pipeline.run_tick();
    }
    ReplayReport::new(pipeline.counters().clone(), gates)
}

/// The result of one replay run: every `SkipReason` this run counted, plus
/// the gate table that produced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayReport {
    pub stored: u64,
    pub url_only_stored: u64,
    drops: Vec<(SkipReason, u64)>,
    gates: GatePolicyTable,
}

impl ReplayReport {
    fn new(counters: CaptureCounters, gates: GatePolicyTable) -> Self {
        let mut drops: Vec<(SkipReason, u64)> = counters.skipped.into_iter().collect();
        drops.sort_by_key(|(reason, _)| *reason);
        Self {
            stored: counters.stored,
            url_only_stored: counters.url_only_stored,
            drops,
            gates,
        }
    }

    pub fn drop_count(&self, reason: SkipReason) -> u64 {
        self.drops
            .iter()
            .find(|(candidate, _)| *candidate == reason)
            .map(|(_, count)| *count)
            .unwrap_or(0)
    }

    /// Every observed `SkipReason` and its count, in `SkipReason`'s
    /// canonical declaration order.
    pub fn drops(&self) -> &[(SkipReason, u64)] {
        &self.drops
    }

    pub fn gates(&self) -> &GatePolicyTable {
        &self.gates
    }

    /// Per-`SkipReason` delta against an earlier run. Every reason that
    /// appeared in either report gets an entry — including one that fell
    /// to zero, or rose from zero — so a gate silently absorbing another
    /// gate's frames shows up as one count falling to zero while another
    /// rises by the same amount, never as a row that quietly disappears.
    /// This is the T-309 AC artifact: running the harness before and after
    /// a gate policy change surfaces exactly which named gate's drop count
    /// moved, and by how much.
    pub fn delta(&self, before: &ReplayReport) -> Vec<GateDelta> {
        let mut reasons: BTreeSet<SkipReason> = BTreeSet::new();
        reasons.extend(self.drops.iter().map(|(reason, _)| *reason));
        reasons.extend(before.drops.iter().map(|(reason, _)| *reason));
        reasons
            .into_iter()
            .map(|reason| {
                let before_count = before.drop_count(reason);
                let after_count = self.drop_count(reason);
                GateDelta {
                    reason,
                    before: before_count,
                    after: after_count,
                    delta: after_count as i64 - before_count as i64,
                }
            })
            .collect()
    }

    /// A human-readable report: totals, then every observed drop reason in
    /// canonical order, annotated with whether the declarative table
    /// governs it and its current enabled state.
    pub fn render(&self) -> String {
        let mut out = format!(
            "stored={} url_only_stored={}\n",
            self.stored, self.url_only_stored
        );
        for (reason, count) in &self.drops {
            match self
                .gates
                .entries()
                .iter()
                .find(|entry| entry.reason == *reason)
            {
                Some(entry) => {
                    let state = if entry.enabled { "enabled" } else { "disabled" };
                    out.push_str(&format!("  {reason:?}: {count} ({state})\n"));
                }
                None => out.push_str(&format!("  {reason:?}: {count}\n")),
            }
        }
        out
    }

    /// A human-readable before/after delta table, e.g. for a PR body.
    pub fn render_delta(&self, before: &ReplayReport) -> String {
        let mut out = String::new();
        for GateDelta {
            reason,
            before: before_count,
            after,
            delta,
        } in self.delta(before)
        {
            out.push_str(&format!(
                "  {reason:?}: {before_count} -> {after} ({delta:+})\n"
            ));
        }
        out
    }
}

/// One row of a [`ReplayReport::delta`] table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GateDelta {
    pub reason: SkipReason,
    pub before: u64,
    pub after: u64,
    pub delta: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(app_name: &str, ocr_text: &str, pixel_rgb: [u8; 3], at_ms: u64) -> ReplayRecord {
        ReplayRecord {
            app_name: app_name.to_owned(),
            bundle_id: Some("com.example.app".to_owned()),
            window_title: "Window".to_owned(),
            url: None,
            observed_at_ms: at_ms,
            captured_at_ms: at_ms,
            pixel_rgb,
            ocr_text: ocr_text.to_owned(),
            ocr_confidence: 0.9,
            ocr_block_count: 3,
            private: false,
        }
    }

    #[test]
    fn each_fixture_advances_dedup_state_like_a_real_tick_sequence() {
        let records = vec![
            record("Finder", "first meaningful capture text", [0, 0, 0], 1_000),
            record("Finder", "first meaningful capture text", [0, 0, 0], 1_100),
        ];
        let report = run_replay(records, CapturePipelineConfig::default());
        assert_eq!(report.stored, 1);
        assert_eq!(report.drop_count(SkipReason::PerceptualDuplicate), 1);
    }

    #[test]
    fn disabling_a_gate_shows_a_visible_delta_not_a_silent_behavior_change() {
        let records = vec![
            record("Finder", "short", [10, 10, 10], 1_000),
            record(
                "Finder",
                "a completely different and long enough capture",
                [200, 10, 10],
                2_000,
            ),
        ];
        let before = run_replay(records.clone(), CapturePipelineConfig::default());
        let mut disabled_config = CapturePipelineConfig::default();
        disabled_config.gates.set_enabled(SkipReason::LowSignal, false);
        let after = run_replay(records, disabled_config);

        let delta = after.delta(&before);
        let low_signal = delta
            .iter()
            .find(|entry| entry.reason == SkipReason::LowSignal)
            .expect("LowSignal must appear in the delta even though it falls to zero");
        assert_eq!(low_signal.before, 1);
        assert_eq!(low_signal.after, 0);
        assert_eq!(low_signal.delta, -1);
        assert_eq!(after.stored, before.stored + 1);
    }

    #[test]
    fn private_fixture_stops_before_the_gate_table_entirely() {
        let mut private_record = record("1Password", "vault contents", [0, 0, 0], 1_000);
        private_record.private = true;
        let report = run_replay(vec![private_record], CapturePipelineConfig::default());
        assert_eq!(report.drop_count(SkipReason::PreCapturePrivacy), 1);
        assert_eq!(report.stored, 0);
    }

    #[test]
    fn render_includes_every_observed_reason_and_its_configurability() {
        let records = vec![record("Finder", "short", [0, 0, 0], 1_000)];
        let report = run_replay(records, CapturePipelineConfig::default());
        let rendered = report.render();
        assert!(rendered.contains("LowSignal"));
        assert!(rendered.contains("enabled"));
    }

    #[test]
    fn loading_fixtures_skips_blank_and_comment_lines() {
        let dir = std::env::temp_dir().join(format!(
            "fndr-capture-replay-fixture-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tiny.jsonl");
        std::fs::write(
            &path,
            "# a header comment\n\n{\"app_name\":\"Finder\",\"observed_at_ms\":1000,\"captured_at_ms\":1000,\"pixel_rgb\":[0,0,0],\"ocr_text\":\"meaningful captured text\"}\n",
        )
        .unwrap();

        let records = load_fixtures(&path).expect("valid fixture file");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].app_name, "Finder");

        std::fs::remove_dir_all(&dir).ok();
    }
}
