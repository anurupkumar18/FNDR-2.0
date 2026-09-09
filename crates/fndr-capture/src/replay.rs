//! Offline replay of the capture gate policy over recorded fixtures (T-309).
//!
//! The harness runs synthetic metadata/text fixtures through the same
//! [`CaptureGatePolicy`] the live pipeline uses and reports, per gate, what
//! that gate dropped, diverted, or redacted. Comparing two reports gives the
//! per-gate drop deltas that make a policy change reviewable.
//!
//! Deliberately pure: no pixels, no Screen Recording permission, no store, no
//! I/O of its own. The caller supplies fixture JSON (the checked-in corpus
//! lives at `fixtures/capture-gate-replay.json`), so this crate keeps the
//! ADR-004 posture of doing no file or network work.
//!
//! The v1 failure this exists to catch: a chain of inline gates in which one
//! widened gate silently swallowed every frame downstream. Here that shows up
//! as one gate's drop count jumping and every later gate's falling to zero.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{CaptureGatePolicy, GateId, GateInput, GateOutcome, GateStage, SkipReason};

/// One recorded capture opportunity. Fixtures are synthetic by policy: no
/// real screen, no real personal data, ever.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayFixture {
    /// Stable name, so a report line points at a reviewable fixture.
    pub id: String,
    /// Why this fixture is in the corpus. Read back in the report so a gate
    /// change is judged against intent, not just counts.
    #[serde(default)]
    pub note: String,
    pub app_name: String,
    #[serde(default)]
    pub bundle_id: Option<String>,
    #[serde(default)]
    pub window_title: String,
    #[serde(default)]
    pub url: Option<String>,
    /// Recognized text, when the fixture models a post-OCR opportunity.
    #[serde(default)]
    pub ocr_text: Option<String>,
}

/// A parsed fixture corpus.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayCorpus {
    pub name: String,
    pub fixtures: Vec<ReplayFixture>,
}

impl ReplayCorpus {
    /// Parse a corpus from JSON supplied by the caller.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    pub fn len(&self) -> usize {
        self.fixtures.len()
    }

    pub fn is_empty(&self) -> bool {
        self.fixtures.is_empty()
    }
}

/// What the policy did to one fixture, and which gate did it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixtureVerdict {
    pub fixture_id: String,
    /// The gate that decided, by stable name. `None` means the fixture was
    /// admitted with no gate firing.
    pub gate: Option<String>,
    pub disposition: Disposition,
}

/// The terminal disposition of a replayed fixture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Disposition {
    /// Captured and stored whole.
    Stored,
    /// Metadata kept, pixels never taken.
    UrlOnly,
    /// Stored with secret lines removed at the write path.
    StoredRedacted,
    /// Dropped before storage.
    Dropped,
}

/// Per-gate activity for one replay.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateActivity {
    pub dropped: u64,
    pub diverted_url_only: u64,
    pub redacted: u64,
}

impl GateActivity {
    pub fn total(&self) -> u64 {
        self.dropped + self.diverted_url_only + self.redacted
    }
}

/// The replay report. This is T-309's acceptance artifact: every gate that
/// exists is listed, including the ones that fired zero times, because a gate
/// silently doing nothing is exactly the v1 failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayReport {
    pub corpus: String,
    /// Which policy variant produced this report, so a delta names the two
    /// policies being compared rather than repeating the corpus name.
    pub policy_label: String,
    pub fixtures: u64,
    pub stored: u64,
    pub url_only: u64,
    pub stored_redacted: u64,
    pub dropped: u64,
    /// Keyed by [`GateId::as_str`], ordered by the policy table.
    pub per_gate: Vec<(String, GateActivity)>,
    pub verdicts: Vec<FixtureVerdict>,
}

impl ReplayReport {
    pub fn gate(&self, id: GateId) -> GateActivity {
        self.per_gate
            .iter()
            .find(|(name, _)| name == id.as_str())
            .map(|(_, activity)| *activity)
            .unwrap_or_default()
    }

