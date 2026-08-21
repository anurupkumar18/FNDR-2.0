// Ported from FNDR v1 src-tauri/src/ocr/vision.rs (reference/v1) per ADR-005.
// Known follow-up (T-305): the noise filtering and preprocessing in here
// overlap fndr-textsignal responsibilities; split when T-305 finalizes.
//! Apple Vision OCR integration
//!
//! Uses VNRecognizeTextRequest for high-quality text extraction from screenshots.
//! Provides configurable filtering and intelligent text normalization.

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{class, msg_send, msg_send_id};
use objc2_foundation::{NSArray, NSData, NSDictionary, NSString};
use regex::Regex;

use std::ffi::c_void;
use std::sync::{Arc, OnceLock};

/// Errors that can occur during OCR operations
#[derive(Debug, thiserror::Error)]
pub enum OcrError {
    #[error("Vision framework initialization failed: {0}")]
    InitializationError(String),

    #[error("Image processing failed: {0}")]
    ImageProcessingError(String),

    #[error("Text recognition failed: {0}")]
    RecognitionError(String),

    #[error("Result extraction failed: {0}")]
    ExtractionError(String),
}

/// Recognition quality level
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecognitionLevel {
    /// Fast recognition, lower accuracy
    Fast = 0,
    /// Accurate recognition, slower
    Accurate = 1,
}

/// OCR configuration options
#[derive(Debug, Clone)]
pub struct OcrConfig {
    /// Recognition quality level
    pub recognition_level: RecognitionLevel,

    /// Enable automatic language correction
    pub language_correction: bool,

    /// Minimum confidence threshold (0.0 to 1.0)
    pub min_confidence: f32,

    /// Enable aggressive noise filtering
    pub aggressive_filtering: bool,

    /// Minimum line length to keep (characters)
    pub min_line_length: usize,

    /// Custom noise patterns to filter
    pub custom_noise_patterns: Vec<String>,

    /// Enable duplicate line removal
    pub remove_duplicates: bool,

    /// Preserve formatting (newlines, spacing)
    pub preserve_formatting: bool,

    /// Minimum text height as fraction of image height (0.0 to 1.0)
    /// Lower values (0.0) capture all text including tiny UI elements
    /// Higher values (0.02-0.05) filter small text, improving confidence scores
    pub minimum_text_height: f32,
}

impl Default for OcrConfig {
    fn default() -> Self {
        Self {
            recognition_level: RecognitionLevel::Accurate,
            language_correction: true,
            // Use 0.30 so we collect all Apple Vision confidence values before
            // per-line filtering, previously 0.50 biased the average upward.
            min_confidence: 0.30,
            aggressive_filtering: true,
            min_line_length: 7,
            custom_noise_patterns: Vec::new(),
            remove_duplicates: true,
            preserve_formatting: false,
            // Use 0.02 to filter tiny UI elements (< 36px on typical 1800px height).
            // This improves reported confidence (0.40-0.65 → 0.50-0.75 typical range)
            // by excluding text with inherently lower OCR confidence.
            // Trade-off: loses capture of very small status indicators/labels.
            minimum_text_height: 0.02,
        }
    }
}

impl OcrConfig {
    /// Create a fast configuration for real-time processing
    pub fn fast() -> Self {
        Self {
            recognition_level: RecognitionLevel::Fast,
            language_correction: false,
            min_confidence: 0.3,
            aggressive_filtering: false,
            min_line_length: 5,
            custom_noise_patterns: Vec::new(),
            remove_duplicates: false,
            preserve_formatting: true,
            minimum_text_height: 0.015,
        }
    }

    /// Create a high-quality configuration for document processing
    pub fn high_quality() -> Self {
        Self {
            recognition_level: RecognitionLevel::Accurate,
            language_correction: true,
            min_confidence: 0.7,
            aggressive_filtering: false,
            min_line_length: 1,
            custom_noise_patterns: Vec::new(),
            remove_duplicates: false,
            preserve_formatting: true,
            minimum_text_height: 0.01,
        }
    }
}

