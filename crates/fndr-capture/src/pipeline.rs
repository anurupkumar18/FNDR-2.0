//! The synchronous, stage-testable core of the continuous capture pipeline.
//!
//! Platform work (foreground metadata, ScreenCaptureKit, Vision, SQLite, and
//! Lance flushing) stays behind narrow traits. The macOS scheduler owns those
//! adapters on its dedicated thread; this module owns the ordering and makes
//! every terminal outcome observable.

use std::collections::HashMap;

use crate::{
    CaptureError, CaptureSurfacePolicy, Frame, FrameSource, PerceptualDeduper, SemanticDedupWindow,
    classify_capture_surface_policy, semantic_signature,
};

/// Foreground metadata acquired before any pixel capture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureContext {
    pub app_name: String,
    pub bundle_id: Option<String>,
    pub window_title: String,
    pub url: Option<String>,
    pub observed_at_ms: u64,
}

/// A typed boundary error. The scheduler reports the stage and message rather
/// than quietly treating a failed subsystem as an empty frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineError {
    pub stage: CaptureStage,
    pub message: String,
}

impl PipelineError {
    pub fn new(stage: CaptureStage, message: impl Into<String>) -> Self {
        Self {
            stage,
            message: message.into(),
        }
    }
}

/// A named stage for failure reporting and operational diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureStage {
    Metadata,
    Capture,
    Ocr,
    Persistence,
}

/// The single terminal reason counted for a skipped or failed tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SkipReason {
    MetadataUnavailable,
    PreCapturePrivacy,
    /// Metadata carried a private/incognito browsing cue. This is separate
    /// from the generic privacy bucket so the owner can see why capture was
    /// withheld without receiving the window title that triggered it.
    PrivateBrowsing,
    AdmissionPolicy,
    PerceptualDuplicate,
    MissingPerceptualSignature,
    LowSignal,
    SemanticDuplicate,
    FinalPrivacy,
    /// Screen Recording is unavailable or the macOS capture boundary failed
    /// before a frame was available. This remains distinct from generic
    /// capture failure so the desktop owner can give safe re-grant guidance
    /// without exposing the underlying system error.
    ScreenRecordingOrCaptureUnavailable,
    CaptureFailed,
    OcrFailed,
    PersistenceFailed,
}

/// Where to get metadata for the currently foreground surface.
pub trait CaptureContextSource {
    fn current_context(&self) -> Result<CaptureContext, PipelineError>;
}

/// The privacy gate which must decide before pixels are captured.
pub trait PreCaptureGate {
    fn evaluate(&self, context: &CaptureContext) -> GateDecision;
}

/// The pre-capture privacy decision, deliberately separate from the final
/// persistence recheck.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateDecision {
    Allow,
    Skip(SkipReason),
}

/// How much evidence the OCR and cleanup stages discarded from one frame.
///
/// This is the content-free half of the OCR boundary's result: line counts
/// only, never a line, an app name, a URL, or a window title. It exists so a
/// capture-health caller can answer "how much of this capture was thrown
/// away, and by which stage" without ever being handed the captured text.
/// Before it existed, both aggregates were computed at the Vision boundary
/// and immediately dropped, which is exactly the silent-degradation shape
/// invariant 4 exists to prevent.
///
/// `recognized` counts what the recognizer saw; `cleanup_*` counts what the
/// app-aware cleanup pass then did to the lines the recognizer kept. The two
/// stages are deliberately not summed into one number: an operator needs to
/// know whether a thin capture came from a bad screenshot or an aggressive
/// cleanup rule.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct OcrQualitySample {
    /// Lines the recognizer produced before any filtering.
    pub recognized_lines: u32,
    /// Lines the recognizer kept after its own confidence filter.
    pub recognized_lines_kept: u32,
    /// Lines the recognizer dropped for low confidence.
    pub recognized_lines_dropped: u32,
    /// Kept lines the recognizer marked as low confidence.
    pub low_confidence_lines: u32,
    /// Lines the app-aware cleanup pass examined.
    pub cleanup_lines: u32,
    /// Lines cleanup kept as high-signal evidence.
    pub cleanup_lines_kept: u32,
    /// Lines cleanup dropped as chrome/noise (tab rows, menu bars).
    pub cleanup_lines_dropped_noise: u32,
    /// Lines cleanup dropped for scoring below its signal floor.
    pub cleanup_lines_dropped_low_signal: u32,
}

