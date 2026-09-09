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
use fndr_ocr::{OcrEngine, RecognizedText};
use fndr_privacy::{
    Blocklist, SafetyContext, SafetyDecision, SafetyReason, evaluate, sanitize_url_for_storage,
};
use fndr_store::{Store, StoreError};
use fndr_textsignal::{AppIdentity, build_high_signal_text_for_app};

use crate::session_identity::{SessionContext, SessionIdentityDeriver};

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
pub struct VisionOcrAdapter {
    engine: OcrEngine,
}

impl VisionOcrAdapter {
    pub fn new(engine: OcrEngine) -> Self {
        Self { engine }
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
        Ok(normalize_recognized_text(
            AppIdentity::new(app_name, bundle_id),
            recognized,
            min_chars,
        ))
    }
}

fn normalize_recognized_text(
    app: AppIdentity<'_>,
    mut recognized: RecognizedText,
    min_chars: usize,
) -> OcrOutput {
    let high_signal = build_high_signal_text_for_app(app, &recognized.text);
    recognized.text = high_signal.text;

    OcrOutput {
        low_signal: recognized.is_low_signal(min_chars),
        text: recognized.text,
        confidence: recognized.confidence,
        block_count: recognized.block_count,
    }
}

/// The concrete SQLite sink for one scheduler lifetime.
///
/// Session identity is derived per capture from the ported continuity policy
/// (`fndr_memory::continuity`, resolved to local civil time by
/// `SessionIdentityDeriver`), not handed in by the owner. Record identity is
/// derived from that session plus the capture instant, so it is reproducible
/// and does not restart at zero when the process does.
pub struct StoreCaptureSink {
    store: Store,
    blocklist: Blocklist,
    identity: SessionIdentityDeriver,
}

