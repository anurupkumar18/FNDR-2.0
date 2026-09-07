//! Deterministic safety gate for captured context.
//!
//! The keyword and domain data are ported from FNDR v1
//! `src-tauri/src/privacy/safety_gate.rs`; matching and the typed reason
//! contract are rebuilt here because the v1 implementation used unsafe raw
//! substring matching for both user blocklists and domains.

use crate::Blocklist;
use crate::blocklist::{host_from_url, host_matches_suffix, normalize_domain};

const PASSWORD_MANAGER_NAMES: &[&str] = &[
    "1password",
    "bitwarden",
    "keychain",
    "lastpass",
    "dashlane",
    "keepass",
];

const FINANCIAL_DOMAINS: &[&str] = &[
    "chase.com",
    "bankofamerica.com",
    "wellsfargo.com",
    "citibank.com",
    "capitalone.com",
    "usbank.com",
    "fidelity.com",
    "vanguard.com",
    "schwab.com",
    "americanexpress.com",
    "discover.com",
    "paypal.com",
    "venmo.com",
    "robinhood.com",
];

const MEDICAL_DOMAIN_MARKERS: &[&str] = &["mychart", "healthportal", "patientportal"];

const AUTH_INDICATORS: &[&str] = &[
    "sign in",
    "log in",
    "login",
    "authenticate",
    "authorization",
    "oauth",
    "saml",
    "two-factor",
    "2fa",
];

