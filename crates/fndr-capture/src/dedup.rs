//! Perceptual and semantic deduplication seams for the capture scheduler.
//!
//! Perceptual signatures are constructed from a 9 by 8 native-pixel sample,
//! never by decoding a full-resolution PNG. Semantic signatures suppress
//! unchanged OCR bursts for a bounded in-memory window.

use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};

use image::{DynamicImage, RgbaImage};
use img_hash::{HasherConfig, ImageHash};

const SAMPLE_WIDTH: usize = 9;
const SAMPLE_HEIGHT: usize = 8;
const SAMPLE_PIXEL_BYTES: usize = 4;
const HASH_HISTORY: usize = 3;
const MAX_RGB_DISTANCE: u32 = 24;
const LOOP_MAX_RGB_DISTANCE: u32 = 20;

/// A dHash plus a coarse colour guard, generated from a downscaled raster.
#[derive(Debug, Clone)]
pub struct PerceptualSignature {
    hash: ImageHash,
    average_rgb: [u8; 3],
}

impl PerceptualSignature {
    /// Construct a signature from a 9 by 8 RGBA raster.
    // Ported from FNDR v1 src-tauri/src/capture/dedupe.rs at 330a760b.
    pub fn from_downscaled_rgba(
        rgba: [u8; SAMPLE_WIDTH * SAMPLE_HEIGHT * SAMPLE_PIXEL_BYTES],
    ) -> Self {
        let average_rgb = average_rgb(&rgba);
        let image = RgbaImage::from_raw(SAMPLE_WIDTH as u32, SAMPLE_HEIGHT as u32, rgba.to_vec())
            .expect("fixed 9 by 8 RGBA input has the expected byte length");
        let hasher = HasherConfig::new().hash_size(8, 8).to_hasher();
        Self {
            hash: hasher.hash_image(&DynamicImage::ImageRgba8(image)),
            average_rgb,
        }
    }

    /// Sample a native RGBA frame into the compact raster used for dHash.
    ///
    /// The caller provides uncompressed pixels from the capture provider. This
    /// function allocates only the 288-byte sample and deliberately accepts no
    /// PNG input, so the scheduler cannot accidentally add full-PNG decoding to
    /// its hot path.
    pub fn from_native_rgba(width: usize, height: usize, rgba: &[u8]) -> Option<Self> {
        let source_len = width.checked_mul(height)?.checked_mul(SAMPLE_PIXEL_BYTES)?;
        if width < SAMPLE_WIDTH || height < SAMPLE_HEIGHT || rgba.len() < source_len {
            return None;
        }

        let mut sample = [0_u8; SAMPLE_WIDTH * SAMPLE_HEIGHT * SAMPLE_PIXEL_BYTES];
        for sample_y in 0..SAMPLE_HEIGHT {
            let source_y = sample_y * (height - 1) / (SAMPLE_HEIGHT - 1);
            for sample_x in 0..SAMPLE_WIDTH {
                let source_x = sample_x * (width - 1) / (SAMPLE_WIDTH - 1);
                let source_offset = (source_y * width + source_x) * SAMPLE_PIXEL_BYTES;
                let sample_offset = (sample_y * SAMPLE_WIDTH + sample_x) * SAMPLE_PIXEL_BYTES;
                sample[sample_offset..sample_offset + SAMPLE_PIXEL_BYTES]
                    .copy_from_slice(&rgba[source_offset..source_offset + SAMPLE_PIXEL_BYTES]);
            }
        }
        Some(Self::from_downscaled_rgba(sample))
    }

    fn distance_to(&self, other: &Self) -> u32 {
        self.hash.dist(&other.hash)
    }

    fn colour_distance_to(&self, other: &Self) -> u32 {
        rgb_distance(self.average_rgb, other.average_rgb)
    }
}

/// Stateful perceptual deduplication, including v1's A-B-A loop detection.
pub struct PerceptualDeduper {
    threshold: u32,
    last: Option<PerceptualSignature>,
    recent: VecDeque<PerceptualSignature>,
}

impl PerceptualDeduper {
    /// `threshold` is the dHash distance below which frames are duplicates.
    pub fn new(threshold: u32) -> Self {
        Self {
            threshold,
            last: None,
            recent: VecDeque::with_capacity(HASH_HISTORY),
        }
    }

    /// Observe one capture and return whether it is perceptually redundant.
    pub fn should_skip(&mut self, signature: PerceptualSignature) -> bool {
        let mut duplicate = self.last.as_ref().is_some_and(|last| {
            signature.distance_to(last) < self.threshold
                && signature.colour_distance_to(last) <= MAX_RGB_DISTANCE
        });

        if !duplicate {
            let loop_threshold = self.threshold.saturating_sub(1).max(1);
            duplicate = self.recent.iter().any(|previous| {
                signature.distance_to(previous) < loop_threshold
                    && signature.colour_distance_to(previous) <= LOOP_MAX_RGB_DISTANCE
            });
        }

        self.last = Some(signature.clone());
        self.recent.push_back(signature);
        if self.recent.len() > HASH_HISTORY {
            self.recent.pop_front();
        }
        duplicate
    }
}