/// Aggregate stats from OCR preprocessing, stored in memory records.
/// Never contains raw OCR text or per-line text.
#[derive(Debug, Clone, Default)]
pub struct OcrAggregateStats {
    /// Average confidence across all lines (including low-conf)
    pub avg_confidence_all: f32,
    /// Average confidence across kept lines only
    pub avg_confidence_kept: f32,
    /// Lines used after preprocessing filters
    pub lines_used: usize,
    /// Lines dropped (confidence < 0.40)
    pub lines_dropped: usize,
    /// Lines tagged [LOW_CONF] (0.40 <= conf < 0.65)
    pub low_conf_count: usize,
}

/// Recognized text with metadata
#[derive(Debug, Clone)]
pub struct RecognizedText {
    /// The extracted text content (normalized, noise-filtered)
    pub text: String,

    /// Average confidence score (over all recognized lines, not just kept ones)
    pub confidence: f32,

    /// Number of text blocks recognized
    pub block_count: usize,

    /// Aggregate preprocessing stats (safe to store)
    pub ocr_stats: OcrAggregateStats,
}

impl RecognizedText {
    pub fn is_low_signal(&self, min_chars: usize) -> bool {
        let char_count = self.text.trim().len();
        if char_count < min_chars {
            return true;
        }
        // OCR engine failure or completely garbled output.
        if self.confidence < 0.15 {
            return true;
        }
        // Single block with truly low (non-screen-typical) confidence.
        if self.block_count <= 1 && self.confidence < 0.35 {
            return true;
        }
        // Volume-based override: text-heavy frames are not low signal
        // even when Apple Vision confidence is screen-typical (~0.5).
        !text_volume_qualifies(char_count, self.confidence, self.block_count)
    }
}

/// OCR Engine using Apple Vision framework
pub struct OcrEngine {
    config: Arc<OcrConfig>,
    noise_filter: NoiseFilter,
}

impl OcrEngine {
    /// Create a new OCR engine with default configuration
    pub fn new() -> Result<Self, OcrError> {
        Self::with_config(OcrConfig::default())
    }

    /// Create a new OCR engine with custom configuration
    pub fn with_config(config: OcrConfig) -> Result<Self, OcrError> {
        // Verify Vision framework is available
        // Verify Vision framework is available (logic simplified for objc2 safety)
        let _cls = class!(VNImageRequestHandler);

        let noise_filter = NoiseFilter::new(
            config.aggressive_filtering,
            config.custom_noise_patterns.clone(),
        );

        tracing::debug!("OCR engine initialized with config: {:?}", config);

        Ok(Self {
            config: Arc::new(config),
            noise_filter,
        })
    }

    /// Update the configuration
    pub fn update_config(&mut self, config: OcrConfig) {
        self.noise_filter = NoiseFilter::new(
            config.aggressive_filtering,
            config.custom_noise_patterns.clone(),
        );
        self.config = Arc::new(config);
    }

    /// Get the current configuration
    pub fn config(&self) -> &OcrConfig {
        &self.config
    }

    /// Recognize text from image data (PNG format)
    pub fn recognize(&self, image_data: &[u8]) -> Result<String, OcrError> {
        let (result, _) = self.recognize_with_metadata(image_data)?;
        Ok(result.text)
    }

    /// Recognize text with full metadata and transient Qwen cleaned text
    pub fn recognize_with_metadata(
        &self,
        image_data: &[u8],
    ) -> Result<(RecognizedText, String), OcrError> {
        unsafe {
            let ns_data =
                NSData::dataWithBytes_length(image_data.as_ptr() as *mut c_void, image_data.len());

            let handler = self.create_image_request_handler(&ns_data)?;
            let request = self.create_text_request()?;
            self.perform_request(&handler, &request)?;

            // Collect all per-line data transiently (never stored).
            let raw_lines = self.extract_raw_lines(&request)?;

            // Compute true average confidence from ALL lines.
            let avg_confidence_all = if raw_lines.is_empty() {
                0.0
            } else {
                raw_lines.iter().map(|(_, c)| c).sum::<f32>() / raw_lines.len() as f32
            };

            // Build normalized text and compute aggregate stats from all lines.
            let (cleaned_text, ocr_stats_from_all) = preprocess_ocr_for_qwen(&raw_lines);

            // For the text field, apply noise filter on top of the preprocessed output.
            let normalized = self.normalize_text(&cleaned_text);

            let block_count = ocr_stats_from_all.lines_used;

            // raw_lines is dropped here, not stored.
            Ok((
                RecognizedText {
                    text: normalized,
                    confidence: avg_confidence_all,
                    block_count,
                    ocr_stats: ocr_stats_from_all,
                },
                cleaned_text,
            ))
        }
    }

