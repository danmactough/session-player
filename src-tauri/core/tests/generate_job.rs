// `generate_notes` runs Analyze for one session as a job, with a fake `claude`.
#[path = "../../testutil/tempdir.rs"]
mod tempdir;
#[path = "common/ops.rs"]
mod common;

use std::sync::Arc;

use common::{FAILURE_FIRST_LINE, FakeClaude};
use session_player_core::ops::{self, Ctx, GENERATE_NOTES, GenerateDone, GenerateNotesRequest, JobId, JobKind, JobState, OpError, PushedEvent};
use tempdir::TempDir;

fn generate(ctx: &Arc<Ctx>, native_id: &str) -> Result<JobId, OpError> {
    let row = common::row(ctx, native_id);
    ops::call(ctx, &GENERATE_NOTES, &GenerateNotesRequest { key: common::key(&row), path: row.path.clone(), branch: None, focus: None })
}

fn generate_done(events: &[PushedEvent], key: &str) -> Vec<GenerateDone> {
    events
        .iter()
        .filter_map(|e| match e {
            PushedEvent::GenerateDone(done) if done.key == key => Some(done.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn generate_notes_runs_as_a_generate_job_that_saves_the_analysis_and_publishes_one_generate_done_before_it_leaves_running() {
    let dir = TempDir::new("generate-job-done");
    let fake = FakeClaude::new(&dir);
    let ctx = common::open_simple(&dir, Some(fake.claude()));
    common::session(&dir, "s1", "Add a health check endpoint");
    common::reindex(&ctx);
    let key = common::key(&common::row(&ctx, "s1"));

    let id = generate(&ctx, "s1").expect("generate_notes");
    let started = common::job(&ctx, &id);
    assert_eq!(started.kind, JobKind::Generate);
    assert_eq!(started.key.as_deref(), Some(key.as_str()));

    // The events are read the moment the job is first seen to have left Running.
    let (done, events) = common::wait_done(&ctx, &id);

    assert_eq!(done.state, JobState::Finished, "{done:?}");
    assert_eq!(done.error, None);
    assert_eq!(generate_done(&events, &key), [GenerateDone { key: key.clone(), error: None, cancelled: false, rejected: 0 }]);
    assert!(common::is_analyzed(&ctx, &key), "the analysis was not saved");
    assert_eq!(fake.models(), ["haiku"]);
}

#[test]
fn a_second_generate_job_for_the_same_session_is_a_conflict_while_the_first_runs() {
    let dir = TempDir::new("generate-job-conflict");
    let fake = FakeClaude::new(&dir);
    let ctx = common::open_simple(&dir, Some(fake.claude()));
    common::session(&dir, "s1", "Add a health check endpoint");
    common::session(&dir, "s2", "Rename the config loader");
    common::reindex(&ctx);
    fake.hold();

    let first = generate(&ctx, "s1").expect("generate_notes");
    let again = generate(&ctx, "s1");
    let other = generate(&ctx, "s2");

    assert!(matches!(again, Err(OpError::Conflict(_))), "{again:?}");
    let other = other.expect("another session's job starts");
    fake.release();
    assert_eq!(common::wait_done(&ctx, &first).0.state, JobState::Finished);
    assert_eq!(common::wait_done(&ctx, &other).0.state, JobState::Finished);
    let after = generate(&ctx, "s1").expect("a finished job no longer conflicts");
    assert_eq!(common::wait_done(&ctx, &after).0.state, JobState::Finished);
}

#[test]
fn a_failed_generate_job_keeps_only_the_first_line_of_the_error() {
    let dir = TempDir::new("generate-job-failed");
    let fake = FakeClaude::new(&dir);
    let ctx = common::open_simple(&dir, Some(fake.claude()));
    common::session(&dir, "s1", "Add a health check endpoint");
    common::reindex(&ctx);
    let key = common::key(&common::row(&ctx, "s1"));
    fake.fail();

    let id = generate(&ctx, "s1").expect("generate_notes");
    let (done, events) = common::wait_done(&ctx, &id);

    assert_eq!(done.state, JobState::Failed);
    assert_eq!(done.error.as_deref(), Some(FAILURE_FIRST_LINE));
    assert_eq!(
        generate_done(&events, &key),
        [GenerateDone { key: key.clone(), error: Some(FAILURE_FIRST_LINE.to_string()), cancelled: false, rejected: 0 }]
    );
    assert!(!common::is_analyzed(&ctx, &key));
}

#[test]
fn a_panic_in_a_generate_job_ends_it_failed_and_lets_the_session_be_generated_again() {
    let dir = TempDir::new("generate-job-panic");
    let fake = FakeClaude::new(&dir);
    let ctx = common::open_simple(&dir, Some(fake.claude()));
    common::session(&dir, "s1", "Add a health check endpoint");
    common::reindex(&ctx);
    ctx.set_publish_hook(Some(Arc::new(|event: &PushedEvent| {
        if matches!(event, PushedEvent::GenerateProgress(_)) {
            panic!("a bug in the generate job");
        }
    })));

    let id = generate(&ctx, "s1").expect("generate_notes");
    let (done, _) = common::wait_done(&ctx, &id);
    ctx.set_publish_hook(None);

    assert_eq!(done.state, JobState::Failed, "{done:?}");
    assert!(done.error.is_some(), "{done:?}");
    let next = generate(&ctx, "s1").expect("the session can be generated again, not a Conflict");
    let (next, _) = common::wait_done(&ctx, &next);
    assert_eq!(next.state, JobState::Finished, "{next:?}");
}
