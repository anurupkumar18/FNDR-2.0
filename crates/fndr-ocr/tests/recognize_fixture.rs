//! Functional proof the ported wrapper drives Apple Vision end to end.
//! The fixture is a rendered PNG (see docs/journal for how it was made), so
//! this runs headless with no capture permission involved.

use fndr_ocr::OcrEngine;

#[test]
fn recognizes_fixture_text() {
    let png = include_bytes!("fixtures/skeleton_fixture.png");
    let engine = OcrEngine::new().expect("Vision available");
    let text = engine.recognize(png).expect("recognition succeeds");
    let lower = text.to_lowercase();
    assert!(
        lower.contains("walking skeleton"),
        "expected fixture text, got: {text:?}"
    );
    assert!(
        lower.contains("quick brown fox"),
        "expected second line, got: {text:?}"
    );
}
