// GET /api/v1/events: authenticated like the operations; resuming from a Last-Event-ID replays
// exactly the newer events, with no gap and nothing twice; an id the ring no longer holds gets a
// Snapshot.
#[path = "../../testutil/tempdir.rs"]
mod tempdir;
mod common;

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::{Duration, Instant};

use common::{CLOCK, CtxParams, RawRequest, TestClock, loopback_config, local_client, open_ctx, post_json, raw, raw_head, read_token, start};
use session_player_client::{Client, ConnectParams, EventStream, PairingString, connect};
use session_player_core::ops::{DeviceId, JOB, JobId, JobRequest, JobState, PushedEvent, REINDEX, ReindexRequest, ServerEvent};
use session_player_server::pairing::{PairRequest, PairResponse, UnpairRequest};
use tempdir::TempDir;

fn reindex_and_wait(client: &Client) -> JobId {
    let id = client.call(&REINDEX, &ReindexRequest { full: true }).expect("reindex");
    let deadline = Instant::now() + Duration::from_secs(20);
    while client.call(&JOB, &JobRequest { id: id.clone() }).expect("job").state == JobState::Running {
        assert!(Instant::now() < deadline, "the index pass never finished");
        std::thread::sleep(Duration::from_millis(20));
    }
    id
}

fn next(stream: &mut EventStream) -> ServerEvent {
    stream.next_within(Duration::from_secs(10)).expect("an event within 10 s").expect("a readable event")
}

/// The newest event id: a stream resumed from an id older than the ring starts with a Snapshot
/// carrying it.
fn newest_id(client: &Client) -> u64 {
    let mut stream = client.events(Some(0)).expect("open the stream");
    let first = next(&mut stream);
    assert!(matches!(first.event, PushedEvent::Snapshot { .. }), "expected a Snapshot, got {first:?}");
    first.id
}

/// Reads one whole index pass: IndexStarted first, IndexDone last.
fn read_pass(stream: &mut EventStream) -> Vec<ServerEvent> {
    let mut pass = vec![next(stream)];
    assert!(matches!(pass[0].event, PushedEvent::IndexStarted), "the pass began with {:?}", pass[0]);
    while !matches!(pass.last().unwrap().event, PushedEvent::IndexDone(_)) {
        pass.push(next(stream));
    }
    pass
}

fn assert_contiguous_after(events: &[ServerEvent], after: u64) {
    let ids: Vec<u64> = events.iter().map(|e| e.id).collect();
    let expected: Vec<u64> = (after + 1..after + 1 + ids.len() as u64).collect();
    assert_eq!(ids, expected, "ids must follow {after} with no gap and nothing twice");
}

#[test]
fn the_event_stream_needs_a_token_like_the_operations() {
    let dir = TempDir::new("server-events-auth");
    let ctx = open_ctx(CtxParams { dir: &dir, clock: None, event_capacity: None, fixtures: false });
    let server = start(&ctx, loopback_config());
    let addr = server.addrs().loopback;
    let bearer = format!("Bearer {}", read_token(&ctx.paths().data));

    let refused_op = post_json(addr, "/api/v1/hello", &[], "{}");
    let refused = raw(addr, RawRequest { method: "GET", path: "/api/v1/events", host: None, headers: &[], body: b"" });
    let wrong = raw(addr, RawRequest { method: "GET", path: "/api/v1/events", host: None, headers: &[("Authorization", "Bearer nope")], body: b"" });
    let accepted = raw_head(addr, RawRequest { method: "GET", path: "/api/v1/events", host: None, headers: &[("Authorization", &bearer)], body: b"" });

    assert_eq!(refused.status, 401);
    assert_eq!(refused.body, refused_op.body);
    assert_eq!(wrong.status, 401);
    assert_eq!(wrong.body, refused_op.body);
    assert_eq!(accepted.status, 200);
    assert!(accepted.header("content-type").is_some_and(|t| t.starts_with("text/event-stream")), "{:?}", accepted.headers);
    server.shutdown();
}

