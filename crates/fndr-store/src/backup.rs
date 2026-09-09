//! T-209: vault backup and restore.
//!
//! The vault is a live WAL SQLite database, so copying the file while capture
//! is writing is the corruption trap the ticket names (the same one Time
//! Machine users hit). The snapshot is therefore taken with SQLite's own
//! `VACUUM INTO`, from a read-only connection: it runs inside one read
//! transaction, so it sees a single commit boundary; it includes everything
//! already committed to the WAL; and it writes one fresh file with no WAL of
//! its own, which is the only shape that is safe to copy around afterwards.
//!
//! A backup carries truth plus config. The Lance index is deliberately absent:
//! it is a rebuildable derivative of SQLite (ADR-002), and a stale derivative
//! sitting beside restored truth would answer queries from data the truth no
//! longer contains. Restore therefore moves any index it finds aside instead
//! of leaving it to be believed.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{Store, StoreError};

/// The SQLite truth file inside a vault data directory.
pub const VAULT_DATABASE_FILE: &str = "vault.sqlite3";
/// Self-describing header written into every backup manifest.
pub const BACKUP_FORMAT: &str = "fndr.backup";
/// Bumped only when a restore of an older backup would need different steps.
pub const BACKUP_FORMAT_VERSION: u64 = 1;
pub const BACKUP_MANIFEST_FILE: &str = "manifest.json";
/// Config files travel in their own subdirectory so the manifest and the
/// database snapshot can never be mistaken for owner config.
pub const BACKUP_CONFIG_DIR: &str = "config";
/// Prefix a restore uses when moving a superseded derived index aside.
pub const SUPERSEDED_INDEX_PREFIX: &str = "index.superseded-";

/// The on-disk shape of a vault data directory, in one place so backup,
/// restore, and export cannot disagree about what is truth, what is
/// derivative, and what is machine-local runtime state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultLayout {
    /// SQLite truth, relative to the data directory.
    pub database: String,
    /// The derived retrieval index. Never backed up (it rebuilds from truth)
    /// and never left in place over a restored database.
    pub derived_index_dir: String,
    /// Directories that are neither truth nor derived from truth: they are
    /// re-acquirable (`models`, registry-managed and checksummed) or
    /// local-only observability (`logs`). Excluded from a backup, untouched
    /// by a restore.
    pub excluded_dirs: Vec<String>,
    /// Top-level files that are machine-local runtime state rather than
    /// config. The instance lock belongs to the process that holds it.
    pub excluded_files: Vec<String>,
}

impl Default for VaultLayout {
    fn default() -> Self {
        Self {
            database: VAULT_DATABASE_FILE.to_owned(),
            derived_index_dir: "index".to_owned(),
            excluded_dirs: vec!["models".to_owned(), "logs".to_owned()],
            excluded_files: vec![".fndr-instance.lock".to_owned()],
        }
    }
}

impl VaultLayout {
    pub fn database_path(&self, data_dir: &Path) -> PathBuf {
        data_dir.join(&self.database)
    }

    /// SQLite's companion files. They are never copied: the snapshot already
    /// contains their committed content, and a copied WAL beside a copied
    /// database is exactly the torn pair this ticket exists to prevent.
    pub fn database_sidecars(&self, data_dir: &Path) -> Vec<PathBuf> {
        ["-wal", "-shm", "-journal"]
            .iter()
            .map(|suffix| data_dir.join(format!("{}{suffix}", self.database)))
            .collect()
    }

    fn classify(&self, name: &str, is_dir: bool) -> Entry {
        if is_dir {
            if name == self.derived_index_dir {
                return Entry::Skipped(BackupSkipReason::DerivedIndex);
            }
            if self.excluded_dirs.iter().any(|dir| dir == name) {
                return Entry::Skipped(BackupSkipReason::Reacquirable);
            }
            return Entry::Skipped(BackupSkipReason::UnknownDirectory);
        }
        if name == self.database {
            return Entry::Database;
        }
        if ["-wal", "-shm", "-journal"]
            .iter()
            .any(|suffix| name == format!("{}{suffix}", self.database))
        {
            return Entry::Skipped(BackupSkipReason::DatabaseSidecar);
        }
        if self.excluded_files.iter().any(|file| file == name) || name.starts_with('.') {
            return Entry::Skipped(BackupSkipReason::RuntimeState);
        }
        Entry::Config
    }
}

