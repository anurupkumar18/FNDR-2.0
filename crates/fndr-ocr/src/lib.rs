//! Vision OCR wrapper, async at the boundary.

mod vision;

pub use vision::{
    OcrAggregateStats, OcrConfig, OcrEngine, OcrError, RecognizedText, preprocess_ocr_for_qwen,
    text_volume_qualifies,
};
