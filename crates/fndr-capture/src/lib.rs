//! ScreenCaptureKit sampling, dedup, admission policy, session identity, the staged capture pipeline.
//!
//! Walking-skeleton state (T-109): only the `FrameSource` seam and two
//! deliberately simple sources exist. The real SCK provider, adaptive
//! sampling, dedup, and admission stages arrive with T-302 and friends.

mod source;

pub use source::{CaptureError, Frame, FrameSource, PngFileSource, ScreencaptureCliSource};
