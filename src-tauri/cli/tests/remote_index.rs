// `sp --server <name> index` runs the index as a job on the paired server (Backend::run).

#[path = "../../testutil/tempdir.rs"]
mod tempdir;

mod common;

use std::time::{Duration, Instant};

use common::{Machine, ok, server_logs};
use tempdir::TempDir;

#[test]
fn sp_index_with_a_paired_server_indexes_the_servers_folders_on_the_server_and_prints_the_summary() {
    let dir = TempDir::new("cli-remote-index");
    let alpha = Machine::new(&dir, "alpha");
    let beta = Machine::new(&dir, "beta");
    let alpha_widgets = alpha.checkout("widgets");
    let beta_widgets = beta.checkout("widgets");
    beta.session(&beta_widgets, "bbbb2222-local-only", "2026-03-12");
    let _server = alpha.serve_external();
    common::pair(&alpha, &beta, "lab");
    // Written after the server started, so only an index pass after this point lists it.
    alpha.session(&alpha_widgets, "aaaa1111-server-only", "2026-03-10");

    let started = Instant::now();
    let output = beta.sp(&["--server", "lab", "index"]);
    let took = started.elapsed();
    let summary = ok(&output, "sp --server lab index");
    let first = summary.lines().next().unwrap_or_default();
    // The server's file watcher may index the new file first, so the new-or-changed count isn't fixed.
    assert!(first.starts_with("1 files found, ") && first.ends_with("s. 1 sessions listed."), "unexpected summary: {summary}\n{}", server_logs(&alpha.root));
    let alpha_root = alpha.home.join(".claude/projects");
    assert!(summary.contains(&*alpha_root.to_string_lossy()), "the server's session folders are listed: {summary}");
    assert!(!summary.contains(&*beta.home.to_string_lossy()), "beta's folders aren't: {summary}");
    // Completion comes from polling `job`; waiting on a missed end event cost 5 s here once.
    assert!(took < Duration::from_millis(2500), "sp --server lab index took {took:?}");

    let listed = ok(&beta.sp(&["--server", "lab", "sessions", "--project", beta_widgets.to_str().unwrap()]), "sp --server lab sessions");
    assert!(listed.contains("aaaa1111") && !listed.contains("bbbb2222"), "{listed}");
}