enum Entry {
    Database,
    Config,
    Skipped(BackupSkipReason),
}

/// Why a data-directory entry did not travel with the backup. Recorded per
/// entry in the manifest and the report: a backup that quietly omitted
/// something would be a silent partial, which this codebase does not allow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupSkipReason {
    /// Rebuilds from SQLite truth (ADR-002).
    DerivedIndex,
    /// Re-acquirable or local-only: models, logs.
    Reacquirable,
    /// The live database's WAL/SHM companions.
    DatabaseSidecar,
    /// Instance lock and other machine-local dotfiles.
    RuntimeState,
    /// A directory this layout does not know about. Not copied, but named,
    /// so an owner can see it and decide.
    UnknownDirectory,
}

impl BackupSkipReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DerivedIndex => "derived_index_rebuilds_from_truth",
            Self::Reacquirable => "reacquirable_or_local_only",
            Self::DatabaseSidecar => "database_sidecar_included_in_snapshot",
            Self::RuntimeState => "machine_local_runtime_state",
            Self::UnknownDirectory => "unknown_directory_not_copied",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedEntry {
    pub name: String,
    pub reason: BackupSkipReason,
}

#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    #[error("no vault database at {0}")]
    DatabaseMissing(PathBuf),
    #[error("{0} already exists; a backup never overwrites, choose a new destination")]
    DestinationExists(PathBuf),
    #[error("a vault already exists at {0}; restore refuses to overwrite it without an explicit overwrite request")]
    VaultExists(PathBuf),
    #[error("{0} already exists at the restore destination; restore refuses to overwrite config without an explicit overwrite request")]
    ConfigExists(PathBuf),
    #[error("{0} is not valid UTF-8; SQLite needs a UTF-8 path for the snapshot")]
    NonUtf8Path(PathBuf),
    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("store: {0}")]
    Store(#[from] StoreError),
    #[error("the database snapshot failed its integrity check: {0}")]
    SnapshotCorrupt(String),
    #[error("{0} is not an FNDR backup: no manifest.json")]
    NotABackup(PathBuf),
    #[error("backup manifest at {path} is unusable: {reason}")]
    ManifestInvalid { path: PathBuf, reason: String },
    #[error(
        "backup format v{found} is newer than this build supports (v{supported}); update FNDR instead of downgrading"
    )]
    FormatTooNew { found: u64, supported: u64 },
}

fn io_at(path: &Path) -> impl FnOnce(io::Error) -> BackupError {
    let path = path.to_path_buf();
    move |source| BackupError::Io { path, source }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct VaultStats {
    schema_version: i64,
    records: i64,
    chunks: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupReport {
    pub destination: PathBuf,
    pub database_bytes: u64,
    pub schema_version: i64,
    pub records: i64,
    pub chunks: i64,
    pub config_files: Vec<String>,
    pub skipped: Vec<SkippedEntry>,
}

/// What a restore did, in full. `index_rebuild_required` is true whenever the
/// restored truth has chunks, because the derived index is never restored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreReport {
    pub data_dir: PathBuf,
    pub database: PathBuf,
    pub schema_version: i64,
    pub records: i64,
    pub chunks: i64,
    pub config_files: Vec<String>,
    pub replaced_vault: bool,
    pub superseded_index: Option<PathBuf>,
    pub index_rebuild_required: bool,
}

/// Whether a restore may replace a vault that is already at the destination.
/// Defaulting to refusal is deliberate: losing captured memory to a careless
/// restore is far worse than the inconvenience of a second flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestorePolicy {
    RefuseIfVaultExists,
    ReplaceExistingVault,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or_default()
}