    /// Per-gate drop deltas against a baseline report, plus the corpus-level
    /// totals. The ticket's acceptance criterion is that a gate change shows
    /// up here rather than as an unexplained drop in stored records.
    pub fn delta(&self, baseline: &ReplayReport) -> ReplayDelta {
        let mut names: Vec<String> = baseline
            .per_gate
            .iter()
            .map(|(name, _)| name.clone())
            .collect();
        for (name, _) in &self.per_gate {
            if !names.contains(name) {
                names.push(name.clone());
            }
        }
        let lookup = |report: &ReplayReport, name: &str| -> GateActivity {
            report
                .per_gate
                .iter()
                .find(|(candidate, _)| candidate == name)
                .map(|(_, activity)| *activity)
                .unwrap_or_default()
        };
        let per_gate = names
            .into_iter()
            .map(|name| {
                let before = lookup(baseline, &name);
                let after = lookup(self, &name);
                GateDelta {
                    gate: name,
                    before,
                    after,
                    dropped_delta: i64::try_from(after.dropped).unwrap_or(i64::MAX)
                        - i64::try_from(before.dropped).unwrap_or(i64::MAX),
                }
            })
            .collect();

        ReplayDelta {
            corpus: self.corpus.clone(),
            baseline_policy: baseline.policy_label.clone(),
            policy: self.policy_label.clone(),
            stored_delta: i64::try_from(self.stored).unwrap_or(i64::MAX)
                - i64::try_from(baseline.stored).unwrap_or(i64::MAX),
            url_only_delta: i64::try_from(self.url_only).unwrap_or(i64::MAX)
                - i64::try_from(baseline.url_only).unwrap_or(i64::MAX),
            dropped_delta: i64::try_from(self.dropped).unwrap_or(i64::MAX)
                - i64::try_from(baseline.dropped).unwrap_or(i64::MAX),
            per_gate,
        }
    }

    /// The human-readable table an owner or reviewer reads.
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "capture gate replay: policy '{}' over corpus '{}' ({} fixtures)\n",
            self.policy_label, self.corpus, self.fixtures
        ));
        out.push_str(&format!(
            "  stored={} url_only={} stored_redacted={} dropped={}\n\n",
            self.stored, self.url_only, self.stored_redacted, self.dropped
        ));
        out.push_str(&format!(
            "  {:<34} {:>8} {:>10} {:>9}\n",
            "gate", "dropped", "url_only", "redacted"
        ));
        for (name, activity) in &self.per_gate {
            out.push_str(&format!(
                "  {:<34} {:>8} {:>10} {:>9}\n",
                name, activity.dropped, activity.diverted_url_only, activity.redacted
            ));
        }
        out
    }
}

/// One gate's change between two replays.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateDelta {
    pub gate: String,
    pub before: GateActivity,
    pub after: GateActivity,
    pub dropped_delta: i64,
}

/// The comparison between two replay reports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayDelta {
    pub corpus: String,
    pub baseline_policy: String,
    pub policy: String,
    pub stored_delta: i64,
    pub url_only_delta: i64,
    pub dropped_delta: i64,
    pub per_gate: Vec<GateDelta>,
}

impl ReplayDelta {
    /// Gates whose drop count changed, largest movement first. An empty list
    /// means the policy change was inert over this corpus.
    pub fn moved_gates(&self) -> Vec<&GateDelta> {
        let mut moved: Vec<&GateDelta> = self
            .per_gate
            .iter()
            .filter(|delta| delta.dropped_delta != 0)
            .collect();
        moved.sort_by_key(|delta| -delta.dropped_delta.abs());
        moved
    }

    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "gate delta over corpus '{}': '{}' -> '{}'\n",
            self.corpus, self.baseline_policy, self.policy
        ));
        out.push_str(&format!(
            "  stored {:+} url_only {:+} dropped {:+}\n\n",
            self.stored_delta, self.url_only_delta, self.dropped_delta
        ));
        out.push_str(&format!(
            "  {:<34} {:>8} {:>8} {:>8}\n",
            "gate", "before", "after", "delta"
        ));
        for delta in &self.per_gate {
            out.push_str(&format!(
                "  {:<34} {:>8} {:>8} {:>+8}\n",
                delta.gate, delta.before.dropped, delta.after.dropped, delta.dropped_delta
            ));
        }
        out
    }
}

