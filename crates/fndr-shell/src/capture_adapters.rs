//! Shell-owned adapters that join the capture pipeline to the real engine.
//!
//! The generic pipeline remains Tauri-free in `fndr-capture`; this module is
//! its composition boundary for the real privacy, Vision, and SQLite write
//! seams. The continuous worker that owns these adapters lands separately so
//! lifecycle and shutdown flush can be tested as one slice.

use std::sync::{Arc, Mutex};

use fndr_capture::{
    CaptureContext, CaptureSink, CaptureStage, CleanupQuality, CleanupSignal, Frame, GateDecision,
    OcrOutput, OcrRecognizer, PersistenceOutcome, PipelineError, PreCaptureGate, RedactionQuality,
    SkipReason,
};
use fndr_memory::{CaptureForPersistence, PersistCaptureOutcome, persist_capture};
use fndr_ocr::{OcrEngine, RecognizedText};
use fndr_privacy::{
    Blocklist, SafetyContext, SafetyDecision, SafetyReason, evaluate, sanitize_url_for_storage,
};
use fndr_store::{Store, StoreError};
use fndr_textsignal::{AppIdentity, build_high_signal_text_for_app};

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
            SafetyDecision::SkipStorage(SafetyReason::PrivateBrowsing) => {
                GateDecision::Skip(SkipReason::PrivateBrowsing)
            }
            SafetyDecision::SkipStorage(_) => GateDecision::Skip(SkipReason::PreCapturePrivacy),
            SafetyDecision::Allow | SafetyDecision::Redact(_) => GateDecision::Allow,
        }
    }
}

/// Converts Vision output into app-aware, high-signal evidence before either
/// semantic deduplication or persistence can observe it. The cleanup itself is
/// the targeted v1 port owned by `fndr-textsignal`; this adapter only composes
/// that policy with the engine-owned low-signal decision.
///
/// `cleanup_health` accumulates a content-free line-drop signal from every
/// call (T-306 capture-health gap): counts and ratios only, never the
/// dropped or kept text itself. It is behind a `Mutex` because
/// `OcrRecognizer::recognize` takes `&self`; the capture pipeline drives one
/// tick at a time on its own dedicated thread, so contention is not a
/// concern. Call `cleanup_handle` before moving this adapter into the
/// pipeline to retain a queryable clone.
pub struct VisionOcrAdapter {
    engine: OcrEngine,
    cleanup_health: Arc<Mutex<CleanupQuality>>,
}

impl VisionOcrAdapter {
    pub fn new(engine: OcrEngine) -> Self {
        Self {
            engine,
            cleanup_health: Arc::new(Mutex::new(CleanupQuality::default())),
        }
    }

    /// A clone of the shared cleanup-quality handle, for a caller that wants
    /// to query it after this adapter has moved into the pipeline.
    pub fn cleanup_handle(&self) -> Arc<Mutex<CleanupQuality>> {
        Arc::clone(&self.cleanup_health)
    }

    /// The cleanup-quality snapshot observed so far. A poisoned lock (only
    /// possible after an unrelated panic on the capture thread) degrades to
    /// the last-known snapshot rather than propagating into the capture
    /// path; this is a diagnostic aggregate, not a decision input.
    pub fn cleanup_health(&self) -> CleanupQuality {
        self.cleanup_health
            .lock()
            .map(|guard| *guard)
            .unwrap_or_default()
    }
}

impl OcrRecognizer for VisionOcrAdapter {
    fn recognize(
        &self,
        png: &[u8],
        app_name: &str,
        bundle_id: Option<&str>,
        min_chars: usize,
    ) -> Result<OcrOutput, PipelineError> {
        let (recognized, _) = self
            .engine
            .recognize_with_metadata(png)
            .map_err(|error| PipelineError::new(CaptureStage::Ocr, error.to_string()))?;
        let (output, cleanup) = normalize_recognized_text(
            AppIdentity::new(app_name, bundle_id),
            recognized,
            min_chars,
        );
        if let Ok(mut health) = self.cleanup_health.lock() {
            health.record(cleanup);
        }
        Ok(output)
    }
}

