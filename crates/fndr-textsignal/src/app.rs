//! Typed app identity for the app-aware cleanup policy.
//!
//! The v1 donor classified the foreground app by substring-matching its
//! localized name (`name.contains("arc")`, `name.contains("edge")`,
//! `name.contains("code")`). That is wrong in both directions:
//!
//! - **False positives.** "Search" contains `arc` and "Knowledge Base"
//!   contains `edge`, so both were treated as browsers and had their
//!   symbol-heavy lines dropped at the browser threshold.
//! - **False negatives.** The localized name is user- and locale-controlled.
//!   A renamed or non-English Chrome, or Arc (whose bundle is
//!   `company.thebrowser.Browser`), classified as `Other`.
//!
//! macOS already hands the capture pipeline an authoritative, stable
//! identifier next to the name: `CFBundleIdentifier`. This module makes that
//! identifier the primary signal and demotes the name to a fallback that
//! matches whole tokens instead of substrings.

/// What kind of surface the foreground app is, as far as OCR cleanup cares.
///
/// The classes are mutually exclusive by construction. They were three
/// independent booleans in the donor, but no real app is both a browser and a
/// mail client, and an enum makes the precedence rule explicit instead of
/// implicit in the order of `if` arms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AppClass {
    /// Tab strips, toolbars, feed and notification fragments dominate.
    Browser,
    /// Editors and terminals, where file listings and symbol-heavy lines are
    /// evidence rather than noise.
    Code,
    /// Mail clients, where `From:`/`Subject:` headers are evidence but the
    /// sidebar reuses browser-shaped nav labels.
    Mail,
    #[default]
    Other,
}

/// The foreground app as the cleanup policy sees it: a display name for
/// snippets, plus the class resolved once at the capture boundary.
#[derive(Debug, Clone, Copy)]
pub struct AppIdentity<'a> {
    name: &'a str,
    class: AppClass,
}

impl<'a> AppIdentity<'a> {
    /// Resolve an identity from what the foreground metadata source reports.
    ///
    /// A recognized bundle identifier decides the class outright. Only an
    /// absent or unrecognized bundle falls through to the name.
    pub fn new(name: &'a str, bundle_id: Option<&str>) -> Self {
        let class = bundle_id
            .and_then(class_from_bundle_id)
            .unwrap_or_else(|| class_from_name(name));
        Self { name, class }
    }

    /// Resolve from a name alone, for callers with no bundle identifier
    /// (fixtures, stored records written before bundle IDs were threaded
    /// through, and the empty-context case).
    pub fn from_name(name: &'a str) -> Self {
        Self::new(name, None)
    }

    pub fn name(&self) -> &'a str {
        self.name
    }

    pub fn class(&self) -> AppClass {
        self.class
    }

    pub fn is_browser(&self) -> bool {
        self.class == AppClass::Browser
    }

    pub fn is_code(&self) -> bool {
        self.class == AppClass::Code
    }

    pub fn is_mail(&self) -> bool {
        self.class == AppClass::Mail
    }
}

/// Bundle identifiers matched in full (ASCII case-insensitively; macOS treats
/// bundle IDs as case-insensitive).
///
/// Entries marked `verified` were read from `CFBundleIdentifier` on a macOS
/// install; the rest are the identifiers those vendors ship. An identifier we
/// do not list falls through to [`class_from_name`], so an omission costs
/// accuracy, never correctness. Adding a *wrong* identifier does cost
/// correctness, which is why speculative entries stay out.
const BUNDLE_EXACT: &[(&str, AppClass)] = &[
    // Browsers
    ("com.apple.safari", AppClass::Browser), // verified
    ("com.apple.safaritechnologypreview", AppClass::Browser),
    ("org.chromium.chromium", AppClass::Browser),
    ("company.thebrowser.browser", AppClass::Browser), // Arc
    ("org.mozilla.firefox", AppClass::Browser),
    ("org.mozilla.firefoxdeveloperedition", AppClass::Browser),
    ("org.mozilla.nightly", AppClass::Browser),
    ("com.vivaldi.vivaldi", AppClass::Browser),
    // Editors and terminals
    ("com.apple.terminal", AppClass::Code), // verified
    ("com.googlecode.iterm2", AppClass::Code),
    ("com.apple.dt.xcode", AppClass::Code), // verified
    ("com.microsoft.vscode", AppClass::Code),
    ("com.microsoft.vscodeinsiders", AppClass::Code),
    ("com.visualstudio.code.oss", AppClass::Code),
    ("com.todesktop.230313mzl4w4u92", AppClass::Code), // Cursor, verified
    ("com.google.antigravity", AppClass::Code),        // verified
    ("dev.zed.zed", AppClass::Code),
    // Mail
    ("com.apple.mail", AppClass::Mail), // verified
    ("com.microsoft.outlook", AppClass::Mail),
    ("org.mozilla.thunderbird", AppClass::Mail),
    ("com.readdle.smartemail-mac", AppClass::Mail), // Spark
];

