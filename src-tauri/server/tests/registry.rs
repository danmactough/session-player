// The operation registry's invariants (N19, N24, N48, N66) with the server's extensions and the
// client's `pull` registered: names are unique; only `connect` is anonymous; only `service` is
// never reachable over HTTP; only `pull`, `pair`, `unpair` and `devices` are local-only, and a
// paired device or browser calling one gets the same 403 as any other refusal; only `hello`, `job`
// and `cancel_job` run inline; without a token every name but `connect` gets the same 401; no
// operation shares a native shell command's name.
#[path = "../../testutil/tempdir.rs"]
mod tempdir;
mod common;

use std::collections::BTreeSet;
use std::sync::Arc;

use common::{CtxParams, OpenWith, RawRequest, local_client, loopback_config, open_ctx_with, post_json, raw, start};
use session_player_client::{ConnectParams, PULL, PairingString, connect};
use session_player_server::pairing::{PairRequest, PairResponse};
use session_player_core::ops::{Access, Ctx, specs};
use session_player_server::SERVER_EXTENSIONS;
use tempdir::TempDir;

const NATIVE_SHELL_COMMANDS: &[&str] = &["reveal", "open_targets", "open_in_app", "terminal_apps", "open_in_terminal", "open_analyze_log"];

fn all_extensions() -> Vec<session_player_core::ops::Extension> {
    let mut extensions = SERVER_EXTENSIONS.to_vec();
    extensions.push(PULL);
    extensions
}

fn open(dir: &TempDir) -> Arc<Ctx> {
    open_ctx_with(CtxParams { dir, clock: None, event_capacity: None, fixtures: false }, OpenWith { upload_cap: None, extensions: all_extensions() })
}

fn names_where(ctx: &Ctx, keep: impl Fn(&session_player_core::ops::OpSpec) -> bool) -> BTreeSet<&'static str> {
    specs(ctx).iter().filter(|s| keep(s)).map(|s| s.name).collect()
}

#[test]
fn every_operation_name_is_unique_and_the_server_and_client_extensions_are_registered() {
    let dir = TempDir::new("server-registry-names");
    let ctx = open(&dir);

    let all: Vec<&str> = specs(&ctx).iter().map(|s| s.name).collect();
    let unique: BTreeSet<&str> = all.iter().copied().collect();

    assert_eq!(all.len(), unique.len(), "{all:?}");
    for name in ["pair", "connect", "devices", "unpair", "service", "pull"] {
        assert!(unique.contains(name), "{name} is not registered: {all:?}");
    }
}

#[test]
fn a_duplicate_extension_name_makes_opening_the_ctx_fail() {
    let dir = TempDir::new("server-registry-duplicate");
    let mut extensions = all_extensions();
    extensions.push(PULL);
    let data = dir.join("data");
    std::fs::create_dir_all(&data).unwrap();
    let opened = Ctx::open(session_player_core::ops::OpenParams {
        paths: session_player_core::ops::Paths { db: data.join("index.sqlite"), library: data.join("library"), logs: data.join("logs"), data: data.clone() },
        home: dir.join("home"),
        version: common::VERSION.to_string(),
        claude: None,
        clock: None,
        event_capacity: None,
        upload_cap: None,
        extensions,
    });

    assert!(opened.is_err());
}

#[test]
fn only_connect_is_anonymous_and_only_service_is_kept_off_http() {
    let dir = TempDir::new("server-registry-access");
    let ctx = open(&dir);

    assert_eq!(names_where(&ctx, |s| s.access == Access::Anonymous), BTreeSet::from(["connect"]));
    assert_eq!(names_where(&ctx, |s| s.access == Access::NotOverHttp), BTreeSet::from(["service"]));
}

#[test]
fn only_pull_pair_unpair_and_devices_are_local_only() {
    let dir = TempDir::new("server-registry-local-only");
    let ctx = open(&dir);

    assert_eq!(names_where(&ctx, |s| s.access == Access::LocalOnly), BTreeSet::from(["devices", "pair", "pull", "unpair"]));
}

/// The body every 403 has, taken from a request with a Host this server doesn't answer.
fn forbidden_body(addr: std::net::SocketAddr) -> Vec<u8> {
    let refused = raw(addr, RawRequest { method: "POST", path: "/api/v1/hello", host: Some("elsewhere.example:1"), headers: &[], body: b"{}" });
    assert_eq!(refused.status, 403);
    refused.body
}

