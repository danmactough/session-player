// `sp cancel <job>` and Ctrl-C while a command follows a job on a server.

#[path = "../../testutil/tempdir.rs"]
mod tempdir;

mod common;

use std::process::Stdio;
use std::time::Duration;

use common::{Machine, ok, server_logs, signal, stderr, wait_exit};
use session_player_core::ops::{ANALYZE_SESSIONS, AnalyzeSessionsRequest, JobId, JobState};
use tempdir::TempDir;

/// Alpha with one indexed widgets session, ready to serve: the machine, the checkout and the
/// session file.
fn alpha_with_a_session(dir: &TempDir) -> (Machine, std::path::PathBuf, std::path::PathBuf) {
    let alpha = Machine::new(dir, "alpha");
    let widgets = alpha.checkout("widgets");
    let session = alpha.session(&widgets, "aaaa1111-widgets", "2026-03-10");
    ok(&alpha.sp(&["index"]), "sp index");
    (alpha, widgets, session)
}

#[test]
fn sp_cancel_stops_a_running_job_on_the_local_server() {
    let dir = TempDir::new("cli-cancel");
    let (alpha, _, session) = alpha_with_a_session(&dir);
    let claude = alpha.fake_claude();
    claude.hold();
    let server = alpha.serve_local();
    let client = server.client().unwrap();
    let paths = vec![session.to_string_lossy().into_owned()];
    let job = client.call(&ANALYZE_SESSIONS, &AnalyzeSessionsRequest { paths, force: false, focus: None }).unwrap();
    claude.wait_for_calls(1);

    ok(&alpha.sp(&["cancel", &job.0]), "sp cancel");
    server.wait_for_state(&job, JobState::Cancelled);
}

#[test]
fn sp_cancel_with_an_unknown_job_id_fails_and_names_it() {
    let dir = TempDir::new("cli-cancel-unknown");
    let alpha = Machine::new(&dir, "alpha");
    let _server = alpha.serve_local();

    let output = alpha.sp(&["cancel", "j1-99"]);
    assert_ne!(output.status.code(), Some(0));
    assert!(stderr(&output).contains("j1-99") && !stderr(&output).contains("panicked"), "{}", stderr(&output));
}

#[test]
fn ctrl_c_while_following_a_job_on_the_server_detaches_and_leaves_the_job_running() {
    let dir = TempDir::new("cli-detach");
    let (alpha, widgets, _) = alpha_with_a_session(&dir);
    let claude = alpha.fake_claude();
    claude.hold();
    let server = alpha.serve_local();

    let mut analyze = alpha
        .command(&["analyze", "--project", widgets.to_str().unwrap(), "--since", "2026-03-01"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    claude.wait_for_calls(1);
    signal(analyze.id(), "INT");
    let status = wait_exit(&mut analyze, Duration::from_secs(10));
    if status.is_none() {
        let _ = analyze.kill();
    }
    let output = analyze.wait_with_output().unwrap();
    let message = stderr(&output);
    assert!(status.is_some(), "sp analyze didn't exit within 10 s of SIGINT: {message}");

    let id = message
        .split_once("still running as ")
        .and_then(|(_, rest)| rest.split(';').next())
        .unwrap_or_else(|| panic!("no detach message: {message}"))
        .to_string();
    assert!(message.contains(&format!("still running as {id}; stop it with sp cancel {id}")), "{message}");
    assert_eq!(output.status.code(), Some(130), "{message}");
    let job = server.job(&JobId(id.clone()));
    assert_eq!(job.state, JobState::Running, "the job keeps running on the server\n{}", server_logs(&alpha.root));

    claude.release();
    server.wait_for_state(&JobId(id), JobState::Finished);
}