/// Replay a corpus through a policy and report per-gate activity.
///
/// The stage order mirrors the live pipeline: metadata gates decide before
/// pixels, then, for anything still admitted, the text gates run over the
/// fixture's recognized text.
pub fn replay(
    policy: &CaptureGatePolicy,
    corpus: &ReplayCorpus,
    policy_label: &str,
) -> ReplayReport {
    let mut activity: BTreeMap<&'static str, GateActivity> = BTreeMap::new();
    // Every gate in the table is listed even when it never fires: a gate
    // reporting nothing must be visible, not absent.
    let ordered_gates: Vec<&'static str> =
        policy.rules().iter().map(|rule| rule.id.as_str()).collect();
    for gate in &ordered_gates {
        activity.entry(gate).or_default();
    }

    let mut verdicts = Vec::with_capacity(corpus.fixtures.len());
    let (mut stored, mut url_only, mut stored_redacted, mut dropped) = (0, 0, 0, 0);

    for fixture in &corpus.fixtures {
        let input = GateInput {
            app_name: &fixture.app_name,
            bundle_id: fixture.bundle_id.as_deref(),
            window_title: &fixture.window_title,
            url: fixture.url.as_deref(),
            ocr_text: fixture.ocr_text.as_deref(),
        };
        // Metadata-stage input never carries text, matching the live gate.
        let metadata_input = GateInput {
            ocr_text: None,
            ..input
        };

        let metadata = policy.evaluate(&metadata_input, GateStage::Metadata);
        let (gate, disposition) = match metadata.outcome {
            GateOutcome::Drop(_) => (metadata.fired, Disposition::Dropped),
            GateOutcome::UrlOnly => (metadata.fired, Disposition::UrlOnly),
            GateOutcome::Redact | GateOutcome::Admit => {
                let text = policy.evaluate(&input, GateStage::Text);
                match text.outcome {
                    GateOutcome::Drop(_) => (text.fired, Disposition::Dropped),
                    GateOutcome::Redact => (text.fired, Disposition::StoredRedacted),
                    GateOutcome::UrlOnly => (text.fired, Disposition::UrlOnly),
                    GateOutcome::Admit => (None, Disposition::Stored),
                }
            }
        };

        if let Some(gate) = gate {
            let entry = activity.entry(gate.as_str()).or_default();
            match disposition {
                Disposition::Dropped => entry.dropped += 1,
                Disposition::UrlOnly => entry.diverted_url_only += 1,
                Disposition::StoredRedacted => entry.redacted += 1,
                Disposition::Stored => {}
            }
        }
        match disposition {
            Disposition::Stored => stored += 1,
            Disposition::UrlOnly => url_only += 1,
            Disposition::StoredRedacted => stored_redacted += 1,
            Disposition::Dropped => dropped += 1,
        }

        verdicts.push(FixtureVerdict {
            fixture_id: fixture.id.clone(),
            gate: gate.map(|gate| gate.as_str().to_owned()),
            disposition,
        });
    }

    ReplayReport {
        corpus: corpus.name.clone(),
        policy_label: policy_label.to_owned(),
        fixtures: corpus.fixtures.len() as u64,
        stored,
        url_only,
        stored_redacted,
        dropped,
        per_gate: ordered_gates
            .into_iter()
            .map(|gate| {
                (
                    gate.to_owned(),
                    activity.get(gate).copied().unwrap_or_default(),
                )
            })
            .collect(),
        verdicts,
    }
}