#[test]
fn a_paired_device_calling_a_local_only_operation_gets_the_same_403_as_other_refusals() {
    let dir = TempDir::new("server-registry-local-only-device");
    let ctx = open(&dir);
    let server = start(&ctx, loopback_config());
    let addr = server.addrs().loopback;
    let local = local_client(&server, &ctx.paths().data);
    let offer: PairResponse = local.call_named("pair", &PairRequest {}).expect("pair");
    let device = connect(ConnectParams { pairing: &offer.pairing, name: "laptop" }).expect("connect");
    let bearer = format!("Bearer {}", device.token);
    let body_of = |name: &str| match name {
        "pull" => r#"{"server":"lab","project":"/tmp/nowhere"}"#.to_string(),
        "unpair" => format!(r#"{{"id":"{}"}}"#, device.device.0),
        _ => "{}".to_string(),
    };
    assert_eq!(post_json(addr, "/api/v1/hello", &[("Authorization", &bearer)], "{}").status, 200, "the device's token works");

    for name in ["pull", "pair", "unpair", "devices"] {
        let answer = post_json(addr, &format!("/api/v1/{name}"), &[("Authorization", &bearer)], &body_of(name));
        assert_eq!(answer.status, 403, "{name}: {}", String::from_utf8_lossy(&answer.body));
        assert_eq!(answer.body, forbidden_body(addr), "{name}");
    }
    let still: Vec<serde_json::Value> = local.call_named("devices", &serde_json::json!({})).expect("devices with the local token");
    assert_eq!(still.len(), 1, "the device couldn't unpair itself");
    server.shutdown();
}

#[test]
fn a_browser_cookie_calling_a_local_only_operation_gets_403() {
    let dir = TempDir::new("server-registry-local-only-cookie");
    let ctx = open(&dir);
    let server = start(&ctx, loopback_config());
    let addr = server.addrs().loopback;
    let own = format!("http://{addr}");
    let offer: PairResponse = local_client(&server, &ctx.paths().data).call_named("pair", &PairRequest {}).expect("pair");
    let secret = PairingString::parse(&offer.pairing).unwrap().secret;
    let body = format!(r#"{{"secret":"{secret}","name":"browser","browser":true}}"#);
    let connected = post_json(addr, "/api/v1/connect", &[("Origin", &own)], &body);
    let cookie = connected.header("set-cookie").expect("a Set-Cookie header").split(';').next().unwrap().to_string();
    let headers = [("Cookie", cookie.as_str()), ("x-sp-csrf", "1"), ("Origin", own.as_str())];
    assert_eq!(post_json(addr, "/api/v1/hello", &headers, "{}").status, 200, "the cookie works");

    for name in ["pair", "devices"] {
        let answer = post_json(addr, &format!("/api/v1/{name}"), &headers, "{}");
        assert_eq!(answer.status, 403, "{name}");
        assert_eq!(answer.body, forbidden_body(addr), "{name}");
    }
    server.shutdown();
}

#[test]
fn only_hello_job_and_cancel_job_run_inline() {
    let dir = TempDir::new("server-registry-inline");
    let ctx = open(&dir);

    assert_eq!(names_where(&ctx, |s| s.inline), BTreeSet::from(["hello", "job", "cancel_job"]));
}

#[test]
fn without_a_token_every_operation_but_connect_gets_the_same_401() {
    let dir = TempDir::new("server-registry-401");
    let ctx = open(&dir);
    let server = start(&ctx, loopback_config());
    let addr = server.addrs().loopback;
    let refused = post_json(addr, "/api/v1/hello", &[], "{}");
    assert_eq!(refused.status, 401);

    for spec in specs(&ctx).iter().filter(|s| s.name != "connect") {
        let answer = post_json(addr, &format!("/api/v1/{}", spec.name), &[], "{}");
        assert_eq!(answer.status, 401, "{}", spec.name);
        assert_eq!(answer.body, refused.body, "{}", spec.name);
    }
    let unknown = post_json(addr, "/api/v1/not_an_operation", &[], "{}");
    assert_eq!(unknown.body, refused.body);
    server.shutdown();
}

#[test]
fn no_operation_shares_a_native_shell_commands_name() {
    let dir = TempDir::new("server-registry-native");
    let ctx = open(&dir);

    let clashes: Vec<&str> = specs(&ctx).iter().map(|s| s.name).filter(|n| NATIVE_SHELL_COMMANDS.contains(n)).collect();

    assert!(clashes.is_empty(), "{clashes:?}");
}
