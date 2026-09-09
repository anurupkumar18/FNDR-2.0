//! Tauri-facing ownership for the capture lifecycle (T-901).
//!
//! This is intentionally a small shell adapter. It selects the app-owned
//! storage location, translates status into a Tauri event, and joins the
//! worker during process exit. The capture pipeline remains in its dedicated
//! worker and the engine remains usable without a visible window.

use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use core_graphics::access::ScreenCaptureAccess;
use fndr_capture::SamplingPolicy;
use fndr_inference::MODELS;
use fndr_privacy::Blocklist;
use fndr_store::Store;
use fndr_types::{AuditLogEntry, CaptureRuntimeState, CaptureRuntimeStatus, ScreenRecordingAccess};
use tauri::{Emitter, Manager};

use crate::capture_lifecycle::{
    CAPTURE_STATUS_EVENT, CaptureLifecycle, CaptureLifecycleControlError,
    CaptureLifecycleStopError, CaptureStatusSink,
};
use crate::capture_scheduler::RealSchedulerConfig;
use crate::capture_worker::RealCaptureWorkerConfig;

/// The trust window stays intentionally bounded: it is an inspectable recent
/// activity ledger, not a second full audit database browser.
pub const AUDIT_LOG_LIMIT: usize = 50;

/// Command-line overrides for a developer/demo run. A shipped app uses its
/// app-data directory and the downloader's registered model location; an
/// explicit override is useful only for a local demo model and is visible in
/// the launch command rather than hidden in source.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CaptureLaunchOptions {
    pub data_dir: Option<PathBuf>,
    pub model_path: Option<PathBuf>,
    /// Runs a read-only preflight and exits before constructing Tauri, OCR,
    /// ScreenCaptureKit, or the capture worker.
    pub doctor: bool,
    /// A deliberate demo-only lifecycle bound. Normal desktop control belongs
    /// to the later tray/onboarding surface, not a hidden timer.
    pub run_seconds: Option<u64>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CaptureLaunchOptionsError {
    #[error("{0} requires a path")]
    MissingValue(&'static str),
    #[error("unknown shell option: {0}")]
    UnknownOption(String),
    #[error("--run-seconds must be a positive whole number")]
    InvalidRunSeconds,
    #[error("--doctor cannot be combined with --run-seconds")]
    DoctorWithRunSeconds,
}

impl CaptureLaunchOptions {
    pub fn parse(
        args: impl IntoIterator<Item = String>,
    ) -> Result<Self, CaptureLaunchOptionsError> {
        let mut options = Self::default();
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--data-dir" => {
                    options.data_dir = Some(PathBuf::from(
                        args.next()
                            .ok_or(CaptureLaunchOptionsError::MissingValue("--data-dir"))?,
                    ));
                }
                "--model" => {
                    options.model_path = Some(PathBuf::from(
                        args.next()
                            .ok_or(CaptureLaunchOptionsError::MissingValue("--model"))?,
                    ));
                }
                "--run-seconds" => {
                    let seconds = args
                        .next()
                        .ok_or(CaptureLaunchOptionsError::MissingValue("--run-seconds"))?
                        .parse::<u64>()
                        .map_err(|_| CaptureLaunchOptionsError::InvalidRunSeconds)?;
                    if seconds == 0 {
                        return Err(CaptureLaunchOptionsError::InvalidRunSeconds);
                    }
                    options.run_seconds = Some(seconds);
                }
                "--doctor" => options.doctor = true,
                _ => return Err(CaptureLaunchOptionsError::UnknownOption(arg)),
            }
        }
        if options.doctor && options.run_seconds.is_some() {
            return Err(CaptureLaunchOptionsError::DoctorWithRunSeconds);
        }
        Ok(options)
    }
}

/// A non-capturing readiness check for the explicit demo paths. The doctor
/// never writes a store, starts Tauri, opens ScreenCaptureKit, or prompts for
/// Screen Recording. It only reports what can be known safely from paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoctorReport {
    pub data_dir: Option<PathBuf>,
    pub data_dir_state: &'static str,
    pub model_path: Option<PathBuf>,
    pub model_state: &'static str,
    /// Result of the macOS preflight API. It never calls the separate request
    /// API and therefore never opens the Screen Recording consent dialog.
    pub screen_recording_state: &'static str,
    pub ready_for_permission_rehearsal: bool,
}

