// `sp --server <name> analyze` runs Analyze as a job on the paired server, with the server's claude.

#[path = "../../testutil/tempdir.rs"]
mod tempdir;

mod common;

use common::{Machine, ok, server_logs, stderr, stdout};
use tempdir::TempDir;

#[test]
fn sp_analyze_with_a_paired_server_analyzes_the_servers_sessions_with_the_servers_claude() {
    let dir = TempDir::new("cli-remote-analyze");
    let alpha = Machine::new(&dir, "alpha");
    let beta = Machine::new(&dir, "beta");
    let alpha_claude = alpha.fake_claude();
    let beta_claude = beta.fake_claude();
    let alpha_widgets = alpha.checkout("widgets");
    let beta_widgets = beta.checkout("widgets");
    alpha.session(&alpha_widgets, "aaaa1111-server-widgets", "2026-03-10");
    alpha.session(&alpha_widgets, "aaaa2222-server-old", "2026-01-05");
    beta.session(&beta_widgets, "bbbb2222-local-widgets", "2026-03-12");
    ok(&alpha.sp(&["index"]), "sp index on alpha");
    ok(&beta.sp(&["index"]), "sp index on beta");
    let _server = alpha.serve_external();
    common::pair(&alpha, &beta, "lab");

    let output = beta.sp(&["--server", "lab", "analyze", "--project", beta_widgets.to_str().unwrap(), "--since", "2026-03-01"]);
    assert_eq!(output.status.code(), Some(0), "stdout: {}\nstderr: {}\n{}", stdout(&output), stderr(&output), server_logs(&alpha.root));
    assert_eq!(alpha_claude.models(), vec!["haiku"], "the server analyzed its one session in range\n{}", server_logs(&alpha.root));
    assert!(beta_claude.models().is_empty(), "beta's claude is never called");

    let after = ok(&beta.sp(&["--server", "lab", "sessions", "--project", beta_widgets.to_str().unwrap()]), "sp --server lab sessions");
    let line = |id: &str| after.lines().find(|l| l.starts_with(id)).unwrap_or_else(|| panic!("{id} isn't listed: {after}")).to_string();
    assert!(line("aaaa1111").contains("analyzed"), "{after}");
    assert!(!line("aaaa2222").contains("analyzed"), "a session before --since isn't analyzed: {after}");
    let local = ok(&beta.sp(&["sessions", "--project", beta_widgets.to_str().unwrap()]), "sp sessions on beta");
    assert!(!local.contains("analyzed"), "nothing is analyzed on beta: {local}");
}

/// `sp analyze`'s standard error after it analyzes two sessions, one the fake claude answers and
/// one it fails: in process with `server` None, else on alpha's server paired as `server`.
fn analyze_two(dir: &TempDir, server: Option<&str>) -> String {
    let alpha = Machine::new(dir, "alpha");
    let beta = Machine::new(dir, "beta");
    let claude = alpha.fake_claude();
    claude.fail_on("Prompt of aaaa2222-broken");
    let widgets = alpha.checkout("widgets");
    alpha.session(&widgets, "aaaa1111-fine", "2026-03-10");
    alpha.session(&widgets, "aaaa2222-broken", "2026-03-11");
    ok(&alpha.sp(&["index"]), "sp index on alpha");
    let project = widgets.to_str().unwrap();
    let output = match server {
        None => alpha.sp(&["analyze", "--project", project, "--since", "2026-03-01"]),
        Some(name) => {
            let _server = alpha.serve_external();
            common::pair(&alpha, &beta, name);
            let beta_widgets = beta.checkout("widgets");
            beta.sp(&["--server", name, "analyze", "--project", beta_widgets.to_str().unwrap(), "--since", "2026-03-01"])
        }
    };
    assert_eq!(output.status.code(), Some(1), "one session failed\nstdout: {}\nstderr: {}\n{}", stdout(&output), stderr(&output), server_logs(&alpha.root));
    stderr(&output)
}

fn assert_session_lines(err: &str) {
    let fine = err.lines().find(|l| l.contains("Prompt of aaaa1111-fine")).unwrap_or_else(|| panic!("no line for the analyzed session: {err}"));
    assert!(fine.starts_with('[') && fine.contains("/2] Prompt of aaaa1111-fine: succeeded, 1 components, 1 annotations"), "{err}");
    let broken = err.lines().find(|l| l.contains("Prompt of aaaa2222-broken")).unwrap_or_else(|| panic!("no line for the failed session: {err}"));
    assert!(broken.starts_with('[') && broken.contains("/2] Prompt of aaaa2222-broken: failed: claude failed") && broken.contains("claude broke"), "{err}");
    assert!(!err.contains("] done"), "{err}");
}

#[test]
fn sp_analyze_prints_each_sessions_title_with_its_summary_or_the_first_line_of_its_error() {
    let dir = TempDir::new("cli-analyze-lines");
    assert_session_lines(&analyze_two(&dir, None));
}

#[test]
fn sp_analyze_with_a_paired_server_prints_each_sessions_title_with_its_summary_or_the_first_line_of_its_error() {
    let dir = TempDir::new("cli-remote-analyze-lines");
    assert_session_lines(&analyze_two(&dir, Some("lab")));
}