fn utf8_path(path: &Path) -> Result<&str, BackupError> {
    path.to_str()
        .ok_or_else(|| BackupError::NonUtf8Path(path.to_path_buf()))
}

/// Open a snapshot or vault read-only and report what it holds, refusing a
/// database that fails SQLite's own integrity check.
fn verify(database: &Path) -> Result<VaultStats, BackupError> {
    let store = Store::open_read_only(database)?;
    let integrity: String =
        store
            .conn()
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    if integrity != "ok" {
        return Err(BackupError::SnapshotCorrupt(integrity));
    }
    Ok(VaultStats {
        schema_version: store.schema_version()?,
        records: store
            .conn()
            .query_row("SELECT COUNT(*) FROM memory_records", [], |row| row.get(0))?,
        chunks: store
            .conn()
            .query_row("SELECT COUNT(*) FROM chunks", [], |row| row.get(0))?,
    })
}

/// Take a consistent snapshot of a live vault while it is being written.
///
/// `VACUUM INTO` refuses to write over an existing file, so the snapshot path
/// must be fresh; the caller stages it in a directory it owns.
fn snapshot_database(source: &Path, snapshot: &Path) -> Result<(), BackupError> {
    // Read-only on purpose: a backup must never migrate, checkpoint, or in any
    // other way write the vault it is copying.
    let store = Store::open_read_only(source)?;
    store
        .conn()
        .execute("VACUUM INTO ?1", [utf8_path(snapshot)?])?;
    Ok(())
}

/// Snapshot the vault at `data_dir` into a new `destination` directory.
///
/// Safe to run under active capture: that is the acceptance criterion. The
/// destination must not exist; the work is staged in a sibling directory and
/// renamed into place only after the snapshot passes its integrity check, so a
/// failure leaves no half-written directory that looks like a backup.
pub fn backup_vault(
    data_dir: &Path,
    destination: &Path,
    layout: &VaultLayout,
) -> Result<BackupReport, BackupError> {
    let database = layout.database_path(data_dir);
    if !database.is_file() {
        return Err(BackupError::DatabaseMissing(database));
    }
    if destination.exists() {
        return Err(BackupError::DestinationExists(destination.to_path_buf()));
    }

    let staging = staging_sibling(destination);
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging).map_err(io_at(&staging))?;

    let outcome = stage_backup(data_dir, &staging, destination, layout).and_then(|report| {
        fs::rename(&staging, destination).map_err(io_at(destination))?;
        Ok(report)
    });
    if outcome.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    outcome
}

fn staging_sibling(destination: &Path) -> PathBuf {
    let name = destination
        .file_name()
        .map_or_else(|| "backup".to_owned(), |name| name.to_string_lossy().into());
    let staging = format!(".{name}.partial-{}", std::process::id());
    destination
        .parent()
        .map_or_else(|| PathBuf::from(&staging), |parent| parent.join(&staging))
}

fn stage_backup(
    data_dir: &Path,
    staging: &Path,
    destination: &Path,
    layout: &VaultLayout,
) -> Result<BackupReport, BackupError> {
    let snapshot = staging.join(&layout.database);
    snapshot_database(&layout.database_path(data_dir), &snapshot)?;
    let stats = verify(&snapshot)?;
    let database_bytes = fs::metadata(&snapshot).map_err(io_at(&snapshot))?.len();

    let mut config_files = Vec::new();
    let mut skipped = Vec::new();
    let config_dir = staging.join(BACKUP_CONFIG_DIR);
    fs::create_dir_all(&config_dir).map_err(io_at(&config_dir))?;
    for entry in fs::read_dir(data_dir).map_err(io_at(data_dir))? {
        let entry = entry.map_err(io_at(data_dir))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_dir = entry
            .file_type()
            .map_err(io_at(&entry.path()))?
            .is_dir();
        match layout.classify(&name, is_dir) {
            Entry::Database => {}
            Entry::Config => {
                let target = config_dir.join(&name);
                fs::copy(entry.path(), &target).map_err(io_at(&entry.path()))?;
                config_files.push(name);
            }
            Entry::Skipped(reason) => skipped.push(SkippedEntry { name, reason }),
        }
    }
    config_files.sort();
    skipped.sort_by(|a, b| a.name.cmp(&b.name));

    let report = BackupReport {
        destination: destination.to_path_buf(),
        database_bytes,
        schema_version: stats.schema_version,
        records: stats.records,
        chunks: stats.chunks,
        config_files,
        skipped,
    };
    write_manifest(staging, data_dir, layout, &report)?;
    Ok(report)
}

