// Pushed events: numbered from the clock, resumable by id, kept in a bounded ring, with a Snapshot
// for an id the ring no longer holds, and a wait for the next event.
#[path = "../../testutil/tempdir.rs"]
mod tempdir;

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use session_player_core::index;
use session_player_core::ops::{
    self, Ctx, EventsSince, JOB, JobId, JobRequest, JobState, OpenParams, Paths, PushedEvent, REINDEX, ReindexRequest, ServerEvent,
};
use tempdir::TempDir;

const CLOCK: u64 = 7_000_000;
const FIXTURES: [&str; 3] = ["claude.jsonl", "codex.jsonl", "pi.jsonl"];

fn paths(dir: &Path) -> Paths {
    let data = dir.join("data");
    std::fs::create_dir_all(&data).unwrap();
    Paths { db: data.join("index.sqlite"), library: data.join("library"), logs: data.join("logs"), data }
}

fn open(dir: &Path, event_capacity: Option<usize>) -> Arc<Ctx> {
    let home = dir.join("home");
    let root = home.join(".claude/projects/fixtures");
    std::fs::create_dir_all(&root).unwrap();
    for name in FIXTURES {
        std::fs::copy(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name), root.join(name)).unwrap();
    }
    let paths = paths(dir);
    let conn = index::open(&paths.db).unwrap();
    index::add_root(&conn, &home.join(".claude/projects").to_string_lossy()).unwrap();
    drop(conn);
    Ctx::open(OpenParams {
        paths,
        home,
        version: "9.9.9".to_string(),
        claude: None,
        clock: Some(Arc::new(|| CLOCK)),
        event_capacity,
        extensions: vec![],
    })
    .expect("open the Ctx")
}

fn hold_writes(db: &Path) -> rusqlite::Connection {
    let conn = rusqlite::Connection::open(db).unwrap();
    conn.execute_batch("BEGIN IMMEDIATE").unwrap();
    conn
}

fn release(conn: rusqlite::Connection) {
    conn.execute_batch("COMMIT").unwrap();
}

fn reindex_and_wait(ctx: &Arc<Ctx>) -> JobId {
    let id = ops::call(ctx, &REINDEX, &ReindexRequest { full: false }).expect("reindex");
    wait_for(ctx, &id);
    id
}