fn normalize_recognized_text(
    app: AppIdentity<'_>,
    mut recognized: RecognizedText,
    min_chars: usize,
) -> (OcrOutput, CleanupSignal) {
    let high_signal = build_high_signal_text_for_app(app, &recognized.text);
    recognized.text = high_signal.text;
    let stats = high_signal.stats;

    let output = OcrOutput {
        low_signal: recognized.is_low_signal(min_chars),
        text: recognized.text,
        confidence: recognized.confidence,
        block_count: recognized.block_count,
    };
    let cleanup = CleanupSignal {
        total_lines: stats.total_lines,
        kept_lines: stats.kept_lines,
        dropped_noise_lines: stats.dropped_noise_lines,
        dropped_low_signal_lines: stats.dropped_low_signal_lines,
        low_conf_lines: stats.low_conf_lines,
    };
    (output, cleanup)
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
    redaction_health: RedactionQuality,
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
            redaction_health: RedactionQuality::default(),
        })
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    /// The redaction-rate snapshot observed so far (T-306 capture-health
    /// gap): counts and a ratio only, never the redacted text itself.
    pub fn redaction_health(&self) -> RedactionQuality {
        self.redaction_health
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
            PersistCaptureOutcome::Stored { redaction_count, .. }
            | PersistCaptureOutcome::Merged { redaction_count, .. } => {
                self.redaction_health.record(redaction_count);
                Ok(PersistenceOutcome::Stored)
            }
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
    use fndr_ocr::OcrAggregateStats;

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

    fn recognized(text: &str) -> RecognizedText {
        RecognizedText {
            text: text.to_owned(),
            confidence: 0.5,
            block_count: text.lines().count(),
            ocr_stats: OcrAggregateStats::default(),
        }
    }

    #[test]
    fn normalizes_everyday_app_evidence_before_the_pipeline() {
        let fixtures = [
            (
                "Google Chrome",
                "[LOW_CONF] New Tab\n[LOW_CONF] Home\nImplement robust OCR cleanup for capture pipeline",
                "Implement robust OCR cleanup",
                "New Tab",
            ),
            (
                "Terminal",
                "[LOW_CONF] cargo check\nsrc-tauri/src/capture/mod.rs\nfn persist_capture()",
                "src-tauri/src/capture/mod.rs",
                "[LOW_CONF]",
            ),
            (
                "Mail",
                "Inbox\nStarred\nSubject: Updated deployment plan\nPlease review the rollout risks before 4 PM.",
                "Subject: Updated deployment plan",
                "Starred",
            ),
            (
                "Mail",
                "[LOW_CONF] 请审查产品部署计划和风险\nRelease checklist is ready for review",
                "请审查产品部署计划和风险",
                "[LOW_CONF]",
            ),
        ];

        for (app_name, raw, expected, rejected) in fixtures {
            let (output, _cleanup) =
                normalize_recognized_text(AppIdentity::from_name(app_name), recognized(raw), 12);
            assert!(!output.low_signal, "{app_name} fixture should be admitted");
            assert!(
                output.text.contains(expected),
                "{app_name}: {}",
                output.text
            );
            assert!(
                !output.text.contains(rejected),
                "{app_name}: {}",
                output.text
            );
        }
    }

    #[test]
    fn chrome_only_evidence_becomes_an_observable_low_signal_skip() {
        let (output, cleanup) = normalize_recognized_text(
            AppIdentity::from_name("Google Chrome"),
            recognized("[LOW_CONF] New Tab\nHome\nTrending\nNotifications\nExplore"),
            12,
        );

        assert!(output.low_signal);
        assert!(output.text.is_empty());
        // Every line here is browser chrome, so the cleanup-quality signal
        // must show the drop, not silently report nothing observed.
        assert_eq!(cleanup.total_lines, 5);
        assert_eq!(cleanup.kept_lines, 0);
        assert_eq!(
            cleanup.dropped_noise_lines + cleanup.dropped_low_signal_lines,
            5
        );
    }

    #[test]
    fn a_renamed_browser_is_cleaned_by_its_bundle_identity_not_its_name() {
        // The localized app name is user- and locale-controlled; the bundle
        // identifier is not. Before this wiring a renamed or non-English
        // Chrome classified as `Other`, so its tab-strip and nav labels were
        // held to the generic thresholds and reached durable memory.
        let raw = "Navegacion privada\nNew Tab\nTrending\nShip the durable capture cleanup slice";

        let (by_bundle, bundle_cleanup) = normalize_recognized_text(
            AppIdentity::new("Navegador", Some("com.google.Chrome")),
            recognized(raw),
            12,
        );
        let (by_name_only, _) =
            normalize_recognized_text(AppIdentity::from_name("Navegador"), recognized(raw), 12);

        assert!(
            by_bundle
                .text
                .contains("Ship the durable capture cleanup slice")
        );
        assert!(
            !by_bundle.text.contains("Trending"),
            "browser nav label survived bundle-aware cleanup: {}",
            by_bundle.text
        );
        assert!(
            by_name_only.text.contains("Trending"),
            "the name alone should not have identified this browser, \
             so this fixture would not prove the bundle did the work: {}",
            by_name_only.text
        );
        // Bundle-aware cleanup dropped at least the "New Tab"/"Trending" nav
        // lines, so the cleanup-quality signal must reflect that drop.
        assert!(bundle_cleanup.dropped_noise_lines + bundle_cleanup.dropped_low_signal_lines >= 2);
    }

    #[test]
    fn literal_confidence_token_in_captured_code_is_preserved() {
        let (output, _cleanup) = normalize_recognized_text(
            AppIdentity::from_name("Terminal"),
            recognized("let marker = \"[LOW_CONF]\";\nPersist literal tokens in captured code"),
            12,
        );

        assert!(!output.low_signal);
        assert!(output.text.contains("let marker = \"[LOW_CONF]\";"));
    }

    #[test]
    fn normalized_browser_evidence_is_what_durable_search_observes() {
        let (output, _cleanup) = normalize_recognized_text(
            AppIdentity::from_name("Google Chrome"),
            recognized(
                "[LOW_CONF] New Tab\n[LOW_CONF] Home\nImplement durable OCR evidence cleanup",
            ),
            12,
        );
        let mut sink = sink();
        let frame = Frame {
            png: vec![],
            captured_at_ms: 1_100,
            perceptual_signature: None,
        };

        assert_eq!(
            sink.persist_capture(
                &context("Google Chrome", "FNDR work", None),
                &frame,
                &output,
            ),
            Ok(PersistenceOutcome::Stored)
        );
        assert_eq!(
            sink.store()
                .search_chunks("durable evidence cleanup", 10)
                .unwrap()
                .len(),
            1
        );
        assert!(
            sink.store()
                .search_chunks("New Tab", 10)
                .unwrap()
                .is_empty()
        );
        assert!(
            sink.store()
                .search_chunks("LOW_CONF", 10)
                .unwrap()
                .is_empty()
        );
        // This text carried no secret pattern, so the redaction-health
        // aggregate must count the persisted capture without counting a
        // redaction.
        assert_eq!(sink.redaction_health().persisted_captures(), 1);
        assert_eq!(sink.redaction_health().redacted_captures(), 0);
    }

    #[test]
    fn redaction_health_is_unmeasured_until_a_capture_persists() {
        let sink = sink();
        assert_eq!(
            sink.redaction_health().redaction_rate(),
            fndr_capture::QualityRate::Unmeasured
        );
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
        assert_eq!(
            gate.evaluate(&context("Google Chrome", "New Incognito Window", None)),
            GateDecision::Skip(SkipReason::PrivateBrowsing),
            "a private-browsing cue must be observable before pixels are captured"
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