fn write_manifest(
    staging: &Path,
    data_dir: &Path,
    layout: &VaultLayout,
    report: &BackupReport,
) -> Result<(), BackupError> {
    let manifest = serde_json::json!({
        "format": BACKUP_FORMAT,
        "format_version": BACKUP_FORMAT_VERSION,
        "created_at_ms": now_ms(),
        "source_data_dir": data_dir.to_string_lossy(),
        "database": {
            "file": layout.database,
            "bytes": report.database_bytes,
            "schema_version": report.schema_version,
            "integrity_check": "ok",
            "snapshot_method": "sqlite VACUUM INTO from a read-only connection",
        },
        "contents": {
            "records": report.records,
            "chunks": report.chunks,
        },
        "config_files": report.config_files,
        "skipped": report
            .skipped
            .iter()
            .map(|entry| serde_json::json!({ "name": entry.name, "reason": entry.reason.as_str() }))
            .collect::<Vec<_>>(),
        "restore": "fndr-vault restore --from <this directory> --data-dir <destination>",
        "derived_index": "not included; rebuild it from restored SQLite truth (LanceWriter::rebuild). Keyword search works immediately; the vector route stays empty until the rebuild runs.",
    });
    let path = staging.join(BACKUP_MANIFEST_FILE);
    let mut text = serde_json::to_string_pretty(&manifest).expect("manifest is plain JSON values");
    text.push('\n');
    fs::write(&path, text).map_err(io_at(&path))
}

/// A backup's manifest, only the fields a restore acts on.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Manifest {
    database_file: String,
    created_at_ms: i64,
}

fn read_manifest(backup_dir: &Path) -> Result<Manifest, BackupError> {
    let path = backup_dir.join(BACKUP_MANIFEST_FILE);
    if !path.is_file() {
        return Err(BackupError::NotABackup(backup_dir.to_path_buf()));
    }
    let text = fs::read_to_string(&path).map_err(io_at(&path))?;
    let value: serde_json::Value = serde_json::from_str(&text).map_err(|error| {
        BackupError::ManifestInvalid {
            path: path.clone(),
            reason: error.to_string(),
        }
    })?;
    let invalid = |reason: &str| BackupError::ManifestInvalid {
        path: path.clone(),
        reason: reason.to_owned(),
    };
    if value.get("format").and_then(serde_json::Value::as_str) != Some(BACKUP_FORMAT) {
        return Err(invalid("format is not fndr.backup"));
    }
    let format_version = value
        .get("format_version")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| invalid("format_version is missing or not a number"))?;
    if format_version > BACKUP_FORMAT_VERSION {
        return Err(BackupError::FormatTooNew {
            found: format_version,
            supported: BACKUP_FORMAT_VERSION,
        });
    }
    let database_file = value
        .get("database")
        .and_then(|database| database.get("file"))
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| invalid("database.file is missing"))?
        .to_owned();
    Ok(Manifest {
        database_file,
        created_at_ms: value
            .get("created_at_ms")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or_else(now_ms),
    })
}

