//! The long-lived owner for real capture. It keeps synchronous ScreenCaptureKit
//! and Vision work off Tauri's async runtime, sends bounded status events, and
//! drains durable SQLite work to Lance before the application exits.

use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fndr_capture::{InputIdleSource, MacOSInputIdle, SamplingDecision, SamplingPolicy};
use fndr_store::FlushError;

use crate::capture_scheduler::{
    RealCaptureScheduler, RealSchedulerConfig, SchedulerStartError, SchedulerTickOutcome,
};

/// The smallest supported active-interval floor for the adaptive sampler
/// (T-308). This deliberately avoids a busy loop on an 8 GB machine.
pub const MIN_CAPTURE_INTERVAL: Duration = Duration::from_secs(2);
const STATUS_EVENT_CAPACITY: usize = 64;

/// Configuration for the shell-owned worker. The caller selects paths and
/// blocklist through `RealSchedulerConfig`; there is no hidden app-data path.
pub struct RealCaptureWorkerConfig {
    pub scheduler: RealSchedulerConfig,
    pub sampling: SamplingPolicy,
}

/// A status event carries outcomes and no image, OCR text, URL, or model data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureWorkerEvent {
    pub observed_at_ms: u64,
    pub outcome: SchedulerTickOutcome,
}

/// The normal, explicit shutdown result. `ticks` includes every attempted
/// capture, including policy skips and recoverable capture/flush failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureWorkerReport {
    pub ticks: u64,
    pub shutdown_flushed_chunks: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum CaptureWorkerStartError {
    #[error("capture interval must be at least two seconds")]
    InvalidCaptureInterval,
    #[error("could not start capture thread: {0}")]
    ThreadSpawn(#[from] std::io::Error),
    #[error("capture scheduler: {0}")]
    Scheduler(#[from] SchedulerStartError),
    #[error("capture worker exited before signalling startup")]
    StartupChannelClosed,
}

#[derive(Debug, thiserror::Error)]
pub enum CaptureWorkerStopError {
    #[error("capture worker panicked")]
    Panicked,
    #[error("shutdown flush: {0}")]
    ShutdownFlush(#[from] FlushError),
}

enum WorkerCommand {
    Shutdown,
}

/// Handle retained by the Tauri lifecycle owner. Call `shutdown` from the
/// app's exit-requested path; dropping the handle also causes the worker to
/// notice the disconnected command channel and perform the same drain.
pub struct CaptureWorkerHandle {
    command_tx: mpsc::Sender<WorkerCommand>,
    join: Option<JoinHandle<Result<CaptureWorkerReport, CaptureWorkerStopError>>>,
}

impl CaptureWorkerHandle {
    /// Request a drain and wait for the dedicated capture thread. This is the
    /// only blocking lifecycle call; it must not run on Tauri's async runtime.
    pub fn shutdown(mut self) -> Result<CaptureWorkerReport, CaptureWorkerStopError> {
        let _ = self.command_tx.send(WorkerCommand::Shutdown);
        self.join
            .take()
            .expect("capture worker handle always owns its join handle")
            .join()
            .map_err(|_| CaptureWorkerStopError::Panicked)?
    }
}

/// Start the real macOS capture owner. The scheduler is constructed inside the
/// dedicated thread so ScreenCaptureKit and Vision setup share the same owner
/// as every later capture tick. Model loading stays lazy in `ModelWorkerHandle`
/// and does not occur merely by starting the worker.
pub fn start_real_capture_worker(
    config: RealCaptureWorkerConfig,
) -> Result<(CaptureWorkerHandle, Receiver<CaptureWorkerEvent>), CaptureWorkerStartError> {
    start_worker(config.sampling, MacOSInputIdle, move || {
        RealCaptureScheduler::open(config.scheduler, now_ms())
    })
}

trait CaptureLoop: Send + 'static {
    fn tick(&mut self, now_ms: u64) -> SchedulerTickOutcome;
    fn flush_on_shutdown(&mut self, now_ms: u64) -> Result<usize, FlushError>;
}

impl CaptureLoop for RealCaptureScheduler {
    fn tick(&mut self, now_ms: u64) -> SchedulerTickOutcome {
        Self::tick(self, now_ms)
    }

    fn flush_on_shutdown(&mut self, now_ms: u64) -> Result<usize, FlushError> {
        Self::flush_on_shutdown(self, now_ms)
    }
}

fn start_worker<S, I>(
    sampling: SamplingPolicy,
    idle_source: I,
    open: impl FnOnce() -> Result<S, SchedulerStartError> + Send + 'static,
) -> Result<(CaptureWorkerHandle, Receiver<CaptureWorkerEvent>), CaptureWorkerStartError>
where
    S: CaptureLoop,
    I: InputIdleSource + Send + 'static,
{
    if sampling.active_interval < MIN_CAPTURE_INTERVAL {
        return Err(CaptureWorkerStartError::InvalidCaptureInterval);
    }
    let (command_tx, command_rx) = mpsc::channel();
    let (started_tx, started_rx) = mpsc::sync_channel(1);
    let (event_tx, event_rx) = mpsc::sync_channel(STATUS_EVENT_CAPACITY);
    let join = thread::Builder::new()
        .name("fndr-capture".into())
        .spawn(move || match open() {
            Ok(scheduler) => {
                let _ = started_tx.send(Ok(()));
                run_capture_loop(scheduler, sampling, idle_source, command_rx, event_tx)
            }
            Err(error) => {
                let _ = started_tx.send(Err(error));
                Ok(CaptureWorkerReport {
                    ticks: 0,
                    shutdown_flushed_chunks: 0,
                })
            }
        })?;
    match started_rx.recv() {
        Ok(Ok(())) => Ok((
            CaptureWorkerHandle {
                command_tx,
                join: Some(join),
            },
            event_rx,
        )),
        Ok(Err(error)) => {
            let _ = join.join();
            Err(CaptureWorkerStartError::Scheduler(error))
        }
        Err(_) => {
            let _ = join.join();
            Err(CaptureWorkerStartError::StartupChannelClosed)
        }
    }
}

fn run_capture_loop<S: CaptureLoop, I: InputIdleSource>(
    mut scheduler: S,
    sampling: SamplingPolicy,
    idle_source: I,
    command_rx: Receiver<WorkerCommand>,
    event_tx: SyncSender<CaptureWorkerEvent>,
) -> Result<CaptureWorkerReport, CaptureWorkerStopError> {
    let mut ticks = 0;
    // Zero forces an immediate first decision: `since_capture` is huge, so
    // the very first iteration captures right away unless input is already
    // deep-idle, matching the fixed-cadence loop's prior startup behavior.
    let mut last_capture_ms: u64 = 0;
    loop {
        let now = now_ms();
        let since_capture = Duration::from_millis(now.saturating_sub(last_capture_ms));
        let idle = idle_source.input_idle();

        match sampling.decide(idle, since_capture) {
            SamplingDecision::CaptureNow => {
                let outcome = scheduler.tick(now);
                ticks += 1;
                last_capture_ms = now;
                // UI/MCP event consumers must never make the capture pipeline
                // wait. A later health slice can coalesce or persist status;
                // this owner keeps the newest bounded telemetry best-effort
                // without retaining content.
                let _ = event_tx.try_send(CaptureWorkerEvent {
                    observed_at_ms: now,
                    outcome,
                });

                match command_rx.try_recv() {
                    Ok(WorkerCommand::Shutdown) | Err(mpsc::TryRecvError::Disconnected) => {
                        return finish(&mut scheduler, ticks);
                    }
                    Err(mpsc::TryRecvError::Empty) => {}
                }
            }
            SamplingDecision::Wait(wait) => match command_rx.recv_timeout(wait) {
                Ok(WorkerCommand::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return finish(&mut scheduler, ticks);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            },
            SamplingDecision::DeepIdle => match command_rx.recv_timeout(sampling.idle_interval) {
                Ok(WorkerCommand::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return finish(&mut scheduler, ticks);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            },
        }
    }
}

fn finish<S: CaptureLoop>(
    scheduler: &mut S,
    ticks: u64,
) -> Result<CaptureWorkerReport, CaptureWorkerStopError> {
    let shutdown_flushed_chunks = scheduler.flush_on_shutdown(now_ms())?;
    Ok(CaptureWorkerReport {
        ticks,
        shutdown_flushed_chunks,
    })
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use fndr_capture::CaptureTickOutcome;

    use super::*;
    use crate::capture_scheduler::FlushTickOutcome;

    struct FakeScheduler {
        ticks: Arc<AtomicUsize>,
        shutdown_calls: Arc<AtomicUsize>,
    }

    impl CaptureLoop for FakeScheduler {
        fn tick(&mut self, _now_ms: u64) -> SchedulerTickOutcome {
            self.ticks.fetch_add(1, Ordering::SeqCst);
            SchedulerTickOutcome {
                capture: CaptureTickOutcome::Stored,
                flush: FlushTickOutcome::NotDue,
            }
        }

        fn flush_on_shutdown(&mut self, _now_ms: u64) -> Result<usize, FlushError> {
            self.shutdown_calls.fetch_add(1, Ordering::SeqCst);
            Ok(3)
        }
    }

    #[derive(Clone, Copy)]
    struct FixedIdle(Duration);

    impl InputIdleSource for FixedIdle {
        fn input_idle(&self) -> Duration {
            self.0
        }
    }

    fn fast_policy() -> SamplingPolicy {
        SamplingPolicy {
            active_interval: MIN_CAPTURE_INTERVAL,
            idle_interval: Duration::from_millis(20),
            idle_after: Duration::from_millis(50),
            deep_idle_after: Duration::from_millis(200),
            forced_capture_after: Duration::from_secs(120),
        }
    }

    #[test]
    fn worker_ticks_off_thread_emits_status_and_drains_on_shutdown() {
        let ticks = Arc::new(AtomicUsize::new(0));
        let shutdown_calls = Arc::new(AtomicUsize::new(0));
        let fake_ticks = Arc::clone(&ticks);
        let fake_shutdown_calls = Arc::clone(&shutdown_calls);
        let (worker, events) = start_worker(fast_policy(), FixedIdle(Duration::ZERO), move || {
            Ok(FakeScheduler {
                ticks: fake_ticks,
                shutdown_calls: fake_shutdown_calls,
            })
        })
        .unwrap();

        let event = events.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(event.outcome.capture, CaptureTickOutcome::Stored);
        let report = worker.shutdown().unwrap();
        assert_eq!(report.ticks, 1);
        assert_eq!(report.shutdown_flushed_chunks, 3);
        assert_eq!(ticks.load(Ordering::SeqCst), 1);
        assert_eq!(shutdown_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn worker_rejects_busy_active_interval_before_opening_scheduler() {
        let mut policy = fast_policy();
        policy.active_interval = Duration::from_millis(1);
        let result = start_worker(
            policy,
            FixedIdle(Duration::ZERO),
            || -> Result<FakeScheduler, _> {
                panic!("invalid policy must not initialize the scheduler")
            },
        );
        assert!(matches!(
            result,
            Err(CaptureWorkerStartError::InvalidCaptureInterval)
        ));
    }

    #[test]
    fn deep_idle_input_pauses_capture_until_shutdown() {
        let ticks = Arc::new(AtomicUsize::new(0));
        let shutdown_calls = Arc::new(AtomicUsize::new(0));
        let fake_ticks = Arc::clone(&ticks);
        let fake_shutdown_calls = Arc::clone(&shutdown_calls);
        // Idle already past `deep_idle_after` on the very first iteration:
        // the loop must never capture, only wait, and still shut down fast.
        let (worker, events) = start_worker(
            fast_policy(),
            FixedIdle(Duration::from_secs(10)),
            move || {
                Ok(FakeScheduler {
                    ticks: fake_ticks,
                    shutdown_calls: fake_shutdown_calls,
                })
            },
        )
        .unwrap();

        assert!(matches!(
            events.recv_timeout(Duration::from_millis(100)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        let started = std::time::Instant::now();
        let report = worker.shutdown().unwrap();
        assert!(started.elapsed() < Duration::from_secs(1));
        assert_eq!(report.ticks, 0);
        assert_eq!(ticks.load(Ordering::SeqCst), 0);
        assert_eq!(shutdown_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn active_input_recaptures_after_the_active_interval_wait() {
        let ticks = Arc::new(AtomicUsize::new(0));
        let shutdown_calls = Arc::new(AtomicUsize::new(0));
        let fake_ticks = Arc::clone(&ticks);
        let fake_shutdown_calls = Arc::clone(&shutdown_calls);
        let (worker, events) = start_worker(fast_policy(), FixedIdle(Duration::ZERO), move || {
            Ok(FakeScheduler {
                ticks: fake_ticks,
                shutdown_calls: fake_shutdown_calls,
            })
        })
        .unwrap();

        events.recv_timeout(Duration::from_secs(1)).unwrap();
        // The next capture should only arrive after ~active_interval, not
        // immediately (proves the loop actually waits between captures).
        assert!(matches!(
            events.recv_timeout(Duration::from_millis(200)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        events
            .recv_timeout(Duration::from_secs(5))
            .expect("second capture eventually arrives at the active cadence");
        let report = worker.shutdown().unwrap();
        assert!(report.ticks >= 2);
        assert_eq!(shutdown_calls.load(Ordering::SeqCst), 1);
    }
}
