//! T-109 walking skeleton runner. Deliberately ugly; every stage it strings
//! together already lives in its real crate.
//!
//! One frame in, memory out, served over authenticated MCP:
//!
//! ```sh
//! # From a screenshot file (no permissions needed):
//! cargo run -p fndr-mcp --example skeleton -- --image path/to/screen.png \
//!   --store /tmp/fndr-alpha.sqlite3
//!
//! # From the live screen (grants Screen Recording to your terminal):
//! cargo run -p fndr-mcp --example skeleton
//!
//! # One-shot search instead of serving:
//! cargo run -p fndr-mcp --example skeleton -- --image x.png --query "hello"
//! ```

use fndr_capture::{FrameSource, PngFileSource, ScreencaptureCliSource};
use fndr_mcp::{FndrMcpServer, generate_token, serve_loopback};
use fndr_ocr::OcrEngine;
use fndr_privacy::{Blocklist, SafetyContext, SafetyDecision, evaluate, redact_secret_lines};
use fndr_store::SkeletonStore;

fn main() {
    tracing_subscriber::fmt().init();

    let mut image: Option<String> = None;
    let mut query: Option<String> = None;
    let mut store_path: Option<String> = None;
    let mut app_name: Option<String> = None;
    let mut url: Option<String> = None;
    let mut window_title: Option<String> = None;
    let mut blocked_apps: Vec<String> = Vec::new();
    let mut blocked_domains: Vec<String> = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--image" => image = args.next(),
            "--query" => query = args.next(),
            "--store" => store_path = args.next(),
            "--app" => app_name = args.next(),
            "--url" => url = args.next(),
            "--title" => window_title = args.next(),
            "--block-app" => {
                let Some(app) = args.next() else {
                    eprintln!("--block-app requires an app name");
                    std::process::exit(2);
                };
                blocked_apps.push(app);
            }
            "--block-domain" => {
                let Some(domain) = args.next() else {
                    eprintln!("--block-domain requires a domain");
                    std::process::exit(2);
                };
                blocked_domains.push(domain);
            }
            other => {
                eprintln!("unknown arg: {other}");
                std::process::exit(2);
            }
        }
    }

    let frame = match &image {
        Some(path) => PngFileSource { path: path.into() }.grab(),
        None => ScreencaptureCliSource.grab(),
    };
    let frame = match frame {
        Ok(frame) => frame,
        Err(e) => {
            // Typed, loud, and actionable: the invariant-4 behavior.
            eprintln!("capture failed: {e}");
            eprintln!("hint: pass --image <png> to run without screen recording permission");
            std::process::exit(1);
        }
    };
    println!("captured {} bytes of PNG", frame.png.len());

    let blocklist = Blocklist::new(&blocked_apps, &blocked_domains);
    let pre_ocr_decision = evaluate(
        SafetyContext {
            app_name: app_name.as_deref(),
            bundle_id: None,
            url: url.as_deref(),
            window_title: window_title.as_deref(),
            ocr_text: None,
        },
        &blocklist,
    );
    if let SafetyDecision::SkipStorage(reason) = pre_ocr_decision {
        eprintln!("capture skipped before OCR: {reason:?}");
        std::process::exit(3);
    }

    let engine = OcrEngine::new().expect("Vision framework unavailable");
    let recognized = engine
        .recognize_with_metadata(&frame.png)
        .expect("ocr failed");
    println!(
        "ocr: {} chars, {} blocks, confidence {:.2}",
        recognized.0.text.len(),
        recognized.0.block_count,
        recognized.0.confidence
    );

    let decision = evaluate(
        SafetyContext {
            app_name: app_name.as_deref(),
            bundle_id: None,
            url: url.as_deref(),
            window_title: window_title.as_deref(),
            ocr_text: Some(&recognized.0.text),
        },
        &blocklist,
    );
    let text = match decision {
        SafetyDecision::Allow => recognized.0.text,
        SafetyDecision::Redact(reason) => {
            let (redacted, count) = redact_secret_lines(&recognized.0.text);
            println!("redacted {count} OCR line(s): {reason:?}");
            redacted
        }
        SafetyDecision::SkipStorage(reason) => {
            eprintln!("capture skipped after OCR: {reason:?}");
            std::process::exit(3);
        }
    };

    let store = match store_path {
        Some(path) => SkeletonStore::open(std::path::Path::new(&path)),
        None => SkeletonStore::open_in_memory(),
    }
    .expect("store");
    store
        .insert_record(frame.captured_at_ms as i64, "screen", &text)
        .expect("insert");
    println!(
        "stored 1 record; total records: {}",
        store.record_count().expect("count")
    );

    if let Some(q) = query {
        for hit in store.search(&q, 10).expect("search") {
            println!("hit #{}: {}", hit.record_id, hit.snippet);
        }
        return;
    }

    let token = generate_token();
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    runtime.block_on(async move {
        let (addr, handle) = serve_loopback(FndrMcpServer::with_blocklist(store, blocklist), token.clone(), 0)
            .await
            .expect("serve");
        println!("\nMCP serving at http://{addr}/mcp");
        println!("Authorization: Bearer {token}");
        println!("\nAdd to Claude Code:");
        println!(
            "  claude mcp add fndr --transport http http://{addr}/mcp --header \"Authorization: Bearer {token}\""
        );
        println!("\nCtrl-C to stop.");
        let _ = tokio::signal::ctrl_c().await;
        handle.abort();
    });
}