    unsafe fn create_image_request_handler(
        &self,
        data: &NSData,
    ) -> Result<Retained<AnyObject>, OcrError> {
        let cls = class!(VNImageRequestHandler);
        let options = NSDictionary::<AnyObject, AnyObject>::new();

        let handler = msg_send_id![cls, alloc];
        let handler: Retained<AnyObject> =
            msg_send_id![handler, initWithData:data options:&*options];

        Ok(handler)
    }

    unsafe fn create_text_request(&self) -> Result<Retained<AnyObject>, OcrError> {
        let cls = class!(VNRecognizeTextRequest);

        let request = msg_send_id![cls, alloc];
        let request: Retained<AnyObject> = msg_send_id![request, init];

        // Set recognition level
        let level = self.config.recognition_level as i64;
        let _: () = msg_send![&request, setRecognitionLevel: level];

        // Set language correction
        let _: () = msg_send![&request, setUsesLanguageCorrection: self.config.language_correction];

        // Set minimum text height (filters out very small text with low confidence)
        let _: () = msg_send![&request, setMinimumTextHeight: self.config.minimum_text_height];

        Ok(request)
    }

    unsafe fn perform_request(
        &self,
        handler: &AnyObject,
        request: &AnyObject,
    ) -> Result<(), OcrError> {
        // Edition 2024 explicit-unsafe blocks added during the port; logic unchanged.
        let request_retained =
            unsafe { Retained::retain(request as *const AnyObject as *mut AnyObject) }.ok_or_else(
                || OcrError::RecognitionError("Failed to retain request".to_string()),
            )?;

        let requests = NSArray::from_id_slice(&[request_retained]);

        let mut error: *mut AnyObject = std::ptr::null_mut();
        let success: bool = msg_send![handler, performRequests:&*requests error:&mut error];

        if !success {
            let error_msg = if !error.is_null() {
                let description: *const NSString = msg_send![error, localizedDescription];
                if !description.is_null() {
                    unsafe { (*description).to_string() }
                } else {
                    "Unknown error".to_string()
                }
            } else {
                "Request failed".to_string()
            };

            return Err(OcrError::RecognitionError(error_msg));
        }

        Ok(())
    }

    /// Extract per-line (text, confidence) pairs from Apple Vision results.
    /// Collects ALL lines (including low-confidence) so averages are accurate.
    unsafe fn extract_raw_lines(
        &self,
        request: &AnyObject,
    ) -> Result<Vec<(String, f32)>, OcrError> {
        let results: *const AnyObject = msg_send![request, results];
        if results.is_null() {
            return Ok(Vec::new());
        }

        let count: usize = msg_send![results, count];
        if count == 0 {
            return Ok(Vec::new());
        }

        let mut lines = Vec::with_capacity(count);

        for i in 0..count {
            let observation: *const AnyObject = msg_send![results, objectAtIndex: i];
            if observation.is_null() {
                continue;
            }

            let candidates: *const AnyObject = msg_send![observation, topCandidates: 1usize];
            if candidates.is_null() {
                continue;
            }

            let candidate_count: usize = msg_send![candidates, count];
            if candidate_count == 0 {
                continue;
            }

            let candidate: *const AnyObject = msg_send![candidates, objectAtIndex: 0usize];
            if candidate.is_null() {
                continue;
            }

            let confidence: f32 = msg_send![candidate, confidence];

            let ns_string: *const NSString = msg_send![candidate, string];
            if !ns_string.is_null() {
                let text = unsafe { (*ns_string).to_string() };
                if !text.trim().is_empty() {
                    lines.push((text, confidence));
                }
            }
        }

        Ok(lines)
    }

