// Index requests coalesce: a burst during a running pass joins it and leaves at most one follow-up
// pass, full if any request in the burst was full; nothing runs while nobody asks.
#[path = "../../testutil/tempdir.rs"]
mod tempdir;

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use session_player_core::index::{self, IndexStats};
use session_player_core::ops::{self, Ctx, EventsSince, JobId, OpenParams, Paths, PushedEvent, REINDEX, ReindexRequest, ServerEvent};
use tempdir::TempDir;

const CLOCK: u64 = 7_000_000;
const FIXTURES: [&str; 3] = ["claude.jsonl", "codex.jsonl", "pi.jsonl"];
const BURST: usize = 25;

fn paths(dir: &Path) -> Paths {
    let data = dir.join("data");
    std::fs::create_dir_all(&data).unwrap();
    Paths { db: data.join("index.sqlite"), library: data.join("library"), logs: data.join("logs"), data }
}

fn open(dir: &Path) -> Arc<Ctx> {
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
        event_capacity: None,
        extensions: vec![],
    })
    .expect("open the Ctx")
}

/// Holds the database's write lock, so the first pass stays running until `release`.
fn hold_writes(db: &Path) -> rusqlite::Connection {
    let conn = rusqlite::Connection::open(db).unwrap();
    conn.execute_batch("BEGIN IMMEDIATE").unwrap();
    conn
}

fn release(conn: rusqlite::Connection) {
    conn.execute_batch("COMMIT").unwrap();
}

fn reindex(ctx: &Arc<Ctx>, full: bool) -> JobId {
    ops::call(ctx, &REINDEX, &ReindexRequest { full }).expect("reindex")
}

/// Sends BURST reindex requests at once, the one at `full_at` (if any) asking for a full pass.
fn burst(ctx: &Arc<Ctx>, full_at: Option<usize>) -> Vec<JobId> {
    let threads: Vec<_> = (0..BURST)
        .map(|i| {
            let ctx = ctx.clone();
            std::thread::spawn(move || reindex(&ctx, full_at == Some(i)))
        })
        .collect();
    threads.into_iter().map(|t| t.join().unwrap()).collect()
}

/// Every event from the start until `passes` index passes are done, then any that follow within a
/// short quiet period.
fn index_events(ctx: &Ctx, passes: usize) -> Vec<ServerEvent> {
    let mut events: Vec<ServerEvent> = Vec::new();
    let mut cursor = CLOCK - 1;
    let deadline = Instant::now() + Duration::from_secs(20);
    let done = |events: &[ServerEvent]| events.iter().filter(|e| matches!(e.event, PushedEvent::IndexDone(_))).count();
    while done(&events) < passes {
        assert!(Instant::now() < deadline, "only {} of {passes} passes finished: {events:?}", done(&events));
        let more = ctx.events_since(EventsSince { after: Some(cursor), wait: Duration::from_millis(200) });
        if let Some(last) = more.last() {
            cursor = last.id;
        }
        events.extend(more);
    }
    events.extend(ctx.events_since(EventsSince { after: Some(cursor), wait: Duration::from_millis(500) }));
    assert!(!events.iter().any(|e| matches!(e.event, PushedEvent::Snapshot { .. })), "{events:?}");
    events
}

fn passes_started(events: &[ServerEvent]) -> usize {
    events.iter().filter(|e| matches!(e.event, PushedEvent::IndexStarted)).count()
}

fn stats(events: &[ServerEvent]) -> Vec<IndexStats> {
    events
        .iter()
        .filter_map(|e| match &e.event {
            PushedEvent::IndexDone(stats) => Some(stats.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn reindex_requests_during_a_running_pass_return_the_running_jobs_id() {
    let dir = TempDir::new("gate-same-id");
    let ctx = open(&dir);
    let lock = hold_writes(&ctx.paths().db);
    let running = reindex(&ctx, false);

    let ids = burst(&ctx, Some(7));

    release(lock);
    index_events(&ctx, 2);
    assert!(ids.iter().all(|id| *id == running), "running {running:?}, burst got {ids:?}");
}

#[test]
fn a_burst_during_a_pass_leaves_exactly_one_follow_up_pass() {
    let dir = TempDir::new("gate-one-follow-up");
    let ctx = open(&dir);
    let lock = hold_writes(&ctx.paths().db);
    reindex(&ctx, false);

    burst(&ctx, None);
    release(lock);
    let events = index_events(&ctx, 2);

    assert_eq!(passes_started(&events), 2, "{events:?}");
    let stats = stats(&events);
    assert_eq!(stats[0].updated, FIXTURES.len());
    assert_eq!(stats[1].updated, 0, "the follow-up pass should be incremental");
}

#[test]
fn the_follow_up_pass_is_full_when_any_request_in_the_burst_was_full() {
    let dir = TempDir::new("gate-full-follow-up");
    let ctx = open(&dir);
    let lock = hold_writes(&ctx.paths().db);
    reindex(&ctx, false);

    burst(&ctx, Some(BURST - 1));
    release(lock);
    let events = index_events(&ctx, 2);

    assert_eq!(passes_started(&events), 2, "{events:?}");
    assert_eq!(stats(&events)[1].updated, FIXTURES.len(), "a full pass reads every session again");
}

#[test]
fn an_idle_ctx_runs_no_index_passes() {
    let dir = TempDir::new("gate-idle");
    let ctx = open(&dir);

    let events = ctx.events_since(EventsSince { after: Some(CLOCK - 1), wait: Duration::from_millis(500) });

    assert!(events.is_empty(), "{events:?}");
    let conn = index::open(&ctx.paths().db).unwrap();
    assert!(index::list(&conn).unwrap().is_empty(), "something indexed the sessions");
}
