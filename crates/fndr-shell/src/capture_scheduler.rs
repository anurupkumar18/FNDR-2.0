//! The capture-owned scheduler: one tick through the staged pipeline plus
//! bounded, queue-disciplined flushing of SQLite truth into the Lance index.
//!
//! This code intentionally has no Tauri callback or async task of its own.
//! The shell starts it on its dedicated capture thread (never the async
//! runtime); the scheduler owns the synchronous capture stages and uses the
//! Tauri runtime only to wait for Lance's async API after a record is already
//! durable in SQLite. A flush failure remains visible and leaves that truth
//! pending for the next tick or shutdown flush.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use fndr_capture::{
    CaptureContextSource, CapturePipeline, CapturePipelineConfig, CaptureTickOutcome, FrameSource,
    MacOSForegroundContextSource, OcrRecognizer, PreCaptureGate, ScreenCaptureKitSource,
};
use fndr_inference::{
    CHUNK_EMBEDDING_V1, Embedder, GgufEmbedder, ModelWorkerHandle, Priority, QueuedEmbedder,
};
use fndr_ocr::{OcrEngine, OcrError};
use fndr_privacy::Blocklist;
use fndr_store::{FlushError, FlushReport, LanceWriter, Store, StoreError};

use crate::capture_adapters::{PrivacyGate, StoreCaptureSink, VisionOcrAdapter};

/// The architecture's minimum 30-second interval. A full pending batch may
/// request another flush sooner, but ordinary ticks never churn Lance commits.
pub const MIN_FLUSH_INTERVAL: Duration = Duration::from_secs(30);

/// Observable result of the durable-index portion of one capture tick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlushTickOutcome {
    NotDue,
    Flushed(FlushReport),
    Failed(String),
}

/// The two independent outcomes of a scheduler tick. A capture can be stored
/// even when indexing fails: SQLite truth is intentionally retained pending a
/// later retry rather than being rolled back or silently dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchedulerTickOutcome {
    pub capture: CaptureTickOutcome,
    pub flush: FlushTickOutcome,
}

/// The scheduler's one-owner composition. Capture dependencies stay generic
/// for deterministic stage tests; only the sink is fixed because flushing
/// must borrow the same SQLite connection that wrote the capture.
pub struct CaptureScheduler<C, F, G, O> {
    pipeline: CapturePipeline<C, F, G, O, StoreCaptureSink>,
    writer: LanceWriter,
    embedder: QueuedEmbedder,
    last_successful_flush_ms: u64,
    flush_interval_ms: u64,
}

impl<C, F, G, O> CaptureScheduler<C, F, G, O>
where
    C: CaptureContextSource,
    F: FrameSource,
    G: PreCaptureGate,
    O: OcrRecognizer,
{
    pub fn new(
        pipeline: CapturePipeline<C, F, G, O, StoreCaptureSink>,
        writer: LanceWriter,
        embedder: QueuedEmbedder,
        started_at_ms: u64,
        flush_interval: Duration,
    ) -> Result<Self, SchedulerStartError> {
        let flush_interval_ms = u64::try_from(flush_interval.as_millis()).map_err(|_| {
            SchedulerStartError::InvalidFlushInterval("interval does not fit in milliseconds")
        })?;
        if flush_interval_ms == 0 {
            return Err(SchedulerStartError::InvalidFlushInterval(
                "interval must be greater than zero",
            ));
        }
        Ok(Self {
            pipeline,
            writer,
            embedder,
            last_successful_flush_ms: started_at_ms,
            flush_interval_ms,
        })
    }

    pub fn counters(&self) -> &fndr_capture::CaptureCounters {
        self.pipeline.counters()
    }

    pub fn store(&self) -> &Store {
        self.pipeline.sink().store()
    }

    /// Run exactly one capture opportunity, then flush only when the bounded
    /// cadence is due. The caller supplies the monotonic wall-clock value so
    /// tests do not sleep and the worker can use one time source for status.
    pub fn tick(&mut self, now_ms: u64) -> SchedulerTickOutcome {
        let capture = self.pipeline.run_tick();
        let flush =
            if now_ms.saturating_sub(self.last_successful_flush_ms) >= self.flush_interval_ms {
                self.flush_once(now_ms)
            } else {
                FlushTickOutcome::NotDue
            };
        SchedulerTickOutcome { capture, flush }
    }

    /// Drain every pending SQLite batch before the capture owner stops. A
    /// failure is returned to the host as a typed error; it never clears the
    /// pending rows, so retry/rebuild remains possible after restart.
    pub fn flush_on_shutdown(&mut self, now_ms: u64) -> Result<usize, FlushError> {
        let mut written = 0;
        loop {
            let report = self.flush_report(now_ms)?;
            written += report.written;
            if report.written == 0 {
                self.last_successful_flush_ms = now_ms;
                return Ok(written);
            }
        }
    }

    fn flush_once(&mut self, now_ms: u64) -> FlushTickOutcome {
        match self.flush_report(now_ms) {
            Ok(report) => {
                self.last_successful_flush_ms = now_ms;
                FlushTickOutcome::Flushed(report)
            }
            Err(error) => FlushTickOutcome::Failed(error.to_string()),
        }
    }

    fn flush_report(&mut self, now_ms: u64) -> Result<FlushReport, FlushError> {
        let now_ms = i64::try_from(now_ms).unwrap_or(i64::MAX);
        tauri::async_runtime::block_on(self.writer.flush_once(
            self.pipeline.sink_mut().store_mut(),
            &self.embedder,
            now_ms,
        ))
    }
}

