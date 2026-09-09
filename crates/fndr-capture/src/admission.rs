//! Capture-admission policy for browser surfaces.
//!
//! This stage deliberately classifies metadata only. The scheduler owns
//! metadata acquisition and persistence; keeping the policy pure makes its
//! decisions replayable without Screen Recording permission.

/// How the capture pipeline should handle a browser surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureSurfacePolicy {
    /// Capture and process the frame normally.
    Normal,
    /// Keep only URL metadata; do not capture pixels or OCR the page.
    UrlOnly,
    /// Drop this tick before capture.
    SkipFrame,
}

/// Which named admission rule produced a non-`Normal` classification.
///
/// The stable identity matters more than the outcome: two rules both mean
/// "SkipFrame", and the replay report has to say which of them dropped a
/// fixture. Rows in the capture gate policy table are keyed by this value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AdmissionRule {
    /// A browser window showing its own chrome (new tab, start page).
    GenericBrowserChrome,
    /// A search-results or feed surface: transient navigation, not content.
    NavigationSurface,
    /// A profile/channel/listing page: worth the URL, not the pixels.
    ListingSurface,
}

impl AdmissionRule {
    /// The classification this rule produces when it matches.
    pub fn policy(self) -> CaptureSurfacePolicy {
        match self {
            Self::GenericBrowserChrome | Self::NavigationSurface => CaptureSurfacePolicy::SkipFrame,
            Self::ListingSurface => CaptureSurfacePolicy::UrlOnly,
        }
    }

    /// Does this single rule fire for this surface? Asked one rule at a time
    /// by the capture gate policy table, so attribution is not reconstructed
    /// from the aggregate answer.
    pub fn matches(self, app_name: &str, window_title: &str, url: Option<&str>) -> bool {
        if !is_browser_app(app_name) {
            return false;
        }
        let Some(url) = url else {
            return false;
        };
        let title = window_title.to_ascii_lowercase();
        match self {
            Self::GenericBrowserChrome => is_generic_browser_chrome_title(&title),
            Self::NavigationSurface => is_navigation_surface(&UrlSurface::from_url(url)),
            Self::ListingSurface => is_listing_surface(&UrlSurface::from_url(url), &title),
        }
    }
}

/// The admission rules in evaluation order. First match wins.
pub const ADMISSION_RULES: &[AdmissionRule] = &[
    AdmissionRule::GenericBrowserChrome,
    AdmissionRule::NavigationSurface,
    AdmissionRule::ListingSurface,
];

/// Classify a browser surface before it reaches the capture source, naming
/// the rule responsible.
// Ported from FNDR v1 src-tauri/src/capture/admission.rs at 330a760b.
pub fn classify_capture_surface(
    app_name: &str,
    window_title: &str,
    url: Option<&str>,
) -> (CaptureSurfacePolicy, Option<AdmissionRule>) {
    for rule in ADMISSION_RULES {
        if rule.matches(app_name, window_title, url) {
            return (rule.policy(), Some(*rule));
        }
    }
    (CaptureSurfacePolicy::Normal, None)
}

/// Classification without attribution, for callers that only need the
/// outcome.
pub fn classify_capture_surface_policy(
    app_name: &str,
    window_title: &str,
    url: Option<&str>,
) -> CaptureSurfacePolicy {
    classify_capture_surface(app_name, window_title, url).0
}

fn is_browser_app(app_name: &str) -> bool {
    let app = app_name.to_ascii_lowercase();
    [
        "chrome", "safari", "firefox", "arc", "edge", "brave", "opera",
    ]
    .iter()
    .any(|needle| app.contains(needle))
}

fn is_generic_browser_chrome_title(title: &str) -> bool {
    ["new tab", "start page", "speed dial", "blank page"]
        .iter()
        .any(|needle| title.contains(needle))
}

#[derive(Debug, Clone)]
struct UrlSurface {
    domain: String,
    path: String,
    path_segments: Vec<String>,
    query_keys: Vec<String>,
}

impl UrlSurface {
    fn from_url(url: &str) -> Self {
        let lower_url = url.to_ascii_lowercase();
        let without_scheme = lower_url
            .split("://")
            .nth(1)
            .unwrap_or(lower_url.as_str())
            .split('#')
            .next()
            .unwrap_or_default();

        let path_and_query = without_scheme
            .split_once('/')
            .map(|(_, rest)| format!("/{rest}"))
            .unwrap_or_else(|| "/".to_string());
        let (path_raw, query_raw) = path_and_query
            .split_once('?')
            .unwrap_or((path_and_query.as_str(), ""));

        let path = if path_raw.is_empty() {
            "/".to_string()
        } else {
            path_raw.to_string()
        };
        let path_segments = path
            .split('/')
            .filter(|segment| !segment.is_empty())
            .map(ToString::to_string)
            .collect();
        let query_keys = query_raw
            .split('&')
            .filter_map(|entry| entry.split_once('=').map(|(key, _)| key.to_string()))
            .collect();

        Self {
            domain: extract_domain(url),
            path,
            path_segments,
            query_keys,
        }
    }
}

