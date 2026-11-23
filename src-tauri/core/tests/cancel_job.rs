// Only an explicit cancel stops a job, and only the job it names.
#[path = "../../testutil/tempdir.rs"]
mod tempdir;
#[path = "common/ops.rs"]
mod common;

use std::sync::Arc;
use std::time::Duration;

use common::FakeClaude;
use session_player_core::ops::{
    self, CANCEL_GENERATE, CANCEL_JOB, CancelGenerateRequest, CancelJobRequest, Ctx, GENERATE_NOTES, GenerateDone, GenerateNotesRequest, JobId,
    JobState, PushedEvent,
};
use tempdir::TempDir;

fn generate(ctx: &Arc<Ctx>, native_id: &str) -> JobId {
    let row = common::row(ctx, native_id);
    ops::call(ctx, &GENERATE_NOTES, &GenerateNotesRequest { key: common::key(&row), path: row.path.clone(), branch: None, focus: None }).expect("generate_notes")
}

fn setup(dir: &TempDir, ids: &[&str]) -> (FakeClaude, Arc<Ctx>) {
    let fake = FakeClaude::new(dir);
    let ctx = common::open_simple(dir, Some(fake.claude()));
    for id in ids {
        common::session(dir, id, &format!("Work on {id}"));
    }
    common::reindex(&ctx);
    (fake, ctx)
}

fn cancelled_done(events: &[PushedEvent], key: &str) -> Vec<GenerateDone> {
    events
        .iter()
        .filter_map(|e| match e {
            PushedEvent::GenerateDone(done) if done.key == key => Some(done.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn cancel_job_stops_a_generate_job_and_kills_its_claude_process() {
    let dir = TempDir::new("cancel-job-generate");
    let (fake, ctx) = setup(&dir, &["s1"]);
    let key = common::key(&common::row(&ctx, "s1"));
    fake.hold();
    let id = generate(&ctx, "s1");
    fake.wait_for_calls(1);
    let pid = fake.pids()[0];

    ops::call(&ctx, &CANCEL_JOB, &CancelJobRequest { id: id.clone() }).expect("cancel_job");
    let (done, events) = common::wait_done(&ctx, &id);

    assert_eq!(done.state, JobState::Cancelled);
    common::wait_until("the claude process is gone", || !common::alive(pid));
    assert_eq!(cancelled_done(&events, &key), [GenerateDone { key: key.clone(), error: None, cancelled: true, rejected: 0 }]);
    assert!(!common::is_analyzed(&ctx, &key));
    fake.release();
}

#[test]
fn cancel_generate_by_key_stops_that_sessions_job_and_kills_its_claude_process() {
    let dir = TempDir::new("cancel-generate-key");
    let (fake, ctx) = setup(&dir, &["s1"]);
    let key = common::key(&common::row(&ctx, "s1"));
    fake.hold();
    let id = generate(&ctx, "s1");
    fake.wait_for_calls(1);
    let pid = fake.pids()[0];

    ops::call(&ctx, &CANCEL_GENERATE, &CancelGenerateRequest { key: key.clone() }).expect("cancel_generate");
    let (done, events) = common::wait_done(&ctx, &id);

    assert_eq!(done.state, JobState::Cancelled);
    common::wait_until("the claude process is gone", || !common::alive(pid));
    assert_eq!(cancelled_done(&events, &key).len(), 1, "{events:?}");
    assert!(cancelled_done(&events, &key)[0].cancelled);
    fake.release();
}

#[test]
fn cancelling_one_job_leaves_another_running_until_it_finishes() {
    let dir = TempDir::new("cancel-job-only-one");
    let (fake, ctx) = setup(&dir, &["s1", "s2"]);
    fake.hold();
    let first = generate(&ctx, "s1");
    let second = generate(&ctx, "s2");
    fake.wait_for_calls(2);

    ops::call(&ctx, &CANCEL_JOB, &CancelJobRequest { id: first.clone() }).expect("cancel_job");
    assert_eq!(common::wait_done(&ctx, &first).0.state, JobState::Cancelled);
    std::thread::sleep(Duration::from_millis(300));

    assert_eq!(common::job(&ctx, &second).state, JobState::Running);
    assert_eq!(fake.pids().iter().filter(|pid| common::alive(**pid)).count(), 1, "the other job's claude still runs");
    fake.release();
    assert_eq!(common::wait_done(&ctx, &second).0.state, JobState::Finished);
}

#[test]
fn a_caller_that_goes_away_does_not_cancel_the_job_it_started() {
    let dir = TempDir::new("cancel-job-disconnect");
    let (fake, ctx) = setup(&dir, &["s1"]);
    fake.hold();

    // The caller starts the job on its own thread and is gone before the job ends.
    let caller = {
        let ctx = ctx.clone();
        std::thread::spawn(move || generate(&ctx, "s1"))
    };
    let id = caller.join().expect("the caller's thread");
    fake.wait_for_calls(1);
    std::thread::sleep(Duration::from_millis(300));

    assert_eq!(common::job(&ctx, &id).state, JobState::Running);
    fake.release();
    let (done, _) = common::wait_done(&ctx, &id);
    assert_eq!(done.state, JobState::Finished);
}
