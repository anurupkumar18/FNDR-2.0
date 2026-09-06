//! Shell-owned adapters that join the capture pipeline to the real engine.
//!
//! The generic pipeline remains Tauri-free in `fndr-capture`; this module is
//! its composition boundary for the real privacy, Vision, and SQLite write
//! seams. The continuous worker that owns these adapters lands separately so
//! lifecycle and shutdown flush can be tested as one slice.

use fndr_capture::{
    CaptureContext, CaptureSink, CaptureStage, Frame, GateDecision, OcrOutput, OcrRecognizer,
    PersistenceOutcome, PipelineError, PreCaptureGate, SkipReason,
};
use fndr_memory::{CaptureForPersistence, PersistCaptureOutcome, persist_capture};
use fndr_ocr::OcrEngine;
use fndr_privacy::{Blocklist, SafetyContext, SafetyDecision, evaluate, sanitize_url_for_storage};
use fndr_store::{Store, StoreError};

/// The metadata-only safety check which runs before `FrameSource::grab`.
#[derive(Debug, Clone)]
pub struct PrivacyGate {
    blocklist: Blocklist,
}

impl PrivacyGate {
    pub fn new(blocklist: Blocklist) -> Self {
        Self { blocklist }
    }
}

impl PreCaptureGate for PrivacyGate {
    fn evaluate(&self, context: &CaptureContext) -> GateDecision {
        match evaluate(
            SafetyContext {
                app_name: Some(&context.app_name),
                bundle_id: context.bundle_id.as_deref(),
                url: context.url.as_deref(),
                window_title: Some(&context.window_title),
                ocr_text: None,
            },
            &self.blocklist,
        ) {
            SafetyDecision::SkipStorage(_) => GateDecision::Skip(SkipReason::PreCapturePrivacy),
            SafetyDecision::Allow | SafetyDecision::Redact(_) => GateDecision::Allow,
        }
    }
}

/// Converts the existing Vision result into the capture pipeline's normalized
/// output without copying its low-signal policy into another crate.
pub struct VisionOcrAdapter {
    engine: OcrEngine,
}

impl VisionOcrAdapter {
    pub fn new(engine: OcrEngine) -> Self {
        Self { engine }
    }
}

impl OcrRecognizer for VisionOcrAdapter {
    fn recognize(&self, png: &[u8], min_chars: usize) -> Result<OcrOutput, PipelineError> {
        let (recognized, _) = self
            .engine
            .recognize_with_metadata(png)
            .map_err(|error| PipelineError::new(CaptureStage::Ocr, error.to_string()))?;
        Ok(OcrOutput {
            low_signal: recognized.is_low_signal(min_chars),
            text: recognized.text,
            confidence: recognized.confidence,
            block_count: recognized.block_count,
        })
    }
}

/// The concrete SQLite sink for one scheduler lifetime.
///
/// `session_id` comes from the scheduler owner. Its temporary monotonically
/// numbered record IDs intentionally do not claim to implement T-307's
/// session-continuity policy; that ticket replaces this local allocator.
pub struct StoreCaptureSink {
    store: Store,
    blocklist: Blocklist,
    session_id: String,
    next_sequence: u64,
}

impl StoreCaptureSink {
    pub fn new(
        store: Store,
        blocklist: Blocklist,
        session_id: impl Into<String>,
    ) -> Result<Self, PipelineError> {
        let session_id = session_id.into();
        if session_id.trim().is_empty() {
            return Err(PipelineError::new(
                CaptureStage::Persistence,
                "capture sink requires a non-empty session id",
            ));
        }
        Ok(Self {
            store,
            blocklist,
            session_id,
            next_sequence: 0,
        })
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    /// The scheduler is the single owner of this sink and borrows the store
    /// only while it performs a bounded Lance flush.
    pub fn store_mut(&mut self) -> &mut Store {
        &mut self.store
    }

    fn ids(&mut self) -> (String, String) {
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.saturating_add(1);
        let record_id = format!("{}-{sequence}", self.session_id);
        let chunk_id = format!("{record_id}-0");
        (record_id, chunk_id)
    }

    fn persist(
        &mut self,
        source: &str,
        context: &CaptureContext,
        captured_at_ms: u64,
        text: &str,
    ) -> Result<PersistenceOutcome, PipelineError> {
        let captured_at_ms = i64::try_from(captured_at_ms).map_err(|_| {
            PipelineError::new(CaptureStage::Persistence, "capture timestamp exceeds i64")
        })?;
        let (record_id, chunk_id) = self.ids();
        match persist_capture(
            &mut self.store,
            CaptureForPersistence {
                record_id: &record_id,
                session_id: &self.session_id,
                chunk_id: &chunk_id,
                source,
                app_name: &context.app_name,
                bundle_id: context.bundle_id.as_deref(),
                url: context.url.as_deref(),
                window_title: &context.window_title,
                ocr_text: text,
                captured_at_ms,
                created_at_ms: captured_at_ms,
            },
            &self.blocklist,
        )
        .map_err(store_error)?
        {
            PersistCaptureOutcome::Stored { .. } => Ok(PersistenceOutcome::Stored),
            PersistCaptureOutcome::Skipped { .. } => Ok(PersistenceOutcome::SkippedFinalPrivacy),
        }
    }
}

impl CaptureSink for StoreCaptureSink {
    fn persist_capture(
        &mut self,
        context: &CaptureContext,
        frame: &Frame,
        ocr: &OcrOutput,
    ) -> Result<PersistenceOutcome, PipelineError> {
        self.persist("screen", context, frame.captured_at_ms, &ocr.text)
    }