fn extract_domain(url: &str) -> String {
    let without_scheme = url.split("://").nth(1).unwrap_or(url);
    without_scheme
        .split('/')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}

fn contains_path_segment(surface: &UrlSurface, candidates: &[&str]) -> bool {
    surface
        .path_segments
        .iter()
        .any(|segment| candidates.iter().any(|candidate| segment == candidate))
}

fn contains_search_query_key(surface: &UrlSurface) -> bool {
    surface.query_keys.iter().any(|key| {
        matches!(
            key.as_str(),
            "q" | "query" | "search" | "search_query" | "text" | "term"
        )
    })
}

fn is_navigation_surface(surface: &UrlSurface) -> bool {
    if contains_search_query_key(surface)
        && (contains_path_segment(surface, &["search", "results"])
            || surface.path.contains("/search")
            || surface.path.contains("/results"))
    {
        return true;
    }

    contains_path_segment(
        surface,
        &["feed", "explore", "discover", "home", "trending", "hashtag"],
    )
}

fn is_listing_surface(surface: &UrlSurface, title: &str) -> bool {
    if title.contains("search results") || title.contains("videos -") {
        return true;
    }

    let primary_segment = surface
        .path_segments
        .first()
        .map(String::as_str)
        .unwrap_or("");
    if primary_segment.starts_with('@')
        || matches!(
            primary_segment,
            "u" | "user"
                | "users"
                | "profile"
                | "profiles"
                | "channel"
                | "channels"
                | "topic"
                | "topics"
                | "tag"
                | "tags"
        )
    {
        return true;
    }

    if surface.domain.ends_with("youtube.com") && primary_segment == "c" {
        return true;
    }

    let looks_like_collection = contains_path_segment(
        surface,
        &[
            "videos",
            "posts",
            "reels",
            "playlist",
            "playlists",
            "top",
            "best",
            "latest",
        ],
    );
    looks_like_collection && surface.path_segments.len() <= 3
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_outcome_names_the_rule_that_produced_it() {
        // Two different rules both mean SkipFrame; without the identity the
        // replay report can only say "admission dropped it", which is the v1
        // failure this ticket exists to prevent.
        assert_eq!(
            classify_capture_surface("Google Chrome", "New Tab", Some("https://example.com/")),
            (
                CaptureSurfacePolicy::SkipFrame,
                Some(AdmissionRule::GenericBrowserChrome)
            )
        );
        assert_eq!(
            classify_capture_surface(
                "Google Chrome",
                "Search results - YouTube",
                Some("https://www.youtube.com/results?search_query=screenpipe"),
            ),
            (
                CaptureSurfacePolicy::SkipFrame,
                Some(AdmissionRule::NavigationSurface)
            )
        );
        assert_eq!(
            classify_capture_surface(
                "Google Chrome",
                "screen_pipe - YouTube",
                Some("https://www.youtube.com/@screen_pipe/videos"),
            ),
            (
                CaptureSurfacePolicy::UrlOnly,
                Some(AdmissionRule::ListingSurface)
            )
        );
        assert_eq!(
            classify_capture_surface("Finder", "Project", None),
            (CaptureSurfacePolicy::Normal, None)
        );
    }

    #[test]
    fn skips_known_navigation_results_pages() {
        let policy = classify_capture_surface_policy(
            "Google Chrome",
            "Search results - YouTube",
            Some("https://www.youtube.com/results?search_query=screenpipe"),
        );
        assert_eq!(policy, CaptureSurfacePolicy::SkipFrame);
    }

    #[test]
    fn uses_url_only_for_channel_listing_pages() {
        let policy = classify_capture_surface_policy(
            "Google Chrome",
            "screen_pipe - YouTube",
            Some("https://www.youtube.com/@screen_pipe/videos"),
        );
        assert_eq!(policy, CaptureSurfacePolicy::UrlOnly);
    }

    #[test]
    fn allows_normal_article_capture() {
        let policy = classify_capture_surface_policy(
            "Google Chrome",
            "Screenpipe Architecture Deep Dive",
            Some("https://docs.screenpi.pe/architecture/memory-cards"),
        );
        assert_eq!(policy, CaptureSurfacePolicy::Normal);
    }

    #[test]
    fn does_not_apply_browser_policy_without_browser_metadata() {
        assert_eq!(
            classify_capture_surface_policy(
                "Finder",
                "Search results",
                Some("https://example.com/search?q=x")
            ),
            CaptureSurfacePolicy::Normal
        );
        assert_eq!(
            classify_capture_surface_policy("Google Chrome", "Anything", None),
            CaptureSurfacePolicy::Normal
        );
    }
}
