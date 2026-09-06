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

/// Classify a browser surface before it reaches the capture source.
// Ported from FNDR v1 src-tauri/src/capture/admission.rs at 330a760b.
pub fn classify_capture_surface_policy(
    app_name: &str,
    window_title: &str,
    url: Option<&str>,
) -> CaptureSurfacePolicy {
    if !is_browser_app(app_name) {
        return CaptureSurfacePolicy::Normal;
    }
    let Some(url) = url else {
        return CaptureSurfacePolicy::Normal;
    };

    let title = window_title.to_ascii_lowercase();
    if is_generic_browser_chrome_title(&title) {
        return CaptureSurfacePolicy::SkipFrame;
    }

    let surface = UrlSurface::from_url(url);
    if is_navigation_surface(&surface) {
        return CaptureSurfacePolicy::SkipFrame;
    }
    if is_listing_surface(&surface, &title) {
        return CaptureSurfacePolicy::UrlOnly;
    }
    CaptureSurfacePolicy::Normal
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
