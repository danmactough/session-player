// GET /api/v1/events: authenticated like the operations; resuming from a Last-Event-ID replays
// exactly the newer events, with no gap and nothing twice; an id the ring no longer holds gets a
// Snapshot.
#[path = "../../testutil/tempdir.rs"]
mod tempdir;
mod common;

use std::time::{Duration, Instant};

use common::{CLOCK, CtxParams, RawRequest, loopback_config, local_client, open_ctx, post_json, raw, raw_head, read_token, start};
use session_player_client::{Client, EventStream};
use session_player_core::ops::{JOB, JobId, JobRequest, JobState, PushedEvent, REINDEX, ReindexRequest, ServerEvent};
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
