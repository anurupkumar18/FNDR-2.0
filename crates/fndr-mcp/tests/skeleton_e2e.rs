//! T-109 walking skeleton, end to end and headless: a rendered screen image
//! goes through real Vision OCR, lands in the store, and comes back out of
//! the same search call the fndr.search tool serves.

use fndr_capture::{FrameSource, PngFileSource};
use fndr_mcp::{FndrMcpServer, SearchParams};
use fndr_ocr::OcrEngine;
use fndr_store::SkeletonStore;
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

    let store = SkeletonStore::open_in_memory().unwrap();
    store
        .insert_record(frame.captured_at_ms as i64, "screen", &recognized)
        .unwrap();

    let hits = store.search("skeleton", 10).unwrap();
    assert_eq!(hits.len(), 1, "stored frame must be findable");
    assert!(hits[0].snippet.to_lowercase().contains("[skeleton]"));

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
