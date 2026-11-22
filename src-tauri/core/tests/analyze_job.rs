// `analyze_sessions` analyzes many sessions as one job, a few at a time.
#[path = "../../testutil/tempdir.rs"]
mod tempdir;
#[path = "common/ops.rs"]
mod common;

use std::sync::Arc;
use std::time::Duration;

use common::FakeClaude;
use session_player_core::ops::{
    self, ANALYZE_AT_ONCE, ANALYZE_SESSIONS, AnalyzeDone, AnalyzeProgress, AnalyzeSessionsRequest, Ctx, JobId, JobKind, JobState, Progress,
    PushedEvent,
};
use tempdir::TempDir;

fn analyze(ctx: &Arc<Ctx>, ids: &[&str], force: bool) -> JobId {
    let paths = ids.iter().map(|id| common::row(ctx, id).path).collect();
    ops::call(ctx, &ANALYZE_SESSIONS, &AnalyzeSessionsRequest { paths, force, focus: None }).expect("analyze_sessions")
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

fn analyze_progress(events: &[PushedEvent]) -> Vec<AnalyzeProgress> {
    events
        .iter()
        .filter_map(|e| match e {
            PushedEvent::AnalyzeProgress(p) => Some(p.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn analyze_sessions_is_one_job_that_skips_analyzed_sessions_counts_a_failed_one_and_ends_with_one_analyze_done() {
    let dir = TempDir::new("analyze-job-counts");
    let fake = FakeClaude::new(&dir);
    let ctx = common::open_simple(&dir, Some(fake.claude()));
    common::session(&dir, "s1", "Already analyzed");
    common::session(&dir, "s2", "FAIL-ME when analyzed");
    for id in ["s3", "s4", "s5"] {
        common::session(&dir, id, &format!("Work on {id}"));
    }
    common::reindex(&ctx);
    common::mark_analyzed(&ctx, &common::key(&common::row(&ctx, "s1")));

    let id = analyze(&ctx, &["s1", "s2", "s3", "s4", "s5"], false);
    assert_eq!(common::job(&ctx, &id).kind, JobKind::Analyze);
    let (done, events) = common::wait_done(&ctx, &id);

    assert_eq!(analyze_done(&events), [AnalyzeDone { job: id.clone(), analyzed: 3, skipped: 1, failed: 1, cancelled: false }]);
    assert_eq!(done.state, JobState::Failed, "a session that couldn't be analyzed fails the job: {done:?}");
    assert!(done.error.as_deref().is_some_and(|e| e.contains("1 of 4") && !e.contains('\n')), "{done:?}");
    assert_eq!(done.progress, Some(Progress { done: 4, total: 4 }));
    assert_eq!(analyze_progress(&events).last(), Some(&AnalyzeProgress { job: id.clone(), done: 4, total: 4 }));
    assert_eq!(fake.models(), ["haiku"; 4], "the analyzed session is skipped");
    for id in ["s3", "s4", "s5"] {
        assert!(common::is_analyzed(&ctx, &common::key(&common::row(&ctx, id))), "{id} not analyzed");
    }
    assert!(!common::is_analyzed(&ctx, &common::key(&common::row(&ctx, "s2"))));
}

#[test]
fn an_analyze_job_where_every_session_succeeds_finishes() {
    let dir = TempDir::new("analyze-job-finished");
    let fake = FakeClaude::new(&dir);
    let ctx = common::open_simple(&dir, Some(fake.claude()));
    common::session(&dir, "s1", "Work on s1");
    common::session(&dir, "s2", "Work on s2");
    common::reindex(&ctx);

    let id = analyze(&ctx, &["s1", "s2"], false);
    let (done, events) = common::wait_done(&ctx, &id);

    assert_eq!(done.state, JobState::Finished, "{done:?}");
    assert_eq!(done.error, None);
    assert_eq!(analyze_done(&events), [AnalyzeDone { job: id, analyzed: 2, skipped: 0, failed: 0, cancelled: false }]);
}

#[test]
fn analyze_sessions_works_on_three_sessions_at_a_time() {
    let dir = TempDir::new("analyze-job-three");
    let fake = FakeClaude::new(&dir);
    let ctx = common::open_simple(&dir, Some(fake.claude()));
    let ids = ["s1", "s2", "s3", "s4", "s5"];
    for id in ids {
        common::session(&dir, id, &format!("Work on {id}"));
    }
    common::reindex(&ctx);
    fake.hold();

    let id = analyze(&ctx, &ids, false);
    fake.wait_for_calls(ANALYZE_AT_ONCE);
    std::thread::sleep(Duration::from_millis(500));

    assert_eq!(ANALYZE_AT_ONCE, 3);
    assert_eq!(fake.pids().len(), 3, "no more than three at a time");
    fake.release();
    let (done, events) = common::wait_done(&ctx, &id);
    assert_eq!(done.state, JobState::Finished);
    assert_eq!(fake.pids().len(), 5);
    assert_eq!(analyze_done(&events).len(), 1);
}
