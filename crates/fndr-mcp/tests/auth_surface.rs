//! Named regression tests from the invariants checklist (ADR-007): the v1
//! audit found the MCP surface readable by any web origin with auth off.
//! These tests pin the opposite behavior at the real network boundary using
//! raw HTTP so no client library can paper over a hole.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use fndr_mcp::{FndrMcpServer, serve_loopback};
use fndr_store::SkeletonStore;

const INITIALIZE_BODY: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"0"}}}"#;

fn raw_post(addr: SocketAddr, extra_headers: &[String]) -> String {
    let mut stream = TcpStream::connect(addr).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut request = format!(
        "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n",
        addr.port(),
        INITIALIZE_BODY.len()
    );
    for header in extra_headers {
        request.push_str(header);
        request.push_str("\r\n");
    }
    request.push_str("\r\n");
    request.push_str(INITIALIZE_BODY);
    stream.write_all(request.as_bytes()).unwrap();
    let mut response = String::new();
    let _ = stream.read_to_string(&mut response);
    response
}

async fn start() -> (SocketAddr, String) {
    let store = SkeletonStore::open_in_memory().unwrap();
    let server = FndrMcpServer::new(store);
    let token = fndr_mcp::generate_token();
    let (addr, _handle) = serve_loopback(server, token.clone(), 0).await.unwrap();
    (addr, token)
}

#[tokio::test]
async fn mcp_rejects_unauthenticated_loopback() {
    let (addr, _token) = start().await;
    let response = tokio::task::spawn_blocking(move || raw_post(addr, &[]))
        .await
        .unwrap();
    assert!(
        response.starts_with("HTTP/1.1 401"),
        "expected 401, got: {}",
        response.lines().next().unwrap_or("<empty>")
    );
}

#[tokio::test]
async fn mcp_rejects_web_origin_with_valid_token() {
    let (addr, token) = start().await;
    let headers = vec![
        format!("Authorization: Bearer {token}"),
        "Origin: https://evil.example".to_string(),
    ];
    let response = tokio::task::spawn_blocking(move || raw_post(addr, &headers))
        .await
        .unwrap();
    assert!(
        response.starts_with("HTTP/1.1 403"),
        "expected 403, got: {}",
        response.lines().next().unwrap_or("<empty>")
    );
}

#[tokio::test]
async fn mcp_accepts_authenticated_initialize() {
    let (addr, token) = start().await;
    let headers = vec![format!("Authorization: Bearer {token}")];
    let response = tokio::task::spawn_blocking(move || raw_post(addr, &headers))
        .await
        .unwrap();
    assert!(
        response.starts_with("HTTP/1.1 200"),
        "expected 200, got: {}",
        response.lines().next().unwrap_or("<empty>")
    );
    assert!(
        response.contains("serverInfo"),
        "expected an initialize result body"
    );
}
