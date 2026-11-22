// Paths outside /api/ serve the web UI embedded from the repository's dist/ (a build.rs writes a
// placeholder index.html when dist/ hasn't been built), without a token but still behind the Host
// and Origin checks. HEAD works. A missing file under /assets/ is a 404; any other path gets
// index.html, so the UI's own routes load.
#[path = "../../testutil/tempdir.rs"]
mod tempdir;
mod common;

use std::net::SocketAddr;

use common::{CtxParams, RawRequest, RawResponse, loopback_config, open_ctx, raw, start};
use tempdir::TempDir;

fn request(addr: SocketAddr, method: &str, path: &str, host: Option<&str>, headers: &[(&str, &str)]) -> RawResponse {
    raw(addr, RawRequest { method, path, host, headers, body: b"" })
}

fn is_html(answer: &RawResponse) -> bool {
    answer.header("content-type").is_some_and(|t| t.starts_with("text/html"))
}

#[test]
fn the_root_serves_the_ui_page_without_a_token() {
    let dir = TempDir::new("server-assets-root");
    let ctx = open_ctx(CtxParams { dir: &dir, clock: None, event_capacity: None, fixtures: false });
    let server = start(&ctx, loopback_config());

    let answer = request(server.addrs().loopback, "GET", "/", None, &[]);

    assert_eq!(answer.status, 200, "{}", String::from_utf8_lossy(&answer.body));
    assert!(is_html(&answer), "{:?}", answer.header("content-type"));
    assert!(String::from_utf8_lossy(&answer.body).to_ascii_lowercase().contains("<html"), "{}", String::from_utf8_lossy(&answer.body));
    server.shutdown();
}

#[test]
fn any_other_path_outside_api_and_assets_gets_index_html() {
    let dir = TempDir::new("server-assets-fallback");
    let ctx = open_ctx(CtxParams { dir: &dir, clock: None, event_capacity: None, fixtures: false });
    let server = start(&ctx, loopback_config());
    let addr = server.addrs().loopback;
    let root = request(addr, "GET", "/", None, &[]);

    for path in ["/index.html", "/sessions/some-session", "/nothing/here?x=1"] {
        let answer = request(addr, "GET", path, None, &[]);
        assert_eq!(answer.status, 200, "{path}");
        assert!(is_html(&answer), "{path}");
        assert_eq!(answer.body, root.body, "{path}");
    }
    server.shutdown();
}

#[test]
fn head_answers_like_get_without_a_body() {
    let dir = TempDir::new("server-assets-head");
    let ctx = open_ctx(CtxParams { dir: &dir, clock: None, event_capacity: None, fixtures: false });
    let server = start(&ctx, loopback_config());

    let answer = request(server.addrs().loopback, "HEAD", "/", None, &[]);

    assert_eq!(answer.status, 200);
    assert!(is_html(&answer), "{:?}", answer.header("content-type"));
    assert!(answer.body.is_empty(), "{}", String::from_utf8_lossy(&answer.body));
    server.shutdown();
}

#[test]
fn a_missing_file_under_assets_is_a_404_not_the_page() {
    let dir = TempDir::new("server-assets-missing");
    let ctx = open_ctx(CtxParams { dir: &dir, clock: None, event_capacity: None, fixtures: false });
    let server = start(&ctx, loopback_config());

    let answer = request(server.addrs().loopback, "GET", "/assets/no-such-file-0000.js", None, &[]);

    assert_eq!(answer.status, 404);
    assert!(!is_html(&answer), "the page must not stand in for a missing script");
    server.shutdown();
}

#[test]
fn the_ui_is_still_behind_the_host_and_origin_checks() {
    let dir = TempDir::new("server-assets-host");
    let ctx = open_ctx(CtxParams { dir: &dir, clock: None, event_capacity: None, fixtures: false });
    let server = start(&ctx, loopback_config());
    let addr = server.addrs().loopback;

    for path in ["/", "/assets/app.js", "/sessions/x"] {
        let wrong_host = request(addr, "GET", path, Some("evil.example"), &[]);
        assert_eq!(wrong_host.status, 403, "{path}");
        assert_eq!(wrong_host.code(), "forbidden_origin", "{path}");
        let wrong_origin = request(addr, "GET", path, None, &[("Origin", "http://evil.example")]);
        assert_eq!(wrong_origin.status, 403, "{path}");
    }
    let localhost = request(addr, "GET", "/", Some(&format!("localhost:{}", addr.port())), &[]);
    assert_eq!(localhost.status, 200);
    server.shutdown();
}
