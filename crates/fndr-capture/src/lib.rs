//! ScreenCaptureKit sampling, dedup, admission policy, session identity, the staged capture pipeline.
//!
//! The `FrameSource` seam plus three sources: the real ScreenCaptureKit
//! provider (T-302), a checked-in-PNG source for tests and demos, and the
//! original `screencapture(1)` shellout the walking skeleton used. Adaptive
//! sampling, dedup, and admission stages arrive with T-303/T-304/T-306.

mod admission;
mod dedup;
mod foreground;
mod pipeline;
mod sampling;
mod source;

pub use admission::{CaptureSurfacePolicy, classify_capture_surface_policy};
pub use dedup::{PerceptualDeduper, PerceptualSignature, SemanticDedupWindow, semantic_signature};
pub use foreground::MacOSForegroundContextSource;
pub use pipeline::{
    CaptureContext, CaptureContextSource, CaptureCounters, CapturePipeline, CapturePipelineConfig,
    CaptureSink, CaptureStage, CaptureTickOutcome, GateDecision, OcrOutput, OcrRecognizer,
    PersistenceOutcome, PipelineError, PreCaptureGate, SkipReason,
};
pub use sampling::{SamplingDecision, SamplingPolicy};
pub use source::{
    CaptureError, Frame, FrameSource, PngFileSource, ScreenCaptureKitSource, ScreencaptureCliSource,
};