/// Restore a backup into `data_dir`.
///
/// Nothing at the destination is touched until the snapshot has passed its
/// integrity check, and an existing vault is never replaced without
/// `RestorePolicy::ReplaceExistingVault`. Stale WAL companions of the replaced
/// database are removed before the new file lands, because SQLite would
/// otherwise apply a WAL belonging to a different database.
///
/// The destination must not be in use: stop FNDR (which holds the data
/// directory's instance lock) before restoring into it.
pub fn restore_vault(
    backup_dir: &Path,
    data_dir: &Path,
    policy: RestorePolicy,
    layout: &VaultLayout,
) -> Result<RestoreReport, BackupError> {
    let manifest = read_manifest(backup_dir)?;
    let snapshot = backup_dir.join(&manifest.database_file);
    if !snapshot.is_file() {
        return Err(BackupError::DatabaseMissing(snapshot));
    }
    let stats = verify(&snapshot)?;

    let database = layout.database_path(data_dir);
    let replaced_vault = database.exists();
    if replaced_vault && policy == RestorePolicy::RefuseIfVaultExists {
        return Err(BackupError::VaultExists(database));
    }
    fs::create_dir_all(data_dir).map_err(io_at(data_dir))?;

    let config_files = restore_config(backup_dir, data_dir, policy)?;

    // Order matters. The stale WAL and SHM go first: after this the old
    // database is merely out of date, whereas a stale WAL left beside the new
    // file would be replayed into it.
    for sidecar in layout.database_sidecars(data_dir) {
        if sidecar.exists() {
            fs::remove_file(&sidecar).map_err(io_at(&sidecar))?;
        }
    }
    let staged = data_dir.join(format!("{}.restoring-{}", layout.database, std::process::id()));
    let _ = fs::remove_file(&staged);
    fs::copy(&snapshot, &staged).map_err(io_at(&staged))?;
    fs::rename(&staged, &database).map_err(io_at(&database))?;

    let superseded_index = supersede_index(data_dir, layout, manifest.created_at_ms)?;
    // Read the installed file back rather than trusting the copy.
    let installed = verify(&database)?;

    Ok(RestoreReport {
        data_dir: data_dir.to_path_buf(),
        database,
        schema_version: installed.schema_version,
        records: installed.records,
        chunks: installed.chunks,
        config_files,
        replaced_vault,
        superseded_index,
        index_rebuild_required: stats.chunks > 0,
    })
}

fn restore_config(
    backup_dir: &Path,
    data_dir: &Path,
    policy: RestorePolicy,
) -> Result<Vec<String>, BackupError> {
    let config_dir = backup_dir.join(BACKUP_CONFIG_DIR);
    if !config_dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut restored = Vec::new();
    for entry in fs::read_dir(&config_dir).map_err(io_at(&config_dir))? {
        let entry = entry.map_err(io_at(&config_dir))?;
        if entry.file_type().map_err(io_at(&entry.path()))?.is_dir() {
            continue;
        }
        let target = data_dir.join(entry.file_name());
        if target.exists() && policy == RestorePolicy::RefuseIfVaultExists {
            return Err(BackupError::ConfigExists(target));
        }
        fs::copy(entry.path(), &target).map_err(io_at(&target))?;
        restored.push(entry.file_name().to_string_lossy().into_owned());
    }
    restored.sort();
    Ok(restored)
}

