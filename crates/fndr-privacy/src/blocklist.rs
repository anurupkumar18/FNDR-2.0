//! Blocklist v2 (T-801). Rewritten, deliberately NOT ported: v1 matched with
//! bidirectional substring containment (`app.contains(entry) ||
//! entry.contains(app)` and raw substring over whole URLs), so blocking
//! "Arc" blocked "Architecture Tool", a short entry blocked almost
//! everything, blocking "bank.com" blocked "burbank.com", and blocking
//! "mail.google.com" escalated to blocking all of google.com. The ADR-005
//! DISCARD ruling applies; only the semantics people expect survive:
//! exact-token app matching and suffix-domain matching on the parsed host.

use url::Url;

/// User-configured capture exclusions. Entries are normalized on
/// construction; matching is total (no allocation-order or config-order
/// dependence) and case-insensitive.
#[derive(Debug, Clone, Default)]
pub struct Blocklist {
    /// Normalized app entries: lowercase, trimmed, inner whitespace collapsed.
    apps: Vec<String>,
    /// Normalized domain entries: lowercase host labels, no scheme, no port,
    /// no leading dot.
    domains: Vec<String>,
}

fn normalize_app(entry: &str) -> String {
    entry
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

pub(crate) fn normalize_domain(entry: &str) -> Option<String> {
    let trimmed = entry.trim().trim_start_matches('.').to_lowercase();
    // Accept either a bare host ("bank.com") or a pasted URL.
    let host = if trimmed.contains("://") {
        Url::parse(&trimmed).ok()?.host_str()?.to_string()
    } else {
        trimmed.split(['/', ':']).next()?.to_string()
    };
    if host.is_empty() { None } else { Some(host) }
}

/// True when `host` equals `suffix` or is a subdomain of it, on label
/// boundaries. "bank.com" matches "online.bank.com", never "burbank.com".
pub(crate) fn host_matches_suffix(host: &str, suffix: &str) -> bool {
    host == suffix
        || (host.len() > suffix.len()
            && host.ends_with(suffix)
            && host.as_bytes()[host.len() - suffix.len() - 1] == b'.')
}

pub(crate) fn host_from_url(url: &str) -> Option<String> {
    Url::parse(url.trim())
        .ok()
        .and_then(|url| url.host_str().map(|host| host.to_lowercase()))
        .or_else(|| normalize_domain(url))
}

impl Blocklist {
    pub fn new<S: AsRef<str>>(app_entries: &[S], domain_entries: &[S]) -> Self {
        Self {
            apps: app_entries
                .iter()
                .map(|e| normalize_app(e.as_ref()))
                .filter(|e| !e.is_empty())
                .collect(),
            domains: domain_entries
                .iter()
                .filter_map(|e| normalize_domain(e.as_ref()))
                .collect(),
        }
    }

    /// Exact-token matching: an entry blocks an app when it equals the whole
    /// normalized name or appears as a contiguous run of whole tokens.
    /// "arc" blocks "Arc" but never "Architecture Tool"; "chrome" blocks
    /// "Google Chrome"; "s" blocks only an app literally named "s".
    pub fn blocks_app(&self, app_name: &str) -> bool {
        let name = normalize_app(app_name);
        if name.is_empty() {
            return false;
        }
        let name_tokens: Vec<&str> = name.split(' ').collect();
        self.apps.iter().any(|entry| {
            if *entry == name {
                return true;
            }
            let entry_tokens: Vec<&str> = entry.split(' ').collect();
            name_tokens
                .windows(entry_tokens.len())
                .any(|window| window == entry_tokens.as_slice())
        })
    }

    /// Suffix-domain matching on the parsed host only. A blocked domain
    /// covers itself and its subdomains; it never escalates to a parent
    /// domain and never matches inside path, query, or unrelated hosts.
    pub fn blocks_url(&self, url: &str) -> bool {
        let Some(host) = host_from_url(url) else {
            return false;
        };
        self.domains
            .iter()
            .any(|suffix| host_matches_suffix(&host, suffix))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blocklist(apps: &[&str], domains: &[&str]) -> Blocklist {
        Blocklist::new(apps, domains)
    }

    // The v1 false positives, pinned so they cannot recur.

    #[test]
    fn short_app_entry_does_not_match_by_substring() {
        let bl = blocklist(&["Arc"], &[]);
        assert!(bl.blocks_app("Arc"));
        assert!(
            !bl.blocks_app("Architecture Tool"),
            "v1 regression: substring"
        );
        assert!(
            !bl.blocks_app("Arcade Machine"),
            "token prefix is not a token"
        );
    }

    #[test]
    fn single_letter_entry_only_matches_itself() {
        let bl = blocklist(&["s"], &[]);
        assert!(bl.blocks_app("s"));
        assert!(!bl.blocks_app("Safari"));
        assert!(
            !bl.blocks_app("iOS Simulator"),
            "v1 regression: contains('s')"
        );
    }

    #[test]
    fn app_name_containing_entry_as_token_matches() {
        let bl = blocklist(&["chrome"], &[]);
        assert!(bl.blocks_app("Google Chrome"));
        assert!(bl.blocks_app("chrome"));
        assert!(!bl.blocks_app("Chromium"), "different token");
    }

    #[test]
    fn multi_token_entry_matches_contiguously() {
        let bl = blocklist(&["google chrome"], &[]);
        assert!(bl.blocks_app("Google Chrome"));
        assert!(bl.blocks_app("Google Chrome Beta"));
        assert!(!bl.blocks_app("Google Meet"));
    }

    #[test]
    fn reverse_containment_never_matches() {
        // v1: entry.contains(app) meant an app named "a" was blocked by any
        // entry containing the letter a.
        let bl = blocklist(&["Password Manager Pro"], &[]);
        assert!(!bl.blocks_app("Password"));
        assert!(!bl.blocks_app("Manager"));
    }

    #[test]
    fn domain_blocks_itself_and_subdomains_only() {
        let bl = blocklist(&[], &["bank.com"]);
        assert!(bl.blocks_url("https://bank.com/login"));
        assert!(bl.blocks_url("https://online.bank.com/account?id=1"));
        assert!(
            !bl.blocks_url("https://burbank.com/"),
            "v1 regression: substring"
        );
        assert!(!bl.blocks_url("https://notbank.com/"));
        assert!(
            !bl.blocks_url("https://bank.com.evil.example/"),
            "suffix spoof"
        );
    }

    #[test]
    fn subdomain_entry_does_not_escalate_to_parent() {
        let bl = blocklist(&[], &["mail.google.com"]);
        assert!(bl.blocks_url("https://mail.google.com/u/0"));
        assert!(bl.blocks_url("https://a.mail.google.com/"));
        assert!(
            !bl.blocks_url("https://google.com/"),
            "v1 regression: parent-domain escalation"
        );
        assert!(!bl.blocks_url("https://docs.google.com/"));
    }

    #[test]
    fn domain_never_matches_path_or_query() {
        let bl = blocklist(&[], &["bank.com"]);
        assert!(!bl.blocks_url("https://example.com/bank.com/page"));
        assert!(!bl.blocks_url("https://example.com/?next=bank.com"));
    }

    #[test]
    fn entries_normalize_case_scheme_port_and_dots() {
        let bl = blocklist(&[], &["https://Bank.com:443", ".chase.com"]);
        assert!(bl.blocks_url("https://BANK.COM/"));
        assert!(bl.blocks_url("https://www.chase.com/"));
        let bl = blocklist(&["  Google   Chrome  "], &[]);
        assert!(bl.blocks_app("google chrome"));
    }

    #[test]
    fn bare_host_input_still_matches() {
        let bl = blocklist(&[], &["bank.com"]);
        assert!(bl.blocks_url("online.bank.com"));
        assert!(!bl.blocks_url("burbank.com"));
        assert!(!bl.blocks_url("not a url at all"));
    }

    #[test]
    fn empty_inputs_block_nothing() {
        let bl = blocklist(&[], &[]);
        assert!(!bl.blocks_app("Anything"));
        assert!(!bl.blocks_url("https://anything.example/"));
        let bl = blocklist(&["", "   "], &["", "   "]);
        assert!(!bl.blocks_app("Anything"));
    }
}