    /// Normalize OCR text according to configuration
    fn normalize_text(&self, text: &str) -> String {
        if self.config.preserve_formatting {
            return text.to_string();
        }

        let mut result = String::with_capacity(text.len());
        let mut last_line = String::new();
        let mut seen = std::collections::HashSet::new();
        let mut kept = 0usize;

        for line in text.lines() {
            let trimmed = line.trim();

            // Skip empty lines or too-short lines
            if trimmed.len() < self.config.min_line_length {
                continue;
            }

            // Apply noise filtering
            if self.noise_filter.is_noise(trimmed) {
                tracing::trace!("Filtered noise: {}", trimmed);
                continue;
            }

            // Skip duplicate consecutive lines
            if self.config.remove_duplicates && trimmed == last_line {
                continue;
            }
            if self.config.remove_duplicates && !seen.insert(trimmed.to_lowercase()) {
                continue;
            }

            if !result.is_empty() {
                result.push('\n');
            }
            result.push_str(trimmed);
            last_line = trimmed.to_string();
            kept += 1;
            if kept >= 220 {
                break;
            }
        }

        result
    }
}

impl Default for OcrEngine {
    fn default() -> Self {
        Self::new().expect("Failed to initialize OCR engine")
    }
}

/// Intelligent noise filter for OCR text
struct NoiseFilter {
    aggressive: bool,
    base_patterns: Vec<String>,
    custom_patterns: Vec<String>,
}

impl NoiseFilter {
    fn new(aggressive: bool, custom_patterns: Vec<String>) -> Self {
        let base_patterns = vec![
            // Common macOS UI elements
            "File Edit View Window Help".to_string(),
            "Apple Inc.".to_string(),
            "System Settings".to_string(),
            "System Preferences".to_string(),
            "Finder".to_string(),
            // App identifiers
            "com.apple.".to_string(),
            "com.google.".to_string(),
            "com.microsoft.".to_string(),
            // Time/date fragments (when alone)
            "AM".to_string(),
            "PM".to_string(),
            // Common metadata
            "Version".to_string(),
            "Build".to_string(),
            "Copyright ©".to_string(),
            "All Rights Reserved".to_string(),
            // Login/auth UI
            "Sign in".to_string(),
            "Sign up".to_string(),
            "Log in".to_string(),
            "Log out".to_string(),
            "Forgot password".to_string(),
            // Generic UI
            "Loading...".to_string(),
            "Please wait".to_string(),
            "OK".to_string(),
            "Cancel".to_string(),
        ];

        Self {
            aggressive,
            base_patterns,
            custom_patterns,
        }
    }

    fn is_noise(&self, text: &str) -> bool {
        // Check custom patterns first (user-defined)
        for pattern in &self.custom_patterns {
            if text.contains(pattern) {
                return true;
            }
        }

        // Check base patterns
        for pattern in &self.base_patterns {
            if text.contains(pattern) {
                return true;
            }
        }

        if !self.aggressive {
            return false;
        }

        // Aggressive filtering: additional heuristics

        // Filter lines with mostly special characters
        let alnum_count = text.chars().filter(|c| c.is_alphanumeric()).count();
        let total_count = text.chars().count();
        if total_count > 0 && (alnum_count as f32 / total_count as f32) < 0.5 {
            return true;
        }

        // Filter single words that are common UI elements
        let words: Vec<&str> = text.split_whitespace().collect();
        if words.len() == 1 {
            let word = words[0].to_lowercase();
            let ui_words = [
                "close",
                "minimize",
                "maximize",
                "menu",
                "settings",
                "preferences",
                "about",
                "help",
                "quit",
                "exit",
                "new",
                "open",
                "save",
                "print",
                "copy",
                "paste",
                "cut",
                "undo",
                "redo",
                "search",
                "find",
            ];

            if ui_words.contains(&word.as_str()) {
                return true;
            }
        }

        // Filter timestamp-like patterns (HH:MM)
        if time_regex().is_match(text) {
            return true;
        }

        // Filter percentage-only lines
        if text.trim().ends_with('%')
            && text
                .trim()
                .chars()
                .all(|c| c.is_numeric() || c == '%' || c == '.')
        {
            return true;
        }

        // Filter file size indicators (e.g., "123 KB")
        if size_regex().is_match(text.trim()) {
            return true;
        }

        // Filter common date patterns when alone
        let date_prefixes = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
        let month_names = [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ];

        for prefix in &date_prefixes {
            if text.starts_with(prefix) && text.len() < 20 {
                return true;
            }
        }

        for month in &month_names {
            if text.contains(month) && text.len() < 15 {
                return true;
            }
        }

        false
    }
}