#[test]
fn resuming_from_a_last_event_id_replays_exactly_the_newer_events_with_no_gap_or_duplicate() {
    let dir = TempDir::new("server-events-resume");
    let ctx = open_ctx(CtxParams { dir: &dir, clock: None, event_capacity: None, fixtures: true });
    let server = start(&ctx, loopback_config());
    let client = local_client(&server, &ctx.paths().data);

    // A whole pass runs while no stream is connected, so only the server's history can replay it.
    reindex_and_wait(&client);
    let last = newest_id(&client);
    reindex_and_wait(&client);

    let mut stream = client.events(Some(last)).expect("resume the stream");
    let replayed = read_pass(&mut stream);
    assert_contiguous_after(&replayed, last);
    assert!(stream.next_within(Duration::from_millis(500)).is_none(), "the stream sent more than the second pass");

    // A pass that starts while the stream connects is still delivered whole, in order, once.
    let after_second = replayed.last().unwrap().id;
    let third = {
        let client = local_client(&server, &ctx.paths().data);
        std::thread::spawn(move || reindex_and_wait(&client))
    };
    let mut stream = client.events(Some(after_second)).expect("resume again");
    let live = read_pass(&mut stream);
    third.join().expect("the third pass");
    assert_contiguous_after(&live, after_second);
    assert!(stream.next_within(Duration::from_millis(500)).is_none(), "the stream sent more than the third pass");
    drop(stream);
    server.shutdown();
}

#[test]
fn resuming_from_an_id_the_ring_no_longer_holds_starts_with_a_snapshot() {
    let dir = TempDir::new("server-events-snapshot");
    let ctx = open_ctx(CtxParams { dir: &dir, clock: None, event_capacity: Some(2), fixtures: true });
    let server = start(&ctx, loopback_config());
    let client = local_client(&server, &ctx.paths().data);
    reindex_and_wait(&client);

    // Ids start at the start time; a pass publishes more than 2 events, so the first is gone.
    let mut stream = client.events(Some(CLOCK)).expect("open the stream");
    let first = next(&mut stream);

    match first.event {
        PushedEvent::Snapshot { running } => assert!(running.is_empty(), "no job is running: {running:?}"),
        other => panic!("expected a Snapshot, got {other:?}"),
    }
    drop(stream);
    server.shutdown();
}

/// `GET /api/v1/events` on a socket of its own, after its 200 head.
fn open_raw_stream(addr: SocketAddr, header: (&str, &str)) -> TcpStream {
    let mut stream = TcpStream::connect(addr).unwrap();
    let request = format!("GET /api/v1/events HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n{}: {}\r\n\r\n", header.0, header.1);
    stream.write_all(request.as_bytes()).unwrap();
    stream.set_read_timeout(Some(Duration::from_millis(50))).unwrap();
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    let deadline = Instant::now() + Duration::from_secs(10);
    while !head.ends_with(b"\r\n\r\n") {
        assert!(Instant::now() < deadline, "no response head");
        if let Ok(1) = stream.read(&mut byte) {
            head.push(byte[0]);
        }
    }
    assert!(head.starts_with(b"HTTP/1.1 200"), "{}", String::from_utf8_lossy(&head));
    stream
}

/// Whether the server ends the stream within `wait`.
fn ends_within(stream: &mut TcpStream, wait: Duration) -> bool {
    let deadline = Instant::now() + wait;
    let mut seen = Vec::new();
    let mut buf = [0u8; 1024];
    while Instant::now() < deadline {
        match stream.read(&mut buf) {
            Ok(0) => return true,
            Ok(n) => {
                seen.extend_from_slice(&buf[..n]);
                if seen.ends_with(b"\r\n0\r\n\r\n") {
                    return true;
                }
            }
            Err(_) => {}
        }
    }
    false
}

