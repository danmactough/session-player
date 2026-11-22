// CORS: only tauri://localhost (the app's webview) and the listener's own origin; a preflight
// answers 204 with the allowed methods and headers; any other Origin is 403, before the token is
// checked. Never `*`, never Access-Control-Allow-Credentials.
#[path = "../../testutil/tempdir.rs"]
mod tempdir;
mod common;

use common::{CtxParams, RawRequest, RawResponse, loopback_config, open_ctx, raw, read_token, start};
use tempdir::TempDir;

fn assert_no_wildcard_or_credentials(answer: &RawResponse, what: &str) {
    assert!(answer.header("access-control-allow-origin") != Some("*"), "{what}: allow-origin *");
    assert_eq!(answer.header("access-control-allow-credentials"), None, "{what}");
}

fn preflight(addr: std::net::SocketAddr, origin: &str) -> RawResponse {
    raw(addr, RawRequest {
        method: "OPTIONS",
        path: "/api/v1/hello",
        host: None,
        headers: &[("Origin", origin), ("Access-Control-Request-Method", "POST"), ("Access-Control-Request-Headers", "authorization, content-type")],
        body: b"",
    })
}

#[test]
fn a_preflight_from_the_app_webview_gets_204_with_the_allowed_methods_and_headers() {
    let dir = TempDir::new("server-origin-preflight");
    let ctx = open_ctx(CtxParams { dir: &dir, clock: None, event_capacity: None, fixtures: false });
    let server = start(&ctx, loopback_config());

    let answer = preflight(server.addrs().loopback, "tauri://localhost");

    assert_eq!(answer.status, 204);
    assert_eq!(answer.header("access-control-allow-origin"), Some("tauri://localhost"));
    let methods = answer.header("access-control-allow-methods").unwrap_or_default().to_ascii_uppercase();
    assert!(methods.contains("POST") && methods.contains("GET"), "{methods}");
    let headers = answer.header("access-control-allow-headers").unwrap_or_default().to_ascii_lowercase();
    for wanted in ["authorization", "content-type", "last-event-id"] {
        assert!(headers.contains(wanted), "{wanted} missing from {headers}");
    }
    assert_no_wildcard_or_credentials(&answer, "preflight");
    server.shutdown();
}

#[test]
fn a_preflight_or_request_from_another_origin_is_403_even_with_the_token() {
    let dir = TempDir::new("server-origin-other");
    let ctx = open_ctx(CtxParams { dir: &dir, clock: None, event_capacity: None, fixtures: false });
    let server = start(&ctx, loopback_config());
    let addr = server.addrs().loopback;
    let bearer = format!("Bearer {}", read_token(&ctx.paths().data));

    let pre = preflight(addr, "http://evil.example");
    assert_eq!(pre.status, 403);
    assert_eq!(pre.header("access-control-allow-origin"), None);

    for origin in ["http://evil.example", "null", &format!("http://127.0.0.1:{}", addr.port() + 1)] {
        let answer = raw(addr, RawRequest {
            method: "POST",
            path: "/api/v1/hello",
            host: None,
            headers: &[("Content-Type", "application/json"), ("Authorization", &bearer), ("Origin", origin)],
            body: b"{}",
        });
        assert_eq!(answer.status, 403, "{origin}");
        assert_eq!(answer.code(), "forbidden_origin", "{origin}");
        assert_eq!(answer.header("access-control-allow-origin"), None, "{origin}");
    }
    let unauthenticated = raw(addr, RawRequest { method: "POST", path: "/api/v1/hello", host: None, headers: &[("Origin", "http://evil.example")], body: b"{}" });
    assert_eq!(unauthenticated.status, 403, "Origin is checked before the token");
    server.shutdown();
}

#[test]
fn a_request_from_the_app_webview_or_the_listeners_own_origin_is_answered_with_that_origin() {
    let dir = TempDir::new("server-origin-allowed");
    let ctx = open_ctx(CtxParams { dir: &dir, clock: None, event_capacity: None, fixtures: false });
    let server = start(&ctx, loopback_config());
    let addr = server.addrs().loopback;
    let bearer = format!("Bearer {}", read_token(&ctx.paths().data));

    for origin in ["tauri://localhost".to_string(), "http://tauri.localhost".to_string(), format!("http://{addr}"), format!("http://localhost:{}", addr.port())] {
        let answer = raw(addr, RawRequest {
            method: "POST",
            path: "/api/v1/hello",
            host: None,
            headers: &[("Content-Type", "application/json"), ("Authorization", &bearer), ("Origin", &origin)],
            body: b"{}",
        });
        assert_eq!(answer.status, 200, "{origin}");
        if origin.starts_with("tauri:") {
            assert_eq!(answer.header("access-control-allow-origin"), Some(origin.as_str()));
        }
        assert_no_wildcard_or_credentials(&answer, &origin);
    }
    let unauthenticated = raw(addr, RawRequest { method: "POST", path: "/api/v1/hello", host: None, headers: &[("Origin", "tauri://localhost")], body: b"{}" });
    assert_eq!(unauthenticated.status, 401);
    assert_no_wildcard_or_credentials(&unauthenticated, "401");
    server.shutdown();
}
