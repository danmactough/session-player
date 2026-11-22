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
