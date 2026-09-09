//! The named regression test T-309 requires: FNDR v1's "stacked gates"
//! defect, ported forward as a concrete assertion against this crate's
//! replay harness.
//!
//! ## The v1 defect (history, not code — `reference/v1` is read-only per
//! ADR-005; nothing below is copied from it)
//!
//! v1's capture loop combined two inline extraction-quality checks into one
//! "stacked" drop condition: a narrow one (grounding confidence below 0.55)
//! and a broader one (grounding confidence below 0.80) that is a strict
//! superset of the narrow one. Whenever the narrow condition was true, the
//! broad one was necessarily true too, so their combination always equalled
//! "both fired" for every weakly-grounded record — silently absorbing every
//! LLM-touched frame into one drop path and defeating the separate
//! "OCR is strong enough, keep it anyway" rescue logic that downstream code
//! depended on. Nothing in the loop's plain inline `if`/`continue` chain
//! made that reallocation visible; it was found by reading a diff, not by
//! any report.
//!
//! ## The class of bug, generalized
//!
//! An earlier, broadened gate condition can silently swallow frames a
//! later, narrower gate was supposed to attribute distinctly, whenever the
//! two conditions overlap and evaluation order lets the broad one run
//! first. This is not specific to grounding scores: it is a property of
//! *any* pipeline with sequential inline gates and no per-gate replay
//! observability.
//!
//! ## The port: a real analog in *this* pipeline
//!
//! `pipeline::run_tick_inner` runs `LowSignal` before `SemanticDuplicate`
//! (see `gate_policy::GATE_ORDER`). `LowSignal`'s threshold
//! (`min_ocr_chars`) is exactly the kind of config value an operator might
//! reasonably raise to cut noise. Raise it too far, though, and it starts
//! swallowing frames upstream of `SemanticDuplicate` that would otherwise
//! have been correctly attributed as repeats — or, worse, frames that
//! should have been stored outright. That is the same mechanism as the v1
//! defect (a broadened, earlier condition absorbing a narrower, later one's
//! population) using this crate's own real config surface instead of a
//! hypothetical one.
//!
//! This test proves the replay harness turns that reallocation into a
//! visible, attributable delta — the T-309 AC ("gate changes show per-gate
//! drop deltas in the replay report") applied to exactly the class of bug
//! the ticket names — instead of a silent behavior change discovered later.

use fndr_capture::replay::{ReplayRecord, run_replay};
use fndr_capture::{CapturePipelineConfig, SkipReason};

fn record(ocr_text: &str, captured_at_ms: u64, pixel_rgb: [u8; 3]) -> ReplayRecord {
    ReplayRecord {
        app_name: "Notes".to_owned(),
        bundle_id: Some("com.apple.Notes".to_owned()),
        window_title: "Draft".to_owned(),
        url: None,
        observed_at_ms: captured_at_ms,
        captured_at_ms,
        pixel_rgb,
        ocr_text: ocr_text.to_owned(),
        ocr_confidence: 0.9,
        ocr_block_count: 3,
        private: false,
    }
}

/// Three capture opportunities with three distinct *intended* fates:
///   A: a substantive note, first appearance -> should be `Stored`.
///   B: the exact same note content moments later -> a genuine repeat,
///      should be `SemanticDuplicate`.
///   C: two characters of noise, unrelated content -> should be
///      `LowSignal`.
/// Pixel colours are spaced far enough apart (see
/// `pipeline::tests::perceptual_dedup_threshold_is_config_driven_not_hardcoded`
/// for why flat-colour distance matters here) that perceptual dedup never
/// fires and cannot confound the result.
fn fixtures() -> Vec<ReplayRecord> {
    let note = "Meeting notes: discussed the Q3 roadmap timeline in detail with the team.";
    vec![
        record(note, 1_000, [0, 0, 0]),
        record(note, 6_000, [250, 0, 0]),
        record("ok", 11_000, [0, 250, 0]),
    ]
}

#[test]
fn correctly_scoped_thresholds_keep_low_signal_and_semantic_duplicate_distinct() {
    // The default `min_ocr_chars` (12) is well below the note's length, so
    // each gate reports exactly the record it was meant to.
    let report = run_replay(fixtures(), CapturePipelineConfig::default());

    assert_eq!(report.stored, 1, "record A should be stored");
    assert_eq!(
        report.drop_count(SkipReason::SemanticDuplicate),
        1,
        "record B is a genuine repeat of A"
    );
    assert_eq!(
        report.drop_count(SkipReason::LowSignal),
        1,
        "record C is genuinely short, unrelated to A/B"
    );
}

/// The regression: a `min_ocr_chars` raised past the note's length
/// reproduces the v1 mechanism (an earlier, broadened gate condition
/// absorbing a narrower, later one's population) using this pipeline's
/// real config surface. Both A and B now read as "low signal" before
/// `SemanticDuplicate` (or storage) ever gets a chance to see them — A's
/// legitimate `Stored` outcome and B's `SemanticDuplicate` attribution both
/// silently disappear into `LowSignal`.
///
/// The test's job is not to say this misconfiguration is wrong to allow
/// (an operator may have a real reason to raise the threshold) — it is to
/// prove the replay harness makes the reallocation visible as a concrete,
/// attributable delta, which is what would have caught the v1 defect
/// before it shipped.
#[test]
fn misconfigured_threshold_reallocation_is_visible_in_the_replay_delta_not_silent() {
    let baseline = run_replay(fixtures(), CapturePipelineConfig::default());

    let mut misconfigured = CapturePipelineConfig::default();
    // Past the note's length (74 chars) but well above "ok"'s.
    misconfigured.min_ocr_chars = 100;
    let after = run_replay(fixtures(), misconfigured);

    // The bug, reproduced: both A and B now read as `LowSignal`, and
    // `SemanticDuplicate` + `Stored` both silently lose their expected
    // population to it.
    assert_eq!(after.drop_count(SkipReason::LowSignal), 3);
    assert_eq!(after.drop_count(SkipReason::SemanticDuplicate), 0);
    assert_eq!(after.stored, 0);

    // The harness's job: none of that reallocation is silent. The delta
    // table attributes it precisely — LowSignal gains exactly the two
    // records (A and B) that SemanticDuplicate and Stored between them
    // lost.
    let delta = after.delta(&baseline);
    let low_signal = delta
        .iter()
        .find(|entry| entry.reason == SkipReason::LowSignal)
        .expect("LowSignal is present in both reports");
    let semantic_duplicate = delta
        .iter()
        .find(|entry| entry.reason == SkipReason::SemanticDuplicate)
        .expect("SemanticDuplicate must still appear even though it falls to zero");

    assert_eq!(low_signal.before, 1);
    assert_eq!(low_signal.after, 3);
    assert_eq!(low_signal.delta, 2);
    assert_eq!(semantic_duplicate.before, 1);
    assert_eq!(semantic_duplicate.after, 0);
    assert_eq!(semantic_duplicate.delta, -1);
    // The remaining one of the two absorbed records came from `Stored`,
    // which is not a `SkipReason` and so is reported on the struct
    // directly rather than in the delta table.
    assert_eq!(baseline.stored - after.stored, 1);

    let rendered = after.render_delta(&baseline);
    assert!(rendered.contains("LowSignal: 1 -> 3 (+2)"));
    assert!(rendered.contains("SemanticDuplicate: 1 -> 0 (-1)"));
}