impl DoctorReport {
    pub fn lines(&self) -> Vec<String> {
        let data_dir = self.data_dir.as_ref().map_or_else(
            || "app-managed location (not resolved by doctor)".into(),
            |path| path.display().to_string(),
        );
        let model_path = self.model_path.as_ref().map_or_else(
            || "not resolved (pass --model or --data-dir)".into(),
            |path| path.display().to_string(),
        );
        let overall = if self.ready_for_permission_rehearsal {
            "ready_for_permission_rehearsal"
        } else {
            "blocked"
        };

        vec![
            "FNDR doctor (no capture attempted)".into(),
            format!("data_dir: {} ({})", data_dir, self.data_dir_state),
            format!("model: {} ({})", model_path, self.model_state),
            format!(
                "screen_recording: {} (preflight only; doctor never requests permission)",
                self.screen_recording_state
            ),
            format!("overall: {overall}"),
        ]
    }
}

pub fn doctor(options: &CaptureLaunchOptions) -> DoctorReport {
    doctor_with_screen_recording_preflight(options, || ScreenCaptureAccess.preflight())
}

/// Read the current Screen Recording status without invoking macOS's request
/// API. This must stay separate from `ShellCaptureState::start`, which is the
/// only normal desktop path allowed to reach the real capture worker.
pub fn screen_recording_preflight() -> ScreenRecordingAccess {
    screen_recording_preflight_with(|| ScreenCaptureAccess.preflight())
}

fn screen_recording_preflight_with(
    screen_recording_granted: impl FnOnce() -> bool,
) -> ScreenRecordingAccess {
    if screen_recording_granted() {
        ScreenRecordingAccess::Granted
    } else {
        ScreenRecordingAccess::NotGranted
    }
}

fn doctor_with_screen_recording_preflight(
    options: &CaptureLaunchOptions,
    screen_recording_granted: impl FnOnce() -> bool,
) -> DoctorReport {
    let data_dir_state = match options.data_dir.as_deref() {
        None => "not_resolved",
        Some(path) if path.is_dir() => "existing",
        Some(path) if path.exists() => "not_a_directory",
        Some(path) if path.parent().is_some_and(std::path::Path::is_dir) => "will_create",
        Some(_) => "parent_missing",
    };
    let model_path = options.model_path.clone().or_else(|| {
        options
            .data_dir
            .as_ref()
            .map(|path| path.join("models").join(MODELS[0].filename))
    });
    let model_state = match model_path.as_deref() {
        Some(path) if path.is_file() => "ready",
        Some(_) => "missing",
        None => "not_resolved",
    };

    DoctorReport {
        data_dir: options.data_dir.clone(),
        data_dir_state,
        model_path,
        model_state,
        screen_recording_state: match screen_recording_preflight_with(screen_recording_granted) {
            ScreenRecordingAccess::Granted => "granted",
            ScreenRecordingAccess::NotGranted => "not_granted",
        },
        ready_for_permission_rehearsal: matches!(data_dir_state, "existing" | "will_create")
            && model_state == "ready",
    }
}

/// The owner-facing audit viewer reads only a bounded, existing SQLite vault.
/// Unlike `Store::open`, this path cannot create the database or apply a
/// migration. An absent vault truthfully means there is no local MCP activity
/// to show yet.
pub fn recent_audit_entries(data_dir: &Path, limit: usize) -> Result<Vec<AuditLogEntry>, String> {
    let database_path = data_dir.join("vault.sqlite3");
    if !database_path.exists() {
        return Ok(Vec::new());
    }

    Store::open_read_only(&database_path)
        .and_then(|store| store.recent_tool_calls(limit))
        .map(|entries| {
            entries
                .into_iter()
                .map(|entry| AuditLogEntry {
                    at_ms: entry.at_ms as f64,
                    tool: entry.tool,
                    outcome: entry.outcome,
                    raw_released: entry.raw_released,
                })
                .collect()
        })
        .map_err(|_| "audit_log_unavailable".to_owned())
}

/// Tauri-managed app state. The lifecycle exists only after the real worker
/// starts; a blocked start retains a visible status and leaves no orphaned
/// capture thread behind.
pub struct ShellCaptureState {
    instance_lock: Mutex<Option<InstanceLock>>,
    lifecycle: Mutex<Option<CaptureLifecycle>>,
    status: Arc<Mutex<CaptureRuntimeStatus>>,
}

impl Default for ShellCaptureState {
    fn default() -> Self {
        Self {
            instance_lock: Mutex::new(None),
            lifecycle: Mutex::new(None),
            status: Arc::new(Mutex::new(CaptureRuntimeStatus {
                state: CaptureRuntimeState::Stopped,
                observed_at_ms: 0.0,
                tick: None,
                quality: None,
                shutdown_flushed_chunks: None,
                reason: Some("not_started".into()),
            })),
        }
    }
}

impl ShellCaptureState {
    pub fn status(&self) -> CaptureRuntimeStatus {
        self.status
            .lock()
            .expect("shell capture status mutex is not poisoned")
            .clone()
    }

