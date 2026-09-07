//! App-lifecycle ownership for the real capture worker (T-901).
//!
//! The capture thread owns ScreenCaptureKit, Vision, SQLite, and the model
//! queue. This module owns only lifecycle: it publishes content-free status,
//! keeps the worker alive while the app is alive, and performs the mandatory
//! drain before the app exits. It deliberately does not duplicate a capture
//! loop or expose raw capture data to an event consumer.

use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{SystemTime, UNIX_EPOCH};

use fndr_capture::{CaptureTickOutcome, SkipReason};
use fndr_types::{
    CaptureFlushState, CaptureRuntimeState, CaptureRuntimeStatus, CaptureTickState,
    CaptureTickStatus,
};

use crate::capture_scheduler::FlushTickOutcome;
use crate::capture_worker::{
    CaptureWorkerControlError, CaptureWorkerEvent, CaptureWorkerHandle, CaptureWorkerReport,
    CaptureWorkerStartError, CaptureWorkerStopError, RealCaptureWorkerConfig,
    start_real_capture_worker,
};

/// Consumers receive only the generated IPC status type. A Tauri adapter can
/// emit it, while tests can collect it without needing a desktop runtime.
pub trait CaptureStatusSink: Send + Sync + 'static {
    fn publish(&self, status: CaptureRuntimeStatus);
}

impl<F> CaptureStatusSink for F
where
    F: Fn(CaptureRuntimeStatus) + Send + Sync + 'static,
{
    fn publish(&self, status: CaptureRuntimeStatus) {
        self(status);
    }
}

/// The app-owned capture worker. `shutdown` is explicit and blocking because
/// it is the only way to guarantee SQLite-to-Lance drain before process exit.
pub struct CaptureLifecycle {
    worker: Mutex<Option<CaptureWorkerHandle>>,
    dispatcher: Mutex<Option<JoinHandle<()>>>,
    status: Arc<Mutex<CaptureRuntimeStatus>>,
    sink: Arc<dyn CaptureStatusSink>,
}

/// The Tauri event name for the generated `CaptureRuntimeStatus` payload.
pub const CAPTURE_STATUS_EVENT: &str = "capture://status";

#[derive(Debug, thiserror::Error)]
pub enum CaptureLifecycleStartError {
    #[error("capture worker could not start: {0}")]
    Worker(#[from] CaptureWorkerStartError),
}

#[derive(Debug, thiserror::Error)]
pub enum CaptureLifecycleStopError {
    #[error("capture lifecycle is already stopped")]
    AlreadyStopped,
    #[error("capture worker shutdown: {0}")]
    Worker(#[from] CaptureWorkerStopError),
    #[error("capture status dispatcher panicked")]
    DispatcherPanicked,
}

#[derive(Debug, thiserror::Error)]
pub enum CaptureLifecycleControlError {
    #[error("capture is not running")]
    NotRunning,
    #[error("capture worker control: {0}")]
    Worker(#[from] CaptureWorkerControlError),
}

impl CaptureLifecycle {
    /// Start the worker and its status bridge. A startup failure is published
    /// as `blocked` before it is returned, so the caller never has to infer
    /// why capture is absent from a missing event stream.
    pub fn start(
        config: RealCaptureWorkerConfig,
        sink: Arc<dyn CaptureStatusSink>,
    ) -> Result<Self, CaptureLifecycleStartError> {
        let current_status = Arc::new(Mutex::new(status(CaptureRuntimeState::Starting)));
        publish(
            &current_status,
            &sink,
            status(CaptureRuntimeState::Starting),
        );

        let (worker, events) = match start_real_capture_worker(config) {
            Ok(started) => started,
            Err(error) => {
                publish(
                    &current_status,
                    &sink,
                    CaptureRuntimeStatus {
                        state: CaptureRuntimeState::Blocked,
                        observed_at_ms: now_ms(),
                        tick: None,
                        shutdown_flushed_chunks: None,
                        reason: Some(start_error_reason(&error).to_owned()),
                    },
                );
                return Err(error.into());
            }
        };

        publish(&current_status, &sink, status(CaptureRuntimeState::Running));
        let dispatcher_status = Arc::clone(&current_status);
        let dispatcher_sink = Arc::clone(&sink);
        let dispatcher = thread::Builder::new()
            .name("fndr-capture-status".into())
            .spawn(move || {
                while let Ok(event) = events.recv() {
                    let is_paused = dispatcher_status
                        .lock()
                        .expect("capture lifecycle status mutex is not poisoned")
                        .state
                        == CaptureRuntimeState::Paused;
                    publish(
                        &dispatcher_status,
                        &dispatcher_sink,
                        worker_event_status(event, is_paused),
                    );
                }
            })
            .expect("status dispatcher thread should be spawnable after capture starts");

        Ok(Self {
            worker: Mutex::new(Some(worker)),
            dispatcher: Mutex::new(Some(dispatcher)),
            status: current_status,
            sink,
        })
    }

