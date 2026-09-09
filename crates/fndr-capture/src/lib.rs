//! ScreenCaptureKit sampling, dedup, admission policy, session identity, the staged capture pipeline.
//!
//! The `FrameSource` seam plus three sources: the real ScreenCaptureKit
//! provider (T-302), a checked-in-PNG source for tests and demos, and the
//! original `screencapture(1)` shellout the walking skeleton used. Adaptive
//! sampling, dedup, and admission stages arrive with T-303/T-304/T-306.

mod admission;
mod dedup;
mod foreground;
mod gate_policy;
mod pipeline;
mod replay;
mod sampling;
mod source;

pub use admission::{
    ADMISSION_RULES, AdmissionRule, CaptureSurfacePolicy, classify_capture_surface,
    classify_capture_surface_policy,
};
pub use dedup::{PerceptualDeduper, PerceptualSignature, SemanticDedupWindow, semantic_signature};
pub use foreground::MacOSForegroundContextSource;
pub use gate_policy::{
    CaptureGatePolicy, DEFAULT_GATE_RULES, GateAction, GateCheck, GateEvaluation, GateId,
    GateInput, GateOutcome, GateRule, GateStage, PolicyGate,
};
pub use pipeline::{
    CaptureContext, CaptureContextSource, CaptureCounters, CapturePipeline, CapturePipelineConfig,
    CaptureSink, CaptureStage, CaptureTickOutcome, GateDecision, OcrOutput, OcrRecognizer,
    PersistenceOutcome, PipelineError, PreCaptureGate, SkipReason,
};
pub use replay::{
    Disposition, FixtureVerdict, GateActivity, GateDelta, ReplayCorpus, ReplayDelta, ReplayFixture,
    ReplayReport, replay, skip_reason_for,
};
pub use sampling::{InputIdleSource, MacOSInputIdle, SamplingDecision, SamplingPolicy};
pub use source::{
    CaptureError, Frame, FrameSource, PngFileSource, ScreenCaptureKitSource, ScreencaptureCliSource,
};
