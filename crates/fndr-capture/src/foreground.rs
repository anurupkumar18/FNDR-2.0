//! macOS foreground metadata for the capture scheduler.
//!
//! This provider deliberately fails closed. A browser tick without a current
//! title and URL cannot establish that it is not an incognito, authentication,
//! financial, medical, or owner-blocked surface, so it must not reach pixel
//! capture. The AppleScript calls may require macOS Automation/Accessibility
//! approval when the scheduler is first started; that is an explicit platform
//! state, not a reason to capture with incomplete metadata.

use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use objc2_app_kit::NSWorkspace;

use crate::{CaptureContext, CaptureContextSource, CaptureStage, PipelineError};

/// Reads frontmost-app metadata from AppKit and, for supported browsers, the
/// active tab title and URL from the browser's own AppleScript dictionary.
#[derive(Debug, Default, Clone, Copy)]
pub struct MacOSForegroundContextSource;

impl CaptureContextSource for MacOSForegroundContextSource {
    fn current_context(&self) -> Result<CaptureContext, PipelineError> {
        let workspace = NSWorkspace::sharedWorkspace();
        let app = workspace
            .frontmostApplication()
            .ok_or_else(|| metadata_error("AppKit did not report a frontmost application"))?;
        let app_name = app
            .localizedName()
            .map(|name| name.to_string())
            .filter(|name| !name.trim().is_empty())
            .ok_or_else(|| metadata_error("frontmost application has no localized name"))?;
        let bundle_id = app
            .bundleIdentifier()
            .map(|identifier| identifier.to_string());

        let (window_title, url) = match browser_kind(&app_name, bundle_id.as_deref()) {
            Some(browser) => browser_metadata(browser)?,
            None => (front_window_title()?, None),
        };

        Ok(CaptureContext {
            app_name,
            bundle_id,
            window_title,
            url,
            observed_at_ms: now_ms(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Browser {
    Safari,
    Chrome,
    Arc,
    Brave,
    Edge,
    Unsupported,
}

fn browser_kind(app_name: &str, bundle_id: Option<&str>) -> Option<Browser> {
    let app = app_name.to_ascii_lowercase();
    let bundle = bundle_id.unwrap_or_default().to_ascii_lowercase();
    let identifies = |needle: &str| app.contains(needle) || bundle.contains(needle);

    if identifies("safari") {
        Some(Browser::Safari)
    } else if identifies("chrome") {
        Some(Browser::Chrome)
    } else if identifies("arc") {
        Some(Browser::Arc)
    } else if identifies("brave") {
        Some(Browser::Brave)
    } else if identifies("edge") {
        Some(Browser::Edge)
    } else if identifies("firefox") || identifies("opera") {
        Some(Browser::Unsupported)
    } else {
        None
    }
}

fn browser_metadata(browser: Browser) -> Result<(String, Option<String>), PipelineError> {
    let script = match browser {
        Browser::Safari => {
            r#"tell application "Safari"
                set frontTab to current tab of front window
                return (name of frontTab) & (ASCII character 31) & (URL of frontTab)
            end tell"#
        }
        Browser::Chrome => {
            r#"tell application "Google Chrome"
                set frontTab to active tab of front window
                return (title of frontTab) & (ASCII character 31) & (URL of frontTab)
            end tell"#
        }
        Browser::Arc => {
            r#"tell application "Arc"
                set frontTab to active tab of front window
                return (title of frontTab) & (ASCII character 31) & (URL of frontTab)
            end tell"#
        }
        Browser::Brave => {
            r#"tell application "Brave Browser"
                set frontTab to active tab of front window
                return (title of frontTab) & (ASCII character 31) & (URL of frontTab)
            end tell"#
        }
        Browser::Edge => {
            r#"tell application "Microsoft Edge"
                set frontTab to active tab of front window
                return (title of frontTab) & (ASCII character 31) & (URL of frontTab)
            end tell"#
        }
        Browser::Unsupported => {
            return Err(metadata_error(
                "frontmost browser does not expose a supported URL metadata interface",
            ));
        }
    };

    let output = run_osascript(script)?;
    let (title, url) = output.split_once('\u{1f}').ok_or_else(|| {
        metadata_error("browser metadata response did not contain a title and URL")
    })?;
    let title = title.trim();
    let url = url.trim();
    if title.is_empty() || !is_http_url(url) {
        return Err(metadata_error(
            "browser metadata response omitted a non-empty title or HTTP(S) URL",
        ));
    }
    Ok((title.to_owned(), Some(url.to_owned())))
}

fn front_window_title() -> Result<String, PipelineError> {
    let output = run_osascript(
        r#"tell application "System Events"
            tell (first process whose frontmost is true)
                if (count of windows) > 0 then
                    return name of front window
                end if
            end tell
        end tell"#,
    )?;
    let title = output.trim();
    if title.is_empty() {
        return Err(metadata_error(
            "frontmost application has no readable window title",
        ));
    }
    Ok(title.to_owned())
}

fn run_osascript(script: &str) -> Result<String, PipelineError> {
    let output = Command::new("/usr/bin/osascript")
        .args(["-e", script])
        .output()
        .map_err(|error| metadata_error(format!("could not invoke osascript: {error}")))?;
    if !output.status.success() {
        return Err(metadata_error(format!(
            "osascript denied or failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn is_http_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    let authority = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"));
    authority.is_some_and(|authority| {
        authority
            .split(['/', '?', '#'])
            .next()
            .is_some_and(|host| !host.is_empty())
    })
}

fn metadata_error(message: impl Into<String>) -> PipelineError {
    PipelineError::new(CaptureStage::Metadata, message)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_supported_browsers_by_app_or_bundle_identity() {
        assert_eq!(browser_kind("Google Chrome", None), Some(Browser::Chrome));
        assert_eq!(
            browser_kind("Chromium", Some("com.microsoft.edgemac")),
            Some(Browser::Edge)
        );
        assert_eq!(browser_kind("Finder", Some("com.apple.finder")), None);
    }

    #[test]
    fn unsupported_browsers_are_explicitly_not_treated_as_generic_apps() {
        assert_eq!(browser_kind("Firefox", None), Some(Browser::Unsupported));
        assert_eq!(browser_kind("Opera", None), Some(Browser::Unsupported));
    }

    #[test]
    fn only_http_urls_are_safe_to_admit_as_browser_metadata() {
        assert!(is_http_url("https://docs.example.com/path"));
        assert!(is_http_url("HTTP://example.com"));
        assert!(!is_http_url("https://"));
        assert!(!is_http_url("about:blank"));
        assert!(!is_http_url("file:///private/tmp/note.html"));
    }
}