/// Normalized OCR output used by the scheduler's low-signal and semantic
/// stages. The Vision adapter converts its richer result into this value.
#[derive(Debug, Clone, PartialEq)]
pub struct OcrOutput {
    pub text: String,
    pub confidence: f32,
    pub block_count: usize,
    /// Calculated by the OCR adapter with its full, engine-owned signal rule.
    /// Keeping that rule at the Vision boundary avoids a second, drift-prone
    /// interpretation of OCR quality in the scheduler.
    pub low_signal: bool,
    /// Content-free counts describing what the OCR and cleanup stages
    /// discarded. Carried, not consumed: the pipeline never routes on it.
    pub quality: OcrQualitySample,
}

/// The OCR boundary; implementations must return cleaned, not raw, text.
///
/// `app_name` and `bundle_id` are metadata acquired before pixels and let the
/// adapter apply app-aware cleanup without leaking that policy into the
/// scheduler. Both are passed because the pair is the app's identity: the
/// bundle identifier is authoritative and stable, while the localized name is
/// user- and locale-controlled and only resolves apps the identifier misses.
/// The scheduler forwards them and stays policy-agnostic; it deliberately does
/// not hand the recognizer the whole `CaptureContext`, whose URL and window
/// title are not the OCR boundary's business.
pub trait OcrRecognizer {
    fn recognize(
        &self,
        png: &[u8],
        app_name: &str,
        bundle_id: Option<&str>,
        min_chars: usize,
    ) -> Result<OcrOutput, PipelineError>;
}

/// The result from the final persistence boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersistenceOutcome {
    Stored,
    SkippedFinalPrivacy,
}

/// SQLite-facing storage boundary. Implementations must perform the final
/// privacy recheck immediately before their transaction.
pub trait CaptureSink {
    fn persist_capture(
        &mut self,
        context: &CaptureContext,
        frame: &Frame,
        ocr: &OcrOutput,
    ) -> Result<PersistenceOutcome, PipelineError>;

    fn persist_url_only(
        &mut self,
        context: &CaptureContext,
    ) -> Result<PersistenceOutcome, PipelineError>;
}

/// Scheduler-tunable values which do not change stage ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapturePipelineConfig {
    pub min_ocr_chars: usize,
    pub semantic_dedup_window_ms: u64,
}

impl Default for CapturePipelineConfig {
    fn default() -> Self {
        Self {
            min_ocr_chars: 12,
            semantic_dedup_window_ms: 30_000,
        }
    }
}

/// The terminal result from one scheduler tick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureTickOutcome {
    Stored,
    UrlOnlyStored,
    Skipped(SkipReason),
    Failed {
        reason: SkipReason,
        error: PipelineError,
    },
}

/// Counters for the current scheduler lifetime. Every `run_tick` call updates
/// exactly one terminal category, so an operator can distinguish inactivity
/// from policy suppression or a broken boundary.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CaptureCounters {
    pub stored: u64,
    pub url_only_stored: u64,
    pub skipped: HashMap<SkipReason, u64>,
    /// Lifetime totals of what OCR and cleanup discarded, so capture health
    /// can report degradation instead of only counting terminal outcomes.
    pub quality: CaptureQualityTotals,
}

/// Scheduler-lifetime sums of `OcrQualitySample`, plus the ratios a health
/// surface actually reads. Content-free by construction: it is built only
/// from counts, and there is no field a line of captured text could reach.
///
/// Samples accumulate for every tick whose OCR stage returned, including
/// ticks later skipped as low-signal or semantically duplicate. That is
/// deliberate: the discard a health surface most needs to explain is exactly
/// the one that made a capture too thin to store.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CaptureQualityTotals {
    /// Ticks that reached and completed the OCR boundary.
    pub samples: u64,
    pub recognized_lines: u64,
    pub recognized_lines_kept: u64,
    pub recognized_lines_dropped: u64,
    pub low_confidence_lines: u64,
    pub cleanup_lines: u64,
    pub cleanup_lines_kept: u64,
    pub cleanup_lines_dropped_noise: u64,
    pub cleanup_lines_dropped_low_signal: u64,
}

