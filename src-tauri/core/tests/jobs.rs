// Jobs: `reindex` starts an index pass as a job on its own thread, and `job` reports how it is going.
#[path = "../../testutil/tempdir.rs"]
mod tempdir;

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use session_player_core::index;
use session_player_core::ops::{
    self, CANCEL_JOB, CancelJobRequest, Ctx, JOB, Job, JobId, JobKind, JobRequest, JobState, OpError, OpenParams, Paths, Progress, REINDEX,
    ReindexRequest,
};
use tempdir::TempDir;

const CLOCK: u64 = 7_000_000;
const FIXTURES: [&str; 3] = ["claude.jsonl", "codex.jsonl", "pi.jsonl"];

fn paths(dir: &Path) -> Paths {
    let data = dir.join("data");
    std::fs::create_dir_all(&data).unwrap();
    Paths { db: data.join("index.sqlite"), library: data.join("library"), logs: data.join("logs"), data }
}

/// A home folder holding the fixture sessions under a registered session folder, and a Ctx on it.
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

/// Holds the database's write lock, so an index pass stays running until `release`.
fn hold_writes(db: &Path) -> rusqlite::Connection {
    let conn = rusqlite::Connection::open(db).unwrap();
    conn.execute_batch("BEGIN IMMEDIATE").unwrap();
    conn
}

fn release(conn: rusqlite::Connection) {
    conn.execute_batch("COMMIT").unwrap();
}

fn job(ctx: &Arc<Ctx>, id: &JobId) -> Result<Job, OpError> {
    ops::call(ctx, &JOB, &JobRequest { id: id.clone() })
}

fn wait_until_done(ctx: &Arc<Ctx>, id: &JobId) -> Job {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let job = job(ctx, id).expect("job");
        if job.state != JobState::Running {
            return job;
        }
        assert!(Instant::now() < deadline, "{id:?} still running: {job:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn is_job_id(id: &JobId) -> bool {
    let Some(n) = id.0.strip_prefix(&format!("j{CLOCK}-")) else { return false };
    !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())
}

#[test]
fn reindex_returns_at_once_while_the_index_job_runs_on_its_own_thread_until_it_finishes() {
    let dir = TempDir::new("jobs-reindex");
    let ctx = open(&dir);
    let lock = hold_writes(&ctx.paths().db);

    let started = Instant::now();
    let id = ops::call(&ctx, &REINDEX, &ReindexRequest { full: false }).expect("reindex");
    assert!(started.elapsed() < Duration::from_secs(2), "reindex waited for the pass");

    let running = job(&ctx, &id).expect("job while running");
    assert_eq!(running.id, id);
    assert_eq!(running.kind, JobKind::Index);
    assert_eq!(running.state, JobState::Running);
    assert_eq!(running.key, None);
    assert_eq!(running.started_at, CLOCK);
    assert_eq!(running.ended_at, None);

    release(lock);
    let done = wait_until_done(&ctx, &id);

    assert_eq!(done.state, JobState::Finished);
    assert_eq!(done.error, None);
    assert_eq!(done.ended_at, Some(CLOCK));
    assert_eq!(done.progress, Some(Progress { done: FIXTURES.len(), total: FIXTURES.len() }));
}

#[test]
fn job_ids_are_j_then_the_start_time_in_ms_then_a_number_and_never_repeat() {
    let dir = TempDir::new("jobs-ids");
    let ctx = open(&dir);

    let first = ops::call(&ctx, &REINDEX, &ReindexRequest { full: false }).expect("reindex");
    wait_until_done(&ctx, &first);
    let second = ops::call(&ctx, &REINDEX, &ReindexRequest { full: true }).expect("reindex");
    wait_until_done(&ctx, &second);

    assert!(is_job_id(&first), "{first:?}");
    assert!(is_job_id(&second), "{second:?}");
    assert_ne!(first, second);
}

#[test]
fn asking_for_an_unknown_job_is_not_found() {
    let dir = TempDir::new("jobs-unknown");
    let ctx = open(&dir);

    let answer = job(&ctx, &JobId(format!("j{CLOCK}-999")));

    assert!(matches!(answer, Err(OpError::NotFound(_))), "{answer:?}");
}

#[test]
fn cancelling_an_index_job_is_refused_and_the_pass_still_finishes() {
    let dir = TempDir::new("jobs-cancel-index");
    let ctx = open(&dir);
    let lock = hold_writes(&ctx.paths().db);
    let id = ops::call(&ctx, &REINDEX, &ReindexRequest { full: false }).expect("reindex");

    let refused = ops::call(&ctx, &CANCEL_JOB, &CancelJobRequest { id: id.clone() });

    assert!(matches!(refused, Err(OpError::Conflict(_))), "{refused:?}");
    release(lock);
    assert_eq!(wait_until_done(&ctx, &id).state, JobState::Finished);
}

#[test]
fn cancelling_an_unknown_job_is_not_found() {
    let dir = TempDir::new("jobs-cancel-unknown");
    let ctx = open(&dir);

    let refused = ops::call(&ctx, &CANCEL_JOB, &CancelJobRequest { id: JobId(format!("j{CLOCK}-999")) });

    assert!(matches!(refused, Err(OpError::NotFound(_))), "{refused:?}");
}