/// Explicit paths and policy needed to assemble the real macOS scheduler.
/// There is no implicit application-data location: the future Tauri lifecycle
/// owner chooses it after permissions/onboarding have made that location
/// visible to the user.
pub struct RealSchedulerConfig {
    pub database_path: PathBuf,
    pub index_dir: PathBuf,
    pub model_path: PathBuf,
    pub blocklist: Blocklist,
    pub session_id: String,
    pub display_index: usize,
    pub flush_interval: Duration,
    pub model_idle_timeout: Duration,
}

/// The concrete macOS composition. Keeping `model_worker` alongside the
/// scheduler ensures one long-lived queue serves every capture flush and the
/// model unloads once the owner is dropped.
pub struct RealCaptureScheduler {
    scheduler: CaptureScheduler<
        MacOSForegroundContextSource,
        ScreenCaptureKitSource,
        PrivacyGate,
        VisionOcrAdapter,
    >,
    _model_worker: Arc<ModelWorkerHandle>,
}

impl RealCaptureScheduler {
    pub fn open(
        config: RealSchedulerConfig,
        started_at_ms: u64,
    ) -> Result<Self, SchedulerStartError> {
        if config.flush_interval < MIN_FLUSH_INTERVAL {
            return Err(SchedulerStartError::InvalidFlushInterval(
                "flush interval must be at least 30 seconds",
            ));
        }
        if !config.model_path.is_file() {
            return Err(SchedulerStartError::ModelMissing(config.model_path));
        }

        let store = Store::open(&config.database_path)?;
        let sink = StoreCaptureSink::new(store, config.blocklist.clone(), config.session_id)
            .map_err(SchedulerStartError::pipeline)?;
        let pipeline = CapturePipeline::new(
            MacOSForegroundContextSource,
            ScreenCaptureKitSource {
                display_index: config.display_index,
            },
            PrivacyGate::new(config.blocklist),
            VisionOcrAdapter::new(OcrEngine::new()?),
            sink,
            CapturePipelineConfig::default(),
        );
        let model_path = config.model_path;
        let worker = Arc::new(ModelWorkerHandle::spawn(
            move || {
                Ok(
                    Box::new(GgufEmbedder::load(&model_path, CHUNK_EMBEDDING_V1.clone())?)
                        as Box<dyn Embedder>,
                )
            },
            config.model_idle_timeout,
        ));
        let queued = QueuedEmbedder::new(
            Arc::clone(&worker),
            Priority::Backfill,
            CHUNK_EMBEDDING_V1.clone(),
        );
        let scheduler = CaptureScheduler::new(
            pipeline,
            LanceWriter::new(&config.index_dir),
            queued,
            started_at_ms,
            config.flush_interval,
        )?;
        Ok(Self {
            scheduler,
            _model_worker: worker,
        })
    }

    pub fn tick(&mut self, now_ms: u64) -> SchedulerTickOutcome {
        self.scheduler.tick(now_ms)
    }

    pub fn counters(&self) -> &fndr_capture::CaptureCounters {
        self.scheduler.counters()
    }