impl CaptureQualityTotals {
    fn accumulate(&mut self, sample: &OcrQualitySample) {
        self.samples = self.samples.saturating_add(1);
        self.recognized_lines = self
            .recognized_lines
            .saturating_add(u64::from(sample.recognized_lines));
        self.recognized_lines_kept = self
            .recognized_lines_kept
            .saturating_add(u64::from(sample.recognized_lines_kept));
        self.recognized_lines_dropped = self
            .recognized_lines_dropped
            .saturating_add(u64::from(sample.recognized_lines_dropped));
        self.low_confidence_lines = self
            .low_confidence_lines
            .saturating_add(u64::from(sample.low_confidence_lines));
        self.cleanup_lines = self
            .cleanup_lines
            .saturating_add(u64::from(sample.cleanup_lines));
        self.cleanup_lines_kept = self
            .cleanup_lines_kept
            .saturating_add(u64::from(sample.cleanup_lines_kept));
        self.cleanup_lines_dropped_noise = self
            .cleanup_lines_dropped_noise
            .saturating_add(u64::from(sample.cleanup_lines_dropped_noise));
        self.cleanup_lines_dropped_low_signal = self
            .cleanup_lines_dropped_low_signal
            .saturating_add(u64::from(sample.cleanup_lines_dropped_low_signal));
    }

    /// Fraction of recognizer lines dropped for low confidence, or `None`
    /// when no line has been observed yet. `None` rather than `0.0` because
    /// "nothing captured" and "nothing discarded" are different health
    /// answers, and collapsing them is the silent-degradation shape.
    pub fn recognized_discard_ratio(&self) -> Option<f64> {
        ratio(self.recognized_lines_dropped, self.recognized_lines)
    }

    /// Fraction of cleanup-examined lines dropped by either cleanup rule.
    pub fn cleanup_discard_ratio(&self) -> Option<f64> {
        ratio(
            self.cleanup_lines_dropped_noise
                .saturating_add(self.cleanup_lines_dropped_low_signal),
            self.cleanup_lines,
        )
    }
}

fn ratio(part: u64, whole: u64) -> Option<f64> {
    (whole > 0).then(|| part as f64 / whole as f64)
}

impl CaptureCounters {
    pub fn skip_count(&self, reason: SkipReason) -> u64 {
        self.skipped.get(&reason).copied().unwrap_or(0)
    }

    fn record(&mut self, outcome: &CaptureTickOutcome) {
        match outcome {
            CaptureTickOutcome::Stored => self.stored += 1,
            CaptureTickOutcome::UrlOnlyStored => self.url_only_stored += 1,
            CaptureTickOutcome::Skipped(reason) | CaptureTickOutcome::Failed { reason, .. } => {
                *self.skipped.entry(*reason).or_default() += 1;
            }
        }
    }
}

/// A composable capture tick. This is synchronous by design: the shell runs
/// it on the capture-owned worker, never on the Tauri async runtime.
pub struct CapturePipeline<C, F, G, O, S> {
    context_source: C,
    frame_source: F,
    gate: G,
    ocr: O,
    sink: S,
    config: CapturePipelineConfig,
    perceptual_deduper: PerceptualDeduper,
    semantic_deduper: SemanticDedupWindow,
    counters: CaptureCounters,
}

