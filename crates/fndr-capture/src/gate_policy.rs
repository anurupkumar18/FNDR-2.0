//! The declarative capture gate policy table (T-309).
//!
//! Every reason a capture opportunity is dropped, diverted to URL-only, or
//! redacted before storage is one named row of [`DEFAULT_GATE_RULES`]. The
//! table owns the evaluation order, the action, and a stable [`GateId`] per
//! gate; the evaluator is a loop over it. Adding or retuning a gate is a
//! change to this data (or to the [`Blocklist`] / [`SensitiveContextPolicy`]
//! values it reads), not another inline `continue` in the capture loop.
//!
//! Why this shape: in v1 the capture loop was a long chain of sequential
//! inline gates. One of them widened, every LLM-touched frame was dropped,
//! and nothing reported which link in the chain had done it. With a table,
//! each gate has an identity that the offline replay harness (`crate::replay`)
//! can count, so the same mistake shows up as a per-gate drop delta instead
//! of a mysterious zero.
//!
//! The rows do not restate policy. Safety rows delegate to
//! `fndr_privacy::SAFETY_RULES` and admission rows to
//! `crate::AdmissionRule`; a duplicated predicate is a predicate that drifts
//! open.

use fndr_privacy::{Blocklist, SafetyContext, SafetyReason, SensitiveContextPolicy, safety_rule};

use crate::{AdmissionRule, CaptureContext, GateDecision, SkipReason};

/// The stable identity of one gate. Report lines, fixtures, and deltas are
/// keyed by this, so a renamed or reordered row is a visible change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum GateId {
    PrivacyUserBlocklist,
    PrivacyFndrSelfCapture,
    PrivacyPasswordManager,
    PrivacyPrivateBrowsing,
    PrivacyFinancialSite,
    PrivacyMedicalSite,
    PrivacyAuthentication,
    AdmissionGenericBrowserChrome,
    AdmissionNavigationSurface,
    AdmissionListingSurface,
    PrivacySecretPattern,
}

impl GateId {
    /// The wire/report name. Stable across releases; changing one is a
    /// deliberate edit that the ordering test will surface.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PrivacyUserBlocklist => "privacy.user_blocklist",
            Self::PrivacyFndrSelfCapture => "privacy.fndr_self_capture",
            Self::PrivacyPasswordManager => "privacy.password_manager",
            Self::PrivacyPrivateBrowsing => "privacy.private_browsing",
            Self::PrivacyFinancialSite => "privacy.financial_site",
            Self::PrivacyMedicalSite => "privacy.medical_site",
            Self::PrivacyAuthentication => "privacy.authentication",
            Self::AdmissionGenericBrowserChrome => "admission.generic_browser_chrome",
            Self::AdmissionNavigationSurface => "admission.navigation_surface",
            Self::AdmissionListingSurface => "admission.listing_surface",
            Self::PrivacySecretPattern => "privacy.secret_pattern",
        }
    }
}

impl std::fmt::Display for GateId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Which inputs a gate needs. Metadata gates decide before a single pixel is
/// captured; text gates see recognized text and therefore run after OCR.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GateStage {
    /// Foreground metadata only: app, bundle id, window title, URL.
    Metadata,
    /// Recognized text, after OCR and before persistence.
    Text,
}

/// What a matching gate does to the capture opportunity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateAction {
    /// Terminate the tick and count this `SkipReason`.
    Drop(SkipReason),
    /// Keep the URL metadata, never the pixels.
    DivertUrlOnly,
    /// Persist, with the matching lines removed at the write path.
    Redact,
}

/// The predicate a row delegates to. Both arms point at the single existing
/// implementation of that policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateCheck {
    Safety(SafetyReason),
    Admission(AdmissionRule),
}

/// One row of the capture gate policy table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GateRule {
    pub id: GateId,
    pub stage: GateStage,
    pub check: GateCheck,
    pub action: GateAction,
    /// Data, not code: a disabled row is skipped, and the replay report
    /// shows exactly which drops moved to a later gate as a result.
    pub enabled: bool,
}