/// The `SkipReason` a dropped fixture would have counted at runtime. Kept so
/// the report can be reconciled against `CaptureCounters` without the replay
/// harness inventing a second reason vocabulary.
pub fn skip_reason_for(policy: &CaptureGatePolicy, gate: GateId) -> Option<SkipReason> {
    policy
        .rules()
        .iter()
        .find(|rule| rule.id == gate)
        .and_then(|rule| match rule.action {
            crate::GateAction::Drop(reason) => Some(reason),
            _ => None,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use fndr_privacy::{Blocklist, SensitiveContextPolicy};

    const CORPUS_JSON: &str = include_str!("../fixtures/capture-gate-replay.json");

    fn corpus() -> ReplayCorpus {
        ReplayCorpus::from_json(CORPUS_JSON).expect("fixture corpus must parse")
    }

    fn shipped() -> CaptureGatePolicy {
        CaptureGatePolicy::new(Blocklist::new(&["Notes"], &["internal-wiki.example"]))
    }

    #[test]
    fn the_corpus_parses_and_is_not_trivially_small() {
        let corpus = corpus();
        assert!(
            corpus.len() >= 16,
            "a replay corpus that cannot distinguish gates is theater: {} fixtures",
            corpus.len()
        );
    }

    #[test]
    fn the_report_lists_every_gate_including_the_silent_ones() {
        let report = replay(&shipped(), &corpus(), "shipped");
        assert_eq!(report.per_gate.len(), crate::DEFAULT_GATE_RULES.len());
        assert_eq!(report.fixtures, corpus().len() as u64);
        assert_eq!(
            report.stored + report.url_only + report.stored_redacted + report.dropped,
            report.fixtures,
            "every fixture must land in exactly one terminal bucket"
        );
    }

    #[test]
    fn each_gate_class_is_exercised_by_the_corpus() {
        // A corpus that never triggers a gate cannot detect that gate
        // breaking, so the coverage itself is asserted.
        let report = replay(&shipped(), &corpus(), "shipped");
        for gate in [
            GateId::PrivacyUserBlocklist,
            GateId::PrivacyFndrSelfCapture,
            GateId::PrivacyPasswordManager,
            GateId::PrivacyPrivateBrowsing,
            GateId::PrivacyFinancialSite,
            GateId::PrivacyMedicalSite,
            GateId::PrivacyAuthentication,
            GateId::AdmissionGenericBrowserChrome,
            GateId::AdmissionNavigationSurface,
            GateId::AdmissionListingSurface,
            GateId::PrivacySecretPattern,
        ] {
            assert!(
                report.gate(gate).total() > 0,
                "no fixture exercises {gate}; the corpus cannot detect it regressing"
            );
        }
        assert!(
            report.stored > 0,
            "a corpus where nothing is stored cannot show a gate over-firing"
        );
    }

    #[test]
    fn a_dropped_fixture_maps_back_to_a_real_skip_reason_counter() {
        let policy = shipped();
        let report = replay(&policy, &corpus(), "shipped");
        for verdict in &report.verdicts {
            if verdict.disposition != Disposition::Dropped {
                continue;
            }
            let gate = verdict.gate.as_deref().expect("a drop names its gate");
            let id = crate::DEFAULT_GATE_RULES
                .iter()
                .find(|rule| rule.id.as_str() == gate)
                .expect("gate name is a table row")
                .id;
            assert!(
                skip_reason_for(&policy, id).is_some(),
                "{gate} drops fixtures but maps to no SkipReason counter"
            );
        }
    }

    /// The named T-309 regression: the v1 stacked-gates failure.
    ///
    /// In v1 a widened gate in the middle of the inline chain silently
    /// swallowed every LLM-touched frame. Nothing downstream ever ran, and
    /// the only symptom was that capture had quietly stopped producing
    /// records. The replay report has to turn that into an attributable
    /// per-gate delta.
    #[test]
    fn stacked_gates_regression_shows_a_per_gate_delta_not_a_silent_zero() {
        let corpus = corpus();
        let baseline = replay(&shipped(), &corpus, "shipped");
        assert!(baseline.stored > 0);

        // The policy change: one gate's data widened by a single careless
        // entry. "e" is a substring of nearly every window title, so the
        // authentication gate now matches almost everything.
        let widened =
            CaptureGatePolicy::new(Blocklist::new(&["Notes"], &["internal-wiki.example"]))
                .with_sensitive_context(SensitiveContextPolicy::new(
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
                    // The mistake: a one-character indicator.
                    &["sign in", "log in", "login", "e"],
                    &["api_key", "sk-"],
                ));
        let after = replay(&widened, &corpus, "authentication-gate-widened");

        // The symptom the v1 team saw, and could not explain.
        assert_eq!(
            after.stored, 0,
            "fixture must reproduce the total-suppression failure"
        );

        let delta = after.delta(&baseline);
        assert!(delta.stored_delta < 0);

        // The property the ticket asks for: the report names the culprit.
        let moved = delta.moved_gates();
        assert!(!moved.is_empty(), "a total suppression must move some gate");
        let culprit = moved[0];
        assert_eq!(
            culprit.gate,
            GateId::PrivacyAuthentication.as_str(),
            "the biggest drop movement must point at the gate that widened:\n{}",
            delta.render()
        );
        assert!(culprit.dropped_delta > 0);

        // And the downstream starvation is visible as gates falling to zero,
        // rather than as an unexplained absence of records.
        let starved: Vec<&GateDelta> = delta
            .per_gate
            .iter()
            .filter(|entry| entry.before.dropped > 0 && entry.after.dropped == 0)
            .collect();
        assert!(
            starved
                .iter()
                .any(|entry| entry.gate == GateId::AdmissionNavigationSurface.as_str()),
            "a gate downstream of the widened one must show its drops going to zero:\n{}",
            delta.render()
        );

        // The regression test would be theater if it passed without the bug,
        // so assert the unwidened policy produces no movement at all.
        let unchanged = replay(&shipped(), &corpus, "shipped").delta(&baseline);
        assert!(
            unchanged.moved_gates().is_empty(),
            "an unchanged policy must produce an empty delta"
        );
    }

    #[test]
    fn disabling_a_gate_is_visible_as_a_negative_delta_on_that_gate() {
        // The other direction of the same failure: a gate that stops firing.
        // Fail-open is quieter than fail-closed, so it must be at least as
        // visible in the report.
        let corpus = corpus();
        let baseline = replay(&shipped(), &corpus, "shipped");
        let weakened = shipped().with_gate_disabled(GateId::PrivacyMedicalSite);
        let delta = replay(&weakened, &corpus, "medical-gate-disabled").delta(&baseline);

        let medical = delta
            .per_gate
            .iter()
            .find(|entry| entry.gate == GateId::PrivacyMedicalSite.as_str())
            .expect("the disabled gate is still listed");
        assert!(
            medical.dropped_delta < 0,
            "turning a gate off must show as a negative drop delta:\n{}",
            delta.render()
        );
        assert!(
            delta.stored_delta > 0,
            "content the medical gate used to block now reaches storage:\n{}",
            delta.render()
        );
    }

    #[test]
    fn the_rendered_report_names_gates_and_counts() {
        let rendered = replay(&shipped(), &corpus(), "shipped").render();
        assert!(rendered.contains("privacy.password_manager"));
        assert!(rendered.contains("admission.listing_surface"));
        assert!(rendered.contains("privacy.secret_pattern"));
    }
}