impl<C, F, G, O, S> CapturePipeline<C, F, G, O, S>
where
    C: CaptureContextSource,
    F: FrameSource,
    G: PreCaptureGate,
    O: OcrRecognizer,
    S: CaptureSink,
{
    pub fn new(
        context_source: C,
        frame_source: F,
        gate: G,
        ocr: O,
        sink: S,
        config: CapturePipelineConfig,
    ) -> Self {
        Self {
            context_source,
            frame_source,
            gate,
            ocr,
            sink,
            config,
            perceptual_deduper: PerceptualDeduper::default(),
            semantic_deduper: SemanticDedupWindow::default(),
            counters: CaptureCounters::default(),
        }
    }

    pub fn counters(&self) -> &CaptureCounters {
        &self.counters
    }

    pub fn sink(&self) -> &S {
        &self.sink
    }

    pub fn sink_mut(&mut self) -> &mut S {
        &mut self.sink
    }

    /// Execute the documented scheduler order for one capture opportunity.
    pub fn run_tick(&mut self) -> CaptureTickOutcome {
        let outcome = self.run_tick_inner();
        self.counters.record(&outcome);
        outcome
    }

    fn run_tick_inner(&mut self) -> CaptureTickOutcome {
        let context = match self.context_source.current_context() {
            Ok(context) => context,
            Err(error) => return failed(SkipReason::MetadataUnavailable, error),
        };

        if let GateDecision::Skip(reason) = self.gate.evaluate(&context) {
            return CaptureTickOutcome::Skipped(reason);
        }

        // Ported classification from FNDR v1 admission.rs at 330a760b; the
        // loop itself is a clean T-306 rewrite per ADR-005.
        match classify_capture_surface_policy(
            &context.app_name,
            &context.window_title,
            context.url.as_deref(),
        ) {
            CaptureSurfacePolicy::SkipFrame => {
                return CaptureTickOutcome::Skipped(SkipReason::AdmissionPolicy);
            }
            CaptureSurfacePolicy::UrlOnly => {
                return self.persist_url_only(&context);
            }
            CaptureSurfacePolicy::Normal => {}
        }

        let frame = match self.frame_source.grab() {
            Ok(frame) => frame,
            Err(error) => {
                let reason = if matches!(&error, CaptureError::PermissionOrTool(_)) {
                    SkipReason::ScreenRecordingOrCaptureUnavailable
                } else {
                    SkipReason::CaptureFailed
                };
                return failed(
                    reason,
                    PipelineError::new(CaptureStage::Capture, error.to_string()),
                );
            }
        };
        let Some(signature) = frame.perceptual_signature.clone() else {
            return CaptureTickOutcome::Skipped(SkipReason::MissingPerceptualSignature);
        };
        if self.perceptual_deduper.should_skip(signature) {
            return CaptureTickOutcome::Skipped(SkipReason::PerceptualDuplicate);
        }

        let ocr = match self.ocr.recognize(
            &frame.png,
            &context.app_name,
            context.bundle_id.as_deref(),
            self.config.min_ocr_chars,
        ) {
            Ok(ocr) => ocr,
            Err(error) => return failed(SkipReason::OcrFailed, error),
        };
        // Recorded before the low-signal branch on purpose: a capture that is
        // discarded for being thin is precisely the one whose discard counts
        // explain why.
        self.counters.quality.accumulate(&ocr.quality);
        if ocr.low_signal {
            return CaptureTickOutcome::Skipped(SkipReason::LowSignal);
        }

        let signature = semantic_signature(&context.app_name, &context.window_title, &ocr.text);
        if self.semantic_deduper.should_skip(
            signature,
            frame.captured_at_ms,
            self.config.semantic_dedup_window_ms,
        ) {
            return CaptureTickOutcome::Skipped(SkipReason::SemanticDuplicate);
        }

        match self.sink.persist_capture(&context, &frame, &ocr) {
            Ok(PersistenceOutcome::Stored) => CaptureTickOutcome::Stored,
            Ok(PersistenceOutcome::SkippedFinalPrivacy) => {
                CaptureTickOutcome::Skipped(SkipReason::FinalPrivacy)
            }
            Err(error) => failed(SkipReason::PersistenceFailed, error),
        }
    }

    fn persist_url_only(&mut self, context: &CaptureContext) -> CaptureTickOutcome {
        let url = context.url.as_deref().unwrap_or_default();
        let signature = semantic_signature(&context.app_name, &context.window_title, url);
        if self.semantic_deduper.should_skip(
            signature,
            context.observed_at_ms,
            self.config.semantic_dedup_window_ms,
        ) {
            return CaptureTickOutcome::Skipped(SkipReason::SemanticDuplicate);
        }

        match self.sink.persist_url_only(context) {
            Ok(PersistenceOutcome::Stored) => CaptureTickOutcome::UrlOnlyStored,
            Ok(PersistenceOutcome::SkippedFinalPrivacy) => {
                CaptureTickOutcome::Skipped(SkipReason::FinalPrivacy)
            }
            Err(error) => failed(SkipReason::PersistenceFailed, error),
        }
    }
}

