// The relay against a real `serve` with a TLS external listener, paired through a ServersStore the
// way the app pairs: a call comes back as one frame (head + raw body) whatever its status, the
// device token never reaches the page, and a name or op the page made up fails before any network
// I/O. Uploads pass through byte for byte, images come through the GET route, and the event stream
// reconnects by itself after the server restarts.
#[path = "../testutil/tempdir.rs"]
mod tempdir;
#[path = "../server/tests/common/mod.rs"]
mod common;

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::time::Duration;

use base64::Engine;
use common::{CLOCK, CtxParams, SessionFile, external_config, local_client, open_ctx, reindex, row, session_file, start, wait_until};
use session_player_client::{PairedServer, ServersStore};
use session_player_core::ops::{Ctx, LOAD_SESSION, LoadSessionRequest, PushedEvent, REINDEX, ReindexRequest, ServerEvent};
use session_player_core::test_support::{bundle, index};
use session_player_lib::paired;
use session_player_lib::relay::{Clients, RelayCall, RelayImage, StreamParams, parse_framed, relay_call, relay_image, run_stream};
use session_player_server::pairing::{PairRequest, PairResponse};
use session_player_server::{ServerConfig, ServerHandle, TlsMode};
use tempdir::TempDir;

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n not really a png";

struct Setup {
    ctx: Arc<Ctx>,
    server: ServerHandle,
    store: ServersStore,
    paired: Vec<PairedServer>,
    clients: Clients,
}

/// A server with one session (holding an image), and this "app" paired with it as `work`.
fn setup(dir: &Path) -> Setup {
    let ctx = open_ctx(CtxParams { dir, clock: None, event_capacity: None, fixtures: false });
    let home = dir.join("home");
    let conn = index::open(&ctx.paths().db).unwrap();
    index::add_root(&conn, &home.join(".claude/projects").to_string_lossy()).unwrap();
    drop(conn);
    let png = base64::engine::general_purpose::STANDARD.encode(PNG);
    session_file(SessionFile { home: &home, cwd: &home.join("code/app"), id: "s1", prompt: "Look at this", images: &[("image/png", &png)] });
    reindex(&ctx);
    let server = start(&ctx, external_config());
    let offer: PairResponse = local_client(&server, &ctx.paths().data).call_named("pair", &PairRequest {}).expect("pair");
    let store = ServersStore::new(&dir.join("app"));
    let clients = Clients::default();
    paired::connect(paired::ConnectParams { store: &store, clients: &clients, pairing: &offer.pairing, name: Some("work"), device_name: "laptop" }).expect("connect");
    let paired = store.list().unwrap();
    Setup { ctx, server, store, paired, clients }
}

fn call<'a>(clients: &'a Clients, paired: &'a [PairedServer], op: &'a str, body: &str) -> RelayCall<'a> {
    RelayCall { clients, paired, server: "work", op, query: "", content_type: "application/json", body: body.as_bytes().to_vec() }
}

fn json(body: &[u8]) -> serde_json::Value {
    serde_json::from_slice(body).unwrap_or_else(|e| panic!("not JSON ({e}): {}", String::from_utf8_lossy(body)))
}

#[test]
fn relay_call_returns_the_servers_answer_as_one_frame() {
    let dir = TempDir::new("shell-relay-frame");
    let s = setup(&dir);

    let framed = relay_call(call(&s.clients, &s.paired, "hello", "{}")).expect("relay hello");

    let (head, body) = parse_framed(&framed).expect("a frame");
    assert_eq!(head.status, 200);
    assert!(head.headers.iter().any(|(n, v)| n == "content-type" && v.starts_with("application/json")), "{:?}", head.headers);
    assert_eq!(json(body)["server_version"], "9.9.9");
    s.server.shutdown();
}

#[test]
fn the_device_token_never_comes_back_to_the_page() {
    let dir = TempDir::new("shell-relay-token");
    let s = setup(&dir);
    let token = s.paired[0].token.clone();
    assert!(!token.is_empty());

    let mut answers = Vec::new();
    for (op, body) in [("hello", "{}"), ("devices", "{}"), ("list_sessions", "{}"), ("no_such_op", "{}"), ("load_session", "not json")] {
        answers.push(relay_call(call(&s.clients, &s.paired, op, body)).expect(op));
    }
    let listed = serde_json::to_string(&paired::list(&s.store).unwrap()).unwrap();

    for framed in &answers {
        assert!(!framed.windows(token.len()).any(|w| w == token.as_bytes()), "{}", String::from_utf8_lossy(framed));
    }
    assert!(!listed.contains(&token), "{listed}");
    s.server.shutdown();
}