    pub fn flush_on_shutdown(&mut self, now_ms: u64) -> Result<usize, FlushError> {
        self.scheduler.flush_on_shutdown(now_ms)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SchedulerStartError {
    #[error("capture scheduler requires a non-empty, readable GGUF model at {0}")]
    ModelMissing(PathBuf),
    #[error("invalid flush interval: {0}")]
    InvalidFlushInterval(&'static str),
    #[error("store: {0}")]
    Store(#[from] StoreError),
    #[error("OCR: {0}")]
    Ocr(#[from] OcrError),
    #[error("pipeline {stage:?}: {message}")]
    Pipeline {
        stage: fndr_capture::CaptureStage,
        message: String,
    },
}

impl SchedulerStartError {
    fn pipeline(error: fndr_capture::PipelineError) -> Self {
        Self::Pipeline {
            stage: error.stage,
            message: error.message,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::VecDeque;

    use fndr_capture::{
        CaptureContext, CaptureContextSource, Frame, GateDecision, OcrOutput, PerceptualSignature,
        PipelineError,
    };
    use fndr_inference::{EmbedError, Embedder, EmbeddingSpec};

    use super::*;

    const TEST_SPEC: EmbeddingSpec = EmbeddingSpec {
        model_id: "scheduler-test",
        dim: 3,
        lance_table: "scheduler_test_chunks",
    };

    struct Context;

    impl CaptureContextSource for Context {
        fn current_context(&self) -> Result<CaptureContext, PipelineError> {
            Ok(CaptureContext {
                app_name: "Finder".to_owned(),
                bundle_id: Some("com.apple.finder".to_owned()),
                window_title: "Scheduler test".to_owned(),
                url: None,
                observed_at_ms: 1_000,
            })
        }
    }

    struct Frames(RefCell<VecDeque<Frame>>);

    impl FrameSource for Frames {
        fn grab(&self) -> Result<Frame, fndr_capture::CaptureError> {
            self.0
                .borrow_mut()
                .pop_front()
                .ok_or(fndr_capture::CaptureError::Empty)
        }
    }

    struct Gate;

    impl PreCaptureGate for Gate {
        fn evaluate(&self, _context: &CaptureContext) -> GateDecision {
            GateDecision::Allow
        }
    }

    struct Ocr;

    impl OcrRecognizer for Ocr {
        fn recognize(&self, _png: &[u8], _min_chars: usize) -> Result<OcrOutput, PipelineError> {
            Ok(OcrOutput {
                text: "scheduler writes durable truth".to_owned(),
                confidence: 0.9,
                block_count: 2,
                low_signal: false,
            })
        }
    }

    struct TestEmbedder;

    impl Embedder for TestEmbedder {
        fn spec(&self) -> &EmbeddingSpec {
            &TEST_SPEC
        }

        fn embed_documents(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
            Ok(texts.iter().map(|_| vec![1.0, 0.0, 0.0]).collect())
        }
    }

    fn scheduler(flush_interval: Duration) -> CaptureScheduler<Context, Frames, Gate, Ocr> {
        let frame = Frame {
            png: vec![1],
            captured_at_ms: 1_000,
            perceptual_signature: Some(PerceptualSignature::from_downscaled_rgba([255; 9 * 8 * 4])),
        };
        let pipeline = CapturePipeline::new(
            Context,
            Frames(RefCell::new(VecDeque::from([frame]))),
            Gate,
            Ocr,
            StoreCaptureSink::new(
                Store::open_in_memory().unwrap(),
                Blocklist::default(),
                "test",
            )
            .unwrap(),
            CapturePipelineConfig::default(),
        );
        let worker = Arc::new(ModelWorkerHandle::spawn(
            || Ok(Box::new(TestEmbedder) as Box<dyn Embedder>),
            Duration::from_secs(1),
        ));
        let queued = QueuedEmbedder::new(worker, Priority::Backfill, TEST_SPEC.clone());
        // A nanosecond timestamp alone is not a reliable uniqueness source:
        // tests in this file run on separate threads within one process, and
        // two threads can observe the same clock reading, colliding on the
        // same Lance table directory (`TableAlreadyExists`, T-306 flake).
        static NEXT_TEST_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let index_dir = std::env::temp_dir().join(format!(
            "fndr-scheduler-test-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT_TEST_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        CaptureScheduler::new(
            pipeline,
            LanceWriter::new(&index_dir),
            queued,
            1_000,
            flush_interval,
        )
        .unwrap()
    }

    #[test]
    fn due_tick_persists_then_flushes_through_the_model_queue() {
        let mut scheduler = scheduler(Duration::from_millis(1));
        let result = scheduler.tick(1_001);

        assert_eq!(result.capture, CaptureTickOutcome::Stored);
        assert_eq!(
            result.flush,
            FlushTickOutcome::Flushed(FlushReport {
                written: 1,
                batch_was_full: false,
            })
        );
        assert!(scheduler.store().pending_chunks(10).unwrap().is_empty());
    }

    #[test]
    fn shutdown_flush_drains_records_that_are_not_yet_due() {
        let mut scheduler = scheduler(Duration::from_secs(60));
        assert_eq!(scheduler.tick(1_001).capture, CaptureTickOutcome::Stored);
        assert_eq!(scheduler.store().pending_chunks(10).unwrap().len(), 1);

        assert_eq!(scheduler.flush_on_shutdown(1_002).unwrap(), 1);
        assert!(scheduler.store().pending_chunks(10).unwrap().is_empty());
    }
}
