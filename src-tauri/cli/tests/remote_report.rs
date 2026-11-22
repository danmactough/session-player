// `sp --server <name> report` writes the report as a job on the paired server and saves the
// Markdown on this machine.

#[path = "../../testutil/tempdir.rs"]
mod tempdir;

mod common;

use common::{FAKE_REPORT_TITLE, Machine, server_logs, stderr, stdout};
use tempdir::TempDir;

#[test]
fn sp_report_with_a_paired_server_saves_the_report_on_the_server_and_writes_the_markdown_here() {
    let dir = TempDir::new("cli-remote-report");
    let alpha = Machine::new(&dir, "alpha");
    let beta = Machine::new(&dir, "beta");
    let alpha_claude = alpha.fake_claude();
    let beta_claude = beta.fake_claude();
    let alpha_widgets = alpha.checkout("widgets");
    let beta_widgets = beta.checkout("widgets");
    alpha.session(&alpha_widgets, "aaaa1111-server-widgets", "2026-03-10");
    beta.session(&beta_widgets, "bbbb2222-local-widgets", "2026-03-12");
    ok_index(&alpha);
    ok_index(&beta);
    let _server = alpha.serve_external();
    common::pair(&alpha, &beta, "lab");
    let out = beta.root.join("report.md");

    let output = beta.sp(&["--server", "lab", "report", "--project", beta_widgets.to_str().unwrap(), "--since", "2026-03-01", "--out", out.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(0), "stdout: {}\nstderr: {}\n{}", stdout(&output), stderr(&output), server_logs(&alpha.root));

    let markdown = std::fs::read_to_string(&out).unwrap_or_else(|e| panic!("{} wasn't written: {e}\n{}", out.display(), stderr(&output)));
    assert!(markdown.contains(FAKE_REPORT_TITLE), "{markdown}");
    let saved = alpha.report_ids();
    assert_eq!(saved.len(), 1, "the report is saved on the server: {saved:?}");
    assert!(stderr(&output).contains(&saved[0]), "the saved report's id is named: {}", stderr(&output));
    assert!(beta.report_ids().is_empty(), "nothing is saved on beta");
    // The session had no analysis, so it was analyzed first (haiku), then the report written.
    assert_eq!(alpha_claude.models().first().map(String::as_str), Some("haiku"), "{:?}", alpha_claude.models());
    assert_eq!(alpha_claude.models().len(), 2, "{:?}", alpha_claude.models());
    assert!(beta_claude.models().is_empty(), "beta's claude is never called");
}

fn ok_index(machine: &Machine) {
    common::ok(&machine.sp(&["index"]), "sp index");
}