fn time_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\d{1,2}:\d{2}(\s*(AM|PM))?$").expect("valid time regex"))
}

fn size_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\d+(\.\d+)?\s*(B|KB|MB|GB|TB)$").expect("valid size regex"))
}

/// Assess whether OCR evidence is worth storing based on text volume + confidence,
/// independently of Apple Vision's per-line confidence scores.
///
/// Screen OCR naturally produces confidence ~0.40–0.65 regardless of text quality
/// because Apple Vision's confidence model is calibrated for documents, not UI.
/// Do not use confidence alone to gate high-volume frames.
pub fn text_volume_qualifies(char_count: usize, ocr_confidence: f32, block_count: usize) -> bool {
    if char_count == 0 {
        return false;
    }
    // Hard floor: very short text is never worth storing.
    if char_count < 80 {
        return false;
    }
    // Long text with screen-typical confidence (>= 0.35) always qualifies.
    // Apple Vision returns 0.35–0.65 for most UI text; do not penalize this.
    if char_count >= 400 && ocr_confidence >= 0.35 {
        return true;
    }
    // Medium text: require block count as corroboration.
    if char_count >= 200 && block_count >= 10 && ocr_confidence >= 0.35 {
        return true;
    }
    // Short-medium text: require both higher confidence AND significant blocks.
    if char_count >= 80 && block_count >= 5 && ocr_confidence >= 0.55 {
        return true;
    }
    false
}

