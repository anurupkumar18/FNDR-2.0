//! The capture seam: everything downstream consumes `FrameSource`, so the
//! pipeline is testable without a screen and the SCK provider (T-302) slots
//! in without touching consumers.

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// One captured frame, PNG-encoded.
#[derive(Debug, Clone)]
pub struct Frame {
    pub png: Vec<u8>,
    pub captured_at_ms: u64,
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
        })
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
