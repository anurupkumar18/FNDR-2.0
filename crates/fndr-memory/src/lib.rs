//! Record assembly, merge and continuity, insight derivation, synthesis prompts and validators, review worker.

mod write_path;

pub use write_path::{CaptureForPersistence, PersistCaptureOutcome, persist_capture};
