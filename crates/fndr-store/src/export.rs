//! T-209: vault export.
//!
//! Export is the readable half of "the memory stays mine". A backup is a
//! restorable copy of the vault, faithful down to schema version and FTS
//! shadow tables, and it is opaque to anything that is not SQLite. An export
//! is the opposite trade: newline-delimited JSON that a person can read, `jq`
//! can filter, and another tool can ingest, at the cost of not being a restore
//! source. They are deliberately two commands because one artifact cannot be
//! both without the restore path depending on a lossy format.
//!
//! Like the backup, the export reads through a read-only connection inside one
//! read transaction, so it is safe to run while capture is writing.

use std::fs;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::backup::VaultLayout;
use crate::{Store, StoreError};

pub const EXPORT_FORMAT: &str = "fndr.export";
pub const EXPORT_FORMAT_VERSION: u64 = 1;
pub const EXPORT_MANIFEST_FILE: &str = "manifest.json";
pub const EXPORT_RECORDS_FILE: &str = "records.jsonl";
pub const EXPORT_DECISIONS_FILE: &str = "decisions.jsonl";
pub const EXPORT_README_FILE: &str = "README.md";

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("no vault database at {0}")]
    DatabaseMissing(PathBuf),
    #[error("{0} already exists; an export never overwrites, choose a new destination")]
    DestinationExists(PathBuf),
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
}

fn io_at(path: &Path) -> impl FnOnce(io::Error) -> ExportError {
    let path = path.to_path_buf();
    move |source| ExportError::Io { path, source }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportReport {
    pub destination: PathBuf,
    pub records: i64,
    pub chunks: i64,
    pub decisions: i64,
    /// Bytes of capture text written out in the clear. Named because an
    /// export puts the vault's plain content on disk outside the vault, which
    /// is exactly the point and exactly what an owner should see.
    pub text_bytes: u64,
}

/// Everything the export deliberately leaves out, with the reason. It travels
/// in the manifest so an export can never be read as "this is all FNDR kept".
const OMITTED: &[(&str, &str)] = &[
    (
        "embeddings and the Lance index",
        "derived from the text in this export and rebuilt from it, never a source of truth (ADR-002)",
    ),
    (
        "mcp_audit",
        "a log about tool calls, not about your memory; use a backup to carry it",
    ),
    (
        "result_feedback",
        "evaluation history, not memory content; use a backup to carry it",
    ),
    (
        "empty schema-v1 domains (tasks, meetings, graph)",
        "nothing writes them yet; a backup carries them the moment something does",
    ),
];

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or_default()
}