#[test]
fn unpairing_a_device_ends_its_open_event_stream_within_a_second_and_leaves_the_local_one_open() {
    let dir = TempDir::new("server-events-unpair");
    let ctx = open_ctx(CtxParams { dir: &dir, clock: None, event_capacity: None, fixtures: false });
    let server = start(&ctx, loopback_config());
    let addr = server.addrs().loopback;
    let local = local_client(&server, &ctx.paths().data);
    let offer: PairResponse = local.call_named("pair", &PairRequest {}).expect("pair");
    let device = connect(ConnectParams { pairing: &offer.pairing, name: "laptop" }).expect("connect");
    let mut device_stream = open_raw_stream(addr, ("Authorization", &format!("Bearer {}", device.token)));
    let mut local_stream = open_raw_stream(addr, ("Authorization", &format!("Bearer {}", read_token(&ctx.paths().data))));

    local.call_named::<_, ()>("unpair", &UnpairRequest { id: device.device.clone() }).expect("unpair");

    assert!(ends_within(&mut device_stream, Duration::from_millis(1500)), "the unpaired device's stream stayed open");
    assert!(!ends_within(&mut local_stream, Duration::from_millis(500)), "the local stream ended");
    server.shutdown();
}

#[test]
fn unpairing_a_browser_ends_its_cookie_event_stream_within_a_second() {
    let dir = TempDir::new("server-events-unpair-cookie");
    let ctx = open_ctx(CtxParams { dir: &dir, clock: None, event_capacity: None, fixtures: false });
    let server = start(&ctx, loopback_config());
    let addr = server.addrs().loopback;
    let local = local_client(&server, &ctx.paths().data);
    let offer: PairResponse = local.call_named("pair", &PairRequest {}).expect("pair");
    let secret = PairingString::parse(&offer.pairing).unwrap().secret;
    let own = format!("http://{addr}");
    let connected = post_json(addr, "/api/v1/connect", &[("Origin", &own)], &format!(r#"{{"secret":"{secret}","name":"browser","browser":true}}"#));
    let cookie = connected.header("set-cookie").expect("a Set-Cookie header").split(';').next().unwrap().to_string();
    let id = DeviceId(connected.json()["device"].as_str().expect("the device id").to_string());
    let mut stream = open_raw_stream(addr, ("Cookie", &cookie));

    local.call_named::<_, ()>("unpair", &UnpairRequest { id }).expect("unpair");

    assert!(ends_within(&mut stream, Duration::from_millis(1500)), "the unpaired browser's stream stayed open");
    server.shutdown();
}

#[test]
fn after_a_restart_with_a_lower_clock_the_reader_accepts_the_snapshot_and_follows_the_ids_after_it() {
    let dir = TempDir::new("server-events-lower-clock");
    let later = TestClock(Arc::new(AtomicU64::new(CLOCK + 10_000_000)));
    let ctx = open_ctx(CtxParams { dir: &dir, clock: Some(later), event_capacity: None, fixtures: true });
    let server = start(&ctx, loopback_config());
    let client = local_client(&server, &ctx.paths().data);
    reindex_and_wait(&client);
    let last = newest_id(&client);
    server.shutdown();
    drop(ctx);

    let ctx = open_ctx(CtxParams { dir: &dir, clock: None, event_capacity: None, fixtures: false });
    let server = start(&ctx, loopback_config());
    let client = local_client(&server, &ctx.paths().data);
    let mut stream = client.events(Some(last)).expect("resume after the restart");
    let snapshot = next(&mut stream);

    assert!(matches!(snapshot.event, PushedEvent::Snapshot { .. }), "{snapshot:?}");
    assert!(snapshot.id < last, "the restarted server's ids are lower: {} vs {last}", snapshot.id);
    assert_eq!(stream.last_id(), Some(snapshot.id), "the Snapshot resets the reader's last id");
    let follower = local_client(&server, &ctx.paths().data);
    let pass = std::thread::spawn(move || reindex_and_wait(&follower));
    let started = next(&mut stream);
    pass.join().unwrap();
    assert!(matches!(started.event, PushedEvent::IndexStarted), "{started:?}");
    assert_eq!(stream.last_id(), Some(started.id));
    drop(stream);
    server.shutdown();
}
