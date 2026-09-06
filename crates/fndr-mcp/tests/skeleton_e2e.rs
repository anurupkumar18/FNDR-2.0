//! T-109 walking skeleton, end to end and headless: a rendered screen image
//! goes through real Vision OCR, lands in the store, and comes back out of
//! the same search call the fndr.search tool serves.

use fndr_capture::{FrameSource, PngFileSource};
use fndr_mcp::{
    FndrMcpServer, PrivacyStatusParams, RememberDecisionParams, SearchParams, SourceEvidenceParams,
    TimelineGrain, TimelineParams,
};
use fndr_ocr::OcrEngine;
use fndr_privacy::Blocklist;
use fndr_retrieval::KeywordRetriever;
use fndr_store::{NewChunk, NewRecord, SkeletonStore, Store};
use rmcp::handler::server::wrapper::Parameters;

#[test]
fn capture_ocr_store_search_round_trip() {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../fndr-ocr/tests/fixtures/skeleton_fixture.png");
    let frame = PngFileSource { path: fixture }
        .grab()
        .expect("fixture frame");

    let engine = OcrEngine::new().expect("Vision available");
    let recognized = engine.recognize(&frame.png).expect("ocr");
    assert!(recognized.to_lowercase().contains("walking skeleton"));

    let mut store = Store::open_in_memory().unwrap();
    store
        .insert_capture(
            &NewRecord {
                id: "r1".into(),
                session_id: "s1".into(),
                source: "screen".into(),
                app_name: "Finder".into(),
                bundle_id: None,
                url: None,
                window_title: "skeleton e2e".into(),
                captured_at_ms: frame.captured_at_ms as i64,
                created_at_ms: frame.captured_at_ms as i64,
            },
            &[NewChunk {
                id: "c1".into(),
                ord: 0,
                text: recognized.clone(),
            }],
        )
        .unwrap();

    let hits = KeywordRetriever::new(&store)
        .search("skeleton", 10)
        .unwrap();
    assert_eq!(hits.len(), 1, "stored frame must be findable");

    // The very same path the MCP tool serves.
    let server = FndrMcpServer::new(store);
    let result = server
        .search(Parameters(SearchParams {
            query: "quick brown fox".into(),
            limit: None,
        }))
        .expect("tool call");
    assert_eq!(result.0.hits.len(), 1);
    assert_eq!(result.0.hits[0].source, "screen");
}

#[test]
fn privacy_status_reports_posture_without_exposing_entries() {
    let server = FndrMcpServer::with_blocklist(
        Store::open_in_memory().unwrap(),
        Blocklist::new(&["Figma", "1Password"], &["bank.com"]),
    );

    let status = server
        .privacy_status(Parameters(PrivacyStatusParams {}))
        .expect("privacy status tool call")
        .0;

    assert!(status.local_default);
    assert!(!status.planner_enabled);
    assert_eq!(status.configured_blocked_apps, 2);
    assert_eq!(status.configured_blocked_domains, 1);
    assert!(!status.raw_pixels_persisted);
}

fn store_with_one_record(text: &str) -> Store {
    let mut store = Store::open_in_memory().unwrap();
    store
        .insert_capture(
            &NewRecord {
                id: "r1".into(),
                session_id: "s1".into(),
                source: "screen".into(),
                app_name: "Safari".into(),
                bundle_id: None,
                url: None,
                window_title: "evidence".into(),
                captured_at_ms: 42,
                created_at_ms: 42,
            },
            &[NewChunk {
                id: "c1".into(),
                ord: 0,
                text: text.into(),
            }],
        )
        .unwrap();
    store
}

#[test]
fn timeline_reports_activity_counts_and_never_capture_text() {
    let secret = "a private sentence that must not appear in a timeline";
    let server = FndrMcpServer::new(store_with_one_record(secret));

    let timeline = server
        .timeline(Parameters(TimelineParams {
            from_ms: 0,
            to_ms: 1_000_000,
            granularity: Some(TimelineGrain::Hour),
            utc_offset_minutes: None,
            limit: None,
        }))
        .expect("tool call")
        .0;

    assert_eq!(timeline.granularity, "hour");
    assert!(!timeline.truncated);
    assert_eq!(timeline.buckets.len(), 1);
    assert_eq!(timeline.buckets[0].app_name, "Safari");
    assert_eq!(timeline.buckets[0].record_count, 1);

    let rendered = serde_json::to_string(&timeline).expect("serializes");
    assert!(
        !rendered.contains("private sentence"),
        "a timeline must carry counts, never capture text"
    );
}