#[test]
fn an_unknown_server_name_fails_before_any_network_io() {
    let dir = TempDir::new("shell-relay-unknown");
    // A paired server where nothing listens: had the relay tried it, the error would be about reaching it.
    let clients = Clients::default();
    let nowhere = [PairedServer { name: "work".to_string(), address: "127.0.0.1".to_string(), port: common::free_port(), fingerprint: None, token: "t".to_string(), device: session_player_core::ops::DeviceId("d".to_string()) }];

    let unknown = relay_call(RelayCall { server: "home", ..call(&clients, &nowhere, "hello", "{}") }).unwrap_err();
    let reached = relay_call(call(&clients, &nowhere, "hello", "{}")).unwrap_err();

    assert!(unknown.contains("sp connect"), "{unknown}");
    assert!(reached.contains("Couldn't reach"), "the control call reached for the server: {reached}");
    drop(dir);
}

#[test]
fn a_bad_operation_name_fails_before_any_network_io() {
    let dir = TempDir::new("shell-relay-bad-op");
    let clients = Clients::default();
    let nowhere = [PairedServer { name: "work".to_string(), address: "127.0.0.1".to_string(), port: common::free_port(), fingerprint: None, token: "t".to_string(), device: session_player_core::ops::DeviceId("d".to_string()) }];

    for op in ["../hello", "hello?x=1", "", "Hello", "image/1/2/3"] {
        let e = relay_call(call(&clients, &nowhere, op, "{}")).unwrap_err();
        assert!(!e.contains("Couldn't reach"), "{op:?}: {e}");
    }
    drop(dir);
}

#[test]
fn a_server_whose_certificate_is_not_the_pinned_one_is_refused() {
    let dir = TempDir::new("shell-relay-pin");
    let s = setup(&dir);
    let mut wrong = s.paired.clone();
    wrong[0].fingerprint = Some("00".repeat(32));

    let e = relay_call(call(&s.clients, &wrong, "hello", "{}")).unwrap_err();

    assert!(e.contains("Couldn't reach"), "{e}");
    s.server.shutdown();
}

#[test]
fn a_non_2xx_answer_comes_back_as_a_frame_with_the_servers_status_and_error() {
    let dir = TempDir::new("shell-relay-non-2xx");
    let s = setup(&dir);

    let unknown = relay_call(call(&s.clients, &s.paired, "no_such_op", "{}")).expect("relay");
    let bad = relay_call(call(&s.clients, &s.paired, "load_session", "not json")).expect("relay");
    let bad_query = relay_call(RelayCall { query: "on_conflict=bogus%20value", content_type: "application/octet-stream", ..call(&s.clients, &s.paired, "import_bundle", "x") }).expect("relay");

    let (head, body) = parse_framed(&unknown).unwrap();
    assert_eq!(head.status, 404);
    assert_eq!(json(body)["code"], "unknown_operation");
    let (head, body) = parse_framed(&bad).unwrap();
    assert_eq!(head.status, 422);
    assert_eq!(json(body)["code"], "bad_request");
    let (head, body) = parse_framed(&bad_query).unwrap();
    assert_eq!(head.status, 422);
    assert!(json(body)["message"].as_str().unwrap().contains("bogus value"), "the query arrived decoded: {}", String::from_utf8_lossy(body));
    s.server.shutdown();
}

#[test]
fn an_octet_stream_upload_passes_through_unchanged() {
    let dir = TempDir::new("shell-relay-upload");
    let s = setup(&dir);
    // A bundle of the server's own session, imported again under replace: it imports only if every
    // byte of the compressed bundle arrived.
    let conn = index::open(&s.ctx.paths().db).unwrap();
    let built = bundle::build(&conn, bundle::ExportRequest { path: &row(&s.ctx, "s1").path, branch: None, subagents: None }, "test").unwrap();
    let file = dir.join(format!("s1.{}", bundle::EXTENSION));
    bundle::write(&built, &file).unwrap();
    let bytes = std::fs::read(&file).unwrap();

    let framed = relay_call(RelayCall { query: "on_conflict=replace", content_type: "application/octet-stream", body: bytes, ..call(&s.clients, &s.paired, "import_bundle", "") }).expect("relay");

    let (head, body) = parse_framed(&framed).unwrap();
    assert_eq!(head.status, 200, "{}", String::from_utf8_lossy(body));
    assert_eq!(json(body)["imported"], serde_json::json!(["claude:s1"]));
    s.server.shutdown();
}