/// The capture gate policy, in evaluation order. First match wins.
///
/// This order reproduces the shipped pipeline exactly: the pre-pixel privacy
/// gate, then browser admission, then the post-OCR secret scan. Reordering it
/// changes which reason an owner is shown and is asserted by a test.
pub const DEFAULT_GATE_RULES: &[GateRule] = &[
    GateRule {
        id: GateId::PrivacyUserBlocklist,
        stage: GateStage::Metadata,
        check: GateCheck::Safety(SafetyReason::UserBlocklist),
        action: GateAction::Drop(SkipReason::PreCapturePrivacy),
        enabled: true,
    },
    GateRule {
        id: GateId::PrivacyFndrSelfCapture,
        stage: GateStage::Metadata,
        check: GateCheck::Safety(SafetyReason::FndrSelfCapture),
        action: GateAction::Drop(SkipReason::PreCapturePrivacy),
        enabled: true,
    },
    GateRule {
        id: GateId::PrivacyPasswordManager,
        stage: GateStage::Metadata,
        check: GateCheck::Safety(SafetyReason::PasswordManager),
        action: GateAction::Drop(SkipReason::PreCapturePrivacy),
        enabled: true,
    },
    GateRule {
        id: GateId::PrivacyPrivateBrowsing,
        stage: GateStage::Metadata,
        check: GateCheck::Safety(SafetyReason::PrivateBrowsing),
        // Its own counter so the owner can be told why capture was withheld
        // without being shown the window title that triggered it.
        action: GateAction::Drop(SkipReason::PrivateBrowsing),
        enabled: true,
    },
    GateRule {
        id: GateId::PrivacyFinancialSite,
        stage: GateStage::Metadata,
        check: GateCheck::Safety(SafetyReason::FinancialSite),
        action: GateAction::Drop(SkipReason::PreCapturePrivacy),
        enabled: true,
    },
    GateRule {
        id: GateId::PrivacyMedicalSite,
        stage: GateStage::Metadata,
        check: GateCheck::Safety(SafetyReason::MedicalSite),
        action: GateAction::Drop(SkipReason::PreCapturePrivacy),
        enabled: true,
    },
    GateRule {
        id: GateId::PrivacyAuthentication,
        stage: GateStage::Metadata,
        check: GateCheck::Safety(SafetyReason::Authentication),
        action: GateAction::Drop(SkipReason::PreCapturePrivacy),
        enabled: true,
    },
    GateRule {
        id: GateId::AdmissionGenericBrowserChrome,
        stage: GateStage::Metadata,
        check: GateCheck::Admission(AdmissionRule::GenericBrowserChrome),
        action: GateAction::Drop(SkipReason::AdmissionPolicy),
        enabled: true,
    },
    GateRule {
        id: GateId::AdmissionNavigationSurface,
        stage: GateStage::Metadata,
        check: GateCheck::Admission(AdmissionRule::NavigationSurface),
        action: GateAction::Drop(SkipReason::AdmissionPolicy),
        enabled: true,
    },
    GateRule {
        id: GateId::AdmissionListingSurface,
        stage: GateStage::Metadata,
        check: GateCheck::Admission(AdmissionRule::ListingSurface),
        action: GateAction::DivertUrlOnly,
        enabled: true,
    },
    GateRule {
        id: GateId::PrivacySecretPattern,
        stage: GateStage::Text,
        check: GateCheck::Safety(SafetyReason::SecretPattern),
        action: GateAction::Redact,
        enabled: true,
    },
];

/// Everything a gate row may read.
#[derive(Debug, Clone, Copy, Default)]
pub struct GateInput<'a> {
    pub app_name: &'a str,
    pub bundle_id: Option<&'a str>,
    pub window_title: &'a str,
    pub url: Option<&'a str>,
    /// `None` before OCR. Text-stage gates cannot fire without it, which is
    /// why the secret scan is structurally unable to run pre-pixel.
    pub ocr_text: Option<&'a str>,
}

impl<'a> GateInput<'a> {
    /// The pre-pixel view of a capture opportunity.
    pub fn from_context(context: &'a CaptureContext) -> Self {
        Self {
            app_name: &context.app_name,
            bundle_id: context.bundle_id.as_deref(),
            window_title: &context.window_title,
            url: context.url.as_deref(),
            ocr_text: None,
        }
    }

    /// The same opportunity once OCR text exists.
    pub fn with_ocr_text(mut self, text: &'a str) -> Self {
        self.ocr_text = Some(text);
        self
    }

    fn safety_context(&self) -> SafetyContext<'a> {
        SafetyContext {
            app_name: Some(self.app_name),
            bundle_id: self.bundle_id,
            url: self.url,
            window_title: Some(self.window_title),
            ocr_text: self.ocr_text,
        }
    }
}