#[test]
fn timeline_refuses_a_backwards_window_and_an_impossible_offset() {
    let server = FndrMcpServer::new(store_with_one_record("anything"));

    let backwards = server.timeline(Parameters(TimelineParams {
        from_ms: 500,
        to_ms: 100,
        granularity: None,
        utc_offset_minutes: None,
        limit: None,
    }));
    assert!(backwards.is_err(), "to_ms before from_ms must be refused");

    let bad_offset = server.timeline(Parameters(TimelineParams {
        from_ms: 0,
        to_ms: 500,
        granularity: None,
        utc_offset_minutes: Some(5_000),
        limit: None,
    }));
    assert!(
        bad_offset.is_err(),
        "an impossible UTC offset must be refused"
    );
}

#[test]
fn source_evidence_withholds_capture_text_until_include_raw_is_explicit() {
    let secret = "the quiet part written down";
    let server = FndrMcpServer::new(store_with_one_record(secret));

    let gated = server
        .source_evidence(Parameters(SourceEvidenceParams {
            record_id: "r1".into(),
            include_raw: None,
        }))
        .expect("tool call")
        .0;
    assert!(!gated.raw_included);
    assert_eq!(gated.app_name, "Safari");
    assert_eq!(gated.chunks.len(), 1);
    assert_eq!(gated.chunks[0].text_len, secret.len() as u32);
    assert_eq!(
        gated.chunks[0].text, None,
        "capture text must not cross the surface by default"
    );

    let opened = server
        .source_evidence(Parameters(SourceEvidenceParams {
            record_id: "r1".into(),
            include_raw: Some(true),
        }))
        .expect("tool call")
        .0;
    assert!(opened.raw_included);
    assert_eq!(opened.chunks[0].text.as_deref(), Some(secret));
}

#[test]
fn source_evidence_for_an_unknown_record_is_a_typed_refusal() {
    let server = FndrMcpServer::new(Store::open_in_memory().unwrap());
    let result = server.source_evidence(Parameters(SourceEvidenceParams {
        record_id: "nope".into(),
        include_raw: Some(true),
    }));
    assert!(
        result.is_err(),
        "an unknown record must not return an empty success"
    );
}

#[test]
fn remember_decision_appends_and_rejects_an_empty_statement() {
    let server = FndrMcpServer::new(Store::open_in_memory().unwrap());

    let recorded = server
        .remember_decision(Parameters(RememberDecisionParams {
            statement: "keep fndr.search on the durable store".into(),
            record_id: None,
            decided_at_ms: Some(1_000),
        }))
        .expect("tool call")
        .0;
    assert_eq!(recorded.decided_at_ms, 1_000);

    let rejected = server.remember_decision(Parameters(RememberDecisionParams {
        statement: "   ".into(),
        record_id: None,
        decided_at_ms: None,
    }));
    assert!(rejected.is_err(), "an empty statement must not be recorded");
}

#[test]
fn raw_png_bytes_are_absent_from_every_skeleton_store_artifact() {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../fndr-ocr/tests/fixtures/skeleton_fixture.png");
    let frame = PngFileSource { path: fixture }
        .grab()
        .expect("fixture frame");
    let recognized = OcrEngine::new()
        .expect("Vision available")
        .recognize(&frame.png)
        .expect("ocr");

    let database_path = std::env::temp_dir().join(format!(
        "fndr-no-pixels-{}-{}.sqlite3",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("wall clock after Unix epoch")
            .as_nanos()
    ));

    {
        let store = SkeletonStore::open(&database_path).expect("open persistent store");
        store
            .insert_record(frame.captured_at_ms as i64, "screen", &recognized)
            .expect("store OCR text only");
    }

    for suffix in ["", "-wal", "-shm"] {
        let path = format!("{}{}", database_path.display(), suffix);
        if let Ok(bytes) = std::fs::read(&path) {
            assert!(
                !bytes
                    .windows(frame.png.len())
                    .any(|window| window == frame.png.as_slice()),
                "raw PNG bytes leaked into {}",
                path
            );
        }
        let _ = std::fs::remove_file(path);
    }
}
