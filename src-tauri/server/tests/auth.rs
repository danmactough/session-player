// Every request needs a token, even on loopback, and every refusal looks the same; a wrong Host is
// refused before the token is looked at.
#[path = "../../testutil/tempdir.rs"]
mod tempdir;
mod common;

use common::{CtxParams, RawRequest, loopback_config, open_ctx, post_json, raw, read_token, start};
use tempdir::TempDir;

#[test]
fn a_request_with_no_token_is_refused_the_same_way_as_one_with_a_wrong_token() {
    let dir = TempDir::new("server-auth");
    let ctx = open_ctx(CtxParams { dir: &dir, clock: None, event_capacity: None, fixtures: false });
    let server = start(&ctx, loopback_config());
    let addr = server.addrs().loopback;

    let none = post_json(addr, "/api/v1/hello", &[], "{}");
    let wrong = post_json(addr, "/api/v1/hello", &[("Authorization", "Bearer not-the-token")], "{}");
    let not_bearer = post_json(addr, "/api/v1/hello", &[("Authorization", &read_token(&ctx.paths().data))], "{}");

    assert_eq!(none.status, 401);
    assert_eq!(none.code(), "unauthenticated");
    for other in [&wrong, &not_bearer] {
        assert_eq!(other.status, 401);
        assert_eq!(other.body, none.body, "every 401 has one body");
    }
    server.shutdown();
}

#[test]
fn an_unknown_operation_without_a_token_is_refused_like_a_known_one() {
    let dir = TempDir::new("server-auth-unknown");
    let ctx = open_ctx(CtxParams { dir: &dir, clock: None, event_capacity: None, fixtures: false });
    let server = start(&ctx, loopback_config());
    let addr = server.addrs().loopback;

    let known = post_json(addr, "/api/v1/hello", &[], "{}");
    let unknown = post_json(addr, "/api/v1/no_such_operation", &[], "{}");

    assert_eq!(unknown.status, 401);
    assert_eq!(unknown.body, known.body);
    server.shutdown();
}

#[test]
fn a_wrong_host_is_refused_with_403_before_the_token_is_checked() {
    let dir = TempDir::new("server-auth-host");
    let ctx = open_ctx(CtxParams { dir: &dir, clock: None, event_capacity: None, fixtures: false });
    let server = start(&ctx, loopback_config());
    let addr = server.addrs().loopback;
    let token = read_token(&ctx.paths().data);
    let bearer = format!("Bearer {token}");
    let port = addr.port();

    for host in ["evil.example".to_string(), format!("evil.example:{port}"), format!("127.0.0.1:{}", port + 1)] {
        for headers in [vec![("Content-Type", "application/json")], vec![("Content-Type", "application/json"), ("Authorization", bearer.as_str())]] {
            let answer = raw(addr, RawRequest { method: "POST", path: "/api/v1/hello", host: Some(&host), headers: &headers, body: b"{}" });
            assert_eq!(answer.status, 403, "Host {host} with headers {headers:?}");
            assert_eq!(answer.code(), "forbidden_origin");
        }
    }
    for host in [format!("127.0.0.1:{port}"), format!("localhost:{port}")] {
        let answer = raw(addr, RawRequest {
            method: "POST",
            path: "/api/v1/hello",
            host: Some(&host),
            headers: &[("Content-Type", "application/json"), ("Authorization", &bearer)],
            body: b"{}",
        });
        assert_eq!(answer.status, 200, "Host {host}");
    }
    server.shutdown();
}
