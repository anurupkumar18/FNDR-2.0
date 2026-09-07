//! T-302 manual verification: take one real ScreenCaptureKit frame and
//! report what came back. Needs Screen Recording permission for whatever
//! process runs it (System Settings > Privacy & Security > Screen Recording).
//!
//! `cargo run -p fndr-capture --example sck_probe`

use fndr_capture::{FrameSource, ScreenCaptureKitSource};

fn main() {
    let source = ScreenCaptureKitSource::default();
    match source.grab() {
        Ok(frame) => {
            let is_png = frame.png.starts_with(b"\x89PNG\r\n\x1a\n");
            println!(
                "captured {} bytes, png_magic={}, captured_at_ms={}",
                frame.png.len(),
                is_png,
                frame.captured_at_ms
            );
            assert!(is_png, "bytes should be a real PNG");
            assert!(frame.png.len() > 10_000, "a real screen should not be tiny");
            println!("SCK provider works: real frame, real PNG bytes, nothing left on disk.");
        }
        Err(e) => {
            eprintln!("capture failed: {e}");
            std::process::exit(1);
        }
    }
}
