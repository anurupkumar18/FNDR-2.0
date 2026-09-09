//! Shell-owned adapters that join the capture pipeline to the real engine.
//!
//! The generic pipeline remains Tauri-free in `fndr-capture`; this module is
//! its composition boundary for the real privacy, Vision, and SQLite write
//! seams. The continuous worker that owns these adapters lands separately so
//! lifecycle and shutdown flush can be tested as one slice.

use fndr_capture::{
    CaptureContext, CaptureSink, CaptureStage, Frame, GateDecision, OcrOutput, OcrRecognizer,
    PersistenceOutcome, PipelineError, PreCaptureGate, SkipReason, TextCleanupOutcome,
};
use fndr_memory::{CaptureForPersistence, PersistCaptureOutcome, persist_capture};
use fndr_ocr::{OcrEngine, RecognizedText};
use fndr_privacy::{
    Blocklist, SafetyContext, SafetyDecision, SafetyReason, evaluate, sanitize_url_for_storage,
};
use fndr_store::{Store, StoreError};
use fndr_textsignal::build_high_signal_text_for_app;

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

/// Converts the existing Vision result into the capture pipeline's normalized
/// output without copying its low-signal policy into another crate.
///
/// This is where `fndr-textsignal` joins the live capture path. Vision's raw
/// text carries the internal `[LOW_CONF] ` per-line marker from
/// `preprocess_ocr_for_qwen` plus whatever browser chrome was on screen;
/// neither belongs in storage or in front of the owner.
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
        context: &CaptureContext,
        png: &[u8],
        min_chars: usize,
    ) -> Result<OcrOutput, PipelineError> {
        let (recognized, _) = self
            .engine
            .recognize_with_metadata(png)
            .map_err(|error| PipelineError::new(CaptureStage::Ocr, error.to_string()))?;
        Ok(clean_for_storage(&context.app_name, recognized, min_chars))
    }
}

