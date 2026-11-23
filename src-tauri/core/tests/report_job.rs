// `generate_report` writes and saves a report as a job, analyzing sessions first when asked.
#[path = "../../testutil/tempdir.rs"]
mod tempdir;
#[path = "common/ops.rs"]
mod common;

use std::sync::Arc;

use common::FakeClaude;
use session_player_core::ops::{
    self, Ctx, GENERATE_REPORT, GET_REPORT, GenerateReportRequest, GetReportRequest, JobId, JobKind, JobState, OpError, PushedEvent, ReportDone,
};
use session_player_core::test_support::reports;
use tempdir::TempDir;

fn report(ctx: &Arc<Ctx>, ids: &[&str], model: Option<&str>, analyze_first: bool) -> Result<JobId, OpError> {
    let paths = ids.iter().map(|id| common::row(ctx, id).path).collect();
    ops::call(ctx, &GENERATE_REPORT, &GenerateReportRequest { paths, model: model.map(str::to_string), focus: None, analyze_first })
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
    (fake, ctx)
}

fn saved_keys(ctx: &Arc<Ctx>, done: &ReportDone) -> Vec<String> {
    let id = done.report.clone().expect("the report was saved");
    let saved = ops::call(ctx, &GET_REPORT, &GetReportRequest { id }).expect("get_report").expect("the saved report");
    let mut keys: Vec<String> = saved.sessions.into_iter().map(|s| s.key).collect();
    keys.sort();
    keys
}

#[test]
fn with_analyze_first_the_unanalyzed_sessions_are_analyzed_then_the_report_is_written_with_the_default_model_and_saved() {
    let dir = TempDir::new("report-job-analyze-first");
    let (fake, ctx) = setup(&dir);

    let id = report(&ctx, &["s1", "s2"], None, true).expect("generate_report");
    assert_eq!(common::job(&ctx, &id).kind, JobKind::Report);
    let (done, events) = common::wait_done(&ctx, &id);

    assert_eq!(done.state, JobState::Finished, "{done:?}");
    assert_eq!(reports::DEFAULT_MODEL, "sonnet");
    assert_eq!(fake.models(), ["haiku", reports::DEFAULT_MODEL], "s2 is analyzed, then the report is written");
    assert!(common::is_analyzed(&ctx, &common::key(&common::row(&ctx, "s2"))));
    let done_events = report_done(&events);
    assert_eq!(done_events.len(), 1, "{done_events:?}");
    assert_eq!((done_events[0].job.as_str(), &done_events[0].error, done_events[0].cancelled), (id.0.as_str(), &None, false));
    let mut expected = vec![common::key(&common::row(&ctx, "s1")), common::key(&common::row(&ctx, "s2"))];
    expected.sort();
    assert_eq!(saved_keys(&ctx, &done_events[0]), expected);
}

#[test]
fn without_analyze_first_sessions_with_no_analysis_are_left_out_and_the_model_asked_for_is_used() {
    let dir = TempDir::new("report-job-left-out");
    let (fake, ctx) = setup(&dir);

    let id = report(&ctx, &["s1", "s2"], Some("opus"), false).expect("generate_report");
    let (done, events) = common::wait_done(&ctx, &id);

    assert_eq!(done.state, JobState::Finished, "{done:?}");
    assert_eq!(fake.models(), ["opus"]);
    let done_events = report_done(&events);
    assert_eq!(done_events.len(), 1, "{done_events:?}");
    assert_eq!(saved_keys(&ctx, &done_events[0]), [common::key(&common::row(&ctx, "s1"))]);
}

#[test]
fn a_report_on_sessions_none_of_which_has_an_analysis_fails_without_asking_claude() {
    let dir = TempDir::new("report-job-none");
    let (fake, ctx) = setup(&dir);

    let id = report(&ctx, &["s2"], None, false).expect("generate_report");
    let (done, events) = common::wait_done(&ctx, &id);

    assert_eq!(done.state, JobState::Failed);
    assert!(done.error.is_some());
    assert!(fake.models().is_empty(), "{:?}", fake.models());
    let done_events = report_done(&events);
    assert_eq!(done_events.len(), 1);
    assert_eq!(done_events[0].report, None);
    assert_eq!(done_events[0].error, done.error);
}

#[test]
fn a_report_on_no_sessions_is_a_bad_request() {
    let dir = TempDir::new("report-job-empty");
    let (_fake, ctx) = setup(&dir);

    let refused = report(&ctx, &[], None, true);

    assert!(matches!(refused, Err(OpError::BadRequest(_))), "{refused:?}");
}

#[test]
fn a_panic_in_a_report_job_ends_it_failed_and_lets_the_next_report_start() {
    let dir = TempDir::new("report-job-panic");
    let (_fake, ctx) = setup(&dir);
    ctx.set_publish_hook(Some(Arc::new(|event: &PushedEvent| {
        if matches!(event, PushedEvent::ReportProgress(_)) {
            panic!("a bug in the report job");
        }
    })));

    let id = report(&ctx, &["s1"], None, false).expect("generate_report");
    let (done, _) = common::wait_done(&ctx, &id);
    ctx.set_publish_hook(None);

    assert_eq!(done.state, JobState::Failed, "{done:?}");
    assert!(done.error.is_some(), "{done:?}");
    let next = report(&ctx, &["s1"], None, false).expect("the next report starts, not a Conflict");
    let (next, _) = common::wait_done(&ctx, &next);
    assert_eq!(next.state, JobState::Finished, "{next:?}");
}

#[test]
fn the_next_report_can_start_the_moment_the_report_done_event_is_published() {
    let dir = TempDir::new("report-job-done-then-next");
    let (_fake, ctx) = setup(&dir);
    let path = common::row(&ctx, "s1").path;
    let started = Arc::new(std::sync::Mutex::new(None));
    let weak = Arc::downgrade(&ctx);
    let seen = started.clone();
    ctx.set_publish_hook(Some(Arc::new(move |event: &PushedEvent| {
        if matches!(event, PushedEvent::ReportDone(_))
            && let Some(ctx) = weak.upgrade()
            && seen.lock().unwrap().is_none()
        {
            let next = ops::call(&ctx, &GENERATE_REPORT, &GenerateReportRequest { paths: vec![path.clone()], model: None, focus: None, analyze_first: false });
            *seen.lock().unwrap() = Some(next);
        }
    })));

    let id = report(&ctx, &["s1"], None, false).expect("generate_report");
    common::wait_done(&ctx, &id);
    common::wait_until("the hook ran", || started.lock().unwrap().is_some());
    ctx.set_publish_hook(None);

    let next = started.lock().unwrap().take().unwrap().expect("the next report started, not a Conflict");
    let (next, _) = common::wait_done(&ctx, &next);
    assert_eq!(next.state, JobState::Finished, "{next:?}");
}