/// Bundle identifier prefixes, for families that append a channel or version
/// (`com.google.Chrome.beta`, `com.jetbrains.pycharm`). Checked only after
/// [`BUNDLE_EXACT`] misses, so a family member that needs a different class
/// can still be listed exactly. `org.mozilla.` is deliberately absent: it
/// covers both Firefox and Thunderbird, which are different classes.
const BUNDLE_PREFIX: &[(&str, AppClass)] = &[
    ("com.google.chrome", AppClass::Browser), // verified for the base id
    ("com.brave.browser", AppClass::Browser),
    ("com.microsoft.edgemac", AppClass::Browser),
    ("com.operasoftware.opera", AppClass::Browser),
    ("com.jetbrains.", AppClass::Code),
    ("com.sublimetext.", AppClass::Code),
];

fn class_from_bundle_id(bundle_id: &str) -> Option<AppClass> {
    let bundle = bundle_id.trim().to_ascii_lowercase();
    if bundle.is_empty() {
        return None;
    }
    if let Some((_, class)) = BUNDLE_EXACT.iter().find(|(id, _)| *id == bundle) {
        return Some(*class);
    }
    BUNDLE_PREFIX
        .iter()
        .find(|(prefix, _)| bundle.starts_with(prefix))
        .map(|(_, class)| *class)
}

/// Whole-token name matches, used only when the bundle identifier is missing
/// or unknown. Tokens, not substrings: `contains("edge")` also matched
/// "Knowledge", and `contains("arc")` also matched "Search".
const NAME_TOKENS: &[(&str, AppClass)] = &[
    ("chrome", AppClass::Browser),
    ("chromium", AppClass::Browser),
    ("safari", AppClass::Browser),
    ("arc", AppClass::Browser),
    ("firefox", AppClass::Browser),
    ("edge", AppClass::Browser),
    ("brave", AppClass::Browser),
    ("opera", AppClass::Browser),
    ("vivaldi", AppClass::Browser),
    ("terminal", AppClass::Code),
    ("iterm", AppClass::Code),
    ("iterm2", AppClass::Code),
    ("xcode", AppClass::Code),
    ("vscode", AppClass::Code),
    ("code", AppClass::Code),
    ("cursor", AppClass::Code),
    ("zed", AppClass::Code),
    ("sublime", AppClass::Code),
    ("intellij", AppClass::Code),
    ("pycharm", AppClass::Code),
    ("webstorm", AppClass::Code),
    ("goland", AppClass::Code),
    ("alacritty", AppClass::Code),
    ("wezterm", AppClass::Code),
    ("ghostty", AppClass::Code),
    ("warp", AppClass::Code),
    ("mail", AppClass::Mail),
    ("gmail", AppClass::Mail),
    ("outlook", AppClass::Mail),
    ("superhuman", AppClass::Mail),
    ("thunderbird", AppClass::Mail),
    ("airmail", AppClass::Mail),
];

/// Precedence when a name yields tokens of more than one class (for example
/// "Chrome Canary Mail Preview"). Browser wins first because its line
/// thresholds are the most conservative: mistaking a browser for something
/// else keeps chrome noise in memory, which is the failure users notice.
const CLASS_PRECEDENCE: &[AppClass] = &[AppClass::Browser, AppClass::Code, AppClass::Mail];