/// Move an existing derived index aside. It was built from different truth, so
/// leaving it in place would let a restored vault answer queries from records
/// it no longer has. It is renamed rather than deleted: the bytes are
/// disposable, but deciding that for an owner is not this command's call.
fn supersede_index(
    data_dir: &Path,
    layout: &VaultLayout,
    created_at_ms: i64,
) -> Result<Option<PathBuf>, BackupError> {
    let index = data_dir.join(&layout.derived_index_dir);
    if !index.exists() {
        return Ok(None);
    }
    let mut target = data_dir.join(format!("{SUPERSEDED_INDEX_PREFIX}{created_at_ms}"));
    let mut attempt = 1;
    while target.exists() {
        target = data_dir.join(format!("{SUPERSEDED_INDEX_PREFIX}{created_at_ms}-{attempt}"));
        attempt += 1;
    }
    fs::rename(&index, &target).map_err(io_at(&target))?;
    Ok(Some(target))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "fndr-t209-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn seed(data_dir: &Path, records: i64) {
        let mut store = Store::open(&VaultLayout::default().database_path(data_dir)).unwrap();
        for index in 0..records {
            store
                .insert_capture(
                    &crate::NewRecord {
                        id: format!("r{index}"),
                        session_id: "s1".into(),
                        source: "screen".into(),
                        app_name: "Safari".into(),
                        bundle_id: None,
                        url: None,
                        window_title: format!("window {index}"),
                        captured_at_ms: 1_000 + index,
                        created_at_ms: 1_000 + index,
                    },
                    &[crate::NewChunk {
                        id: format!("c{index}"),
                        ord: 0,
                        text: format!("chunk body {index}"),
                    }],
                )
                .unwrap();
        }
    }

    #[test]
    fn backup_carries_truth_and_config_and_names_every_skip() {
        let root = scratch("backup-shape");
        let data_dir = root.join("vault");
        fs::create_dir_all(&data_dir).unwrap();
        seed(&data_dir, 3);
        fs::write(data_dir.join("settings.toml"), "capture = true\n").unwrap();
        fs::create_dir_all(data_dir.join("index")).unwrap();
        fs::write(data_dir.join("index").join("table.lance"), "derived").unwrap();
        fs::create_dir_all(data_dir.join("models")).unwrap();
        fs::write(data_dir.join(".fndr-instance.lock"), "").unwrap();

        let destination = root.join("backup-1");
        let report = backup_vault(&data_dir, &destination, &VaultLayout::default()).unwrap();

        assert_eq!(report.records, 3);
        assert_eq!(report.chunks, 3);
        assert_eq!(report.config_files, vec!["settings.toml".to_owned()]);
        assert!(destination.join("vault.sqlite3").is_file());
        assert!(
            destination
                .join(BACKUP_CONFIG_DIR)
                .join("settings.toml")
                .is_file()
        );
        assert!(destination.join(BACKUP_MANIFEST_FILE).is_file());
        // The derivative and the machine-local state are absent, and each one
        // says why rather than vanishing.
        assert!(!destination.join("index").exists());
        let reasons: Vec<(&str, BackupSkipReason)> = report
            .skipped
            .iter()
            .map(|entry| (entry.name.as_str(), entry.reason))
            .collect();
        assert!(reasons.contains(&("index", BackupSkipReason::DerivedIndex)));
        assert!(reasons.contains(&("models", BackupSkipReason::Reacquirable)));
        assert!(reasons.contains(&(".fndr-instance.lock", BackupSkipReason::RuntimeState)));
        assert!(
            reasons
                .iter()
                .any(|(name, reason)| *name == "vault.sqlite3-wal"
                    && *reason == BackupSkipReason::DatabaseSidecar)
        );

        let manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(destination.join(BACKUP_MANIFEST_FILE)).unwrap())
                .unwrap();
        assert_eq!(manifest["format"], BACKUP_FORMAT);
        assert_eq!(manifest["format_version"], BACKUP_FORMAT_VERSION);
        assert_eq!(manifest["database"]["schema_version"], report.schema_version);
        assert_eq!(manifest["contents"]["records"], 3);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn backup_refuses_an_existing_destination_and_a_missing_vault() {
        let root = scratch("backup-refusals");
        let data_dir = root.join("vault");
        fs::create_dir_all(&data_dir).unwrap();
        seed(&data_dir, 1);

        let destination = root.join("backup-1");
        backup_vault(&data_dir, &destination, &VaultLayout::default()).unwrap();
        assert!(matches!(
            backup_vault(&data_dir, &destination, &VaultLayout::default()),
            Err(BackupError::DestinationExists(_))
        ));
        assert!(matches!(
            backup_vault(&root.join("nowhere"), &root.join("b2"), &VaultLayout::default()),
            Err(BackupError::DatabaseMissing(_))
        ));
        // The refused attempt left no staging directory behind.
        assert!(!staging_sibling(&root.join("b2")).exists());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn restore_refuses_an_existing_vault_until_overwrite_is_explicit() {
        let root = scratch("restore-refusal");
        let source = root.join("source");
        fs::create_dir_all(&source).unwrap();
        seed(&source, 2);
        let backup = root.join("backup");
        backup_vault(&source, &backup, &VaultLayout::default()).unwrap();

        let destination = root.join("destination");
        fs::create_dir_all(&destination).unwrap();
        seed(&destination, 7);

        let layout = VaultLayout::default();
        assert!(matches!(
            restore_vault(
                &backup,
                &destination,
                RestorePolicy::RefuseIfVaultExists,
                &layout
            ),
            Err(BackupError::VaultExists(_))
        ));
        // The refusal changed nothing.
        let store = Store::open_read_only(&layout.database_path(&destination)).unwrap();
        drop(store);
        assert_eq!(verify(&layout.database_path(&destination)).unwrap().records, 7);

        let report = restore_vault(
            &backup,
            &destination,
            RestorePolicy::ReplaceExistingVault,
            &layout,
        )
        .unwrap();
        assert!(report.replaced_vault);
        assert_eq!(report.records, 2);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn restore_moves_a_superseded_index_aside_and_clears_stale_wal() {
        let root = scratch("restore-index");
        let source = root.join("source");
        fs::create_dir_all(&source).unwrap();
        seed(&source, 2);
        let backup = root.join("backup");
        backup_vault(&source, &backup, &VaultLayout::default()).unwrap();

        let destination = root.join("destination");
        fs::create_dir_all(&destination).unwrap();
        seed(&destination, 5);
        fs::create_dir_all(destination.join("index")).unwrap();
        fs::write(destination.join("index").join("chunks.lance"), "old rows").unwrap();
        assert!(destination.join("vault.sqlite3-wal").exists());

        let layout = VaultLayout::default();
        let report = restore_vault(
            &backup,
            &destination,
            RestorePolicy::ReplaceExistingVault,
            &layout,
        )
        .unwrap();

        let superseded = report.superseded_index.expect("index moved aside");
        assert!(superseded.join("chunks.lance").is_file());
        assert!(!destination.join("index").exists());
        assert!(!destination.join("vault.sqlite3-wal").exists());
        assert!(report.index_rebuild_required);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn restore_rejects_a_directory_that_is_not_a_backup() {
        let root = scratch("restore-not-a-backup");
        fs::create_dir_all(root.join("random")).unwrap();
        assert!(matches!(
            restore_vault(
                &root.join("random"),
                &root.join("destination"),
                RestorePolicy::RefuseIfVaultExists,
                &VaultLayout::default()
            ),
            Err(BackupError::NotABackup(_))
        ));

        let root2 = root.join("future");
        fs::create_dir_all(&root2).unwrap();
        fs::write(
            root2.join(BACKUP_MANIFEST_FILE),
            r#"{"format":"fndr.backup","format_version":99,"database":{"file":"vault.sqlite3"}}"#,
        )
        .unwrap();
        assert!(matches!(
            restore_vault(
                &root2,
                &root.join("destination"),
                RestorePolicy::RefuseIfVaultExists,
                &VaultLayout::default()
            ),
            Err(BackupError::FormatTooNew { found: 99, .. })
        ));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_corrupt_snapshot_is_never_installed() {
        let root = scratch("restore-corrupt");
        let source = root.join("source");
        fs::create_dir_all(&source).unwrap();
        seed(&source, 2);
        let backup = root.join("backup");
        backup_vault(&source, &backup, &VaultLayout::default()).unwrap();

        // Overwrite the snapshot's header with garbage: the file exists and is
        // named right, but SQLite cannot read it.
        fs::write(backup.join("vault.sqlite3"), b"not a database at all").unwrap();

        let destination = root.join("destination");
        let result = restore_vault(
            &backup,
            &destination,
            RestorePolicy::RefuseIfVaultExists,
            &VaultLayout::default(),
        );
        assert!(
            matches!(
                result,
                Err(BackupError::Sqlite(_)) | Err(BackupError::SnapshotCorrupt(_)) | Err(BackupError::Store(_))
            ),
            "a corrupt snapshot must be a typed failure"
        );
        assert!(
            !destination.join("vault.sqlite3").exists(),
            "nothing may land at the destination when the snapshot is bad"
        );

        let _ = fs::remove_dir_all(&root);
    }
}
