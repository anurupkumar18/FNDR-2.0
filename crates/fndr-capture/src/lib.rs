//! ScreenCaptureKit sampling, dedup, admission policy, session identity, the staged capture pipeline.
//!
//! The `FrameSource` seam plus three sources: the real ScreenCaptureKit
//! provider (T-302), a checked-in-PNG source for tests and demos, and the
//! original `screencapture(1)` shellout the walking skeleton used. Adaptive
//! sampling, dedup, and admission stages arrive with T-303/T-304/T-306.

mod source;

pub use source::{
    CaptureError, Frame, FrameSource, PngFileSource, ScreenCaptureKitSource, ScreencaptureCliSource,
};