fn failed(reason: SkipReason, error: PipelineError) -> CaptureTickOutcome {
    CaptureTickOutcome::Failed { reason, error }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::rc::Rc;

    use super::*;
    use crate::PerceptualSignature;

    #[derive(Clone)]
    struct Context(CaptureContext);

    impl CaptureContextSource for Context {
        fn current_context(&self) -> Result<CaptureContext, PipelineError> {
            Ok(self.0.clone())
        }
    }

    struct Frames(RefCell<VecDeque<Frame>>);

    impl FrameSource for Frames {
        fn grab(&self) -> Result<Frame, crate::CaptureError> {
            self.0
                .borrow_mut()
                .pop_front()
                .ok_or(crate::CaptureError::Empty)
        }
    }

    struct PermissionUnavailableFrame;

    impl FrameSource for PermissionUnavailableFrame {
        fn grab(&self) -> Result<Frame, CaptureError> {
            Err(CaptureError::PermissionOrTool("TCC unavailable".into()))
        }
    }

    #[derive(Clone, Copy)]
    struct Gate(GateDecision);

    impl PreCaptureGate for Gate {
        fn evaluate(&self, _context: &CaptureContext) -> GateDecision {
            self.0
        }
    }

    /// One frame's worth of discard counts, with every field distinct so a
    /// test cannot pass by summing the wrong pair.
    const SAMPLE: OcrQualitySample = OcrQualitySample {
        recognized_lines: 20,
        recognized_lines_kept: 17,
        recognized_lines_dropped: 3,
        low_confidence_lines: 4,
        cleanup_lines: 17,
        cleanup_lines_kept: 9,
        cleanup_lines_dropped_noise: 5,
        cleanup_lines_dropped_low_signal: 3,
    };

    struct Ocr(OcrOutput);

    impl OcrRecognizer for Ocr {
        fn recognize(
            &self,
            _png: &[u8],
            _app_name: &str,
            _bundle_id: Option<&str>,
            _min_chars: usize,
        ) -> Result<OcrOutput, PipelineError> {
            Ok(self.0.clone())
        }
    }

    /// Records the app identity the pipeline forwards, so the OCR contract is
    /// tested rather than assumed.
    type SeenIdentity = Rc<RefCell<Option<(String, Option<String>)>>>;

    struct RecordingOcr(SeenIdentity);

    impl OcrRecognizer for RecordingOcr {
        fn recognize(
            &self,
            _png: &[u8],
            app_name: &str,
            bundle_id: Option<&str>,
            _min_chars: usize,
        ) -> Result<OcrOutput, PipelineError> {
            *self.0.borrow_mut() = Some((app_name.to_owned(), bundle_id.map(str::to_owned)));
            Ok(OcrOutput {
                text: "meaningful captured text".to_owned(),
                confidence: 0.9,
                block_count: 3,
                low_signal: false,
                quality: OcrQualitySample::default(),
            })
        }
    }

    #[derive(Default)]
    struct Sink {
        captures: usize,
        urls: usize,
        outcome: Option<PersistenceOutcome>,
    }

    impl CaptureSink for Sink {
        fn persist_capture(
            &mut self,
            _context: &CaptureContext,
            _frame: &Frame,
            _ocr: &OcrOutput,
        ) -> Result<PersistenceOutcome, PipelineError> {
            self.captures += 1;
            Ok(self.outcome.unwrap_or(PersistenceOutcome::Stored))
        }

        fn persist_url_only(
            &mut self,
            _context: &CaptureContext,
        ) -> Result<PersistenceOutcome, PipelineError> {
            self.urls += 1;
            Ok(self.outcome.unwrap_or(PersistenceOutcome::Stored))
        }
    }

    fn signature(rgb: [u8; 3]) -> PerceptualSignature {
        let mut rgba = [0_u8; 9 * 8 * 4];
        for pixel in rgba.as_chunks_mut::<4>().0 {
            pixel[..3].copy_from_slice(&rgb);
            pixel[3] = u8::MAX;
        }
        PerceptualSignature::from_downscaled_rgba(rgba)
    }

    fn context(app_name: &str, title: &str, url: Option<&str>) -> CaptureContext {
        CaptureContext {
            app_name: app_name.to_owned(),
            bundle_id: Some("com.example.app".to_owned()),
            window_title: title.to_owned(),
            url: url.map(str::to_owned),
            observed_at_ms: 1_000,
        }
    }

    fn frame(signature: Option<PerceptualSignature>, captured_at_ms: u64) -> Frame {
        Frame {
            png: vec![1, 2, 3],
            captured_at_ms,
            perceptual_signature: signature,
        }
    }

    fn pipeline(
        context: CaptureContext,
        frames: Vec<Frame>,
        gate: GateDecision,
        sink: Sink,
    ) -> CapturePipeline<Context, Frames, Gate, Ocr, Sink> {
        CapturePipeline::new(
            Context(context),
            Frames(RefCell::new(frames.into())),
            Gate(gate),
            Ocr(OcrOutput {
                text: "meaningful captured text".to_owned(),
                confidence: 0.9,
                block_count: 3,
                low_signal: false,
                quality: SAMPLE,
            }),
            sink,
            CapturePipelineConfig::default(),
        )
    }

    #[test]
    fn pre_capture_privacy_stops_before_pixels_or_storage() {
        let mut pipeline = pipeline(
            context("1Password", "Vault", None),
            vec![frame(Some(signature([0, 0, 0])), 1_000)],
            GateDecision::Skip(SkipReason::PreCapturePrivacy),
            Sink::default(),
        );

        assert_eq!(
            pipeline.run_tick(),
            CaptureTickOutcome::Skipped(SkipReason::PreCapturePrivacy)
        );
        assert_eq!(pipeline.sink().captures, 0);
        assert_eq!(
            pipeline
                .counters()
                .skip_count(SkipReason::PreCapturePrivacy),
            1
        );
    }

    #[test]
    fn url_only_admission_never_grabs_pixels() {
        let mut pipeline = pipeline(
            context(
                "Google Chrome",
                "screen_pipe - YouTube",
                Some("https://www.youtube.com/@screen_pipe/videos"),
            ),
            vec![],
            GateDecision::Allow,
            Sink::default(),
        );

        assert_eq!(pipeline.run_tick(), CaptureTickOutcome::UrlOnlyStored);
        assert_eq!(pipeline.sink().urls, 1);
        assert_eq!(pipeline.sink().captures, 0);
    }

    #[test]
    fn missing_native_signature_is_a_visible_skip_not_a_png_fallback() {
        let mut pipeline = pipeline(
            context("Finder", "Project", None),
            vec![frame(None, 1_000)],
            GateDecision::Allow,
            Sink::default(),
        );

        assert_eq!(
            pipeline.run_tick(),
            CaptureTickOutcome::Skipped(SkipReason::MissingPerceptualSignature)
        );
        assert_eq!(pipeline.sink().captures, 0);
    }

    #[test]
    fn screen_recording_unavailability_has_a_stable_non_secret_reason() {
        let mut pipeline = CapturePipeline::new(
            Context(context("Finder", "Project", None)),
            PermissionUnavailableFrame,
            Gate(GateDecision::Allow),
            Ocr(OcrOutput {
                text: String::new(),
                confidence: 0.0,
                block_count: 0,
                low_signal: true,
                quality: OcrQualitySample::default(),
            }),
            Sink::default(),
            CapturePipelineConfig::default(),
        );

        let outcome = pipeline.run_tick();
        assert!(matches!(
            outcome,
            CaptureTickOutcome::Failed {
                reason: SkipReason::ScreenRecordingOrCaptureUnavailable,
                error: PipelineError {
                    stage: CaptureStage::Capture,
                    ..
                }
            }
        ));
        assert_eq!(
            pipeline
                .counters()
                .skip_count(SkipReason::ScreenRecordingOrCaptureUnavailable),
            1
        );
        assert_eq!(pipeline.sink().captures, 0);
    }

    #[test]
    fn ocr_adapter_signal_decision_blocks_storage_without_reimplementing_vision_rules() {
        let mut pipeline = CapturePipeline::new(
            Context(context("Finder", "Project", None)),
            Frames(RefCell::new(
                vec![frame(Some(signature([0, 0, 0])), 1_000)].into(),
            )),
            Gate(GateDecision::Allow),
            Ocr(OcrOutput {
                text: "short".to_owned(),
                confidence: 0.9,
                block_count: 2,
                low_signal: true,
                quality: SAMPLE,
            }),
            Sink::default(),
            CapturePipelineConfig::default(),
        );

        assert_eq!(
            pipeline.run_tick(),
            CaptureTickOutcome::Skipped(SkipReason::LowSignal)
        );
        assert_eq!(pipeline.sink().captures, 0);
    }

    #[test]
    fn semantic_duplicate_is_counted_after_ocr_without_second_write() {
        let mut pipeline = pipeline(
            context("Finder", "Project", None),
            vec![
                frame(Some(signature([0, 0, 0])), 1_000),
                frame(Some(signature([255, 0, 0])), 1_100),
            ],
            GateDecision::Allow,
            Sink::default(),
        );

        assert_eq!(pipeline.run_tick(), CaptureTickOutcome::Stored);
        assert_eq!(
            pipeline.run_tick(),
            CaptureTickOutcome::Skipped(SkipReason::SemanticDuplicate)
        );
        assert_eq!(pipeline.sink().captures, 1);
    }

    #[test]
    fn final_privacy_recheck_is_observable_at_the_storage_boundary() {
        let mut pipeline = pipeline(
            context("Finder", "Project", None),
            vec![frame(Some(signature([0, 0, 0])), 1_000)],
            GateDecision::Allow,
            Sink {
                outcome: Some(PersistenceOutcome::SkippedFinalPrivacy),
                ..Sink::default()
            },
        );

        assert_eq!(
            pipeline.run_tick(),
            CaptureTickOutcome::Skipped(SkipReason::FinalPrivacy)
        );
        assert_eq!(pipeline.sink().captures, 1);
    }

    #[test]
    fn quality_totals_accumulate_across_ticks_and_expose_discard_ratios() {
        let mut pipeline = pipeline(
            context("Finder", "Project", None),
            vec![
                frame(Some(signature([0, 0, 0])), 1_000),
                frame(Some(signature([255, 0, 0])), 100_000),
            ],
            GateDecision::Allow,
            Sink::default(),
        );

        assert_eq!(pipeline.run_tick(), CaptureTickOutcome::Stored);
        let after_one = pipeline.counters().quality;
        assert_eq!(after_one.samples, 1);
        assert_eq!(after_one.recognized_lines, 20);
        assert_eq!(after_one.cleanup_lines_dropped_noise, 5);

        // Second tick is far outside the semantic window, so it stores too.
        assert_eq!(pipeline.run_tick(), CaptureTickOutcome::Stored);
        let totals = pipeline.counters().quality;
        assert_eq!(totals.samples, 2);
        assert_eq!(totals.recognized_lines, 40);
        assert_eq!(totals.recognized_lines_dropped, 6);
        assert_eq!(totals.low_confidence_lines, 8);
        assert_eq!(totals.cleanup_lines, 34);
        assert_eq!(totals.cleanup_lines_kept, 18);
        assert_eq!(totals.cleanup_lines_dropped_noise, 10);
        assert_eq!(totals.cleanup_lines_dropped_low_signal, 6);

        // 6/40 recognizer lines dropped; (10+6)/34 cleanup lines dropped.
        assert_eq!(totals.recognized_discard_ratio(), Some(6.0 / 40.0));
        assert_eq!(totals.cleanup_discard_ratio(), Some(16.0 / 34.0));
    }

    #[test]
    fn quality_ratios_are_absent_rather_than_zero_before_any_line_is_seen() {
        // "Nothing captured" must not read as "nothing discarded"; that
        // collapse is exactly the silent-degradation shape (invariant 4).
        let totals = CaptureQualityTotals::default();
        assert_eq!(totals.recognized_discard_ratio(), None);
        assert_eq!(totals.cleanup_discard_ratio(), None);
    }

    #[test]
    fn a_low_signal_discard_still_reports_the_counts_that_explain_it() {
        let mut pipeline = CapturePipeline::new(
            Context(context("Finder", "Project", None)),
            Frames(RefCell::new(
                vec![frame(Some(signature([0, 0, 0])), 1_000)].into(),
            )),
            Gate(GateDecision::Allow),
            Ocr(OcrOutput {
                text: "short".to_owned(),
                confidence: 0.9,
                block_count: 2,
                low_signal: true,
                quality: SAMPLE,
            }),
            Sink::default(),
            CapturePipelineConfig::default(),
        );

        assert_eq!(
            pipeline.run_tick(),
            CaptureTickOutcome::Skipped(SkipReason::LowSignal)
        );
        assert_eq!(pipeline.counters().quality.samples, 1);
        assert_eq!(pipeline.counters().quality.cleanup_lines_dropped_noise, 5);
    }

    #[test]
    fn a_tick_that_never_reaches_ocr_records_no_quality_sample() {
        let mut pipeline = pipeline(
            context("1Password", "Vault", None),
            vec![frame(Some(signature([0, 0, 0])), 1_000)],
            GateDecision::Skip(SkipReason::PreCapturePrivacy),
            Sink::default(),
        );

        assert_eq!(
            pipeline.run_tick(),
            CaptureTickOutcome::Skipped(SkipReason::PreCapturePrivacy)
        );
        assert_eq!(pipeline.counters().quality, CaptureQualityTotals::default());
    }

    #[test]
    fn quality_totals_carry_counts_only_and_never_captured_text() {
        let mut pipeline = pipeline(
            context("Finder", "Secret Project Plan", None),
            vec![frame(Some(signature([0, 0, 0])), 1_000)],
            GateDecision::Allow,
            Sink::default(),
        );

        assert_eq!(pipeline.run_tick(), CaptureTickOutcome::Stored);
        let rendered = format!("{:?}", pipeline.counters().quality);
        assert!(!rendered.contains("meaningful captured text"));
        assert!(!rendered.contains("Secret Project Plan"));
        assert!(!rendered.contains("Finder"));
    }

    #[test]
    fn the_ocr_boundary_receives_the_full_app_identity() {
        // Cleanup policy is app-aware and the bundle identifier is its only
        // authoritative signal, so the scheduler must forward both halves.
        // Forwarding the name alone is what left the policy guessing from a
        // user- and locale-controlled string.
        let seen: SeenIdentity = SeenIdentity::default();
        let mut pipeline = CapturePipeline::new(
            Context(CaptureContext {
                app_name: "Navegador".to_owned(),
                bundle_id: Some("com.google.Chrome".to_owned()),
                window_title: "Project".to_owned(),
                url: None,
                observed_at_ms: 1_000,
            }),
            Frames(RefCell::new(
                vec![frame(Some(signature([0, 0, 0])), 1_000)].into(),
            )),
            Gate(GateDecision::Allow),
            RecordingOcr(Rc::clone(&seen)),
            Sink::default(),
            CapturePipelineConfig::default(),
        );

        assert_eq!(pipeline.run_tick(), CaptureTickOutcome::Stored);
        assert_eq!(
            seen.borrow().clone(),
            Some(("Navegador".to_owned(), Some("com.google.Chrome".to_owned())))
        );
    }
}
