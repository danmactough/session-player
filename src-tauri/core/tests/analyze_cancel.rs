// An Analyze job can be cancelled, one runs at a time, and `force` analyzes sessions again.
#[path = "../../testutil/tempdir.rs"]
mod tempdir;
#[path = "common/ops.rs"]
mod common;

use std::sync::Arc;
use std::time::Duration;

use common::FakeClaude;
use session_player_core::ops::{
    self, ANALYZE_SESSIONS, AnalyzeDone, AnalyzeSessionsRequest, CANCEL_JOB, CancelJobRequest, Ctx, JobId, JobState, OpError, PushedEvent,
};
use tempdir::TempDir;

fn analyze(ctx: &Arc<Ctx>, ids: &[&str], force: bool) -> Result<JobId, OpError> {
    let paths = ids.iter().map(|id| common::row(ctx, id).path).collect();
    ops::call(ctx, &ANALYZE_SESSIONS, &AnalyzeSessionsRequest { paths, force, focus: None })
}

fn analyze_done(events: &[PushedEvent]) -> Vec<AnalyzeDone> {
    events
        .iter()
        .filter_map(|e| match e {
            PushedEvent::AnalyzeDone(done) => Some(done.clone()),
            _ => None,
        })
        .collect()
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

#[test]
fn cancelling_an_analyze_job_kills_its_claude_processes_starts_no_more_and_ends_with_one_cancelled_analyze_done() {
    let dir = TempDir::new("analyze-cancel");
    let ids = ["s1", "s2", "s3", "s4", "s5"];
    let (fake, ctx) = setup(&dir, &ids);
    fake.hold();
    let id = analyze(&ctx, &ids, false).expect("analyze_sessions");
    fake.wait_for_calls(3);
    let running = fake.pids();

    ops::call(&ctx, &CANCEL_JOB, &CancelJobRequest { id: id.clone() }).expect("cancel_job");
    let (done, _) = common::wait_done(&ctx, &id);
    fake.release();
    std::thread::sleep(Duration::from_millis(500));

    assert_eq!(done.state, JobState::Cancelled);
    common::wait_until("every claude process is gone", || running.iter().all(|pid| !common::alive(*pid)));
    assert_eq!(fake.pids().len(), 3, "no session started after the cancel");
    let done_events = analyze_done(&common::events(&ctx));
    assert_eq!(done_events.len(), 1, "{done_events:?}");
    assert!(done_events[0].cancelled);
    assert_eq!(done_events[0].job, id);
    for id in ids {
        assert!(!common::is_analyzed(&ctx, &common::key(&common::row(&ctx, id))), "{id} was analyzed after the cancel");
    }
}

#[test]
fn a_second_analyze_job_while_one_runs_is_a_conflict() {
    let dir = TempDir::new("analyze-conflict");
    let (fake, ctx) = setup(&dir, &["s1", "s2"]);
    fake.hold();
    let first = analyze(&ctx, &["s1"], false).expect("analyze_sessions");

    let second = analyze(&ctx, &["s2"], false);

    assert!(matches!(second, Err(OpError::Conflict(_))), "{second:?}");
    fake.release();
    assert_eq!(common::wait_done(&ctx, &first).0.state, JobState::Finished);
    let after = analyze(&ctx, &["s2"], false).expect("a finished job no longer conflicts");
    assert_eq!(common::wait_done(&ctx, &after).0.state, JobState::Finished);
}

#[test]
fn analyzed_sessions_are_skipped_unless_force_analyzes_them_again() {
    let dir = TempDir::new("analyze-force");
    let (fake, ctx) = setup(&dir, &["s1"]);
    common::mark_analyzed(&ctx, &common::key(&common::row(&ctx, "s1")));

    let skipped = analyze(&ctx, &["s1"], false).expect("analyze_sessions");
    let (skipped_job, events) = common::wait_done(&ctx, &skipped);
    assert_eq!(skipped_job.state, JobState::Finished);
    assert_eq!(analyze_done(&events), [AnalyzeDone { job: skipped.clone(), analyzed: 0, skipped: 1, failed: 0, cancelled: false }]);
    assert!(fake.models().is_empty(), "claude was asked about an analyzed session");

    let forced = analyze(&ctx, &["s1"], true).expect("analyze_sessions with force");
    let (forced_job, events) = common::wait_done(&ctx, &forced);

    assert_eq!(forced_job.state, JobState::Finished);
    let forced_done: Vec<AnalyzeDone> = analyze_done(&events).into_iter().filter(|d| d.job == forced).collect();
    assert_eq!(forced_done, [AnalyzeDone { job: forced, analyzed: 1, skipped: 0, failed: 0, cancelled: false }]);
    assert_eq!(fake.models(), ["haiku"]);
}
