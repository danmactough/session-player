// The router answers every refusal and mistake with a JSON error, and a caller without a token
// learns nothing about which paths or operations exist.
#[path = "../../testutil/tempdir.rs"]
mod tempdir;
mod common;

use common::{CtxParams, RawRequest, loopback_config, open_ctx, post_json, raw, read_token, start};
use tempdir::TempDir;

#[test]
fn an_authenticated_call_to_an_unknown_operation_is_a_json_404() {
    let dir = TempDir::new("server-routing-unknown");
    let ctx = open_ctx(CtxParams { dir: &dir, clock: None, event_capacity: None, fixtures: false });
    let server = start(&ctx, loopback_config());
    let bearer = format!("Bearer {}", read_token(&ctx.paths().data));

    let answer = post_json(server.addrs().loopback, "/api/v1/no_such_operation", &[("Authorization", &bearer)], "{}");

    assert_eq!(answer.status, 404);
    assert_eq!(answer.code(), "unknown_operation");
    server.shutdown();
}

#[test]
fn an_operation_called_with_the_wrong_method_is_a_json_405() {
    let dir = TempDir::new("server-routing-method");
    let ctx = open_ctx(CtxParams { dir: &dir, clock: None, event_capacity: None, fixtures: false });
    let server = start(&ctx, loopback_config());
    let bearer = format!("Bearer {}", read_token(&ctx.paths().data));

    for method in ["GET", "PUT", "DELETE"] {
        let answer = raw(server.addrs().loopback, RawRequest { method, path: "/api/v1/hello", host: None, headers: &[("Authorization", &bearer)], body: b"" });
        assert_eq!(answer.status, 405, "{method}");
        assert_eq!(answer.code(), "method_not_allowed", "{method}");
    }
    server.shutdown();
}

#[test]
fn a_malformed_json_body_is_a_json_422() {
    let dir = TempDir::new("server-routing-json");
    let ctx = open_ctx(CtxParams { dir: &dir, clock: None, event_capacity: None, fixtures: false });
    let server = start(&ctx, loopback_config());
    let bearer = format!("Bearer {}", read_token(&ctx.paths().data));

    for body in ["{not json", "[1, 2", r#"{"id": 5}"#] {
        let answer = post_json(server.addrs().loopback, "/api/v1/job", &[("Authorization", &bearer)], body);
        assert_eq!(answer.status, 422, "{body}");
        assert_eq!(answer.code(), "bad_request", "{body}");
    }
    server.shutdown();
}

#[test]
fn an_unknown_path_under_api_is_a_json_404_for_any_method() {
    let dir = TempDir::new("server-routing-path");
    let ctx = open_ctx(CtxParams { dir: &dir, clock: None, event_capacity: None, fixtures: false });
    let server = start(&ctx, loopback_config());
    let bearer = format!("Bearer {}", read_token(&ctx.paths().data));

    for (method, path) in [("GET", "/api/whatever"), ("POST", "/api/v2/hello"), ("GET", "/api/v1/hello/extra"), ("DELETE", "/api/")] {
        let answer = raw(server.addrs().loopback, RawRequest { method, path, host: None, headers: &[("Authorization", &bearer)], body: b"" });
        assert_eq!(answer.status, 404, "{method} {path}");
        assert!(answer.header("content-type").is_some_and(|t| t.starts_with("application/json")), "{method} {path}");
        assert_eq!(answer.code(), "not_found", "{method} {path}");
    }
    server.shutdown();
}

#[test]
fn without_a_token_every_api_request_except_connect_gets_the_same_401() {
    let dir = TempDir::new("server-routing-401");
    let ctx = open_ctx(CtxParams { dir: &dir, clock: None, event_capacity: None, fixtures: false });
    let server = start(&ctx, loopback_config());
    let addr = server.addrs().loopback;

    let first = post_json(addr, "/api/v1/hello", &[], "{}");
    assert_eq!(first.status, 401);
    for (method, path) in [
        ("POST", "/api/v1/no_such_operation"),
        ("GET", "/api/v1/hello"),
        ("GET", "/api/whatever"),
        ("POST", "/api/v1/service"),
        ("DELETE", "/api/v1/reindex"),
        ("GET", "/api/v1/events"),
        ("POST", "/api/v1/pair"),
        ("POST", "/api/v1/devices"),
    ] {
        let answer = raw(addr, RawRequest { method, path, host: None, headers: &[("Content-Type", "application/json")], body: b"{}" });
        assert_eq!(answer.status, 401, "{method} {path}");
        assert_eq!(answer.body, first.body, "{method} {path}");
    }
    server.shutdown();
}
