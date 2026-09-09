//! The synchronous, stage-testable core of the continuous capture pipeline.
//!
//! Platform work (foreground metadata, ScreenCaptureKit, Vision, SQLite, and
//! Lance flushing) stays behind narrow traits. The macOS scheduler owns those
//! adapters on its dedicated thread; this module owns the ordering and makes
//! every terminal outcome observable.

use std::collections::HashMap;

use crate::{
    CaptureError, CaptureSurfacePolicy, Frame, FrameSource, GatePolicyTable, PerceptualDeduper,
    SemanticDedupWindow, classify_capture_surface_policy, semantic_signature,
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
///
/// `PartialOrd`/`Ord` give the variants a stable, deterministic sort order
/// (their declaration order) so the replay harness (`replay.rs`) can render
/// a gate report and a before/after delta table without inventing a second
/// ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
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
///
/// `gates` is the declarative policy table (T-309): it decides whether a
/// firing threshold check (`perceptual_dedup_distance`, `min_ocr_chars`,
/// `semantic_dedup_window_ms`) or the admission classifier is allowed to
/// drop the tick, without changing where in the sequence that check runs.
/// Not `Copy` because `GatePolicyTable` owns a `Vec`; every other field
/// stays plain data so the common "just override one threshold" case reads
/// as a small struct update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturePipelineConfig {
    pub min_ocr_chars: usize,
    pub semantic_dedup_window_ms: u64,
    /// dHash distance below which two frames are perceptual duplicates.
    /// Was hardcoded via `PerceptualDeduper::default()` before T-309.
    pub perceptual_dedup_distance: u32,
    pub gates: GatePolicyTable,
}

