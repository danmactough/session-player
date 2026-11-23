// A Report job that can't finish ends cleanly, one runs at a time, and progress is published.
#[path = "../../testutil/tempdir.rs"]
mod tempdir;
#[path = "common/ops.rs"]
mod common;

use std::sync::Arc;

use common::{FAILURE_FIRST_LINE, FakeClaude};
use session_player_core::ops::{
    self, CANCEL_REPORT, CancelReportRequest, Ctx, GENERATE_REPORT, GenerateReportRequest, JobId, JobState, LIST_REPORTS, ListReportsRequest,
    OpError, Progress, PushedEvent, ReportDone, ReportProgress,
};
use tempdir::TempDir;

fn report(ctx: &Arc<Ctx>, ids: &[&str]) -> Result<JobId, OpError> {
    let paths = ids.iter().map(|id| common::row(ctx, id).path).collect();
    ops::call(ctx, &GENERATE_REPORT, &GenerateReportRequest { paths, model: None, focus: None, analyze_first: false })
}

fn report_done(events: &[PushedEvent]) -> Vec<ReportDone> {
    events
        .iter()
        .filter_map(|e| match e {
            PushedEvent::ReportDone(done) => Some(done.clone()),
            _ => None,
        })
        .collect()
}

fn setup(dir: &TempDir) -> (FakeClaude, Arc<Ctx>) {
    let fake = FakeClaude::new(dir);
    let ctx = common::open_simple(dir, Some(fake.claude()));
    common::session(dir, "s1", "Work on s1");
    common::session(dir, "s2", "Work on s2");
    common::reindex(&ctx);
    common::mark_analyzed(&ctx, &common::key(&common::row(&ctx, "s1")));
    common::mark_analyzed(&ctx, &common::key(&common::row(&ctx, "s2")));
    (fake, ctx)
}

fn saved_reports(ctx: &Arc<Ctx>) -> usize {
    ops::call(ctx, &LIST_REPORTS, &ListReportsRequest {}).expect("list_reports").len()
}

#[test]
fn a_report_job_whose_claude_call_fails_ends_failed_with_the_first_line_of_the_error() {
    let dir = TempDir::new("report-failure-claude");
    let (fake, ctx) = setup(&dir);
    fake.fail();

    let id = report(&ctx, &["s1"]).expect("generate_report");
    let (done, events) = common::wait_done(&ctx, &id);

    assert_eq!(done.state, JobState::Failed);
    assert_eq!(done.error.as_deref(), Some(FAILURE_FIRST_LINE));
    assert_eq!(report_done(&events), [ReportDone { job: id.0.clone(), report: None, error: Some(FAILURE_FIRST_LINE.to_string()), cancelled: false }]);
    assert_eq!(saved_reports(&ctx), 0);
}

#[test]
fn cancel_report_stops_the_job_and_kills_its_claude_process() {
    let dir = TempDir::new("report-failure-cancel");
    let (fake, ctx) = setup(&dir);
    fake.hold();
    let id = report(&ctx, &["s1"]).expect("generate_report");
    fake.wait_for_calls(1);
    let pid = fake.pids()[0];

    ops::call(&ctx, &CANCEL_REPORT, &CancelReportRequest { job: id.0.clone() }).expect("cancel_report");
    let (done, events) = common::wait_done(&ctx, &id);

    assert_eq!(done.state, JobState::Cancelled);
    common::wait_until("the claude process is gone", || !common::alive(pid));
    assert_eq!(report_done(&events), [ReportDone { job: id.0.clone(), report: None, error: None, cancelled: true }]);
    fake.release();
    assert_eq!(saved_reports(&ctx), 0);
}

#[test]
fn a_second_report_job_while_one_runs_is_a_conflict() {
    let dir = TempDir::new("report-failure-conflict");
    let (fake, ctx) = setup(&dir);
    fake.hold();
    let first = report(&ctx, &["s1"]).expect("generate_report");

    let second = report(&ctx, &["s2"]);

    assert!(matches!(second, Err(OpError::Conflict(_))), "{second:?}");
    fake.release();
    assert_eq!(common::wait_done(&ctx, &first).0.state, JobState::Finished);
}

#[test]
fn a_report_job_publishes_progress_then_one_report_done_naming_the_saved_report() {
    let dir = TempDir::new("report-failure-progress");
    let (_fake, ctx) = setup(&dir);

    let id = report(&ctx, &["s1", "s2"]).expect("generate_report");
    let (done, events) = common::wait_done(&ctx, &id);

    assert_eq!(done.state, JobState::Finished, "{done:?}");
    assert_eq!(done.progress, Some(Progress { done: 1, total: 1 }));
    let progress: Vec<ReportProgress> = events
        .iter()
        .filter_map(|e| match e {
            PushedEvent::ReportProgress(p) => Some(p.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(progress, [ReportProgress { job: id.0.clone(), done: 0, total: 1 }, ReportProgress { job: id.0.clone(), done: 1, total: 1 }]);
    let last_progress = events.iter().rposition(|e| matches!(e, PushedEvent::ReportProgress(_))).unwrap();
    let done_at = events.iter().position(|e| matches!(e, PushedEvent::ReportDone(_))).expect("a report-done event");
    assert!(done_at > last_progress, "report-done comes last");
    let done_events = report_done(&events);
    assert_eq!(done_events.len(), 1);
    assert!(done_events[0].report.is_some());
    assert_eq!(saved_reports(&ctx), 1);
}