const SECRET_PATTERNS: &[&str] = &[
    "api_key",
    "apikey",
    "secret_key",
    "private_key",
    "access_token",
    "password:",
    "passwd:",
    "token:",
    "-----begin rsa",
    "-----begin ec",
    "ghp_",
    "sk-",
    "xoxb-",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SafetyReason {
    UserBlocklist,
    FndrSelfCapture,
    PasswordManager,
    PrivateBrowsing,
    FinancialSite,
    MedicalSite,
    Authentication,
    SecretPattern,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SafetyDecision {
    Allow,
    Redact(SafetyReason),
    SkipStorage(SafetyReason),
}

impl SafetyDecision {
    pub fn reason(self) -> Option<SafetyReason> {
        match self {
            Self::Allow => None,
            Self::Redact(reason) | Self::SkipStorage(reason) => Some(reason),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SafetyContext<'a> {
    pub app_name: Option<&'a str>,
    pub bundle_id: Option<&'a str>,
    pub url: Option<&'a str>,
    pub window_title: Option<&'a str>,
    pub ocr_text: Option<&'a str>,
}

/// The built-in sensitive-context lists (T-802), as owner-editable data
/// instead of compiled-in constants. `default()` reproduces exactly the
/// behavior of the original hardcoded lists; a caller that needs to add or
/// remove entries constructs its own policy with [`SensitiveContextPolicy::new`]
/// and passes it to [`evaluate_with_policy`] / [`redact_secret_lines_with_policy`].
/// This crate does no file or database I/O itself (ADR-004 posture); a caller
/// that wants to load overrides from disk or `settings` does so and hands the
/// parsed values here.
#[derive(Debug, Clone)]
pub struct SensitiveContextPolicy {
    password_manager_names: Vec<String>,
    financial_domains: Vec<String>,
    medical_domain_markers: Vec<String>,
    auth_indicators: Vec<String>,
    secret_patterns: Vec<String>,
}

impl SensitiveContextPolicy {
    /// Entries are normalized the same way as `Blocklist`: names/indicators/
    /// patterns are lowercased and trimmed, domains are parsed to a bare host
    /// so suffix matching stays safe against scheme/port/case spoofing.
    pub fn new<S: AsRef<str>>(
        password_manager_names: &[S],
        financial_domains: &[S],
        medical_domain_markers: &[S],
        auth_indicators: &[S],
        secret_patterns: &[S],
    ) -> Self {
        let normalize_word = |entries: &[S]| -> Vec<String> {
            entries
                .iter()
                .map(|e| e.as_ref().trim().to_ascii_lowercase())
                .filter(|e| !e.is_empty())
                .collect()
        };
        Self {
            password_manager_names: normalize_word(password_manager_names),
            financial_domains: financial_domains
                .iter()
                .filter_map(|e| normalize_domain(e.as_ref()))
                .collect(),
            medical_domain_markers: normalize_word(medical_domain_markers),
            auth_indicators: normalize_word(auth_indicators),
            secret_patterns: normalize_word(secret_patterns),
        }
    }
}

impl Default for SensitiveContextPolicy {
    fn default() -> Self {
        Self::new(
            PASSWORD_MANAGER_NAMES,
            FINANCIAL_DOMAINS,
            MEDICAL_DOMAIN_MARKERS,
            AUTH_INDICATORS,
            SECRET_PATTERNS,
        )
    }
}

pub fn evaluate(context: SafetyContext<'_>, blocklist: &Blocklist) -> SafetyDecision {
    evaluate_core(
        context,
        blocklist,
        PASSWORD_MANAGER_NAMES,
        FINANCIAL_DOMAINS,
        MEDICAL_DOMAIN_MARKERS,
        AUTH_INDICATORS,
        SECRET_PATTERNS,
    )
}

/// Same policy, with the built-in sensitive-context lists replaced by an
/// owner-provided [`SensitiveContextPolicy`] (T-802). The blocklist (user
/// app/domain exclusions) is unaffected; it is a separate mechanism.
pub fn evaluate_with_policy(
    context: SafetyContext<'_>,
    blocklist: &Blocklist,
    policy: &SensitiveContextPolicy,
) -> SafetyDecision {
    evaluate_core(
        context,
        blocklist,
        &policy.password_manager_names,
        &policy.financial_domains,
        &policy.medical_domain_markers,
        &policy.auth_indicators,
        &policy.secret_patterns,
    )
}

#[allow(clippy::too_many_arguments)]
fn evaluate_core<S: AsRef<str>>(
    context: SafetyContext<'_>,
    blocklist: &Blocklist,
    password_manager_names: &[S],
    financial_domains: &[S],
    medical_domain_markers: &[S],
    auth_indicators: &[S],
    secret_patterns: &[S],
) -> SafetyDecision {
    let app = context.app_name.unwrap_or("").to_ascii_lowercase();
    let title = context.window_title.unwrap_or("").to_ascii_lowercase();
    let url = context.url.unwrap_or("");
    let url_lower = url.to_ascii_lowercase();

    if blocklist.blocks_app(&app) || blocklist.blocks_url(url) {
        return SafetyDecision::SkipStorage(SafetyReason::UserBlocklist);
    }

    if context.bundle_id.is_some_and(|bundle_id| {
        let bundle_id = bundle_id.to_ascii_lowercase();
        (bundle_id.starts_with("com.fndr") || bundle_id.contains(".fndr."))
            && !app.contains("fndr meeting")
    }) {
        return SafetyDecision::SkipStorage(SafetyReason::FndrSelfCapture);
    }

    if password_manager_names
        .iter()
        .any(|name| app.contains(name.as_ref()))
    {
        return SafetyDecision::SkipStorage(SafetyReason::PasswordManager);
    }

    if title.contains("incognito")
        || (title.contains("private") && (title.contains("browsing") || title.contains("window")))
    {
        return SafetyDecision::SkipStorage(SafetyReason::PrivateBrowsing);
    }

    if matches_domain_suffix(url, financial_domains) {
        return SafetyDecision::SkipStorage(SafetyReason::FinancialSite);
    }

    if matches_domain_label(url, medical_domain_markers) {
        return SafetyDecision::SkipStorage(SafetyReason::MedicalSite);
    }

    if auth_indicators.iter().any(|indicator| {
        title.contains(indicator.as_ref()) || url_lower.contains(indicator.as_ref())
    }) {
        return SafetyDecision::SkipStorage(SafetyReason::Authentication);
    }

    if context
        .ocr_text
        .is_some_and(|text| contains_secret_pattern(text, secret_patterns))
    {
        return SafetyDecision::Redact(SafetyReason::SecretPattern);
    }

    SafetyDecision::Allow
}

pub fn redact_secret_lines(text: &str) -> (String, usize) {
    redact_secret_lines_matching(text, SECRET_PATTERNS)
}

/// Same redaction, with the built-in secret patterns replaced by the
/// policy's (T-802).
pub fn redact_secret_lines_with_policy(
    text: &str,
    policy: &SensitiveContextPolicy,
) -> (String, usize) {
    redact_secret_lines_matching(text, &policy.secret_patterns)
}

fn redact_secret_lines_matching<S: AsRef<str>>(text: &str, patterns: &[S]) -> (String, usize) {
    let mut redaction_count = 0;
    let redacted = text
        .lines()
        .map(|line| {
            if contains_secret_pattern(line, patterns) {
                redaction_count += 1;
                "[REDACTED: secret pattern]"
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    (redacted, redaction_count)
}

fn matches_domain_suffix<S: AsRef<str>>(url: &str, domains: &[S]) -> bool {
    let Some(host) = host_from_url(url) else {
        return false;
    };
    domains
        .iter()
        .any(|domain| host_matches_suffix(&host, domain.as_ref()))
}

fn matches_domain_label<S: AsRef<str>>(url: &str, markers: &[S]) -> bool {
    let Some(host) = host_from_url(url) else {
        return false;
    };
    host.split('.')
        .any(|label| markers.iter().any(|m| m.as_ref() == label))
}

fn contains_secret_pattern<S: AsRef<str>>(text: &str, patterns: &[S]) -> bool {
    let text = text.to_ascii_lowercase();
    patterns
        .iter()
        .any(|pattern| text.contains(pattern.as_ref()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context<'a>(
        app_name: Option<&'a str>,
        bundle_id: Option<&'a str>,
        url: Option<&'a str>,
        window_title: Option<&'a str>,
        ocr_text: Option<&'a str>,
    ) -> SafetyContext<'a> {
        SafetyContext {
            app_name,
            bundle_id,
            url,
            window_title,
            ocr_text,
        }
    }

    #[test]
    fn normal_content_is_allowed() {
        assert_eq!(
            evaluate(
                context(
                    Some("VS Code"),
                    Some("com.microsoft.VSCode"),
                    Some("https://docs.example.com/fndr"),
                    Some("main.rs"),
                    Some("fn main() {}"),
                ),
                &Blocklist::default(),
            ),
            SafetyDecision::Allow
        );
    }

    #[test]
    fn sensitive_contexts_skip_before_ocr() {
        for (app, url, title, reason) in [
            ("1Password", None, "Vault", SafetyReason::PasswordManager),
            (
                "Safari",
                Some("https://online.chase.com/account"),
                "Account overview",
                SafetyReason::FinancialSite,
            ),
            (
                "Safari",
                Some("https://example.com/login"),
                "Sign in",
                SafetyReason::Authentication,
            ),
            (
                "Chrome",
                None,
                "New Incognito Window",
                SafetyReason::PrivateBrowsing,
            ),
        ] {
            assert_eq!(
                evaluate(
                    context(Some(app), None, url, Some(title), None),
                    &Blocklist::default()
                ),
                SafetyDecision::SkipStorage(reason)
            );
        }
    }

    #[test]
    fn user_blocklist_uses_its_safe_matching_contract() {
        let blocklist = Blocklist::new(&["Google Chrome"], &["bank.com"]);
        assert_eq!(
            evaluate(
                context(Some("Google Chrome Beta"), None, None, Some("work"), None),
                &blocklist,
            ),
            SafetyDecision::SkipStorage(SafetyReason::UserBlocklist)
        );
        assert_eq!(
            evaluate(
                context(
                    Some("Safari"),
                    None,
                    Some("https://online.bank.com/"),
                    Some("work"),
                    None,
                ),
                &blocklist,
            ),
            SafetyDecision::SkipStorage(SafetyReason::UserBlocklist)
        );
        assert_eq!(
            evaluate(
                context(
                    Some("Architecture Tool"),
                    None,
                    Some("https://burbank.com/"),
                    Some("work"),
                    None,
                ),
                &blocklist,
            ),
            SafetyDecision::Allow
        );
    }

    #[test]
    fn secrets_redact_and_remove_the_entire_matching_line() {
        let text = "ordinary context\nexport API_KEY=top-secret\nmore ordinary context";
        assert_eq!(
            evaluate(
                context(Some("Terminal"), None, None, Some("shell"), Some(text)),
                &Blocklist::default(),
            ),
            SafetyDecision::Redact(SafetyReason::SecretPattern)
        );
        let (redacted, count) = redact_secret_lines(text);
        assert_eq!(count, 1);
        assert_eq!(
            redacted,
            "ordinary context\n[REDACTED: secret pattern]\nmore ordinary context"
        );
        assert!(!redacted.contains("top-secret"));
    }

    #[test]
    fn financial_domain_matching_rejects_substring_spoofs() {
        assert!(matches_domain_suffix(
            "https://online.chase.com/account",
            FINANCIAL_DOMAINS
        ));
        assert!(!matches_domain_suffix(
            "https://notchase.com/account",
            FINANCIAL_DOMAINS
        ));
        assert!(!matches_domain_suffix(
            "https://example.com/chase.com",
            FINANCIAL_DOMAINS
        ));
    }

    #[test]
    fn default_policy_matches_built_in_lists_exactly() {
        let policy = SensitiveContextPolicy::default();
        let blocklist = Blocklist::default();
        let cases = [
            (Some("1Password"), None, Some("Vault"), None),
            (
                Some("Safari"),
                Some("https://online.chase.com/account"),
                Some("Account overview"),
                None,
            ),
            (
                Some("Safari"),
                Some("https://mychart.example-hospital.com/"),
                Some("Results"),
                None,
            ),
            (
                Some("Safari"),
                Some("https://example.com/login"),
                Some("Sign in"),
                None,
            ),
            (
                Some("Terminal"),
                None,
                Some("shell"),
                Some("export API_KEY=top-secret"),
            ),
        ];
        for (app, url, title, ocr) in cases {
            let ctx = context(app, None, url, title, ocr);
            assert_eq!(
                evaluate(ctx, &blocklist),
                evaluate_with_policy(ctx, &blocklist, &policy),
                "default policy diverged from built-in lists for {app:?}/{url:?}"
            );
        }
    }

    #[test]
    fn custom_policy_overrides_rather_than_unions_the_built_in_lists() {
        // A policy naming a company's own vault app blocks it, and the
        // built-in "1password" name is absent because this policy replaces
        // rather than extends the defaults (mirrors Blocklist's contract).
        let policy =
            SensitiveContextPolicy::new(&["mycompany-vault"], &[], &[], &[], &["internal-secret:"]);
        let blocklist = Blocklist::default();

        assert_eq!(
            evaluate_with_policy(
                context(Some("MyCompany-Vault"), None, None, Some("Vault"), None),
                &blocklist,
                &policy,
            ),
            SafetyDecision::SkipStorage(SafetyReason::PasswordManager)
        );
        assert_eq!(
            evaluate_with_policy(
                context(Some("1Password"), None, None, Some("Vault"), None),
                &blocklist,
                &policy,
            ),
            SafetyDecision::Allow,
            "custom policy replaces the built-in password-manager list, it does not extend it"
        );

        let (redacted, count) =
            redact_secret_lines_with_policy("note\ninternal-secret: shh\nmore", &policy);
        assert_eq!(count, 1);
        assert_eq!(redacted, "note\n[REDACTED: secret pattern]\nmore");
    }
}