    /// Start the real worker. The app deliberately stays up after a failed
    /// start so the UI/IPC can show the typed blocked status instead of
    /// crashing or silently becoming an inert process.
    pub fn start(
        &self,
        app: &tauri::AppHandle,
        options: CaptureLaunchOptions,
    ) -> tauri::Result<()> {
        if self
            .lifecycle
            .lock()
            .expect("shell capture lifecycle mutex is not poisoned")
            .is_some()
        {
            return Ok(());
        }
        let data_dir = match options.data_dir {
            Some(path) => path,
            None => app.path().app_data_dir()?,
        };
        std::fs::create_dir_all(&data_dir)?;
        let instance_lock = InstanceLock::acquire(&data_dir)?;
        *self
            .instance_lock
            .lock()
            .expect("shell instance lock mutex is not poisoned") = Some(instance_lock);
        let model_path = options
            .model_path
            .unwrap_or_else(|| data_dir.join("models").join(MODELS[0].filename));
        let sink: Arc<dyn CaptureStatusSink> = Arc::new(TauriCaptureStatusSink {
            app: app.clone(),
            status: Arc::clone(&self.status),
        });
        let config = RealCaptureWorkerConfig {
            scheduler: RealSchedulerConfig {
                database_path: data_dir.join("vault.sqlite3"),
                index_dir: data_dir.join("index"),
                model_path,
                blocklist: Blocklist::default(),
                session_id: format!("desktop-{}", std::process::id()),
                display_index: 0,
                flush_interval: Duration::from_secs(30),
                model_idle_timeout: Duration::from_secs(10 * 60),
            },
            sampling: SamplingPolicy::default(),
        };

        if let Ok(lifecycle) = CaptureLifecycle::start(config, sink) {
            *self
                .lifecycle
                .lock()
                .expect("shell capture lifecycle mutex is not poisoned") = Some(lifecycle);
        }
        Ok(())
    }

    /// Called only from the app exit path. It is idempotent from the app's
    /// perspective because an unavailable or already-stopped worker has
    /// already reported its terminal status.
    pub fn shutdown(&self) -> Result<(), CaptureLifecycleStopError> {
        let lifecycle = self
            .lifecycle
            .lock()
            .expect("shell capture lifecycle mutex is not poisoned")
            .take();
        match lifecycle {
            Some(lifecycle) => lifecycle.shutdown().map(|_| ()),
            None => Ok(()),
        }
    }

    /// Changes only the live worker's capture state. The lifecycle owns the
    /// generated status event, so the tray and status window converge on the
    /// same content-free state without polling.
    pub fn set_paused(
        &self,
        paused: bool,
    ) -> Result<CaptureRuntimeStatus, CaptureLifecycleControlError> {
        let lifecycle = self
            .lifecycle
            .lock()
            .expect("shell capture lifecycle mutex is not poisoned");
        lifecycle
            .as_ref()
            .ok_or(CaptureLifecycleControlError::NotRunning)?
            .set_paused(paused)
    }
}

/// An advisory lock retained for the process lifetime. It guards the app data
/// directory's single SQLite writer and derived Lance writer from a second
/// desktop host. The lock file itself is harmless after a crash; the kernel
/// releases the advisory lock when its owning process exits.
struct InstanceLock {
    _file: File,
}

impl InstanceLock {
    const FILE_NAME: &'static str = ".fndr-instance.lock";

    fn acquire(data_dir: &Path) -> std::io::Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(data_dir.join(Self::FILE_NAME))?;
        // `flock` is process-scoped on macOS and the open descriptor keeps the
        // lock alive until FNDR exits. `LOCK_NB` turns a second host into a
        // fast, visible startup failure instead of a blocked launch.
        let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if result == 0 {
            Ok(Self { _file: file })
        } else {
            Err(std::io::Error::last_os_error())
        }
    }
}

struct TauriCaptureStatusSink {
    app: tauri::AppHandle,
    status: Arc<Mutex<CaptureRuntimeStatus>>,
}

