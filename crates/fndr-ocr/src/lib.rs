//! Vision OCR wrapper, async at the boundary.

mod vision;

pub use vision::{
    OcrAggregateStats, OcrConfig, OcrEngine, OcrError, RecognizedText, text_volume_qualifies,
};