impl Default for CapturePipelineConfig {
    fn default() -> Self {
        Self {
            min_ocr_chars: 12,
            semantic_dedup_window_ms: 30_000,
            perceptual_dedup_distance: 5,
            gates: GatePolicyTable::default(),
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
            perceptual_deduper: PerceptualDeduper::new(config.perceptual_dedup_distance),
            semantic_deduper: SemanticDedupWindow::default(),
            counters: CaptureCounters::default(),
            config,
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
        // loop itself is a clean T-306 rewrite per ADR-005. Whether a
        // `SkipFrame` verdict actually drops the tick is a declarative gate
        // policy decision (`self.config.gates`, see `gate_policy.rs`), not
        // baked into this branch: disabling the `AdmissionPolicy` gate
        // treats `SkipFrame` the same as `Normal` and lets the tick proceed
        // to capture, while `UrlOnly` always routes to its own persistence
        // path regardless of the table (it is not a drop, so it is not a
        // table entry).
        match classify_capture_surface_policy(
            &context.app_name,
            &context.window_title,
            context.url.as_deref(),
        ) {
            CaptureSurfacePolicy::SkipFrame
                if self.config.gates.is_enabled(SkipReason::AdmissionPolicy) =>
            {
                return CaptureTickOutcome::Skipped(SkipReason::AdmissionPolicy);
            }
            CaptureSurfacePolicy::UrlOnly => {
                return self.persist_url_only(&context);
            }
            CaptureSurfacePolicy::SkipFrame | CaptureSurfacePolicy::Normal => {}
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
        // The deduper always observes the signature (its A-B-A loop history
        // must advance every tick regardless of policy); the gate table
        // only decides whether a positive result is allowed to drop the
        // tick.
        let is_perceptual_duplicate = self.perceptual_deduper.should_skip(signature);
        if is_perceptual_duplicate && self.config.gates.is_enabled(SkipReason::PerceptualDuplicate)
        {
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
        if ocr.low_signal && self.config.gates.is_enabled(SkipReason::LowSignal) {
            return CaptureTickOutcome::Skipped(SkipReason::LowSignal);
        }

        let signature = semantic_signature(&context.app_name, &context.window_title, &ocr.text);
        let is_semantic_duplicate = self.semantic_deduper.should_skip(
            signature,
            frame.captured_at_ms,
            self.config.semantic_dedup_window_ms,
        );
        if is_semantic_duplicate && self.config.gates.is_enabled(SkipReason::SemanticDuplicate) {
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
        let is_semantic_duplicate = self.semantic_deduper.should_skip(
            signature,
            context.observed_at_ms,
            self.config.semantic_dedup_window_ms,
        );
        if is_semantic_duplicate && self.config.gates.is_enabled(SkipReason::SemanticDuplicate) {
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

    /// Returns a different OCR text on each call, so a test with a fixed
    /// `Context` can still produce distinct semantic-dedup signatures
    /// across ticks (the pipeline hashes `(app_name, window_title, text)`),
    /// isolating whatever earlier gate the test is actually exercising.
    struct DistinctTextOcr(RefCell<VecDeque<&'static str>>);

    impl DistinctTextOcr {
        fn new(texts: &[&'static str]) -> Self {
            Self(RefCell::new(texts.iter().copied().collect()))
        }
    }

    impl OcrRecognizer for DistinctTextOcr {
        fn recognize(
            &self,
            _png: &[u8],
            _app_name: &str,
            _bundle_id: Option<&str>,
            _min_chars: usize,
        ) -> Result<OcrOutput, PipelineError> {
            let text = self
                .0
                .borrow_mut()
                .pop_front()
                .expect("test supplied enough OCR texts for every tick");
            Ok(OcrOutput {
                text: text.to_owned(),
                confidence: 0.9,
                block_count: 3,
                low_signal: false,
            })
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

    // ── Declarative gate policy table (T-309) ───────────────────────────
    //
    // These tests prove the table actually controls `run_tick`, not just
    // that `GatePolicyTable` itself works in isolation (already covered in
    // `gate_policy.rs`): disabling a gate must let a tick that would
    // otherwise have been dropped by it reach storage instead, with every
    // other gate's behavior unchanged.

    #[test]
    fn disabling_the_admission_gate_lets_a_skip_frame_surface_capture_normally() {
        let mut config = CapturePipelineConfig::default();
        config.gates.set_enabled(SkipReason::AdmissionPolicy, false);
        let mut pipeline = CapturePipeline::new(
            Context(context(
                "Google Chrome",
                "Search results - YouTube",
                Some("https://www.youtube.com/results?search_query=screenpipe"),
            )),
            Frames(RefCell::new(
                vec![frame(Some(signature([0, 0, 0])), 1_000)].into(),
            )),
            Gate(GateDecision::Allow),
            Ocr(OcrOutput {
                text: "meaningful captured text".to_owned(),
                confidence: 0.9,
                block_count: 3,
                low_signal: false,
            }),
            Sink::default(),
            config,
        );

        // With the gate enabled (the default), this exact context is the
        // `admission::tests::skips_known_navigation_results_pages` fixture
        // and always drops with `AdmissionPolicy`.
        assert_eq!(pipeline.run_tick(), CaptureTickOutcome::Stored);
        assert_eq!(pipeline.sink().captures, 1);
        assert_eq!(
            pipeline.counters().skip_count(SkipReason::AdmissionPolicy),
            0
        );
    }

    #[test]
    fn disabling_perceptual_dedup_stores_frames_the_default_table_would_drop() {
        let mut config = CapturePipelineConfig::default();
        config
            .gates
            .set_enabled(SkipReason::PerceptualDuplicate, false);
        let mut pipeline = CapturePipeline::new(
            Context(context("Finder", "Project", None)),
            Frames(RefCell::new(
                vec![
                    frame(Some(signature([0, 0, 0])), 1_000),
                    frame(Some(signature([0, 0, 0])), 1_100),
                ]
                .into(),
            )),
            Gate(GateDecision::Allow),
            DistinctTextOcr::new(&["meaningful captured text one", "a distinct second capture"]),
            Sink::default(),
            config,
        );

        assert_eq!(pipeline.run_tick(), CaptureTickOutcome::Stored);
        // Identical pixels would be `PerceptualDuplicate` with the gate on;
        // disabled, the second tick still reaches OCR and storage. The two
        // ticks use distinct OCR text so semantic dedup (a separate,
        // still-enabled gate) does not also catch the second tick and mask
        // what this test is actually proving.
        assert_eq!(pipeline.run_tick(), CaptureTickOutcome::Stored);
        assert_eq!(pipeline.sink().captures, 2);
        assert_eq!(
            pipeline
                .counters()
                .skip_count(SkipReason::PerceptualDuplicate),
            0
        );
    }

    #[test]
    fn disabling_low_signal_stores_the_frame_the_ocr_adapter_flagged() {
        let mut config = CapturePipelineConfig::default();
        config.gates.set_enabled(SkipReason::LowSignal, false);
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
            }),
            Sink::default(),
            config,
        );

        assert_eq!(pipeline.run_tick(), CaptureTickOutcome::Stored);
        assert_eq!(pipeline.sink().captures, 1);
        assert_eq!(pipeline.counters().skip_count(SkipReason::LowSignal), 0);
    }

    #[test]
    fn disabling_semantic_dedup_stores_repeated_content() {
        let mut config = CapturePipelineConfig::default();
        config.gates.set_enabled(SkipReason::SemanticDuplicate, false);
        let mut pipeline = CapturePipeline::new(
            Context(context("Finder", "Project", None)),
            Frames(RefCell::new(
                vec![
                    frame(Some(signature([0, 0, 0])), 1_000),
                    frame(Some(signature([255, 0, 0])), 1_100),
                ]
                .into(),
            )),
            Gate(GateDecision::Allow),
            Ocr(OcrOutput {
                text: "meaningful captured text".to_owned(),
                confidence: 0.9,
                block_count: 3,
                low_signal: false,
            }),
            Sink::default(),
            config,
        );

        assert_eq!(pipeline.run_tick(), CaptureTickOutcome::Stored);
        // Same (app, title, text) as the first tick; with the gate on this
        // is `semantic_duplicate_is_counted_after_ocr_without_second_write`
        // above.
        assert_eq!(pipeline.run_tick(), CaptureTickOutcome::Stored);
        assert_eq!(pipeline.sink().captures, 2);
        assert_eq!(
            pipeline.counters().skip_count(SkipReason::SemanticDuplicate),
            0
        );
    }

    #[test]
    fn perceptual_dedup_threshold_is_config_driven_not_hardcoded() {
        // Two flat-colour frames always hash to the same dHash (a flat
        // image has no internal gradient), so their perceptual "distance"
        // is always 0; `colour_guard_keeps_different_flat_fields` in
        // dedup.rs exists for exactly this reason. Colour distance 22
        // between the two frames below sits between the deduper's two
        // colour guards (primary <= 24, A-B-A loop <= 20): the default
        // threshold (5) clears the primary hash check and stores a
        // duplicate skip on distance+colour alone, while a caller-supplied
        // threshold of 0 fails the primary hash check (0 < 0 is false) and
        // falls through to the loop guard, which distance 22 also fails
        // (22 > 20). The flip below is therefore driven by
        // `perceptual_dedup_distance`, not incidental dedup history.
        let first = [0_u8, 0, 0];
        let second = [22_u8, 0, 0];
        let ocr = || OcrOutput {
            text: "meaningful captured text".to_owned(),
            confidence: 0.9,
            block_count: 3,
            low_signal: false,
        };

        let mut default_pipeline = CapturePipeline::new(
            Context(context("Finder", "Project", None)),
            Frames(RefCell::new(
                vec![
                    frame(Some(signature(first)), 1_000),
                    frame(Some(signature(second)), 1_100),
                ]
                .into(),
            )),
            Gate(GateDecision::Allow),
            Ocr(ocr()),
            Sink::default(),
            CapturePipelineConfig::default(),
        );
        assert_eq!(default_pipeline.run_tick(), CaptureTickOutcome::Stored);
        assert_eq!(
            default_pipeline.run_tick(),
            CaptureTickOutcome::Skipped(SkipReason::PerceptualDuplicate)
        );

        let mut zero_threshold_config = CapturePipelineConfig::default();
        zero_threshold_config.perceptual_dedup_distance = 0;
        let mut zero_threshold_pipeline = CapturePipeline::new(
            Context(context("Finder", "Project", None)),
            Frames(RefCell::new(
                vec![
                    frame(Some(signature(first)), 1_000),
                    frame(Some(signature(second)), 1_100),
                ]
                .into(),
            )),
            Gate(GateDecision::Allow),
            // Distinct text per tick so semantic dedup (unaffected by
            // `perceptual_dedup_distance`) does not also catch the second
            // tick and mask what this test is proving.
            DistinctTextOcr::new(&["meaningful captured text", "a distinct second capture"]),
            Sink::default(),
            zero_threshold_config,
        );
        assert_eq!(
            zero_threshold_pipeline.run_tick(),
            CaptureTickOutcome::Stored
        );
        assert_eq!(
            zero_threshold_pipeline.run_tick(),
            CaptureTickOutcome::Stored
        );
        assert_eq!(zero_threshold_pipeline.sink().captures, 2);
    }
}