fn class_from_name(app_name: &str) -> AppClass {
    let lower = app_name.to_lowercase();
    let mut matched = [false; 3];
    for token in lower.split(|ch: char| !ch.is_alphanumeric()) {
        if token.is_empty() {
            continue;
        }
        for (candidate, class) in NAME_TOKENS {
            if *candidate != token {
                continue;
            }
            if let Some(slot) = CLASS_PRECEDENCE.iter().position(|c| c == class) {
                matched[slot] = true;
            }
        }
    }
    CLASS_PRECEDENCE
        .iter()
        .zip(matched)
        .find(|(_, hit)| *hit)
        .map(|(class, _)| *class)
        .unwrap_or(AppClass::Other)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_id_decides_over_the_localized_name() {
        // A user-renamed or localized Chrome keeps its bundle identifier.
        let identity = AppIdentity::new("Navegador", Some("com.google.Chrome"));
        assert_eq!(identity.class(), AppClass::Browser);
        assert_eq!(identity.name(), "Navegador");
    }

    #[test]
    fn arc_is_a_browser_by_bundle_despite_an_unmatched_name() {
        // "Arc" only matched the donor by the substring that also matched
        // "Search"; the bundle identifier says it outright.
        let identity = AppIdentity::new("Arc", Some("company.thebrowser.Browser"));
        assert_eq!(identity.class(), AppClass::Browser);
    }

    #[test]
    fn bundle_ids_match_case_insensitively_and_by_channel_prefix() {
        assert_eq!(
            AppIdentity::new("Chrome Beta", Some("com.google.Chrome.beta")).class(),
            AppClass::Browser
        );
        assert_eq!(
            AppIdentity::new("Terminal", Some("COM.APPLE.TERMINAL")).class(),
            AppClass::Code
        );
        assert_eq!(
            AppIdentity::new("PyCharm", Some("com.jetbrains.pycharm")).class(),
            AppClass::Code
        );
    }

    #[test]
    fn thunderbird_is_mail_not_a_mozilla_browser() {
        // The reason `org.mozilla.` is not a browser prefix rule.
        assert_eq!(
            AppIdentity::new("Thunderbird", Some("org.mozilla.thunderbird")).class(),
            AppClass::Mail
        );
        assert_eq!(
            AppIdentity::new("Firefox", Some("org.mozilla.firefox")).class(),
            AppClass::Browser
        );
    }

    #[test]
    fn substring_false_positives_from_the_donor_are_gone() {
        // "Search" contains "arc"; "Knowledge Base" contains "edge".
        assert_eq!(AppIdentity::from_name("Search").class(), AppClass::Other);
        assert_eq!(
            AppIdentity::from_name("Knowledge Base").class(),
            AppClass::Other
        );
        // "Barcode Scanner" contains "code".
        assert_eq!(
            AppIdentity::from_name("Barcode Scanner").class(),
            AppClass::Other
        );
    }

    #[test]
    fn name_fallback_still_classifies_known_apps() {
        assert_eq!(
            AppIdentity::from_name("Google Chrome").class(),
            AppClass::Browser
        );
        assert_eq!(
            AppIdentity::from_name("Visual Studio Code").class(),
            AppClass::Code
        );
        assert_eq!(AppIdentity::from_name("iTerm2").class(), AppClass::Code);
        assert_eq!(AppIdentity::from_name("Mail").class(), AppClass::Mail);
    }

    #[test]
    fn an_unknown_bundle_id_falls_through_to_the_name() {
        let identity = AppIdentity::new("Google Chrome", Some("com.example.unknown"));
        assert_eq!(identity.class(), AppClass::Browser);
    }

    #[test]
    fn an_empty_identity_is_other() {
        assert_eq!(AppIdentity::new("", Some("")).class(), AppClass::Other);
        assert_eq!(AppIdentity::from_name("").class(), AppClass::Other);
    }

    #[test]
    fn browser_wins_when_a_name_yields_more_than_one_class() {
        assert_eq!(
            AppIdentity::from_name("Chrome Mail Preview").class(),
            AppClass::Browser
        );
    }
}
