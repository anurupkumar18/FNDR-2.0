//! Perception heuristics ported from v1: OCR cleanup, line and span scoring, salience, noise estimation. Pure, no I/O.

mod cleanup;

pub use cleanup::{
    CaptureQualityStats, HighSignalText, SalientSpan, build_high_signal_text_for_app,
    compress_to_salient_evidence, concise_fallback_snippet, estimate_noise_score,
    looks_like_file_inventory, rank_salient_spans, reduce_chrome_noise,
    reduce_chrome_noise_for_app, salience_concentration, symbol_ratio,
};
