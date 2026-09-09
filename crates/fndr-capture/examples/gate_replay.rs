//! Print the T-309 capture gate replay report for the checked-in synthetic
//! corpus, and the per-gate delta produced by one modelled policy change.
//!
//! Offline and hardware-free: no screen capture, no permissions, no store.
//! Run with `cargo run -p fndr-capture --example gate_replay`.

use fndr_capture::{CaptureGatePolicy, GateId, ReplayCorpus, replay};
use fndr_privacy::{Blocklist, SensitiveContextPolicy};

const CORPUS_JSON: &str = include_str!("../fixtures/capture-gate-replay.json");

fn main() {
    let corpus = ReplayCorpus::from_json(CORPUS_JSON).expect("fixture corpus must parse");
    let owner_blocklist = Blocklist::new(&["Notes"], &["internal-wiki.example"]);

    let shipped = CaptureGatePolicy::new(owner_blocklist.clone());
    let baseline = replay(&shipped, &corpus, "shipped");
    println!("{}", baseline.render());

    // Modelled change: the owner turns the medical-site gate off. Fail-open
    // changes are the quiet ones, so this is what the delta must surface.
    let weakened = CaptureGatePolicy::new(owner_blocklist.clone())
        .with_gate_disabled(GateId::PrivacyMedicalSite);
    println!(
        "{}",
        replay(&weakened, &corpus, "medical-gate-disabled")
            .delta(&baseline)
            .render()
    );

    // Modelled regression: the v1 stacked-gates failure. One careless
    // one-character entry widens the authentication gate until it swallows
    // everything, and every later gate stops seeing traffic.
    let widened = CaptureGatePolicy::new(owner_blocklist).with_sensitive_context(
        SensitiveContextPolicy::new(
            &[
                "1password",
                "bitwarden",
                "keychain",
                "lastpass",
                "dashlane",
                "keepass",
            ],
            &["chase.com", "bankofamerica.com", "paypal.com"],
            &["mychart"],
            &["sign in", "log in", "login", "e"],
            &["api_key", "sk-"],
        ),
    );
    println!(
        "{}",
        replay(&widened, &corpus, "authentication-gate-widened")
            .delta(&baseline)
            .render()
    );
}