    fn persist_url_only(
        &mut self,
        context: &CaptureContext,
    ) -> Result<PersistenceOutcome, PipelineError> {
        let safe_url = context
            .url
            .as_deref()
            .and_then(sanitize_url_for_storage)
            .ok_or_else(|| {
                PipelineError::new(
                    CaptureStage::Persistence,
                    "URL-only admission requires a sanitizable HTTP(S) URL",
                )
            })?;
        let text = format!("{}\n{}", context.window_title, safe_url.as_str());
        self.persist("browser_url_only", context, context.observed_at_ms, &text)
    }
}

fn store_error(error: StoreError) -> PipelineError {
    PipelineError::new(CaptureStage::Persistence, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use fndr_capture::CaptureSink;

    fn context(app: &str, title: &str, url: Option<&str>) -> CaptureContext {
        CaptureContext {
            app_name: app.to_owned(),
            bundle_id: Some("com.example.app".to_owned()),
            window_title: title.to_owned(),
            url: url.map(str::to_owned),
            observed_at_ms: 1_000,
        }
    }

    fn sink() -> StoreCaptureSink {
        StoreCaptureSink::new(
            Store::open_in_memory().unwrap(),
            Blocklist::default(),
            "session-a",
        )
        .unwrap()
    }

    #[test]
    fn pre_capture_gate_uses_the_real_sensitive_context_policy() {
        let gate = PrivacyGate::new(Blocklist::default());
        assert_eq!(
            gate.evaluate(&context("1Password", "Vault", None)),
            GateDecision::Skip(SkipReason::PreCapturePrivacy)
        );
        assert_eq!(
            gate.evaluate(&context("Finder", "Project", None)),
            GateDecision::Allow
        );
    }

    #[test]
    fn real_sink_persists_ocr_text_through_the_write_seam() {
        let mut sink = sink();
        let frame = Frame {
            png: vec![],
            captured_at_ms: 1_100,
            perceptual_signature: None,
        };
        let ocr = OcrOutput {
            text: "engineering notes".to_owned(),
            confidence: 0.9,
            block_count: 2,
            low_signal: false,
        };

        assert_eq!(
            sink.persist_capture(&context("Finder", "Project", None), &frame, &ocr),
            Ok(PersistenceOutcome::Stored)
        );
        let pending = sink.store().pending_chunks(10).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].text, "engineering notes");
    }

    #[test]
    fn url_only_sink_never_puts_query_or_fragment_in_the_chunk_or_metadata() {
        let mut sink = sink();
        assert_eq!(
            sink.persist_url_only(&context(
                "Safari",
                "FNDR docs",
                Some("https://docs.example.com/fndr?token=secret#private"),
            )),
            Ok(PersistenceOutcome::Stored)
        );

        let pending = sink.store().pending_chunks(10).unwrap();
        assert_eq!(pending[0].text, "FNDR docs\nhttps://docs.example.com/fndr");
        assert_eq!(
            sink.store().capture_metadata("session-a-0").unwrap(),
            Some(fndr_store::CaptureMetadata {
                bundle_id: Some("com.example.app".into()),
                url: Some("https://docs.example.com/fndr".into()),
            })
        );
    }
}