/// What the table decided, and which row decided it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GateEvaluation {
    /// `None` means no gate matched at this stage.
    pub fired: Option<GateId>,
    pub outcome: GateOutcome,
}

/// The terminal answer for one stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateOutcome {
    Admit,
    UrlOnly,
    Redact,
    Drop(SkipReason),
}

/// The evaluable capture gate policy: the ordered table plus the owner data
/// its rows read.
#[derive(Debug, Clone)]
pub struct CaptureGatePolicy {
    rules: Vec<GateRule>,
    blocklist: Blocklist,
    sensitive_context: SensitiveContextPolicy,
}

impl CaptureGatePolicy {
    /// The shipped table with the owner's blocklist and the built-in
    /// sensitive-context lists.
    pub fn new(blocklist: Blocklist) -> Self {
        Self {
            rules: DEFAULT_GATE_RULES.to_vec(),
            blocklist,
            sensitive_context: SensitiveContextPolicy::default(),
        }
    }

    /// Replace the sensitive-context lists (T-802 owner overrides, and the
    /// lever the replay harness pulls to model a retuned gate).
    pub fn with_sensitive_context(mut self, sensitive_context: SensitiveContextPolicy) -> Self {
        self.sensitive_context = sensitive_context;
        self
    }

    /// Turn one named gate off. Deliberately explicit and deliberately
    /// visible in the replay report: this widens capture, and the report's
    /// per-gate delta is how a reviewer sees by how much.
    pub fn with_gate_disabled(mut self, id: GateId) -> Self {
        for rule in &mut self.rules {
            if rule.id == id {
                rule.enabled = false;
            }
        }
        self
    }

    pub fn rules(&self) -> &[GateRule] {
        &self.rules
    }

    pub fn blocklist(&self) -> &Blocklist {
        &self.blocklist
    }

    /// Run the rows belonging to one stage, in table order.
    pub fn evaluate(&self, input: &GateInput<'_>, stage: GateStage) -> GateEvaluation {
        let context = input.safety_context();
        for rule in self.rules.iter().filter(|rule| rule.enabled) {
            if rule.stage != stage {
                continue;
            }
            let fired = match rule.check {
                GateCheck::Safety(reason) => safety_rule(reason).is_some_and(|safety| {
                    safety.matches(context, &self.blocklist, &self.sensitive_context)
                }),
                GateCheck::Admission(admission) => {
                    admission.matches(input.app_name, input.window_title, input.url)
                }
            };
            if fired {
                return GateEvaluation {
                    fired: Some(rule.id),
                    outcome: match rule.action {
                        GateAction::Drop(reason) => GateOutcome::Drop(reason),
                        GateAction::DivertUrlOnly => GateOutcome::UrlOnly,
                        GateAction::Redact => GateOutcome::Redact,
                    },
                };
            }
        }
        GateEvaluation {
            fired: None,
            outcome: GateOutcome::Admit,
        }
    }
}

/// The metadata-stage capture gate: the policy table wired into the real
/// pipeline. This is the same object the replay harness evaluates, so the
/// report describes shipped behavior rather than a parallel model of it.
#[derive(Debug, Clone)]
pub struct PolicyGate {
    policy: CaptureGatePolicy,
}

impl PolicyGate {
    pub fn new(blocklist: Blocklist) -> Self {
        Self {
            policy: CaptureGatePolicy::new(blocklist),
        }
    }

    pub fn with_policy(policy: CaptureGatePolicy) -> Self {
        Self { policy }
    }

    pub fn policy(&self) -> &CaptureGatePolicy {
        &self.policy
    }
}

