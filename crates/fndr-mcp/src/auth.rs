//! Auth-always middleware (ADR-007, invariant 2). Every request to the MCP
//! surface passes through here before rmcp sees it. The v1 failure this
//! prevents: a default-local MCP served the whole memory store to any web
//! origin because auth was a follow-up that never followed.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use axum::body::Body;
use axum::http::{Request, Response, StatusCode};
use subtle::ConstantTimeEq;

/// 32 random bytes, hex-encoded. The caller decides where it lives (the
/// skeleton prints it; the real shell stores it owner-only per ADR-007).
pub fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("OS RNG unavailable");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Clone)]
pub struct AuthConfig {
    pub token: String,
    /// Exact Host header values accepted (loopback binds only for now).
    pub allowed_hosts: Vec<String>,
    /// Origin values accepted when the header is present. Empty means any
    /// request carrying an Origin header (a browser) is rejected outright.
    pub allowed_origins: Vec<String>,
    /// Global requests-per-second cap. Crude by design for the skeleton;
    /// per-scope limits come with the full surface (E07).
    pub max_requests_per_second: u64,
}

impl AuthConfig {
    pub fn loopback(token: String, port: u16) -> Self {
        Self {
            token,
            allowed_hosts: vec![
                format!("127.0.0.1:{port}"),
                format!("localhost:{port}"),
                format!("[::1]:{port}"),
            ],
            allowed_origins: Vec::new(),
            max_requests_per_second: 20,
        }
    }
}

pub(crate) struct RateWindow {
    window_start: Mutex<Instant>,
    count: AtomicU64,
}

impl RateWindow {
    pub(crate) fn new() -> Self {
        Self {
            window_start: Mutex::new(Instant::now()),
            count: AtomicU64::new(0),
        }
    }

    fn allow(&self, per_second: u64) -> bool {
        let mut start = self.window_start.lock().expect("rate window lock");
        if start.elapsed().as_secs() >= 1 {
            *start = Instant::now();
            self.count.store(0, Ordering::Relaxed);
        }
        self.count.fetch_add(1, Ordering::Relaxed) < per_second
    }
}

fn deny(status: StatusCode, reason: &str) -> Box<Response<Body>> {
    // Reasons are logged, never echoed: the body stays uniform so probing the
    // surface teaches an attacker nothing.
    tracing::warn!(target: "fndr_mcp::audit", %status, reason, "mcp request denied");
    Box::new(
        Response::builder()
            .status(status)
            .body(Body::from("denied"))
            .expect("static response"),
    )
}

/// The check itself, separated from axum wiring so it is unit-testable.
/// The error is boxed because a Response is large (clippy::result_large_err).
pub(crate) fn check_request(
    config: &AuthConfig,
    rate: &RateWindow,
    req: &Request<Body>,
) -> Result<(), Box<Response<Body>>> {
    if !rate.allow(config.max_requests_per_second) {
        return Err(deny(StatusCode::TOO_MANY_REQUESTS, "rate limit"));
    }

    let host = req
        .headers()
        .get("host")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !config.allowed_hosts.iter().any(|h| h == host) {
        return Err(deny(StatusCode::FORBIDDEN, "host not allowed"));
    }

    if let Some(origin) = req.headers().get("origin") {
        let origin = origin.to_str().unwrap_or("");
        if !config.allowed_origins.iter().any(|o| o == origin) {
            return Err(deny(StatusCode::FORBIDDEN, "origin not allowed"));
        }
    }

    let authorization = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let presented = authorization.strip_prefix("Bearer ").unwrap_or("");
    let equal: bool = presented.as_bytes().ct_eq(config.token.as_bytes()).into();
    if !equal {
        return Err(deny(StatusCode::UNAUTHORIZED, "bad or missing token"));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(headers: &[(&str, &str)]) -> Request<Body> {
        let mut builder = Request::builder().uri("/mcp").method("POST");
        for (k, v) in headers {
            builder = builder.header(*k, *v);
        }
        builder.body(Body::empty()).unwrap()
    }

    fn config() -> AuthConfig {
        AuthConfig::loopback("secret-token".into(), 4127)
    }

    #[test]
    fn rejects_missing_token() {
        let result = check_request(
            &config(),
            &RateWindow::new(),
            &request(&[("host", "127.0.0.1:4127")]),
        );
        assert_eq!(result.unwrap_err().status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn rejects_wrong_token() {
        let result = check_request(
            &config(),
            &RateWindow::new(),
            &request(&[
                ("host", "127.0.0.1:4127"),
                ("authorization", "Bearer wrong"),
            ]),
        );
        assert_eq!(result.unwrap_err().status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn rejects_web_origin_even_with_token() {
        let result = check_request(
            &config(),
            &RateWindow::new(),
            &request(&[
                ("host", "127.0.0.1:4127"),
                ("origin", "https://evil.example"),
                ("authorization", "Bearer secret-token"),
            ]),
        );
        assert_eq!(result.unwrap_err().status(), StatusCode::FORBIDDEN);
    }

    #[test]
    fn rejects_foreign_host_header() {
        let result = check_request(
            &config(),
            &RateWindow::new(),
            &request(&[
                ("host", "fndr.example.com"),
                ("authorization", "Bearer secret-token"),
            ]),
        );
        assert_eq!(result.unwrap_err().status(), StatusCode::FORBIDDEN);
    }

    #[test]
    fn accepts_valid_loopback_request() {
        let result = check_request(
            &config(),
            &RateWindow::new(),
            &request(&[
                ("host", "127.0.0.1:4127"),
                ("authorization", "Bearer secret-token"),
            ]),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn rate_limit_kicks_in() {
        let config = config();
        let rate = RateWindow::new();
        let ok_request = request(&[
            ("host", "127.0.0.1:4127"),
            ("authorization", "Bearer secret-token"),
        ]);
        for _ in 0..config.max_requests_per_second {
            assert!(check_request(&config, &rate, &ok_request).is_ok());
        }
        let result = check_request(&config, &rate, &ok_request);
        assert_eq!(result.unwrap_err().status(), StatusCode::TOO_MANY_REQUESTS);
    }
}