/// Preprocess per-line OCR data for structured Qwen3-VL extraction.
///
/// - Drops lines with confidence < 0.40 (unreliable, not worth sending to VLM)
/// - Prefixes lines with 0.40 <= conf < 0.65 with "[LOW_CONF]" so Qwen can weight them lower
/// - Applies basic normalization (de-duplicate consecutive whitespace)
/// - Returns aggregate-only stats (safe to persist); cleaned text is transient
pub fn preprocess_ocr_for_qwen(lines: &[(String, f32)]) -> (String, OcrAggregateStats) {
    const DROP_THRESHOLD: f32 = 0.40;
    const LOW_CONF_THRESHOLD: f32 = 0.65;

    // DEBUG: Log raw OCR lines before filtering
    let confidence_dist = lines.iter().map(|(_, c)| c).cloned().collect::<Vec<_>>();
    if !confidence_dist.is_empty() {
        let min_conf = confidence_dist
            .iter()
            .cloned()
            .fold(f32::INFINITY, f32::min);
        let max_conf = confidence_dist
            .iter()
            .cloned()
            .fold(f32::NEG_INFINITY, f32::max);
        tracing::debug!(
            "OCR preprocess_ocr_for_qwen: {} raw lines, conf range=[{:.3}, {:.3}], dropped_if_<{:.2}",
            lines.len(),
            min_conf,
            max_conf,
            DROP_THRESHOLD
        );
    }

    let mut output = Vec::with_capacity(lines.len());
    let mut lines_dropped = 0usize;
    let mut low_conf_count = 0usize;
    let mut kept_conf_sum = 0.0f32;
    let mut kept_count = 0usize;
    let all_conf_sum: f32 = lines.iter().map(|(_, c)| c).sum();

    // De-duplicate consecutive equal lines (OCR often repeats the same block)
    let mut prev_key: Option<String> = None;

    for (text, conf) in lines {
        if *conf < DROP_THRESHOLD {
            lines_dropped += 1;
            continue;
        }

        // Normalize whitespace inline
        let normalized: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if normalized.is_empty() {
            lines_dropped += 1;
            continue;
        }

        // Skip exact duplicates from consecutive lines
        let key = normalized.to_lowercase();
        if prev_key.as_deref() == Some(&key) {
            continue;
        }
        prev_key = Some(key);

        let line = if *conf < LOW_CONF_THRESHOLD {
            low_conf_count += 1;
            format!("[LOW_CONF] {}", normalized)
        } else {
            normalized
        };

        kept_conf_sum += conf;
        kept_count += 1;
        output.push(line);
    }

    let avg_confidence_all = if lines.is_empty() {
        0.0
    } else {
        all_conf_sum / lines.len() as f32
    };
    let avg_confidence_kept = if kept_count == 0 {
        0.0
    } else {
        kept_conf_sum / kept_count as f32
    };

    let stats = OcrAggregateStats {
        avg_confidence_all,
        avg_confidence_kept,
        lines_used: output.len(),
        lines_dropped,
        low_conf_count,
    };

    (output.join("\n"), stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_volume_qualifies_large_text_screen_typical_confidence() {
        // 1,459 chars, 38 blocks, confidence 0.49, real observed pattern, must qualify
        assert!(text_volume_qualifies(1459, 0.49, 38));
    }

    #[test]
    fn text_volume_qualifies_medium_text_with_blocks() {
        // 250 chars, 12 blocks, confidence 0.48, hits the medium-text branch (200+ chars, 10+ blocks)
        assert!(text_volume_qualifies(250, 0.48, 12));
    }

    #[test]
    fn text_volume_qualifies_short_medium_with_high_confidence() {
        // 150 chars, 6 blocks, confidence 0.60, hits the short-medium branch
        assert!(text_volume_qualifies(150, 0.60, 6));
    }

    #[test]
    fn text_volume_fails_short_low_confidence() {
        // 80 chars, 2 blocks, confidence 0.42, must NOT qualify
        assert!(!text_volume_qualifies(80, 0.42, 2));
    }

    #[test]
    fn text_volume_fails_tiny_text() {
        // 30 chars regardless of confidence, must NOT qualify
        assert!(!text_volume_qualifies(30, 0.80, 10));
    }

    #[test]
    fn text_volume_fails_zero_text() {
        assert!(!text_volume_qualifies(0, 0.90, 5));
    }

    #[test]
    fn is_low_signal_large_text_screen_confidence_not_low() {
        let rt = RecognizedText {
            text: "a".repeat(500),
            confidence: 0.49,
            block_count: 20,
            ocr_stats: OcrAggregateStats::default(),
        };
        assert!(!rt.is_low_signal(10));
    }

    #[test]
    fn is_low_signal_garbled_confidence_is_low() {
        let rt = RecognizedText {
            text: "a".repeat(500),
            confidence: 0.10, // catastrophically bad
            block_count: 20,
            ocr_stats: OcrAggregateStats::default(),
        };
        assert!(rt.is_low_signal(10));
    }

    #[test]
    fn test_noise_filter_basic() {
        let filter = NoiseFilter::new(true, vec![]);

        assert!(filter.is_noise("File Edit View Window Help"));
        assert!(filter.is_noise("com.apple.finder"));
        assert!(filter.is_noise("Sign in"));
        assert!(!filter.is_noise("This is actual content"));
    }

    #[test]
    fn test_noise_filter_aggressive() {
        let filter = NoiseFilter::new(true, vec![]);

        // Time patterns
        assert!(filter.is_noise("3:45 PM"));
        assert!(filter.is_noise("14:30"));

        // File sizes
        assert!(filter.is_noise("123 KB"));
        assert!(filter.is_noise("45.6 MB"));

        // Percentages
        assert!(filter.is_noise("95%"));

        // Single UI words
        assert!(filter.is_noise("Close"));
        assert!(filter.is_noise("Menu"));

        // Real content should pass
        assert!(!filter.is_noise("Implement new authentication system"));
    }

    #[test]
    fn test_custom_patterns() {
        let filter = NoiseFilter::new(false, vec!["FNDR".to_string(), "CustomApp".to_string()]);

        assert!(filter.is_noise("FNDR Dashboard"));
        assert!(filter.is_noise("CustomApp Settings"));
        assert!(!filter.is_noise("Regular text"));
    }

    #[test]
    fn test_config_presets() {
        let fast = OcrConfig::fast();
        assert_eq!(fast.recognition_level, RecognitionLevel::Fast);
        assert!(!fast.language_correction);
        assert_eq!(fast.minimum_text_height, 0.015);

        let hq = OcrConfig::high_quality();
        assert_eq!(hq.recognition_level, RecognitionLevel::Accurate);
        assert!(hq.language_correction);
        assert_eq!(hq.minimum_text_height, 0.01);
    }

    #[test]
    fn test_minimum_text_height_default() {
        let config = OcrConfig::default();
        assert_eq!(config.minimum_text_height, 0.02);
    }
}
