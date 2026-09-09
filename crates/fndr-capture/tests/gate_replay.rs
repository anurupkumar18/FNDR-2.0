//! T-309 AC: "gate changes show per-gate drop deltas in the replay report."
//!
//! Runs the committed fixture corpus (`fixtures/replay_corpus.jsonl`)
//! through the declarative gate policy table twice — once with the default
//! table, once with one gate disabled — and asserts the exact per-gate drop
//! counts and the delta between the two runs. This is the standalone,
//! `cargo test`-runnable form of the replay harness the ticket requires;
//! `examples/gate_replay.rs` is the same harness as a manual CLI tool.

use fndr_capture::replay::load_fixtures;
use fndr_capture::{CapturePipelineConfig, SkipReason};

const CORPUS_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/replay_corpus.jsonl");

/// The corpus has 10 records with a documented, one-outcome-each design
/// (see the comment above each line in the fixture file): a normal
/// article, an admission-policy skip, a URL-only listing, a perceptual
/// duplicate pair, a low-signal capture, a semantic-duplicate pair, one
/// more distinct capture, and one pre-capture-privacy skip.
#[test]
fn default_gate_table_reproduces_the_documented_tally() {
    let records = load_fixtures(CORPUS_PATH).expect("fixture corpus must parse");
    assert_eq!(records.len(), 10, "fixture corpus changed size unexpectedly");

    let report = fndr_capture::replay::run_replay(records, CapturePipelineConfig::default());

    assert_eq!(report.stored, 4);
    assert_eq!(report.url_only_stored, 1);
    assert_eq!(report.drop_count(SkipReason::AdmissionPolicy), 1);
    assert_eq!(report.drop_count(SkipReason::PerceptualDuplicate), 1);
    assert_eq!(report.drop_count(SkipReason::LowSignal), 1);
    assert_eq!(report.drop_count(SkipReason::SemanticDuplicate), 1);
    assert_eq!(report.drop_count(SkipReason::PreCapturePrivacy), 1);

    // Every record accounted for exactly once.
    let total_dropped: u64 = report.drops().iter().map(|(_, count)| *count).sum();
    assert_eq!(report.stored + report.url_only_stored + total_dropped, 10);
}

/// The AC's actual artifact: disabling a gate must show up as a visible,
/// attributable delta (one reason's count moves, `Stored` absorbs the
/// difference), not as a silent change nothing reports.
#[test]
fn disabling_low_signal_gate_shows_a_visible_per_gate_delta() {
    let records = load_fixtures(CORPUS_PATH).expect("fixture corpus must parse");

    let baseline = fndr_capture::replay::run_replay(records.clone(), CapturePipelineConfig::default());
    let mut disabled_config = CapturePipelineConfig::default();
    assert!(disabled_config.gates.set_enabled(SkipReason::LowSignal, false));
    let with_low_signal_disabled = fndr_capture::replay::run_replay(records, disabled_config);

    let delta = with_low_signal_disabled.delta(&baseline);
    let low_signal_delta = delta
        .iter()
        .find(|entry| entry.reason == SkipReason::LowSignal)
        .expect("LowSignal must be a row in the delta table");
    assert_eq!(low_signal_delta.before, 1);
    assert_eq!(low_signal_delta.after, 0);
    assert_eq!(low_signal_delta.delta, -1);

    // The frame that used to be dropped now reaches storage; every other
    // gate's count is unaffected by this change.
    assert_eq!(with_low_signal_disabled.stored, baseline.stored + 1);
    for entry in &delta {
        if entry.reason != SkipReason::LowSignal {
            assert_eq!(
                entry.delta, 0,
                "disabling LowSignal must not move {:?}'s count",
                entry.reason
            );
        }
    }

    // The rendered delta table is readable and names the gate that moved.
    let rendered = with_low_signal_disabled.render_delta(&baseline);
    assert!(rendered.contains("LowSignal: 1 -> 0 (-1)"));
}