impl Default for PerceptualDeduper {
    fn default() -> Self {
        Self::new(5)
    }
}

/// The in-memory time window that suppresses repeated OCR content bursts.
#[derive(Default)]
pub struct SemanticDedupWindow {
    seen_at_ms: HashMap<u64, u64>,
}

impl SemanticDedupWindow {
    /// Return true if this signature was seen within `window_ms`.
    // Ported from FNDR v1 src-tauri/src/capture/mod.rs at 330a760b.
    pub fn should_skip(&mut self, signature: u64, now_ms: u64, window_ms: u64) -> bool {
        self.seen_at_ms
            .retain(|_, seen_at| now_ms.saturating_sub(*seen_at) <= window_ms);

        if let Some(last_seen) = self.seen_at_ms.get(&signature).copied()
            && now_ms.saturating_sub(last_seen) <= window_ms
        {
            self.seen_at_ms.insert(signature, now_ms);
            return true;
        }

        self.seen_at_ms.insert(signature, now_ms);
        false
    }
}

/// Hash the metadata and cleaned OCR text used by the semantic window.
pub fn semantic_signature(app_name: &str, window_title: &str, clean_text: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    app_name.hash(&mut hasher);
    window_title.hash(&mut hasher);
    clean_text.hash(&mut hasher);
    hasher.finish()
}

fn average_rgb(rgba: &[u8]) -> [u8; 3] {
    let mut sums = [0_u64; 3];
    for pixel in rgba.chunks_exact(SAMPLE_PIXEL_BYTES) {
        for (index, value) in pixel[..3].iter().enumerate() {
            sums[index] += u64::from(*value);
        }
    }
    let pixel_count = (rgba.len() / SAMPLE_PIXEL_BYTES) as u64;
    [
        (sums[0] / pixel_count) as u8,
        (sums[1] / pixel_count) as u8,
        (sums[2] / pixel_count) as u8,
    ]
}

fn rgb_distance(left: [u8; 3], right: [u8; 3]) -> u32 {
    left.iter()
        .zip(right)
        .map(|(left, right)| (*left as i32 - right as i32).unsigned_abs())
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid_signature(rgb: [u8; 3]) -> PerceptualSignature {
        let mut rgba = [0_u8; SAMPLE_WIDTH * SAMPLE_HEIGHT * SAMPLE_PIXEL_BYTES];
        for pixel in rgba.chunks_exact_mut(SAMPLE_PIXEL_BYTES) {
            pixel[..3].copy_from_slice(&rgb);
            pixel[3] = u8::MAX;
        }
        PerceptualSignature::from_downscaled_rgba(rgba)
    }

    #[test]
    fn identical_frames_are_duplicates() {
        let mut deduper = PerceptualDeduper::default();
        assert!(!deduper.should_skip(solid_signature([0, 0, 0])));
        assert!(deduper.should_skip(solid_signature([0, 0, 0])));
    }

    #[test]
    fn colour_guard_keeps_different_flat_fields() {
        let mut deduper = PerceptualDeduper::new(1);
        assert!(!deduper.should_skip(solid_signature([0, 0, 0])));
        assert!(!deduper.should_skip(solid_signature([255, 0, 0])));
    }

    #[test]
    fn detects_short_a_b_a_visual_loops() {
        let mut deduper = PerceptualDeduper::new(5);
        assert!(!deduper.should_skip(solid_signature([0, 0, 0])));
        assert!(!deduper.should_skip(solid_signature([255, 0, 0])));
        assert!(deduper.should_skip(solid_signature([0, 0, 0])));
    }

    #[test]
    fn native_sampling_never_requires_png_input() {
        let mut native = vec![0_u8; 18 * 16 * SAMPLE_PIXEL_BYTES];
        native[0] = 255;
        assert!(PerceptualSignature::from_native_rgba(18, 16, &native).is_some());
        assert!(PerceptualSignature::from_native_rgba(8, 8, &native).is_none());
    }

    #[test]
    fn semantic_window_resets_after_expiry() {
        let mut window = SemanticDedupWindow::default();
        let signature = semantic_signature("Chrome", "FNDR docs", "capture pipeline");
        assert!(!window.should_skip(signature, 1_000, 500));
        assert!(window.should_skip(signature, 1_400, 500));
        assert!(!window.should_skip(signature, 2_000, 500));
    }
}