    pub fn status(&self) -> CaptureRuntimeStatus {
        self.status
            .lock()
            .expect("capture lifecycle status mutex is not poisoned")
            .clone()
    }

    /// Pause or resume the existing worker. Pausing waits for the worker to
    /// observe the command, then publishes a generated status. It does not
    /// discard the scheduler or any already-durable data.
    pub fn set_paused(
        &self,
        paused: bool,
    ) -> Result<CaptureRuntimeStatus, CaptureLifecycleControlError> {
        let worker = self
            .worker
            .lock()
            .expect("capture lifecycle worker mutex is not poisoned");
        let worker = worker
            .as_ref()
            .ok_or(CaptureLifecycleControlError::NotRunning)?;
        worker.set_paused(paused)?;

        let previous = self.status();
        let next = CaptureRuntimeStatus {
            state: if paused {
                CaptureRuntimeState::Paused
            } else {
                CaptureRuntimeState::Running
            },
            observed_at_ms: now_ms(),
            tick: previous.tick,
            shutdown_flushed_chunks: None,
            reason: paused.then(|| "user_paused".into()),
        };
        publish(&self.status, &self.sink, next.clone());
        Ok(next)
    }

    /// Stop capture, wait for the durable drain, then wait for the status
    /// bridge to finish. This must be called from the app exit path, not an
    /// async task that could be torn down before the worker joins.
    pub fn shutdown(&self) -> Result<CaptureWorkerReport, CaptureLifecycleStopError> {
        let worker = self
            .worker
            .lock()
            .expect("capture lifecycle worker mutex is not poisoned")
            .take()
            .ok_or(CaptureLifecycleStopError::AlreadyStopped)?;
        let report = worker.shutdown()?;

        if let Some(dispatcher) = self
            .dispatcher
            .lock()
            .expect("capture lifecycle dispatcher mutex is not poisoned")
            .take()
        {
            dispatcher
                .join()
                .map_err(|_| CaptureLifecycleStopError::DispatcherPanicked)?;
        }

        publish(
            &self.status,
            &self.sink,
            CaptureRuntimeStatus {
                state: CaptureRuntimeState::Stopped,
                observed_at_ms: now_ms(),
                tick: None,
                shutdown_flushed_chunks: Some(
                    u32::try_from(report.shutdown_flushed_chunks).unwrap_or(u32::MAX),
                ),
                reason: None,
            },
        );
        Ok(report)
    }
}

fn publish(
    current: &Arc<Mutex<CaptureRuntimeStatus>>,
    sink: &Arc<dyn CaptureStatusSink>,
    next: CaptureRuntimeStatus,
) {
    *current
        .lock()
        .expect("capture lifecycle status mutex is not poisoned") = next.clone();
    sink.publish(next);
}

fn status(state: CaptureRuntimeState) -> CaptureRuntimeStatus {
    CaptureRuntimeStatus {
        state,
        observed_at_ms: now_ms(),
        tick: None,
        shutdown_flushed_chunks: None,
        reason: None,
    }
}

fn worker_event_status(event: CaptureWorkerEvent, is_paused: bool) -> CaptureRuntimeStatus {
    let (capture, reason) = capture_status(&event.outcome.capture);
    let flush = match event.outcome.flush {
        FlushTickOutcome::NotDue => CaptureFlushState::NotDue,
        FlushTickOutcome::Flushed(_) => CaptureFlushState::Flushed,
        FlushTickOutcome::Failed(_) => CaptureFlushState::Failed,
    };
    CaptureRuntimeStatus {
        state: if is_paused {
            CaptureRuntimeState::Paused
        } else {
            CaptureRuntimeState::Running
        },
        observed_at_ms: event.observed_at_ms as f64,
        tick: Some(CaptureTickStatus {
            capture,
            flush,
            reason,
        }),
        shutdown_flushed_chunks: None,
        reason: is_paused.then(|| "user_paused".into()),
    }
}

fn capture_status(outcome: &CaptureTickOutcome) -> (CaptureTickState, Option<String>) {
    match outcome {
        CaptureTickOutcome::Stored => (CaptureTickState::Stored, None),
        CaptureTickOutcome::UrlOnlyStored => (CaptureTickState::UrlOnlyStored, None),
        CaptureTickOutcome::Skipped(reason) => (
            CaptureTickState::Skipped,
            Some(skip_reason_code(*reason).to_owned()),
        ),
        CaptureTickOutcome::Failed { reason, .. } => (
            CaptureTickState::Failed,
            Some(skip_reason_code(*reason).to_owned()),
        ),
    }
}

fn skip_reason_code(reason: SkipReason) -> &'static str {
    match reason {
        SkipReason::MetadataUnavailable => "metadata_unavailable",
        SkipReason::PreCapturePrivacy => "pre_capture_privacy",
        SkipReason::PrivateBrowsing => "private_browsing",
        SkipReason::AdmissionPolicy => "admission_policy",
        SkipReason::PerceptualDuplicate => "perceptual_duplicate",
        SkipReason::MissingPerceptualSignature => "missing_perceptual_signature",
        SkipReason::LowSignal => "low_signal",
        SkipReason::SemanticDuplicate => "semantic_duplicate",
        SkipReason::FinalPrivacy => "final_privacy",
        SkipReason::ScreenRecordingOrCaptureUnavailable => {
            "screen_recording_or_capture_unavailable"
        }
        SkipReason::CaptureFailed => "capture_failed",
        SkipReason::OcrFailed => "ocr_failed",
        SkipReason::PersistenceFailed => "persistence_failed",
    }
}

fn start_error_reason(error: &CaptureWorkerStartError) -> &'static str {
    match error {
        CaptureWorkerStartError::InvalidCaptureInterval => "invalid_capture_interval",
        CaptureWorkerStartError::ThreadSpawn(_) => "capture_thread_unavailable",
        CaptureWorkerStartError::Scheduler(error) => match error {
            crate::capture_scheduler::SchedulerStartError::ModelMissing(_) => "model_missing",
            crate::capture_scheduler::SchedulerStartError::InvalidFlushInterval(_) => {
                "invalid_flush_interval"
            }
            crate::capture_scheduler::SchedulerStartError::Store(_) => "store_unavailable",
            crate::capture_scheduler::SchedulerStartError::Ocr(_) => "ocr_unavailable",
            crate::capture_scheduler::SchedulerStartError::Pipeline { .. } => {
                "pipeline_unavailable"
            }
        },
        CaptureWorkerStartError::StartupChannelClosed => "capture_startup_interrupted",
    }
}

fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use fndr_capture::{CaptureStage, PipelineError};
    use fndr_store::FlushReport;

