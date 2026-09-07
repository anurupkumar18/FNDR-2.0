//! Record assembly, merge and continuity, insight derivation, synthesis prompts and validators, review worker.

mod continuity;
mod write_path;

pub use continuity::{
    ContinuityRecord, ContinuityScore, SessionIdentityError, build_session_id, build_session_key,
    continuity_anchor, eligible_for_story_merge, merge_story_text, passes_merge_threshold,
    score_candidate, should_merge,
};
pub use write_path::{CaptureForPersistence, PersistCaptureOutcome, persist_capture};