impl CaptureStatusSink for TauriCaptureStatusSink {
    fn publish(&self, status: CaptureRuntimeStatus) {
        *self
            .status
            .lock()
            .expect("shell capture status mutex is not poisoned") = status.clone();
        let _ = self.app.emit(CAPTURE_STATUS_EVENT, status);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_explicit_demo_paths() {
        let options = CaptureLaunchOptions::parse([
            "--data-dir".into(),
            "/tmp/fndr-demo".into(),
            "--model".into(),
            "/tmp/embed.gguf".into(),
            "--run-seconds".into(),
            "30".into(),
        ])
        .unwrap();

        assert_eq!(options.data_dir, Some(PathBuf::from("/tmp/fndr-demo")));
        assert_eq!(options.model_path, Some(PathBuf::from("/tmp/embed.gguf")));
        assert_eq!(options.run_seconds, Some(30));
        assert!(!options.doctor);

        let doctor_options = CaptureLaunchOptions::parse(["--doctor".into()]).unwrap();
        assert!(doctor_options.doctor);
    }

    #[test]
    fn rejects_partial_or_unknown_options() {
        assert_eq!(
            CaptureLaunchOptions::parse(["--model".into()]),
            Err(CaptureLaunchOptionsError::MissingValue("--model"))
        );
        assert_eq!(
            CaptureLaunchOptions::parse(["--surprise".into()]),
            Err(CaptureLaunchOptionsError::UnknownOption(
                "--surprise".into()
            ))
        );
        assert_eq!(
            CaptureLaunchOptions::parse(["--run-seconds".into(), "0".into()]),
            Err(CaptureLaunchOptionsError::InvalidRunSeconds)
        );
        assert_eq!(
            CaptureLaunchOptions::parse(["--doctor".into(), "--run-seconds".into(), "1".into()]),
            Err(CaptureLaunchOptionsError::DoctorWithRunSeconds)
        );
    }

    #[test]
    fn doctor_is_read_only_and_names_the_remaining_human_check() {
        let report = doctor_with_screen_recording_preflight(
            &CaptureLaunchOptions {
                data_dir: Some(PathBuf::from("/tmp/fndr-doctor-path-that-does-not-exist")),
                model_path: Some(PathBuf::from(
                    "/tmp/fndr-doctor-model-that-does-not-exist.gguf",
                )),
                doctor: true,
                run_seconds: None,
            },
            || false,
        );

        assert_eq!(report.data_dir_state, "will_create");
        assert_eq!(report.model_state, "missing");
        assert!(!report.ready_for_permission_rehearsal);
        assert!(
            report
                .lines()
                .iter()
                .any(|line| line.contains("screen_recording: not_granted"))
        );
    }

    #[test]
    fn doctor_preflight_reports_granted_without_changing_path_readiness() {
        let report = doctor_with_screen_recording_preflight(
            &CaptureLaunchOptions {
                data_dir: Some(PathBuf::from("/tmp/fndr-doctor-ready-path")),
                model_path: Some(PathBuf::from("/tmp/fndr-doctor-ready-model.gguf")),
                doctor: true,
                run_seconds: None,
            },
            || true,
        );

        assert_eq!(report.screen_recording_state, "granted");
        assert_eq!(report.data_dir_state, "will_create");
        assert_eq!(report.model_state, "missing");
        assert!(!report.ready_for_permission_rehearsal);
    }

    #[test]
    fn screen_recording_preflight_has_only_granted_or_not_granted_results() {
        assert_eq!(
            screen_recording_preflight_with(|| true),
            ScreenRecordingAccess::Granted
        );
        assert_eq!(
            screen_recording_preflight_with(|| false),
            ScreenRecordingAccess::NotGranted
        );
    }

    #[test]
    fn normal_desktop_state_is_non_capturing_until_explicit_start() {
        let status = ShellCaptureState::default().status();
        assert_eq!(status.state, CaptureRuntimeState::Stopped);
        assert_eq!(status.reason.as_deref(), Some("not_started"));
        assert!(status.tick.is_none());
    }

    #[test]
    fn missing_vault_has_an_empty_audit_view_without_creating_a_database() {
        let directory = std::env::temp_dir().join(format!(
            "fndr-audit-view-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();

        assert_eq!(recent_audit_entries(&directory, 50), Ok(Vec::new()));
        assert!(!directory.join("vault.sqlite3").exists());

        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn audit_view_reads_only_content_free_entries_from_an_existing_vault() {
        let directory = std::env::temp_dir().join(format!(
            "fndr-audit-view-existing-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        let database = directory.join("vault.sqlite3");
        {
            let store = Store::open(&database).unwrap();
            store
                .record_tool_call(1_000, "fndr.search", "ok", false)
                .unwrap();
            store
                .record_tool_call(2_000, "fndr.source_evidence", "ok", true)
                .unwrap();
        }

        let entries = recent_audit_entries(&directory, 1).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].at_ms, 2_000.0);
        assert_eq!(entries[0].tool, "fndr.source_evidence");
        assert_eq!(entries[0].outcome, "ok");
        assert!(entries[0].raw_released);

        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn instance_lock_creates_a_process_lifetime_lock_file() {
        let path = std::env::temp_dir().join(format!(
            "fndr-instance-lock-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&path).unwrap();

        let first = InstanceLock::acquire(&path).unwrap();
        assert!(path.join(InstanceLock::FILE_NAME).is_file());
        drop(first);

        std::fs::remove_dir_all(path).unwrap();
    }
}