#[test]
fn an_image_url_from_load_session_comes_through_the_relay_as_a_frame() {
    let dir = TempDir::new("shell-relay-image");
    let s = setup(&dir);
    let request = serde_json::to_string(&LoadSessionRequest { path: row(&s.ctx, "s1").path, branch: None }).unwrap();
    let (_, body) = parse_framed(&relay_call(call(&s.clients, &s.paired, LOAD_SESSION.spec.name, &request)).expect("relay load_session")).map(|(h, b)| (h, b.to_vec())).unwrap();
    let url = json(&body).as_array().unwrap().iter().flat_map(|e| e["images"].as_array().cloned().unwrap_or_default()).filter_map(|u| u.as_str().map(str::to_string)).find(|u| u.starts_with("/api/v1/image/")).expect("an image URL");

    let framed = relay_image(RelayImage { clients: &s.clients, paired: &s.paired, server: "work", path: &url }).expect("relay image");
    let stale = relay_image(RelayImage { clients: &s.clients, paired: &s.paired, server: "work", path: "/api/v1/image/1/0/0" }).expect("relay stale image");

    let (head, image) = parse_framed(&framed).unwrap();
    assert_eq!(head.status, 200);
    assert!(head.headers.iter().any(|(n, v)| n == "content-type" && v == "image/png"), "{:?}", head.headers);
    assert_eq!(image, PNG);
    assert_eq!(parse_framed(&stale).unwrap().0.status, 404);
    s.server.shutdown();
}

#[test]
fn the_event_stream_reconnects_by_itself_after_the_server_restarts() {
    let dir = TempDir::new("shell-relay-stream");
    let s = setup(&dir);
    let server = s.paired[0].clone();
    let data = s.ctx.paths().data.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let (sender, events) = mpsc::channel::<ServerEvent>();
    let pump = {
        let stop = stop.clone();
        std::thread::spawn(move || {
            let mut on_event = move |e: ServerEvent| sender.send(e).is_ok();
            run_stream(StreamParams { server, last_id: Some(CLOCK - 1), stop: &stop, retry: Duration::from_millis(100), on_event: &mut on_event });
        })
    };
    let first = events.recv_timeout(Duration::from_secs(10)).expect("the setup's index events replay");
    let mut seen = first.id;
    while let Ok(e) = events.recv_timeout(Duration::from_millis(300)) {
        seen = seen.max(e.id);
    }

    let external_port = s.paired[0].port;
    s.server.shutdown();
    let config = ServerConfig { port: 0, external_port, external_bind: Some("127.0.0.1".parse().unwrap()), advertise: Some("127.0.0.1".to_string()), tls: TlsMode::SelfSigned };
    let restarted = start(&s.ctx, config);
    local_client(&restarted, &data).call(&REINDEX, &ReindexRequest { full: false }).expect("reindex");

    let mut after = None;
    wait_until("an event from after the restart arrives", Duration::from_secs(20), || {
        while let Ok(e) = events.recv_timeout(Duration::from_millis(50)) {
            if e.id > seen && matches!(e.event, PushedEvent::IndexStarted) {
                after = Some(e);
            }
        }
        after.is_some()
    });
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    pump.join().unwrap();
    restarted.shutdown();
}

#[test]
fn relayed_calls_to_one_server_share_one_client_until_the_server_is_removed() {
    let dir = TempDir::new("shell-relay-reuse");
    let s = setup(&dir);

    relay_call(call(&s.clients, &s.paired, "hello", "{}")).expect("first call");
    let first = s.clients.cached("work").expect("the first call kept its Client");
    relay_call(call(&s.clients, &s.paired, "list_sessions", "{}")).expect("second call");
    let framed = relay_image(RelayImage { clients: &s.clients, paired: &s.paired, server: "work", path: "/api/v1/image/1/0/0" }).expect("image");

    assert_eq!(parse_framed(&framed).unwrap().0.status, 404);
    assert!(std::sync::Arc::ptr_eq(&first, &s.clients.cached("work").unwrap()), "both calls and the image went through the first Client");
    paired::remove(paired::RemoveParams { store: &s.store, clients: &s.clients, name: "work" }).expect("remove");
    assert!(s.clients.cached("work").is_none(), "removing the server drops its Client");
    s.server.shutdown();
}

#[test]
fn pairing_again_under_the_same_name_drops_the_old_client() {
    let dir = TempDir::new("shell-relay-repair");
    let s = setup(&dir);
    relay_call(call(&s.clients, &s.paired, "hello", "{}")).expect("first call");
    assert!(s.clients.cached("work").is_some());

    let offer: PairResponse = local_client(&s.server, &s.ctx.paths().data).call_named("pair", &PairRequest {}).expect("pair");
    paired::connect(paired::ConnectParams { store: &s.store, clients: &s.clients, pairing: &offer.pairing, name: Some("work"), device_name: "laptop" }).expect("connect again");

    assert!(s.clients.cached("work").is_none(), "the re-paired server gets a new Client on its next call");
    let paired = s.store.list().unwrap();
    relay_call(call(&s.clients, &paired, "hello", "{}")).expect("call with the new token");
    s.server.shutdown();
}