/// The cleanup half of the OCR boundary, split out so it is testable without
/// Vision, a real screenshot, or the loop that drives them.
///
/// Ordering decision: the quality gate runs on **cleaned** text, not raw text.
///
/// `min_ocr_chars` exists to answer "is there enough here to be worth keeping",
/// and the only text that gets kept is the cleaned text. Judging the raw text
/// would gate on characters that are then deleted: with Apple Vision returning
/// 0.50 for essentially every legible line, nearly every line arrives with an
/// 11-character `[LOW_CONF] ` prefix, so a raw-text gate is padded by roughly
/// 11 chars per line by a marker that never reaches disk.
///
/// The consequence is bounded, and the bound is why this ordering is safe.
/// `RecognizedText::is_low_signal` has exactly one rejection rule that reads
/// the text: `char_count < min_chars`. Its other two rules read `confidence`
/// and `block_count`, which are Vision's own numbers and are unaffected by
/// cleanup, and `text_volume_qualifies` is admit-only, so losing it can never
/// by itself reject a frame. So the only frames cleaning can newly reject are
/// those whose *cleaned* text is under `min_ocr_chars`, which is precisely the
/// set that would otherwise have been stored as near-empty junk.
///
/// `confidence` and `block_count` stay as Vision reported them. They describe
/// what the recognizer saw in the image, not what survived our line filter;
/// recomputing `block_count` from kept lines would double-penalize a chrome
/// heavy browser frame (fewer characters *and* fewer blocks) and would drift
/// from the field's documented meaning.
fn clean_for_storage(app_name: &str, recognized: RecognizedText, min_chars: usize) -> OcrOutput {
    let had_raw_text = !recognized.text.trim().is_empty();
    let cleaned = build_high_signal_text_for_app(app_name, &recognized.text);

    let cleanup = match (had_raw_text, cleaned.text.trim().is_empty()) {
        (false, _) => TextCleanupOutcome::NothingToClean,
        (true, true) => TextCleanupOutcome::RemovedAllText,
        (true, false) => TextCleanupOutcome::Cleaned,
    };

    // Reuse the engine-owned rule rather than reinterpreting OCR quality here;
    // only the text it judges changes.
    let judged = RecognizedText {
        text: cleaned.text,
        ..recognized
    };

    OcrOutput {
        low_signal: judged.is_low_signal(min_chars),
        text: judged.text,
        confidence: judged.confidence,
        block_count: judged.block_count,
        cleanup,
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
    use fndr_ocr::{OcrAggregateStats, preprocess_ocr_for_qwen};

    /// Build the exact string Vision hands the adapter for a screen whose lines
    /// OCR at `confidence`. Going through `preprocess_ocr_for_qwen` rather than
    /// hand-writing `[LOW_CONF] ` keeps these fixtures honest: the marker
    /// convention lives in one place and the fixtures follow it.
    fn vision_text(lines: &[&str], confidence: f32) -> String {
        let lines: Vec<(String, f32)> = lines
            .iter()
            .map(|line| ((*line).to_owned(), confidence))
            .collect();
        preprocess_ocr_for_qwen(&lines).0
    }

    fn recognized(text: String, confidence: f32, block_count: usize) -> RecognizedText {
        RecognizedText {
            text,
            confidence,
            block_count,
            ocr_stats: OcrAggregateStats::default(),
        }
    }

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
            cleanup: TextCleanupOutcome::Cleaned,
        };

        assert_eq!(
            sink.persist_capture(&context("Finder", "Project", None), &frame, &ocr),
            Ok(PersistenceOutcome::Stored)
        );
        let pending = sink.store().pending_chunks(10).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].text, "engineering notes");
    }

    /// The regression that matters to the owner: whatever reaches the store,
    /// and therefore the search UI, must never contain the internal
    /// `[LOW_CONF]` marker. This runs the adapter's cleanup seam into the real
    /// SQLite write path, so it fails if either half is unwired.
    #[test]
    fn stored_text_never_contains_the_low_conf_marker() {
        // Every line at 0.50: the measured Apple Vision confidence for legible
        // screen text, so every line arrives marked.
        let raw = vision_text(
            &[
                "Implement robust OCR cleanup for the capture pipeline",
                "The adapter must strip internal markers before storage",
                "Follow up with the calibration numbers in the PR body",
            ],
            0.50,
        );
        assert!(
            raw.contains("[LOW_CONF]"),
            "fixture must reproduce the real Vision output, got: {raw}"
        );

        let ocr = clean_for_storage("Terminal", recognized(raw, 0.50, 3), 12);
        assert_eq!(ocr.cleanup, TextCleanupOutcome::Cleaned);
        assert!(!ocr.low_signal);

        let mut sink = sink();
        let frame = Frame {
            png: vec![],
            captured_at_ms: 1_100,
            perceptual_signature: None,
        };
        assert_eq!(
            sink.persist_capture(&context("Terminal", "zsh", None), &frame, &ocr),
            Ok(PersistenceOutcome::Stored)
        );

        let pending = sink.store().pending_chunks(10).unwrap();
        assert_eq!(pending.len(), 1);
        assert!(
            !pending[0].text.contains("[LOW_CONF]"),
            "stored text leaked the internal low-confidence marker: {}",
            pending[0].text
        );
        assert!(pending[0].text.contains("Implement robust OCR cleanup"));
    }

    #[test]
    fn browser_chrome_is_reduced_before_storage() {
        let raw = vision_text(
            &[
                "New Tab",
                "Home",
                "Gmail · Calendar · Drive · GitHub",
                "Series A memo: the retrieval latency budget is 200ms end to end",
            ],
            0.50,
        );

        let ocr = clean_for_storage("Google Chrome", recognized(raw, 0.50, 4), 12);

        assert!(ocr.text.contains("Series A memo"));
        assert!(!ocr.text.contains("[LOW_CONF]"));
        assert!(!ocr.text.to_lowercase().contains("new tab"));
        assert!(!ocr.text.contains("Calendar"));
        assert!(!ocr.low_signal);
    }

    #[test]
    fn a_frame_cleanup_empties_is_a_named_state_not_a_raw_passthrough() {
        // Chrome-only browser frame. The contract forbids falling back to the
        // raw text, and forbids reporting it as an ordinary low-signal read.
        let raw = vision_text(
            &["New Tab", "Home", "Trending", "Notifications", "Explore"],
            0.50,
        );

        let ocr = clean_for_storage("Google Chrome", recognized(raw, 0.50, 5), 12);

        assert_eq!(ocr.cleanup, TextCleanupOutcome::RemovedAllText);
        assert!(ocr.low_signal);
        assert!(ocr.text.trim().is_empty());
        assert!(!ocr.text.contains("Trending"));
    }

    #[test]
    fn an_empty_vision_read_is_distinguished_from_cleanup_removing_everything() {
        let ocr = clean_for_storage("Finder", recognized(String::new(), 0.0, 0), 12);
        assert_eq!(ocr.cleanup, TextCleanupOutcome::NothingToClean);
        assert!(ocr.low_signal);
    }

    #[test]
    fn vision_confidence_and_block_count_survive_cleanup_unchanged() {
        // They describe what Vision saw in the image, not what our line filter
        // kept. Recomputing them here would double-penalize noisy frames.
        let raw = vision_text(
            &[
                "New Tab",
                "Quarterly planning notes for the retrieval milestone",
            ],
            0.50,
        );
        let ocr = clean_for_storage("Google Chrome", recognized(raw, 0.50, 9), 12);
        assert_eq!(ocr.confidence, 0.50);
        assert_eq!(ocr.block_count, 9);
    }

    /// Calibration measurement for the raw-versus-cleaned gate ordering.
    ///
    /// Pins the two facts the ordering decision rests on: cleaning costs about
    /// 11 characters per line to marker removal alone, and on realistic content
    /// the cleaned text still clears `min_ocr_chars` comfortably. Run with
    /// `cargo test -p fndr-shell -- --nocapture` to see the table.
    #[test]
    fn cleaning_before_the_gate_does_not_starve_realistic_captures() {
        const MIN_OCR_CHARS: usize = 12;

        struct Case {
            app: &'static str,
            label: &'static str,
            lines: &'static [&'static str],
        }

        let cases = [
            Case {
                app: "Terminal",
                label: "git status, clean tree",
                lines: &[
                    "On branch main",
                    "Your branch is up to date with 'origin/main'.",
                    "nothing to commit, working tree clean",
                ],
            },
            Case {
                app: "Cursor",
                label: "code diff hunk",
                lines: &[
                    "crates/fndr-shell/src/capture_adapters.rs",
                    "let cleaned = build_high_signal_text_for_app(app_name, &raw);",
                    "low_signal: judged.is_low_signal(min_chars),",
                ],
            },
            Case {
                app: "Google Chrome",
                label: "article with browser chrome",
                lines: &[
                    "New Tab",
                    "Home",
                    "Gmail · Calendar · Drive · GitHub",
                    "Local-first search keeps the index on the device that made it",
                    "The tradeoff is that every ranking change has to be measured locally",
                ],
            },
            Case {
                app: "Mail",
                label: "email with sidebar nav",
                lines: &[
                    "Inbox",
                    "Starred",
                    "Subject: Updated deployment plan",
                    "Please review the rollout risks before 4 PM.",
                ],
            },
            Case {
                app: "Notes",
                label: "four short note lines",
                lines: &[
                    "Ship the cleanup wiring",
                    "Measure the gate delta",
                    "Update the ticket ledger",
                    "Write the lesson entry",
                ],
            },
        ];

        println!(
            "\n{:<34} {:<16} {:>5} {:>7} {:>7} {:>6}  gate",
            "capture", "app", "lines", "raw", "cleaned", "delta"
        );
        for case in cases {
            let raw = vision_text(case.lines, 0.50);
            let raw_len = raw.trim().len();
            let block_count = case.lines.len();

            let raw_gate_low = recognized(raw.clone(), 0.50, block_count).is_low_signal(12);
            let ocr =
                clean_for_storage(case.app, recognized(raw, 0.50, block_count), MIN_OCR_CHARS);
            let cleaned_len = ocr.text.trim().len();

            println!(
                "{:<34} {:<16} {:>5} {:>7} {:>7} {:>6}  raw_gate_low={} cleaned_gate_low={}",
                case.label,
                case.app,
                case.lines.len(),
                raw_len,
                cleaned_len,
                cleaned_len as i64 - raw_len as i64,
                raw_gate_low,
                ocr.low_signal,
            );

            assert!(
                !ocr.low_signal,
                "cleaning pushed a legitimate capture below min_ocr_chars: {} ({} -> {} chars)",
                case.label, raw_len, cleaned_len
            );
            assert!(!ocr.text.contains("[LOW_CONF]"));
        }

        // The marker cost, isolated: same content, marked and unmarked.
        let lines = [
            "Ship the cleanup wiring",
            "Measure the gate delta",
            "Update the ticket ledger",
            "Write the lesson entry",
        ];
        let marked = vision_text(&lines, 0.50).trim().len();
        let unmarked = vision_text(&lines, 0.90).trim().len();
        println!(
            "\nmarker cost only: {marked} marked - {unmarked} unmarked = {} chars over {} lines ({} per line)\n",
            marked - unmarked,
            lines.len(),
            (marked - unmarked) / lines.len(),
        );
        assert_eq!(
            (marked - unmarked) / lines.len(),
            "[LOW_CONF] ".len(),
            "the marker cost per line is the thing the raw-text gate was counting"
        );
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