/// Write a portable, human-readable export of the vault at `data_dir` into a
/// new `destination` directory. The destination must not exist; the work is
/// staged and renamed into place, so a failure never leaves a directory that
/// looks like a complete export.
pub fn export_vault(
    data_dir: &Path,
    destination: &Path,
    layout: &VaultLayout,
) -> Result<ExportReport, ExportError> {
    let database = layout.database_path(data_dir);
    if !database.is_file() {
        return Err(ExportError::DatabaseMissing(database));
    }
    if destination.exists() {
        return Err(ExportError::DestinationExists(destination.to_path_buf()));
    }
    let staging = staging_sibling(destination);
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging).map_err(io_at(&staging))?;

    let outcome = stage_export(&database, &staging, destination).and_then(|report| {
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
        .map_or_else(|| "export".to_owned(), |name| name.to_string_lossy().into());
    let staging = format!(".{name}.partial-{}", std::process::id());
    destination
        .parent()
        .map_or_else(|| PathBuf::from(&staging), |parent| parent.join(&staging))
}

fn stage_export(
    database: &Path,
    staging: &Path,
    destination: &Path,
) -> Result<ExportReport, ExportError> {
    let store = Store::open_read_only(database)?;
    let conn = store.conn();
    // One read transaction for the whole export: every file describes the same
    // instant even while capture keeps writing.
    let snapshot = conn.unchecked_transaction()?;

    let records_path = staging.join(EXPORT_RECORDS_FILE);
    let mut records_out =
        BufWriter::new(fs::File::create(&records_path).map_err(io_at(&records_path))?);

    let mut records_statement = conn.prepare(
        "SELECT id, session_id, source, app_name, bundle_id, url, window_title,
                captured_at_ms, created_at_ms
         FROM memory_records
         ORDER BY captured_at_ms, id",
    )?;
    let mut chunks_statement =
        conn.prepare("SELECT id, ord, text FROM chunks WHERE record_id = ?1 ORDER BY ord")?;

    let mut records = 0_i64;
    let mut chunks = 0_i64;
    let mut text_bytes = 0_u64;
    let mut rows = records_statement.query([])?;
    while let Some(row) = rows.next()? {
        let record_id: String = row.get(0)?;
        let chunk_values = chunks_statement
            .query_map([&record_id], |chunk| {
                let text: String = chunk.get(2)?;
                Ok((
                    text.len() as u64,
                    serde_json::json!({
                        "chunk_id": chunk.get::<_, String>(0)?,
                        "ord": chunk.get::<_, i64>(1)?,
                        "text": text,
                    }),
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        chunks += chunk_values.len() as i64;
        text_bytes += chunk_values.iter().map(|(bytes, _)| bytes).sum::<u64>();

        let line = serde_json::json!({
            "record_id": record_id,
            "session_id": row.get::<_, String>(1)?,
            "source": row.get::<_, String>(2)?,
            "app_name": row.get::<_, String>(3)?,
            "bundle_id": row.get::<_, Option<String>>(4)?,
            "url": row.get::<_, Option<String>>(5)?,
            "window_title": row.get::<_, String>(6)?,
            "captured_at_ms": row.get::<_, i64>(7)?,
            "created_at_ms": row.get::<_, i64>(8)?,
            "chunks": chunk_values.into_iter().map(|(_, value)| value).collect::<Vec<_>>(),
        });
        writeln!(records_out, "{line}").map_err(io_at(&records_path))?;
        records += 1;
    }
    drop(rows);
    records_out.flush().map_err(io_at(&records_path))?;

    let decisions_path = staging.join(EXPORT_DECISIONS_FILE);
    let mut decisions_out =
        BufWriter::new(fs::File::create(&decisions_path).map_err(io_at(&decisions_path))?);
    let mut decisions = 0_i64;
    let mut decisions_statement =
        conn.prepare("SELECT id, decided_at_ms, statement, record_id FROM decision_ledger ORDER BY id")?;
    let mut decision_rows = decisions_statement.query([])?;
    while let Some(row) = decision_rows.next()? {
        let line = serde_json::json!({
            "id": row.get::<_, i64>(0)?,
            "decided_at_ms": row.get::<_, i64>(1)?,
            "statement": row.get::<_, String>(2)?,
            "record_id": row.get::<_, Option<String>>(3)?,
        });
        writeln!(decisions_out, "{line}").map_err(io_at(&decisions_path))?;
        decisions += 1;
    }
    drop(decision_rows);
    decisions_out.flush().map_err(io_at(&decisions_path))?;

    let schema_version = store.schema_version()?;
    // The read transaction has served its purpose; a read-only rollback is the
    // only correct end for it.
    let _ = snapshot.rollback();

    let report = ExportReport {
        destination: destination.to_path_buf(),
        records,
        chunks,
        decisions,
        text_bytes,
    };
    write_manifest(staging, database, schema_version, &report)?;
    write_readme(staging, &report)?;
    Ok(report)
}

fn write_manifest(
    staging: &Path,
    database: &Path,
    schema_version: i64,
    report: &ExportReport,
) -> Result<(), ExportError> {
    let manifest = serde_json::json!({
        "format": EXPORT_FORMAT,
        "format_version": EXPORT_FORMAT_VERSION,
        "exported_at_ms": now_ms(),
        "source_database": database.to_string_lossy(),
        "schema_version": schema_version,
        "files": {
            EXPORT_RECORDS_FILE: "one JSON object per capture record, with its chunks, oldest first",
            EXPORT_DECISIONS_FILE: "one JSON object per decision-ledger entry",
        },
        "contents": {
            "records": report.records,
            "chunks": report.chunks,
            "decisions": report.decisions,
            "text_bytes": report.text_bytes,
        },
        "omitted": OMITTED
            .iter()
            .map(|(what, why)| serde_json::json!({ "what": what, "why": why }))
            .collect::<Vec<_>>(),
        "restorable": false,
        "restore_note": "This export is for reading and for other tools. To move a vault to another machine, use `fndr-vault backup` and `fndr-vault restore`.",
    });
    let path = staging.join(EXPORT_MANIFEST_FILE);
    let mut text = serde_json::to_string_pretty(&manifest).expect("manifest is plain JSON values");
    text.push('\n');
    fs::write(&path, text).map_err(io_at(&path))
}

fn write_readme(staging: &Path, report: &ExportReport) -> Result<(), ExportError> {
    let ExportReport {
        records,
        chunks,
        decisions,
        ..
    } = report;
    let omitted = OMITTED
        .iter()
        .map(|(what, why)| format!("- **{what}**: {why}\n"))
        .collect::<String>();
    let text = format!(
        "# Your FNDR export\n\n\
         {records} capture records, {chunks} text chunks, {decisions} ledger entries, \
         written by `fndr-vault export`. Everything here is plain text on your disk. \
         Nothing was sent anywhere: FNDR has no network egress.\n\n\
         ## Files\n\n\
         - `{EXPORT_RECORDS_FILE}`: one JSON object per line, one line per captured moment, \
         oldest first. Fields: `record_id`, `session_id`, `source`, `app_name`, `bundle_id`, \
         `url` (sanitized at capture: no credentials, query, or fragment), `window_title`, \
         `captured_at_ms`, `created_at_ms`, and `chunks` (`chunk_id`, `ord`, `text`).\n\
         - `{EXPORT_DECISIONS_FILE}`: one JSON object per line from the decision ledger.\n\
         - `{EXPORT_MANIFEST_FILE}`: counts, schema version, and what this export omits.\n\n\
         ## Reading it\n\n\
         ```sh\n\
         # every app you were in\n\
         jq -r .app_name {EXPORT_RECORDS_FILE} | sort | uniq -c | sort -rn\n\n\
         # the text of one moment\n\
         jq -r 'select(.record_id == \"<id>\") | .chunks[].text' {EXPORT_RECORDS_FILE}\n\n\
         # everything mentioning a word\n\
         grep -i '<word>' {EXPORT_RECORDS_FILE} | jq -r '.captured_at_ms, .window_title'\n\
         ```\n\n\
         ## What is not here\n\n\
         {omitted}\n\
         This export is not a restore source. To move your vault to another machine, \
         take a backup (`fndr-vault backup`) and restore it there (`fndr-vault restore`).\n"
    );
    let path = staging.join(EXPORT_README_FILE);
    fs::write(&path, text).map_err(io_at(&path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NewChunk, NewRecord};

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "fndr-t209-export-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn seed(data_dir: &Path) {
        let mut store = Store::open(&VaultLayout::default().database_path(data_dir)).unwrap();
        store
            .insert_capture(
                &NewRecord {
                    id: "r1".into(),
                    session_id: "s1".into(),
                    source: "screen".into(),
                    app_name: "Safari".into(),
                    bundle_id: Some("com.apple.Safari".into()),
                    url: Some(
                        fndr_privacy::sanitize_url_for_storage("https://docs.example.com/fndr")
                            .unwrap(),
                    ),
                    window_title: "quote \" and newline \n inside".into(),
                    captured_at_ms: 1_000,
                    created_at_ms: 1_000,
                },
                &[NewChunk {
                    id: "c1".into(),
                    ord: 0,
                    text: "release checklist: sign the DMG\tthen notarize".into(),
                }],
            )
            .unwrap();
        store
            .insert_capture(
                &NewRecord {
                    id: "r2".into(),
                    session_id: "s1".into(),
                    source: "screen".into(),
                    app_name: "Terminal".into(),
                    bundle_id: None,
                    url: None,
                    window_title: "zsh".into(),
                    captured_at_ms: 2_000,
                    created_at_ms: 2_000,
                },
                &[NewChunk {
                    id: "c2".into(),
                    ord: 0,
                    text: "cargo test -p fndr-store".into(),
                }],
            )
            .unwrap();
        store.remember_decision(3_000, "we ship the backup command", None).unwrap();
    }

    #[test]
    fn export_writes_readable_jsonl_that_survives_awkward_text() {
        let root = scratch("shape");
        let data_dir = root.join("vault");
        fs::create_dir_all(&data_dir).unwrap();
        seed(&data_dir);

        let destination = root.join("export-1");
        let report = export_vault(&data_dir, &destination, &VaultLayout::default()).unwrap();
        assert_eq!(report.records, 2);
        assert_eq!(report.chunks, 2);
        assert_eq!(report.decisions, 1);
        assert!(report.text_bytes > 0);

        let lines: Vec<serde_json::Value> =
            fs::read_to_string(destination.join(EXPORT_RECORDS_FILE))
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str(line).expect("every line is valid JSON"))
                .collect();
        assert_eq!(lines.len(), 2);
        // Oldest first, and the awkward characters round-trip exactly.
        assert_eq!(lines[0]["record_id"], "r1");
        assert_eq!(lines[0]["window_title"], "quote \" and newline \n inside");
        assert_eq!(
            lines[0]["chunks"][0]["text"],
            "release checklist: sign the DMG\tthen notarize"
        );
        assert_eq!(lines[0]["url"], "https://docs.example.com/fndr");
        assert_eq!(lines[1]["bundle_id"], serde_json::Value::Null);

        let decisions: Vec<serde_json::Value> =
            fs::read_to_string(destination.join(EXPORT_DECISIONS_FILE))
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
        assert_eq!(decisions[0]["statement"], "we ship the backup command");

        let manifest: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(destination.join(EXPORT_MANIFEST_FILE)).unwrap(),
        )
        .unwrap();
        assert_eq!(manifest["format"], EXPORT_FORMAT);
        assert_eq!(manifest["restorable"], false);
        assert_eq!(manifest["contents"]["records"], 2);
        assert_eq!(
            manifest["omitted"].as_array().map(Vec::len),
            Some(OMITTED.len())
        );
        assert!(
            fs::read_to_string(destination.join(EXPORT_README_FILE))
                .unwrap()
                .contains("not a restore source")
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn export_refuses_an_existing_destination_and_a_missing_vault() {
        let root = scratch("refusals");
        let data_dir = root.join("vault");
        fs::create_dir_all(&data_dir).unwrap();
        seed(&data_dir);
        let destination = root.join("export-1");
        export_vault(&data_dir, &destination, &VaultLayout::default()).unwrap();
        assert!(matches!(
            export_vault(&data_dir, &destination, &VaultLayout::default()),
            Err(ExportError::DestinationExists(_))
        ));
        assert!(matches!(
            export_vault(
                &root.join("nowhere"),
                &root.join("export-2"),
                &VaultLayout::default()
            ),
            Err(ExportError::DatabaseMissing(_))
        ));
        assert!(!root.join("export-2").exists());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn an_empty_vault_exports_empty_files_rather_than_failing() {
        let root = scratch("empty");
        let data_dir = root.join("vault");
        fs::create_dir_all(&data_dir).unwrap();
        drop(Store::open(&VaultLayout::default().database_path(&data_dir)).unwrap());

        let destination = root.join("export-1");
        let report = export_vault(&data_dir, &destination, &VaultLayout::default()).unwrap();
        assert_eq!(report.records, 0);
        assert_eq!(
            fs::read_to_string(destination.join(EXPORT_RECORDS_FILE)).unwrap(),
            ""
        );

        let _ = fs::remove_dir_all(&root);
    }
}