fn wait_for(ctx: &Arc<Ctx>, id: &JobId) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while ops::call(ctx, &JOB, &JobRequest { id: id.clone() }).expect("job").state == JobState::Running {
        assert!(Instant::now() < deadline, "the index pass never finished");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn since(ctx: &Ctx, after: u64) -> Vec<ServerEvent> {
    ctx.events_since(EventsSince { after: Some(after), wait: Duration::ZERO })
}

fn snapshot(events: &[ServerEvent]) -> Option<&Vec<session_player_core::ops::Job>> {
    match events {
        [ServerEvent { event: PushedEvent::Snapshot { running }, .. }] => Some(running),
        _ => None,
    }
}

#[test]
fn an_index_pass_publishes_started_progress_and_done_with_ids_that_start_at_the_clock_and_increase() {
    let dir = TempDir::new("events-numbered");
    let ctx = open(&dir, None);

    reindex_and_wait(&ctx);
    let events = since(&ctx, CLOCK - 1);

    assert!(events.len() >= 3, "{events:?}");
    assert_eq!(events[0].id, CLOCK);
    assert!(events.windows(2).all(|w| w[0].id < w[1].id), "ids not increasing: {events:?}");
    assert!(events.iter().all(|e| e.at == CLOCK), "{events:?}");
    assert!(matches!(events[0].event, PushedEvent::IndexStarted), "{:?}", events[0]);
    assert!(events[1..events.len() - 1].iter().all(|e| matches!(e.event, PushedEvent::IndexProgress(_))), "{events:?}");
    assert!(events.iter().any(|e| matches!(e.event, PushedEvent::IndexProgress(p) if p.done == FIXTURES.len() && p.total == FIXTURES.len())));
    assert!(matches!(&events[events.len() - 1].event, PushedEvent::IndexDone(stats) if stats.updated == FIXTURES.len()), "{events:?}");
}

#[test]
fn resuming_after_an_id_returns_only_the_newer_events() {
    let dir = TempDir::new("events-resume");
    let ctx = open(&dir, None);
    reindex_and_wait(&ctx);
    let all = since(&ctx, CLOCK - 1);
    let middle = all[all.len() / 2].id;
    let newest = all[all.len() - 1].id;

    let newer: Vec<u64> = since(&ctx, middle).iter().map(|e| e.id).collect();

    assert_eq!(newer, all.iter().map(|e| e.id).filter(|id| *id > middle).collect::<Vec<_>>());
    assert!(since(&ctx, newest).is_empty());
}

#[test]
fn an_id_the_ring_no_longer_holds_gets_one_snapshot_whose_id_is_the_newest() {
    let dir = TempDir::new("events-ring");
    let ctx = open(&dir, Some(3));

    reindex_and_wait(&ctx);
    let events = since(&ctx, CLOCK - 1);

    let running = snapshot(&events).unwrap_or_else(|| panic!("expected one Snapshot, got {events:?}"));
    assert!(running.is_empty(), "{running:?}");
    let newest = events[0].id;
    assert!(since(&ctx, newest).is_empty(), "resuming after the snapshot repeated events");
    let last = since(&ctx, newest - 1);
    assert_eq!(last.len(), 1, "{last:?}");
    assert_eq!(last[0].id, newest);
    assert!(matches!(last[0].event, PushedEvent::IndexDone(_)), "{last:?}");
}

#[test]
fn an_id_from_before_this_run_gets_a_snapshot() {
    let dir = TempDir::new("events-previous-run");
    let ctx = open(&dir, None);

    reindex_and_wait(&ctx);
    let events = since(&ctx, CLOCK - 1_000_000);

    assert!(snapshot(&events).is_some(), "expected one Snapshot, got {events:?}");
}

#[test]
fn a_snapshot_lists_the_jobs_still_running() {
    let dir = TempDir::new("events-snapshot-running");
    let ctx = open(&dir, None);
    let lock = hold_writes(&ctx.paths().db);
    let id = ops::call(&ctx, &REINDEX, &ReindexRequest { full: false }).expect("reindex");

    let events = since(&ctx, 1);

    let running = snapshot(&events).unwrap_or_else(|| panic!("expected one Snapshot, got {events:?}")).clone();
    release(lock);
    wait_for(&ctx, &id);
    assert_eq!(running.iter().map(|j| j.id.clone()).collect::<Vec<_>>(), vec![id]);
    assert_eq!(running[0].state, JobState::Running);
}

#[test]
fn events_since_with_a_wait_returns_as_soon_as_an_event_arrives() {
    let dir = TempDir::new("events-wait");
    let ctx = open(&dir, None);
    let waiter = {
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let started = Instant::now();
            let events = ctx.events_since(EventsSince { after: Some(CLOCK - 1), wait: Duration::from_secs(10) });
            (events, started.elapsed())
        })
    };

    std::thread::sleep(Duration::from_millis(300));
    let id = ops::call(&ctx, &REINDEX, &ReindexRequest { full: false }).expect("reindex");
    let (events, waited) = waiter.join().unwrap();
    wait_for(&ctx, &id);

    assert!(!events.is_empty());
    assert_eq!(events[0].id, CLOCK);
    assert!(matches!(events[0].event, PushedEvent::IndexStarted), "{events:?}");
    assert!(waited < Duration::from_secs(5), "waited {waited:?}");
}

#[test]
fn events_since_with_a_wait_returns_nothing_after_the_wait_when_no_event_arrives() {
    let dir = TempDir::new("events-wait-empty");
    let ctx = open(&dir, None);

    let started = Instant::now();
    let events = ctx.events_since(EventsSince { after: Some(CLOCK - 1), wait: Duration::from_millis(300) });

    assert!(events.is_empty(), "{events:?}");
    assert!(started.elapsed() >= Duration::from_millis(300));
}