impl StoreCaptureSink {
    pub fn new(store: Store, blocklist: Blocklist, identity: SessionIdentityDeriver) -> Self {
        Self {
            store,
            blocklist,
            identity,
        }
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    /// The scheduler is the single owner of this sink and borrows the store
    /// only while it performs a bounded Lance flush.
    pub fn store_mut(&mut self) -> &mut Store {
        &mut self.store
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
        // Invariant 4: an underivable session is a typed persistence failure
        // that the lifecycle publishes, never a made-up session id.
        let identity = self
            .identity
            .derive(SessionContext {
                app_name: &context.app_name,
                bundle_id: context.bundle_id.as_deref(),
                window_title: &context.window_title,
                url: context.url.as_deref(),
                captured_at_ms,
            })
            .map_err(|error| {
                PipelineError::new(
                    CaptureStage::Persistence,
                    format!("session identity unavailable: {error}"),
                )
            })?;
        let record_id = format!("{}-{captured_at_ms}", identity.session_id);
        let chunk_id = format!("{record_id}-0");
        match persist_capture(
            &mut self.store,
            CaptureForPersistence {
                record_id: &record_id,
                session_id: &identity.session_id,
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
            PersistCaptureOutcome::Stored { .. } | PersistCaptureOutcome::Merged { .. } => {
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

    use crate::session_identity::FixedOffsetClock;

    /// 2026-05-07T04:46:00Z, i.e. 21:46 at the -07:00 offset the sink tests
    /// pin, so every expected id below is a fixed, reviewable string.
    const BASE_MS: u64 = 1_778_129_160_000;

    fn context(app: &str, title: &str, url: Option<&str>) -> CaptureContext {
        CaptureContext {
            app_name: app.to_owned(),
            bundle_id: Some("com.example.app".to_owned()),
            window_title: title.to_owned(),
            url: url.map(str::to_owned),
            observed_at_ms: BASE_MS,
        }
    }

    fn sink() -> StoreCaptureSink {
        StoreCaptureSink::new(
            Store::open_in_memory().unwrap(),
            Blocklist::default(),
            SessionIdentityDeriver::new(FixedOffsetClock {
                offset_minutes: -7 * 60,
            }),
        )
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
            let output =
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
        let output = normalize_recognized_text(
            AppIdentity::from_name("Google Chrome"),
            recognized("[LOW_CONF] New Tab\nHome\nTrending\nNotifications\nExplore"),
            12,
        );

        assert!(output.low_signal);
        assert!(output.text.is_empty());
    }

    #[test]
    fn a_renamed_browser_is_cleaned_by_its_bundle_identity_not_its_name() {
        // The localized app name is user- and locale-controlled; the bundle
        // identifier is not. Before this wiring a renamed or non-English
        // Chrome classified as `Other`, so its tab-strip and nav labels were
        // held to the generic thresholds and reached durable memory.
        let raw = "Navegacion privada\nNew Tab\nTrending\nShip the durable capture cleanup slice";

        let by_bundle = normalize_recognized_text(
            AppIdentity::new("Navegador", Some("com.google.Chrome")),
            recognized(raw),
            12,
        );
        let by_name_only =
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
    }

    #[test]
    fn literal_confidence_token_in_captured_code_is_preserved() {
        let output = normalize_recognized_text(
            AppIdentity::from_name("Terminal"),
            recognized("let marker = \"[LOW_CONF]\";\nPersist literal tokens in captured code"),
            12,
        );

        assert!(!output.low_signal);
        assert!(output.text.contains("let marker = \"[LOW_CONF]\";"));
    }

    #[test]
    fn normalized_browser_evidence_is_what_durable_search_observes() {
        let output = normalize_recognized_text(
            AppIdentity::from_name("Google Chrome"),
            recognized(
                "[LOW_CONF] New Tab\n[LOW_CONF] Home\nImplement durable OCR evidence cleanup",
            ),
            12,
        );
        let mut sink = sink();
        let frame = Frame {
            png: vec![],
            captured_at_ms: BASE_MS,
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
            captured_at_ms: BASE_MS,
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
            sink.store()
                .capture_metadata("20260506-com.example.app-docs_example_com-s43-1778129160000")
                .unwrap(),
            Some(fndr_store::CaptureMetadata {
                bundle_id: Some("com.example.app".into()),
                url: Some("https://docs.example.com/fndr".into()),
            })
        );
    }

    /// Persist one capture whose evidence, title, and path are unrelated to
    /// every other capture in the test, so the continuity *merge* policy never
    /// fires and these tests observe identity alone.
    fn persist_distinct(sink: &mut StoreCaptureSink, title: &str, slug: &str, captured_at_ms: u64) {
        let context = context(
            "Safari",
            title,
            Some(&format!("https://docs.example.com/{slug}")),
        );
        let frame = Frame {
            png: vec![],
            captured_at_ms,
            perceptual_signature: None,
        };
        let ocr = OcrOutput {
            text: format!("{slug} evidence recorded without any shared vocabulary at all"),
            confidence: 0.9,
            block_count: 2,
            low_signal: false,
        };
        assert_eq!(
            sink.persist_capture(&context, &frame, &ocr),
            Ok(PersistenceOutcome::Stored)
        );
    }

    #[test]
    fn session_identity_is_stable_in_a_window_and_rolls_over_at_the_policy_boundary() {
        let mut sink = sink();
        let captures = [
            ("Alpha notes", "alpha", 0),
            ("Beta review", "beta", 3 * 60_000),
            ("Gamma plan", "gamma", 13 * 60_000),
            ("Delta summary", "delta", 14 * 60_000),
        ];
        for (title, slug, offset_ms) in captures {
            persist_distinct(&mut sink, title, slug, BASE_MS + offset_ms);
        }

        let sessions = sink
            .store()
            .pending_chunks(10)
            .unwrap()
            .into_iter()
            .map(|chunk| {
                chunk
                    .record_id
                    .rsplit_once('-')
                    .expect("record id carries its session prefix")
                    .0
                    .to_owned()
            })
            .collect::<Vec<_>>();

        assert_eq!(
            sessions,
            vec![
                "20260506-com.example.app-docs_example_com-s43".to_owned(),
                "20260506-com.example.app-docs_example_com-s43".to_owned(),
                "20260506-com.example.app-docs_example_com-s43".to_owned(),
                "20260506-com.example.app-docs_example_com-s44".to_owned(),
            ],
            "identity must be stable inside the ported 30-minute window and roll at its boundary"
        );
    }

    #[test]
    fn a_restarted_sink_rejoins_the_same_session_instead_of_restarting_at_zero() {
        // The replaced allocator numbered records from zero per process, so a
        // restart inside one window produced colliding record ids under a
        // different, process-derived session. Derived identity does neither.
        let mut first = sink();
        persist_distinct(&mut first, "Alpha notes", "alpha", BASE_MS);

        let mut restarted = StoreCaptureSink::new(
            first.store,
            Blocklist::default(),
            SessionIdentityDeriver::new(FixedOffsetClock {
                offset_minutes: -7 * 60,
            }),
        );
        persist_distinct(&mut restarted, "Beta review", "beta", BASE_MS + 60_000);

        let record_ids = restarted
            .store()
            .pending_chunks(10)
            .unwrap()
            .into_iter()
            .map(|chunk| chunk.record_id)
            .collect::<Vec<_>>();
        assert_eq!(
            record_ids,
            vec![
                "20260506-com.example.app-docs_example_com-s43-1778129160000".to_owned(),
                "20260506-com.example.app-docs_example_com-s43-1778129220000".to_owned(),
            ]
        );
    }

    #[test]
    fn an_underivable_session_fails_the_capture_instead_of_inventing_an_id() {
        struct NoClock;
        impl crate::session_identity::LocalClock for NoClock {
            fn local_civil_time(
                &self,
                _unix_ms: i64,
            ) -> Option<crate::session_identity::LocalCivilTime> {
                None
            }
        }

        let mut sink = StoreCaptureSink::new(
            Store::open_in_memory().unwrap(),
            Blocklist::default(),
            SessionIdentityDeriver::new(NoClock),
        );
        let frame = Frame {
            png: vec![],
            captured_at_ms: BASE_MS,
            perceptual_signature: None,
        };
        let ocr = OcrOutput {
            text: "evidence that must not reach a fabricated session".to_owned(),
            confidence: 0.9,
            block_count: 2,
            low_signal: false,
        };

        let error = sink
            .persist_capture(&context("Finder", "Project", None), &frame, &ocr)
            .expect_err("an underivable session is a typed failure, not a fallback id");
        assert_eq!(error.stage, CaptureStage::Persistence);
        assert!(error.message.contains("session identity unavailable"));
        assert!(sink.store().pending_chunks(10).unwrap().is_empty());
    }
}
