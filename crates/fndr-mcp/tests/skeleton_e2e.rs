//! T-109 walking skeleton, end to end and headless: a rendered screen image
//! goes through real Vision OCR, lands in the store, and comes back out of
//! the same search call the fndr.search tool serves.

use fndr_capture::{FrameSource, PngFileSource};
use fndr_mcp::{FndrMcpServer, PrivacyStatusParams, SearchParams};
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
