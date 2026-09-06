//! The capture seam: everything downstream consumes `FrameSource`, so the
//! pipeline is testable without a screen and the SCK provider (T-302) slots
//! in without touching consumers.

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::PerceptualSignature;

/// One captured frame and, when supplied by the native source, its compact
/// perceptual signature. The signature is not persisted.
#[derive(Debug, Clone)]
pub struct Frame {
    pub png: Vec<u8>,
    pub captured_at_ms: u64,
    pub perceptual_signature: Option<PerceptualSignature>,
}

/// Typed capture failures. Never silently skipped: callers surface these
/// (invariant 4, no silent degradation).
#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("screen recording permission missing or capture tool failed: {0}")]
    PermissionOrTool(String),

    #[error("frame source I/O failed: {0}")]
    Io(#[from] std::io::Error),

    #[error("capture produced no image data")]
    Empty,

    #[error("could not derive the frame's native perceptual signature: {0}")]
    PerceptualSignature(String),
}

pub trait FrameSource {
    fn grab(&self) -> Result<Frame, CaptureError>;
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Reads a PNG from disk. The test and demo source: deterministic input for
/// the pipeline without any permission involved.
pub struct PngFileSource {
    pub path: PathBuf,
}

impl FrameSource for PngFileSource {
    fn grab(&self) -> Result<Frame, CaptureError> {
        let png = std::fs::read(&self.path)?;
        if png.is_empty() {
            return Err(CaptureError::Empty);
        }
        Ok(Frame {
            png,
            captured_at_ms: now_ms(),
            perceptual_signature: None,
        })
    }
}

/// Shells out to /usr/sbin/screencapture for a one-shot full-screen grab.
/// Walking-skeleton only: the interactive TCC grant lands on the invoking
/// terminal, which is enough for the week-3 end-to-end proof. Replaced by the
/// SCK provider in T-302.
pub struct ScreencaptureCliSource;

impl FrameSource for ScreencaptureCliSource {
    fn grab(&self) -> Result<Frame, CaptureError> {
        let tmp = std::env::temp_dir().join(format!("fndr-skeleton-{}.png", std::process::id()));
        let output = Command::new("/usr/sbin/screencapture")
            .args(["-x", "-t", "png"])
            .arg(&tmp)
            .output()?;
        if !output.status.success() {
            return Err(CaptureError::PermissionOrTool(
                String::from_utf8_lossy(&output.stderr).trim().to_string(),
            ));
        }
        let png = std::fs::read(&tmp)?;
        let _ = std::fs::remove_file(&tmp);
        if png.is_empty() {
            // screencapture exits 0 but writes nothing when the permission
            // prompt was dismissed; make that a loud typed state.
            return Err(CaptureError::PermissionOrTool(
                "screencapture wrote an empty file (screen recording permission?)".to_string(),
            ));
        }
        Ok(Frame {
            png,
            captured_at_ms: now_ms(),
            perceptual_signature: None,
        })
    }
}

/// The real ScreenCaptureKit provider (T-302), replacing the
/// `screencapture(1)` shellout. Uses one-shot `SCScreenshotManager`
/// captures rather than a persistent `SCStream`: ADR-001 action item 4
/// prefers this for FNDR's ~0.5 FPS model, and it sidesteps the upstream
/// crate's leak/stalled-callback issue history, which is concentrated in
/// the long-lived stream path.
///
/// Each `grab()` is fully self-contained (enumerate content, filter to a
/// display, capture, encode), so there is no background stream to
/// supervise, leak, or stall. That costs some per-frame setup, which is
/// the right trade at half a frame per second.
#[derive(Debug, Clone, Copy, Default)]
pub struct ScreenCaptureKitSource {
    /// Which display to capture, by index into the shareable-content list.
    /// 0 (the `Default`) is the primary display. Multi-display selection
    /// policy belongs to the capture scheduler (T-306), not to this seam.
    pub display_index: usize,
}

impl FrameSource for ScreenCaptureKitSource {
    fn grab(&self) -> Result<Frame, CaptureError> {
        use screencapturekit::screenshot_manager::{CGImageExt, ImageFormat, SCScreenshotManager};
        use screencapturekit::shareable_content::SCShareableContent;
        use screencapturekit::stream::configuration::SCStreamConfiguration;
        use screencapturekit::stream::content_filter::SCContentFilter;

        // Missing screen-recording permission surfaces here, as a typed
        // error rather than an empty frame (invariant 4).
        let content = SCShareableContent::get().map_err(|e| {
            CaptureError::PermissionOrTool(format!(
                "ScreenCaptureKit could not list shareable content \
                 (screen recording permission?): {e}"
            ))
        })?;
        let displays = content.displays();
        let display = displays.get(self.display_index).ok_or_else(|| {
            CaptureError::PermissionOrTool(format!(
                "display index {} not present ({} display(s) available)",
                self.display_index,
                displays.len()
            ))
        })?;

        let filter = SCContentFilter::create()
            .with_display(display)
            .with_excluding_windows(&[])
            .build();
        let mut config = SCStreamConfiguration::new();
        config
            .set_width(display.width())
            .set_height(display.height());

        let image = SCScreenshotManager::capture_image(&filter, &config)
            .map_err(|e| CaptureError::PermissionOrTool(format!("SCScreenshotManager: {e}")))?;

        // The dHash input comes from ScreenCaptureKit's native raster before
        // PNG encoding. It samples only a 9 by 8 grid and never decodes a
        // full-resolution PNG on the capture hot path (T-303).
        let native_rgba = image
            .rgba_data()
            .map_err(|e| CaptureError::PerceptualSignature(e.to_string()))?;
        let perceptual_signature =
            PerceptualSignature::from_native_rgba(image.width(), image.height(), &native_rgba)
                .ok_or_else(|| {
                    CaptureError::PerceptualSignature(
                        "invalid ScreenCaptureKit RGBA raster".to_owned(),
                    )
                })?;

        // The crate encodes to a file, not to memory, so this round-trips
        // through a temp path exactly as the `screencapture(1)` source did.
        // `TempPng` removes the file on every exit path, including errors,
        // so a raw frame never outlives this call (ADR-004: no raw
        // screenshot persistence).
        let tmp = TempPng::new();
        let tmp_str = tmp
            .path
            .to_str()
            .ok_or_else(|| CaptureError::PermissionOrTool("non-UTF-8 temp path".to_owned()))?;
        image
            .save(tmp_str, ImageFormat::Png)
            .map_err(|e| CaptureError::PermissionOrTool(format!("PNG encode: {e}")))?;
        let png = std::fs::read(&tmp.path)?;
        if png.is_empty() {
            return Err(CaptureError::Empty);
        }
        Ok(Frame {
            png,
            captured_at_ms: now_ms(),
            perceptual_signature: Some(perceptual_signature),
        })
    }
}

/// A temp PNG path that deletes itself on drop, so a captured frame's
/// bytes never survive a `grab()` call — including on the error paths.
struct TempPng {
    path: PathBuf,
}

impl TempPng {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("fndr-sck-{}-{}.png", std::process::id(), now_ms()));
        Self { path }
    }
}

impl Drop for TempPng {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_source_reads_png() {
        let path = std::env::temp_dir().join(format!("fndr-src-test-{}.png", std::process::id()));
        std::fs::write(&path, b"\x89PNG fake bytes").unwrap();
        let frame = PngFileSource { path: path.clone() }.grab().unwrap();
        assert!(!frame.png.is_empty());
        assert!(frame.captured_at_ms > 0);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn file_source_missing_file_is_typed_error() {
        let source = PngFileSource {
            path: PathBuf::from("/nonexistent/fndr-nope.png"),
        };
        assert!(matches!(source.grab(), Err(CaptureError::Io(_))));
    }

    #[test]
    fn file_source_empty_file_is_typed_error() {
        let path = std::env::temp_dir().join(format!("fndr-empty-test-{}.png", std::process::id()));
        std::fs::write(&path, b"").unwrap();
        let source = PngFileSource { path: path.clone() };
        assert!(matches!(source.grab(), Err(CaptureError::Empty)));
        let _ = std::fs::remove_file(&path);
    }
}
