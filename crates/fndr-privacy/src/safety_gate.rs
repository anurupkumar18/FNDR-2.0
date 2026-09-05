//! Deterministic safety gate for captured context.
//!
//! The keyword and domain data are ported from FNDR v1
//! `src-tauri/src/privacy/safety_gate.rs`; matching and the typed reason
//! contract are rebuilt here because the v1 implementation used unsafe raw
//! substring matching for both user blocklists and domains.

use crate::Blocklist;
use crate::blocklist::{host_from_url, host_matches_suffix};

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

pub fn evaluate(context: SafetyContext<'_>, blocklist: &Blocklist) -> SafetyDecision {
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

    if PASSWORD_MANAGER_NAMES.iter().any(|name| app.contains(name)) {
        return SafetyDecision::SkipStorage(SafetyReason::PasswordManager);
    }

    if title.contains("incognito")
        || (title.contains("private") && (title.contains("browsing") || title.contains("window")))
    {
        return SafetyDecision::SkipStorage(SafetyReason::PrivateBrowsing);
    }

    if matches_financial_site(url) {
        return SafetyDecision::SkipStorage(SafetyReason::FinancialSite);
    }

    if matches_medical_site(url) {
        return SafetyDecision::SkipStorage(SafetyReason::MedicalSite);
    }

    if AUTH_INDICATORS
        .iter()
        .any(|indicator| title.contains(indicator) || url_lower.contains(indicator))
    {
        return SafetyDecision::SkipStorage(SafetyReason::Authentication);
    }

    if context.ocr_text.is_some_and(contains_secret_pattern) {
        return SafetyDecision::Redact(SafetyReason::SecretPattern);
    }

    SafetyDecision::Allow
}

pub fn redact_secret_lines(text: &str) -> (String, usize) {
    let mut redaction_count = 0;
    let redacted = text
        .lines()
        .map(|line| {
            if contains_secret_pattern(line) {
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

fn matches_financial_site(url: &str) -> bool {
    let Some(host) = host_from_url(url) else {
        return false;
    };
    FINANCIAL_DOMAINS
        .iter()
        .any(|domain| host_matches_suffix(&host, domain))
}

fn matches_medical_site(url: &str) -> bool {
    let Some(host) = host_from_url(url) else {
        return false;
    };
    host.split('.')
        .any(|label| MEDICAL_DOMAIN_MARKERS.contains(&label))
}

fn contains_secret_pattern(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    SECRET_PATTERNS.iter().any(|pattern| text.contains(pattern))
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
        assert!(matches_financial_site("https://online.chase.com/account"));
        assert!(!matches_financial_site("https://notchase.com/account"));
        assert!(!matches_financial_site("https://example.com/chase.com"));
    }
}
