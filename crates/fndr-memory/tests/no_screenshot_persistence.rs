//! T-805: a CI-enforced, adversarial proof that no path through the real
//! write path ever persists screenshot pixel bytes, or a live reference to a
//! screenshot file, into durable storage.
//!
//! ADR-004's data-at-rest posture already claims "no raw screenshot
//! persistence, test-asserted, carried from POC ADR-004"; before this file
//! the claim held only because no persisted type happens to declare an
//! image field, with nothing in CI that would notice a regression. This
//! suite exercises the real `fndr_memory::persist_capture` write path (the
//! one function that turns an assembled capture into a stored `Store`
//! record, per `docs/ARCHITECTURE.md` section 4.1 stage 8) and the real
//! url-only admission classifier (`fndr_capture::
//! classify_capture_surface_policy`, `CaptureSurfacePolicy::UrlOnly`,
//! T-304) against a real, on-disk SQLite database, then scans that database
//! for image signatures and dereferenced file content -- independently of
//! `Store`'s own read methods, via a fresh `rusqlite::Connection` opened
//! straight onto the file it wrote, plus a raw byte scan of the file(s)
//! themselves.
//!
//! Scope, stated plainly per the ticket: this suite covers the real capture
//! -> OCR -> privacy gate -> persist pipeline (`persist_capture`, exercised
//! here for a normal capture, a redacted capture, and a skipped capture) and
//! the real url-only admission surface. It does NOT cover an autofill
//! capture path: no such path exists anywhere in this codebase today
//! (checked with `rg -i autofill` across `crates/` before writing this
//! suite -- zero hits outside this comment), so there is nothing to
//! exercise. When an autofill capture path is built, it needs its own case
//! here before T-805 can be considered closed against the AC's literal
//! text; until then this gap is real, not silently dropped.
//!
//! What this suite complements rather than duplicates: `fndr-privacy`'s
//! `write_path_never_persists_browser_url_query_or_fragment` (in
//! `fndr-memory/src/write_path.rs`) already proves URL sanitization in
//! isolation. This suite is about a different, orthogonal property --
//! pixel/path absence across every stored column, on disk, under adversarial
//! content -- and reuses that sanitization as a building block rather than
//! re-testing it.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use fndr_capture::{CaptureSurfacePolicy, classify_capture_surface_policy};
use fndr_memory::{CaptureForPersistence, PersistCaptureOutcome, persist_capture};
use fndr_privacy::{Blocklist, SafetyReason, sanitize_url_for_storage};
use fndr_store::{NewChunk, NewRecord, Store};
use rusqlite::Connection;

/// A real PNG raster signature. If this sequence ever shows up in stored
/// text, or anywhere in the store's on-disk bytes, something upstream of
/// persistence stopped treating captured content as OCR text and started
/// treating it as image bytes.
const PNG_MAGIC: &[u8] = b"\x89PNG\r\n\x1a\n";

/// A JPEG start-of-image marker, checked for the same reason as `PNG_MAGIC`.
const JPEG_MAGIC: &[u8] = &[0xFF, 0xD8, 0xFF];

/// Every table `crates/fndr-store/src/migrations/*.sql` declares, excluding
/// the FTS5 virtual table (`chunks_fts`) and its shadow tables: their
/// segment storage is opaque compressed binary derived from `chunks.text`,
/// not itself a distinct persisted domain column, and is not valid UTF-8 to
/// read back as `TEXT`. This list is the source of truth this suite scans;
/// `fndr-store`'s own `fresh_database_migrates_to_latest` test pins the same
/// set, so the two drift together if a migration ever adds a table.
const DOMAIN_TABLES: &[&str] = &[
    "memory_records",
    "memory_texts",
    "memory_scores",
    "chunks",
    "graph_nodes",
    "graph_edges",
    "node_mentions",
    "entity_aliases",
    "tasks",
    "meetings",
    "meeting_segments",
    "decision_ledger",
    "review_queue",
    "settings",
    "devices",
    "tokens",
    "mcp_audit",
    "result_feedback",
];

/// A fresh on-disk database path for one test, never shared across tests in
/// the same process. Lessons learned 2026-09-06: a raw timestamp alone is
/// not a per-run-unique id when tests run concurrently on separate threads
/// in one process; pairing a per-test name with a process-wide counter is.
fn scratch_db_path(name: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "fndr-t805-{name}-{}-{unique}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir.join("fndr.sqlite3")
}