impl crate::PreCaptureGate for PolicyGate {
    fn evaluate(&self, context: &CaptureContext) -> GateDecision {
        let input = GateInput::from_context(context);
        match self.policy.evaluate(&input, GateStage::Metadata).outcome {
            GateOutcome::Admit => GateDecision::Allow,
            GateOutcome::UrlOnly => GateDecision::UrlOnly,
            GateOutcome::Drop(reason) => GateDecision::Skip(reason),
            // No metadata-stage row redacts; the write path owns redaction.
            GateOutcome::Redact => GateDecision::Allow,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CaptureSurfacePolicy, PreCaptureGate, classify_capture_surface_policy};
    use fndr_privacy::{SafetyDecision, evaluate as evaluate_safety};

    fn input<'a>(app: &'a str, title: &'a str, url: Option<&'a str>) -> GateInput<'a> {
        GateInput {
            app_name: app,
            bundle_id: Some("com.example.app"),
            window_title: title,
            url,
            ocr_text: None,
        }
    }

    #[test]
    fn the_table_is_the_order_of_record() {
        assert_eq!(
            DEFAULT_GATE_RULES
                .iter()
                .map(|rule| (rule.id.as_str(), rule.stage, rule.action))
                .collect::<Vec<_>>(),
            vec![
                (
                    "privacy.user_blocklist",
                    GateStage::Metadata,
                    GateAction::Drop(SkipReason::PreCapturePrivacy)
                ),
                (
                    "privacy.fndr_self_capture",
                    GateStage::Metadata,
                    GateAction::Drop(SkipReason::PreCapturePrivacy)
                ),
                (
                    "privacy.password_manager",
                    GateStage::Metadata,
                    GateAction::Drop(SkipReason::PreCapturePrivacy)
                ),
                (
                    "privacy.private_browsing",
                    GateStage::Metadata,
                    GateAction::Drop(SkipReason::PrivateBrowsing)
                ),
                (
                    "privacy.financial_site",
                    GateStage::Metadata,
                    GateAction::Drop(SkipReason::PreCapturePrivacy)
                ),
                (
                    "privacy.medical_site",
                    GateStage::Metadata,
                    GateAction::Drop(SkipReason::PreCapturePrivacy)
                ),
                (
                    "privacy.authentication",
                    GateStage::Metadata,
                    GateAction::Drop(SkipReason::PreCapturePrivacy)
                ),
                (
                    "admission.generic_browser_chrome",
                    GateStage::Metadata,
                    GateAction::Drop(SkipReason::AdmissionPolicy)
                ),
                (
                    "admission.navigation_surface",
                    GateStage::Metadata,
                    GateAction::Drop(SkipReason::AdmissionPolicy)
                ),
                (
                    "admission.listing_surface",
                    GateStage::Metadata,
                    GateAction::DivertUrlOnly
                ),
                (
                    "privacy.secret_pattern",
                    GateStage::Text,
                    GateAction::Redact
                ),
            ]
        );
        assert!(DEFAULT_GATE_RULES.iter().all(|rule| rule.enabled));
    }

    #[test]
    fn every_safety_reason_and_admission_rule_has_a_row() {
        // A policy row missing for a rule that exists is a gate that quietly
        // stops running once the table becomes the evaluator.
        for rule in fndr_privacy::SAFETY_RULES {
            assert!(
                DEFAULT_GATE_RULES
                    .iter()
                    .any(|row| row.check == GateCheck::Safety(rule.reason)),
                "no capture gate row for safety reason {:?}",
                rule.reason
            );
        }
        for admission in crate::ADMISSION_RULES {
            assert!(
                DEFAULT_GATE_RULES
                    .iter()
                    .any(|row| row.check == GateCheck::Admission(*admission)),
                "no capture gate row for admission rule {admission:?}"
            );
        }
    }

    #[test]
    fn the_table_reproduces_the_pre_table_decisions_exactly() {
        // Fail-closed proof for the restructure: for every fixture the table
        // must agree with the two implementations it replaced. A widened or
        // dropped gate shows up here as a disagreement.
        let policy = CaptureGatePolicy::new(Blocklist::default());
        let blocklist = Blocklist::default();
        let cases = [
            ("1Password", "Vault", None),
            ("Google Chrome", "New Incognito Window", None),
            (
                "Safari",
                "Account overview",
                Some("https://online.chase.com/account"),
            ),
            (
                "Safari",
                "Results",
                Some("https://mychart.example-hospital.com/"),
            ),
            ("Safari", "Sign in", Some("https://example.com/login")),
            ("Google Chrome", "New Tab", Some("https://example.com/")),
            (
                "Google Chrome",
                "Search results - YouTube",
                Some("https://www.youtube.com/results?search_query=x"),
            ),
            (
                "Google Chrome",
                "screen_pipe - YouTube",
                Some("https://www.youtube.com/@screen_pipe/videos"),
            ),
            (
                "Google Chrome",
                "Architecture deep dive",
                Some("https://docs.example.com/architecture/memory-cards"),
            ),
            ("Finder", "Project", None),
        ];

        for (app, title, url) in cases {
            let gate_input = input(app, title, url);
            let expected = match evaluate_safety(gate_input.safety_context(), &blocklist) {
                SafetyDecision::SkipStorage(fndr_privacy::SafetyReason::PrivateBrowsing) => {
                    GateOutcome::Drop(SkipReason::PrivateBrowsing)
                }
                SafetyDecision::SkipStorage(_) => GateOutcome::Drop(SkipReason::PreCapturePrivacy),
                SafetyDecision::Allow | SafetyDecision::Redact(_) => {
                    match classify_capture_surface_policy(app, title, url) {
                        CaptureSurfacePolicy::SkipFrame => {
                            GateOutcome::Drop(SkipReason::AdmissionPolicy)
                        }
                        CaptureSurfacePolicy::UrlOnly => GateOutcome::UrlOnly,
                        CaptureSurfacePolicy::Normal => GateOutcome::Admit,
                    }
                }
            };
            assert_eq!(
                policy.evaluate(&gate_input, GateStage::Metadata).outcome,
                expected,
                "table diverged from the pre-table gates for {app}/{title}/{url:?}"
            );
        }
    }

    #[test]
    fn a_text_stage_gate_cannot_fire_before_ocr_exists() {
        let policy = CaptureGatePolicy::new(Blocklist::default());
        let metadata_only = input("Terminal", "shell", None);
        assert_eq!(
            policy.evaluate(&metadata_only, GateStage::Text).outcome,
            GateOutcome::Admit
        );

        let with_text = metadata_only.with_ocr_text("export API_KEY=top-secret");
        assert_eq!(
            policy.evaluate(&with_text, GateStage::Text),
            GateEvaluation {
                fired: Some(GateId::PrivacySecretPattern),
                outcome: GateOutcome::Redact,
            }
        );
    }

    #[test]
    fn the_gate_names_itself_so_a_drop_is_attributable() {
        let policy = CaptureGatePolicy::new(Blocklist::new(&["Notes"], &[]));
        assert_eq!(
            policy
                .evaluate(&input("Notes", "Groceries", None), GateStage::Metadata)
                .fired,
            Some(GateId::PrivacyUserBlocklist)
        );
        assert_eq!(
            policy
                .evaluate(&input("1Password", "Vault", None), GateStage::Metadata)
                .fired,
            Some(GateId::PrivacyPasswordManager)
        );
    }

    #[test]
    fn disabling_a_row_moves_its_drops_to_the_next_matching_gate() {
        // The knob the replay harness uses to model a policy change. It is
        // deliberately not reachable by accident: the default table has every
        // row enabled and `the_table_is_the_order_of_record` asserts it.
        let context = input("Safari", "Sign in", Some("https://online.chase.com/login"));
        let shipped = CaptureGatePolicy::new(Blocklist::default());
        assert_eq!(
            shipped.evaluate(&context, GateStage::Metadata).fired,
            Some(GateId::PrivacyFinancialSite)
        );

        let widened = shipped.with_gate_disabled(GateId::PrivacyFinancialSite);
        assert_eq!(
            widened.evaluate(&context, GateStage::Metadata).fired,
            Some(GateId::PrivacyAuthentication),
            "with the financial gate off, the next gate must still catch this"
        );
    }

    #[test]
    fn the_pipeline_gate_maps_outcomes_onto_the_existing_skip_reasons() {
        let gate = PolicyGate::new(Blocklist::default());
        let context = |app: &str, title: &str, url: Option<&str>| CaptureContext {
            app_name: app.to_owned(),
            bundle_id: Some("com.example.app".to_owned()),
            window_title: title.to_owned(),
            url: url.map(str::to_owned),
            observed_at_ms: 1_000,
        };

        assert_eq!(
            gate.evaluate(&context("1Password", "Vault", None)),
            GateDecision::Skip(SkipReason::PreCapturePrivacy)
        );
        assert_eq!(
            gate.evaluate(&context("Google Chrome", "New Incognito Window", None)),
            GateDecision::Skip(SkipReason::PrivateBrowsing)
        );
        assert_eq!(
            gate.evaluate(&context(
                "Google Chrome",
                "Search results - YouTube",
                Some("https://www.youtube.com/results?search_query=x"),
            )),
            GateDecision::Skip(SkipReason::AdmissionPolicy)
        );
        assert_eq!(
            gate.evaluate(&context(
                "Google Chrome",
                "screen_pipe - YouTube",
                Some("https://www.youtube.com/@screen_pipe/videos"),
            )),
            GateDecision::UrlOnly
        );
        assert_eq!(
            gate.evaluate(&context("Finder", "Project", None)),
            GateDecision::Allow
        );
    }
}
