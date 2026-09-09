//! T-309 standalone replay runner: run the declarative gate policy table
//! over a fixture file and print the drop report, optionally diffed
//! against a second run with one named gate disabled.
//!
//! `cargo run -p fndr-capture --example gate_replay`
//! `cargo run -p fndr-capture --example gate_replay -- path/to/corpus.jsonl`
//! `cargo run -p fndr-capture --example gate_replay -- path/to/corpus.jsonl LowSignal`
//!
//! The optional second argument names a `SkipReason` gate (as it appears in
//! `Debug` output, e.g. `AdmissionPolicy`, `PerceptualDuplicate`,
//! `LowSignal`, `SemanticDuplicate`) to disable for a second run, printing
//! the before/after delta. This is the manual version of what
//! `crates/fndr-capture/tests/gate_replay.rs` asserts automatically.

use fndr_capture::replay::{load_fixtures, run_replay};
use fndr_capture::{CapturePipelineConfig, SkipReason};

const DEFAULT_CORPUS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/fixtures/replay_corpus.jsonl"
);

fn parse_gate(name: &str) -> Option<SkipReason> {
    match name {
        "AdmissionPolicy" => Some(SkipReason::AdmissionPolicy),
        "PerceptualDuplicate" => Some(SkipReason::PerceptualDuplicate),
        "LowSignal" => Some(SkipReason::LowSignal),
        "SemanticDuplicate" => Some(SkipReason::SemanticDuplicate),
        _ => None,
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let corpus_path = args.next().unwrap_or_else(|| DEFAULT_CORPUS.to_owned());
    let disable_gate = args.next();

    let records = match load_fixtures(&corpus_path) {
        Ok(records) => records,
        Err(error) => {
            eprintln!("failed to load fixtures from {corpus_path}: {error}");
            std::process::exit(1);
        }
    };
    println!("loaded {} fixture records from {corpus_path}", records.len());

    let baseline = run_replay(records.clone(), CapturePipelineConfig::default());
    println!("\n== baseline (default gate table) ==");
    print!("{}", baseline.render());

    if let Some(gate_name) = disable_gate {
        let Some(reason) = parse_gate(&gate_name) else {
            eprintln!(
                "unknown gate '{gate_name}'; expected one of AdmissionPolicy, \
                 PerceptualDuplicate, LowSignal, SemanticDuplicate"
            );
            std::process::exit(1);
        };
        let mut config = CapturePipelineConfig::default();
        let changed = config.gates.set_enabled(reason, false);
        assert!(changed, "{reason:?} must be a table entry to disable");

        let changed_report = run_replay(records, config);
        println!("\n== with {gate_name} disabled ==");
        print!("{}", changed_report.render());

        println!("\n== per-gate delta (baseline -> {gate_name} disabled) ==");
        print!("{}", changed_report.render_delta(&baseline));
    }
}