/// Open a connection onto the same on-disk file `Store` wrote, entirely
/// independent of `Store`'s own (curated) read methods. This is the point:
/// a bug that only manifests through raw column content, not through one of
/// `Store`'s typed accessors, must still be caught. The `Store` that wrote
/// the file must already be dropped so nothing races the file underneath a
/// live writer connection.
fn reopen_raw(path: &Path) -> Connection {
    Connection::open(path).expect("reopen the store's own sqlite file directly")
}

/// `(name, declared_type)` for every column of one table, straight from
/// SQLite's own catalog rather than from any Rust-side schema description.
fn table_columns(conn: &Connection, table: &str) -> Vec<(String, String)> {
    let mut statement = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .unwrap();
    statement
        .query_map([], |row| Ok((row.get::<_, String>(1)?, row.get::<_, String>(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// Whole-schema structural guarantee: no declared column, in any domain
/// table, is a BLOB. Pixel bytes need somewhere binary to live; if this ever
/// fails, a migration introduced exactly the kind of column T-805 exists to
/// keep out, and every other check in this file would need a matching BLOB
/// scan to stay meaningful.
fn assert_schema_has_no_blob_column(conn: &Connection) {
    for table in DOMAIN_TABLES {
        for (column, declared_type) in table_columns(conn, table) {
            assert!(
                !declared_type.eq_ignore_ascii_case("blob"),
                "{table}.{column} is declared BLOB; pixel bytes would now have \
                 somewhere to live"
            );
        }
    }
}

/// Every value from every TEXT-typed column, across every domain table, as
/// one flat list. This is the adversarial scan surface: instead of trusting
/// `capture_metadata`/`pending_chunks`/`record_evidence` (which only look at
/// the columns their own callers already expect) to have shown us
/// everything, this walks the catalog and reads every declared TEXT column
/// SQLite actually has.
fn dump_all_text_columns(conn: &Connection) -> Vec<(String, String)> {
    let mut values = Vec::new();
    for table in DOMAIN_TABLES {
        for (column, declared_type) in table_columns(conn, table) {
            if !declared_type.eq_ignore_ascii_case("text") {
                continue;
            }
            let sql = format!("SELECT {column} FROM {table} WHERE {column} IS NOT NULL");
            let mut statement = conn.prepare(&sql).unwrap();
            values.extend(
                statement
                    .query_map([], |row| row.get::<_, String>(0))
                    .unwrap()
                    .map(|value| (format!("{table}.{column}"), value.unwrap())),
            );
        }
    }
    values
}

/// Total row count across every domain table, used to prove a skipped
/// capture leaves the store completely untouched rather than merely absent
/// from the one or two tables a narrower check might think to look at.
fn total_row_count(conn: &Connection) -> i64 {
    DOMAIN_TABLES
        .iter()
        .map(|table| {
            conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap()
        })
        .sum()
}

/// Scans the store's main database file plus its WAL/SHM/rollback-journal
/// siblings (whichever exist) for raster magic bytes, at the raw byte level.
/// This catches anything a column-by-column SQL scan could miss (a stray
/// BLOB, an uncommitted WAL frame, page slack) and is the strongest form of
/// "nowhere in the SQLite store" this suite can assert without a fuzzer.
fn assert_no_raster_bytes_on_disk(db_path: &Path) {
    let mut candidates = vec![db_path.to_path_buf()];
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut sibling = db_path.as_os_str().to_owned();
        sibling.push(suffix);
        candidates.push(PathBuf::from(sibling));
    }
    for path in candidates {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        assert!(
            !contains_subslice(&bytes, PNG_MAGIC),
            "PNG magic bytes found in {}: pixel bytes reached the store's on-disk file",
            path.display()
        );
        assert!(
            !contains_subslice(&bytes, JPEG_MAGIC),
            "JPEG SOI marker found in {}: pixel bytes reached the store's on-disk file",
            path.display()
        );
    }
}

fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && haystack.windows(needle.len()).any(|window| window == needle)
}

fn blocklist() -> Blocklist {
    Blocklist::default()
}

// ---------------------------------------------------------------------
// Structural check (necessary, not sufficient on its own): the types that
// cross into persistence simply have no field a pixel could occupy.
// ---------------------------------------------------------------------

#[test]
fn structural_persisted_types_have_no_pixel_or_image_path_field() {
    // Every field of `NewRecord`, `NewChunk` (what `Store::insert_capture`
    // actually writes) and `CaptureForPersistence` (what a caller hands the
    // write path) named explicitly, with no `..Default::default()` filler.
    // Adding a `pixels: Vec<u8>`, `png: Vec<u8>`, or `image_path: PathBuf`
    // field to any of the three breaks this literal at compile time: the
    // only way pixel bytes could cross this boundary is for it to be
    // representable at all, and today it is not representable in any of
    // them. The behavioral tests below are what make this guarantee
    // adversarial rather than a tautology about field names.
    let record = NewRecord {
        id: "structural-record".into(),
        session_id: "structural-session".into(),
        source: "screen".into(),
        app_name: "Structural Probe".into(),
        bundle_id: None,
        url: None,
        window_title: "probe".into(),
        captured_at_ms: 0,
        created_at_ms: 0,
    };
    let chunk = NewChunk {
        id: "structural-chunk".into(),
        ord: 0,
        text: "probe text".into(),
    };
    let capture = CaptureForPersistence {
        record_id: "structural-record",
        session_id: "structural-session",
        chunk_id: "structural-chunk",
        source: "screen",
        app_name: "Structural Probe",
        bundle_id: None,
        url: None,
        window_title: "probe",
        ocr_text: "probe text",
        captured_at_ms: 0,
        created_at_ms: 0,
    };

    assert_eq!(record.id, "structural-record");
    assert_eq!(chunk.text, "probe text");
    assert_eq!(capture.ocr_text, "probe text");
}

#[test]
fn domain_schema_declares_no_blob_column_anywhere() {
    let path = scratch_db_path("schema-blob-scan");
    drop(Store::open(&path).unwrap());
    let conn = reopen_raw(&path);
    assert_schema_has_no_blob_column(&conn);
}

// ---------------------------------------------------------------------
// Adversarial case 1: a normal OCR capture whose text itself names a real
// screenshot file (the same temp-capture-file naming convention
// `fndr-capture`'s own `ScreenCaptureKitSource`/`TempPng` uses:
// `fndr-sck-<pid>-<millis>.png` under the OS temp dir) and includes a
// base64-looking blob line -- the "terminal screenshot showing a path"
// scenario named in the ticket. The file at that path is real, with real
// PNG magic bytes, so if anything downstream ever treated the OCR text as a
// live reference and dereferenced it, this test would catch it.
// ---------------------------------------------------------------------

#[test]
fn normal_capture_stores_a_lookalike_path_and_blob_as_opaque_text_never_as_a_live_reference() {
    let db_path = scratch_db_path("normal-capture-adversarial");

    // A real file, at the real naming convention, with real PNG bytes.
    let phantom_screenshot = std::env::temp_dir().join(format!(
        "fndr-sck-{}-{}.png",
        std::process::id(),
        9_999_999_999_u64
    ));
    let mut phantom_bytes = PNG_MAGIC.to_vec();
    phantom_bytes.extend_from_slice(b"definitely-real-pixel-data-not-ocr-text");
    std::fs::write(&phantom_screenshot, &phantom_bytes).unwrap();

    let ocr_text = format!(
        "Terminal\n$ ls -la ~/Library/Caches/fndr\n{}\nbase64: iVBORw0KGgoAAAANSUhEUgAAAAEAAAAB",
        phantom_screenshot.display()
    );

    {
        let mut store = Store::open(&db_path).unwrap();
        let outcome = persist_capture(
            &mut store,
            CaptureForPersistence {
                record_id: "record-adversarial-path",
                session_id: "session-a",
                chunk_id: "chunk-adversarial-path",
                source: "screen",
                app_name: "Terminal",
                bundle_id: Some("com.apple.Terminal"),
                url: None,
                window_title: "zsh",
                ocr_text: &ocr_text,
                captured_at_ms: 1_000,
                created_at_ms: 1_000,
            },
            &blocklist(),
        )
        .unwrap();
        assert_eq!(
            outcome,
            PersistCaptureOutcome::Stored {
                record_id: "record-adversarial-path".to_owned(),
                redaction_count: 0,
            }
        );

        // Stored as opaque text, verbatim: the path-looking line and the
        // base64-looking line survive unmodified through the real store
        // accessor, proving nothing rewrote or specially interpreted them.
        let evidence = store
            .record_evidence("record-adversarial-path")
            .unwrap()
            .expect("record exists");
        assert_eq!(evidence.chunks.len(), 1);
        assert_eq!(evidence.chunks[0].text, ocr_text);
    }

    // The independent raw scan: the exact OCR text (including the path and
    // blob lines) is present as ordinary chunk text, and nowhere does the
    // real screenshot file's actual pixel bytes appear.
    let conn = reopen_raw(&db_path);
    let text_values = dump_all_text_columns(&conn);
    assert!(
        text_values
            .iter()
            .any(|(_, value)| value == &ocr_text),
        "the adversarial OCR text must be retained verbatim somewhere in the store"
    );
    assert_no_raster_bytes_on_disk(&db_path);

    // The phantom file itself was never opened by the write path: it is
    // still exactly the bytes this test wrote, proving `persist_capture`
    // never dereferenced the path it happened to find inside OCR text.
    assert_eq!(std::fs::read(&phantom_screenshot).unwrap(), phantom_bytes);

    let _ = std::fs::remove_file(&phantom_screenshot);
}

// ---------------------------------------------------------------------
// Adversarial case 2: the url-only admission surface named explicitly in
// the AC. Uses the real `classify_capture_surface_policy` to reach
// `CaptureSurfacePolicy::UrlOnly` for a real listing-page fixture, then
// persists through the real write path with a raw URL carrying a secret
// query parameter and fragment, mirroring exactly what
// `fndr-shell`'s `StoreCaptureSink::persist_url_only` composes in
// production (title + sanitized URL) so this exercises the write path with
// realistic input rather than a synthetic shortcut.
// ---------------------------------------------------------------------

#[test]
fn url_only_admission_path_persists_sanitized_metadata_only_never_the_query_fragment_or_pixels() {
    let app_name = "Google Chrome";
    let window_title = "screen_pipe - YouTube";
    let raw_url =
        "https://www.youtube.com/@screen_pipe/videos?utm_secret=leak-me#fragment-secret";

    // The real admission classifier, not a stand-in: a channel-listing page
    // is UrlOnly regardless of the query string riding along with it. This
    // function's signature (app name, window title, URL) has no pixel
    // parameter at all -- there is nothing here a frame could travel through.
    let policy = classify_capture_surface_policy(app_name, window_title, Some(raw_url));
    assert_eq!(policy, CaptureSurfacePolicy::UrlOnly);

    let sanitized = sanitize_url_for_storage(raw_url).expect("http(s) url sanitizes");
    assert!(!sanitized.as_str().contains("leak-me"));
    assert!(!sanitized.as_str().contains("fragment-secret"));

    // Mirrors fndr-shell's StoreCaptureSink::persist_url_only composition
    // (title + sanitized URL, source "browser_url_only") without depending
    // on the shell crate, which the engine must never do (ADR-001).
    let composed_text = format!("{window_title}\n{}", sanitized.as_str());

    let db_path = scratch_db_path("url-only-admission");
    {
        let mut store = Store::open(&db_path).unwrap();
        let outcome = persist_capture(
            &mut store,
            CaptureForPersistence {
                record_id: "record-url-only",
                session_id: "session-b",
                chunk_id: "chunk-url-only",
                source: "browser_url_only",
                app_name,
                bundle_id: Some("com.google.Chrome"),
                url: Some(raw_url),
                window_title,
                ocr_text: &composed_text,
                captured_at_ms: 2_000,
                created_at_ms: 2_000,
            },
            &blocklist(),
        )
        .unwrap();
        assert_eq!(
            outcome,
            PersistCaptureOutcome::Stored {
                record_id: "record-url-only".to_owned(),
                redaction_count: 0,
            }
        );

        assert_eq!(
            store.capture_metadata("record-url-only").unwrap(),
            Some(fndr_store::CaptureMetadata {
                bundle_id: Some("com.google.Chrome".into()),
                url: Some(sanitized.as_str().to_owned()),
            })
        );
    }

    // The independent raw scan: the secret query value and fragment must
    // not survive anywhere in the store, not just absent from the url
    // column specifically.
    let conn = reopen_raw(&db_path);
    let text_values = dump_all_text_columns(&conn);
    for (location, value) in &text_values {
        assert!(
            !value.contains("leak-me"),
            "{location} retained the secret query value: {value}"
        );
        assert!(
            !value.contains("fragment-secret"),
            "{location} retained the secret fragment: {value}"
        );
    }
    assert_no_raster_bytes_on_disk(&db_path);
}

// ---------------------------------------------------------------------
// Adversarial case 3: a redacted capture. Mixes a genuine secret-pattern
// line with a lookalike path line and a lookalike base64 line in the same
// OCR text, so the test can tell "the safety gate redacted the one line it
// actually matched" apart from "the pipeline nuked everything that looked
// suspicious" or "the pipeline quietly treated the lookalike content as
// something to dereference".
// ---------------------------------------------------------------------

#[test]
fn redacted_capture_scrubs_only_the_matched_secret_line_and_keeps_other_content_opaque() {
    let db_path = scratch_db_path("redacted-capture");
    let secret_value = "sk-adversarial-do-not-persist-me";
    let ocr_text = format!(
        "deploy notes\napi_key: {secret_value}\nscreenshot saved to /tmp/fndr-sck-4242-777.png\nbase64: iVBORw0KGgoAAAANSUhEUgAAAAEAAAAB\nend of notes"
    );

    {
        let mut store = Store::open(&db_path).unwrap();
        let outcome = persist_capture(
            &mut store,
            CaptureForPersistence {
                record_id: "record-redacted",
                session_id: "session-c",
                chunk_id: "chunk-redacted",
                source: "screen",
                app_name: "Terminal",
                bundle_id: Some("com.apple.Terminal"),
                url: None,
                window_title: "zsh",
                ocr_text: &ocr_text,
                captured_at_ms: 3_000,
                created_at_ms: 3_000,
            },
            &blocklist(),
        )
        .unwrap();
        assert_eq!(
            outcome,
            PersistCaptureOutcome::Stored {
                record_id: "record-redacted".to_owned(),
                redaction_count: 1,
            }
        );

        let stored_text = &store.pending_chunks(10).unwrap()[0].text;
        assert!(!stored_text.contains(secret_value));
        assert!(
            stored_text.contains("/tmp/fndr-sck-4242-777.png"),
            "the lookalike path line is not a secret pattern and must survive as opaque text"
        );
        assert!(
            stored_text.contains("iVBORw0KGgoAAAANSUhEUgAAAAEAAAAB"),
            "the lookalike base64 line is not a secret pattern and must survive as opaque text"
        );
    }

    let conn = reopen_raw(&db_path);
    let text_values = dump_all_text_columns(&conn);
    assert!(
        text_values
            .iter()
            .all(|(_, value)| !value.contains(secret_value)),
        "the matched secret value must not survive anywhere in the store"
    );
    assert_no_raster_bytes_on_disk(&db_path);
}

// ---------------------------------------------------------------------
// Adversarial case 4: a skipped capture. Even when the OCR text itself
// carries a lookalike path and a lookalike blob, a pre-OCR SkipStorage
// decision (password manager) must leave the store completely untouched --
// not "absent from the two tables a narrower check would think to look at",
// but zero rows anywhere.
// ---------------------------------------------------------------------

#[test]
fn skipped_capture_persists_nothing_anywhere_even_with_adversarial_ocr_text() {
    let db_path = scratch_db_path("skipped-capture");
    let ocr_text = format!(
        "vault contents\nscreenshot saved to {}\nbase64: {}",
        std::env::temp_dir()
            .join(format!("fndr-sck-{}-1234.png", std::process::id()))
            .display(),
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAAB"
    );

    {
        let mut store = Store::open(&db_path).unwrap();
        let outcome = persist_capture(
            &mut store,
            CaptureForPersistence {
                record_id: "record-skipped",
                session_id: "session-d",
                chunk_id: "chunk-skipped",
                source: "screen",
                app_name: "1Password",
                bundle_id: Some("com.1password.1password"),
                url: None,
                window_title: "Vault",
                ocr_text: &ocr_text,
                captured_at_ms: 4_000,
                created_at_ms: 4_000,
            },
            &blocklist(),
        )
        .unwrap();
        assert_eq!(
            outcome,
            PersistCaptureOutcome::Skipped {
                reason: SafetyReason::PasswordManager,
            }
        );
    }

    let conn = reopen_raw(&db_path);
    assert_eq!(
        total_row_count(&conn),
        0,
        "a skipped capture must leave every domain table empty, not just the ones a \
         narrower check happens to look at"
    );
    assert_no_raster_bytes_on_disk(&db_path);
}