    use super::*;
    use crate::capture_scheduler::SchedulerTickOutcome;

    #[test]
    fn worker_event_is_content_free_status() {
        let event = CaptureWorkerEvent {
            observed_at_ms: 42,
            outcome: SchedulerTickOutcome {
                capture: CaptureTickOutcome::Failed {
                    reason: SkipReason::CaptureFailed,
                    error: PipelineError::new(CaptureStage::Capture, "private window title"),
                },
                flush: FlushTickOutcome::Failed("private chunk text".into()),
            },
        };

        let status = worker_event_status(event, false);

        assert_eq!(status.state, CaptureRuntimeState::Running);
        assert_eq!(status.observed_at_ms, 42.0);
        assert_eq!(
            status.tick,
            Some(CaptureTickStatus {
                capture: CaptureTickState::Failed,
                flush: CaptureFlushState::Failed,
                reason: Some("capture_failed".into()),
            })
        );
        let rendered = format!("{status:?}");
        assert!(!rendered.contains("private window title"));
        assert!(!rendered.contains("private chunk text"));
    }

    #[test]
    fn skip_and_flush_states_remain_distinct() {
        let event = CaptureWorkerEvent {
            observed_at_ms: 42,
            outcome: SchedulerTickOutcome {
                capture: CaptureTickOutcome::Skipped(SkipReason::PreCapturePrivacy),
                flush: FlushTickOutcome::Flushed(FlushReport {
                    written: 2,
                    batch_was_full: false,
                }),
            },
        };

        let status = worker_event_status(event, false);

        assert_eq!(
            status.tick,
            Some(CaptureTickStatus {
                capture: CaptureTickState::Skipped,
                flush: CaptureFlushState::Flushed,
                reason: Some("pre_capture_privacy".into()),
            })
        );
    }

    #[test]
    fn screen_recording_unavailability_stays_actionable_but_content_free() {
        let event = CaptureWorkerEvent {
            observed_at_ms: 42,
            outcome: SchedulerTickOutcome {
                capture: CaptureTickOutcome::Failed {
                    reason: SkipReason::ScreenRecordingOrCaptureUnavailable,
                    error: PipelineError::new(CaptureStage::Capture, "TCC details"),
                },
                flush: FlushTickOutcome::NotDue,
            },
        };

        let status = worker_event_status(event, false);
        assert_eq!(
            status.tick.as_ref().and_then(|tick| tick.reason.as_deref()),
            Some("screen_recording_or_capture_unavailable")
        );
        assert!(!format!("{status:?}").contains("TCC details"));
    }

    #[test]
    fn private_browsing_skip_is_visible_without_exposing_the_window_title() {
        let event = CaptureWorkerEvent {
            observed_at_ms: 42,
            outcome: SchedulerTickOutcome {
                capture: CaptureTickOutcome::Skipped(SkipReason::PrivateBrowsing),
                flush: FlushTickOutcome::NotDue,
            },
        };

        let status = worker_event_status(event, false);
        assert_eq!(
            status.tick.as_ref().and_then(|tick| tick.reason.as_deref()),
            Some("private_browsing")
        );
    }
}
