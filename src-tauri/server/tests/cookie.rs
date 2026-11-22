// A browser that connects gets its device token as an HttpOnly, SameSite=Strict, Path=/ cookie,
// Secure only over TLS. A cookie-authenticated request other than GET needs the x-sp-csrf header
// and the listener's own Origin.
#[path = "../../testutil/tempdir.rs"]
mod tempdir;
mod common;

use common::{CtxParams, RawRequest, RawResponse, external_config, local_client, loopback_config, open_ctx, raw, raw_head, start};
use session_player_client::{Client, ClientParams, PairingString, RawCall};
use session_player_core::ops::RawData;
use session_player_server::pairing::{PairRequest, PairResponse};
use tempdir::TempDir;

fn attributes(set_cookie: &str) -> Vec<String> {
    set_cookie.split(';').skip(1).map(|a| a.trim().to_ascii_lowercase()).collect()
}

/// `name=value` from a Set-Cookie header.
fn cookie_pair(set_cookie: &str) -> String {
    set_cookie.split(';').next().unwrap().trim().to_string()
}

fn browser_connect(addr: std::net::SocketAddr, secret: &str, origin: &str) -> RawResponse {
    let body = format!(r#"{{"secret":"{secret}","name":"browser","browser":true}}"#);
    raw(addr, RawRequest { method: "POST", path: "/api/v1/connect", host: None, headers: &[("Content-Type", "application/json"), ("Origin", origin)], body: body.as_bytes() })
}

#[test]
fn a_browser_connect_on_loopback_sets_an_httponly_strict_cookie_without_secure_and_no_token_in_the_body() {
    let dir = TempDir::new("server-cookie-set");
    let ctx = open_ctx(CtxParams { dir: &dir, clock: None, event_capacity: None, fixtures: false });
    let server = start(&ctx, loopback_config());
    let addr = server.addrs().loopback;
    let offer: PairResponse = local_client(&server, &ctx.paths().data).call_named("pair", &PairRequest {}).expect("pair");
    let secret = PairingString::parse(&offer.pairing).unwrap().secret;

    let answer = browser_connect(addr, &secret, &format!("http://{addr}"));

    assert_eq!(answer.status, 200, "{}", String::from_utf8_lossy(&answer.body));
    let set_cookie = answer.header("set-cookie").expect("a Set-Cookie header");
    assert!(set_cookie.starts_with("sp_session="), "{set_cookie}");
    let attrs = attributes(set_cookie);
    for wanted in ["httponly", "samesite=strict", "path=/"] {
        assert!(attrs.iter().any(|a| a == wanted), "{wanted} missing from {set_cookie}");
    }
    assert!(!attrs.iter().any(|a| a == "secure"), "Secure on plain HTTP: {set_cookie}");
    assert!(answer.json()["token"].is_null(), "a browser's token belongs only in the cookie");
    server.shutdown();
}

#[test]
fn a_cookie_request_other_than_get_needs_the_csrf_header_and_the_listeners_own_origin() {
    let dir = TempDir::new("server-cookie-csrf");
    let ctx = open_ctx(CtxParams { dir: &dir, clock: None, event_capacity: None, fixtures: false });
    let server = start(&ctx, loopback_config());
    let addr = server.addrs().loopback;
    let own = format!("http://{addr}");
    let offer: PairResponse = local_client(&server, &ctx.paths().data).call_named("pair", &PairRequest {}).expect("pair");
    let connected = browser_connect(addr, &PairingString::parse(&offer.pairing).unwrap().secret, &own);
    let cookie = cookie_pair(connected.header("set-cookie").expect("a Set-Cookie header"));
    let hello = |headers: &[(&str, &str)]| {
        let mut all = vec![("Content-Type", "application/json"), ("Cookie", cookie.as_str())];
        all.extend_from_slice(headers);
        raw(addr, RawRequest { method: "POST", path: "/api/v1/hello", host: None, headers: &all, body: b"{}" })
    };

    assert_eq!(hello(&[("x-sp-csrf", "1"), ("Origin", &own)]).status, 200);
    assert_eq!(hello(&[("Origin", &own)]).status, 403, "no x-sp-csrf header");
    assert_eq!(hello(&[("x-sp-csrf", "1"), ("Origin", "http://evil.example")]).status, 403, "another Origin");
    assert_eq!(hello(&[("x-sp-csrf", "1")]).status, 403, "no Origin");
    let events = raw_head(addr, RawRequest { method: "GET", path: "/api/v1/events", host: None, headers: &[("Cookie", &cookie)], body: b"" });
    assert_eq!(events.status, 200, "a cookie GET needs no CSRF header");
    let wrong = raw(addr, RawRequest { method: "POST", path: "/api/v1/hello", host: None, headers: &[("Cookie", "sp_session=forged"), ("x-sp-csrf", "1"), ("Origin", &own)], body: b"{}" });
    assert_eq!(wrong.status, 401);
    server.shutdown();
}

#[test]
fn a_browser_connect_over_tls_sets_a_secure_cookie() {
    let dir = TempDir::new("server-cookie-secure");
    let ctx = open_ctx(CtxParams { dir: &dir, clock: None, event_capacity: None, fixtures: false });
    let server = start(&ctx, external_config());
    let offer: PairResponse = local_client(&server, &ctx.paths().data).call_named("pair", &PairRequest {}).expect("pair");
    let pairing = PairingString::parse(&offer.pairing).unwrap();
    let anonymous = Client::new(ClientParams { url: format!("https://{}:{}", pairing.address, pairing.port), token: None, fingerprint: pairing.fingerprint.clone() }).unwrap();
    let body = format!(r#"{{"secret":"{}","name":"browser","browser":true}}"#, pairing.secret);

    let answer = anonymous
        .call_raw_answer(RawCall { name: "connect".to_string(), body: RawData::Bytes(body.into_bytes()), content_type: "application/json".to_string(), query: vec![] })
        .expect("reach the external listener");

    assert_eq!(answer.status, 200, "{}", String::from_utf8_lossy(&answer.body));
    let set_cookie = answer.header("set-cookie").expect("a Set-Cookie header");
    let attrs = attributes(set_cookie);
    for wanted in ["secure", "httponly", "samesite=strict", "path=/"] {
        assert!(attrs.iter().any(|a| a == wanted), "{wanted} missing from {set_cookie}");
    }
    server.shutdown();
}
