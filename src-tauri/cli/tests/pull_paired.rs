// `sp pull <name>` for a server paired with `sp connect` pulls over HTTPS (the client crate's
// PULL extension); any other name still goes over SSH. `ssh` is a stub on PATH that fails.

#[path = "../../testutil/tempdir.rs"]
mod tempdir;

mod common;

use common::{Machine, find_files, ok};
use tempdir::TempDir;

fn paired_machines(dir: &TempDir) -> (Machine, Machine, std::path::PathBuf, common::Server) {
    let alpha = Machine::new(dir, "alpha");
    let beta = Machine::new(dir, "beta");
    beta.stub("ssh", "", 255);
    let alpha_widgets = alpha.checkout("widgets");
    let beta_widgets = beta.checkout("widgets");
    alpha.session(&alpha_widgets, "aaaa1111-from-the-server", "2026-03-10");
    ok(&alpha.sp(&["index"]), "sp index on alpha");
    let server = alpha.serve_external();
    common::pair(&alpha, &beta, "lab");
    (alpha, beta, beta_widgets, server)
}

#[test]
fn sp_pull_from_a_paired_server_copies_the_projects_sessions_over_https() {
    let dir = TempDir::new("cli-pull-paired");
    let (_alpha, beta, beta_widgets, _server) = paired_machines(&dir);

    let printed = ok(&beta.sp(&["pull", "lab", "--project", beta_widgets.to_str().unwrap()]), "sp pull lab");
    assert!(beta.stub_calls("ssh").is_empty(), "a paired name never runs ssh: {:?}", beta.stub_calls("ssh"));
    assert!(printed.contains("Sessions: 1 added"), "{printed}");
    let pulled = find_files(&beta.home.join(".claude/projects"), "aaaa1111-from-the-server.jsonl");
    assert_eq!(pulled.len(), 1, "the session lands in beta's Claude Code folder: {pulled:?}");
    let listed = ok(&beta.sp(&["--local", "sessions", "--project", beta_widgets.to_str().unwrap()]), "sp --local sessions");
    assert!(listed.contains("aaaa1111"), "{listed}");

    // A name not in servers.json is a host for ssh, as before.
    let other = beta.sp(&["pull", "elsewhere", "--project", beta_widgets.to_str().unwrap()]);
    assert_ne!(other.status.code(), Some(0));
    assert!(beta.stub_calls("ssh").iter().any(|c| c.contains("elsewhere")), "{:?}", beta.stub_calls("ssh"));
}

#[test]
fn sp_pull_dry_run_from_a_paired_server_lists_the_files_and_writes_nothing() {
    let dir = TempDir::new("cli-pull-paired-dry");
    let (_alpha, beta, beta_widgets, _server) = paired_machines(&dir);

    let printed = ok(&beta.sp(&["pull", "lab", "--project", beta_widgets.to_str().unwrap(), "--dry-run"]), "sp pull lab --dry-run");
    assert!(beta.stub_calls("ssh").is_empty(), "a paired name never runs ssh");
    assert!(printed.contains("Dry run: nothing was written") && printed.contains("aaaa1111-from-the-server.jsonl"), "{printed}");
    assert!(find_files(&beta.home, "aaaa1111-from-the-server.jsonl").is_empty(), "nothing is written");
}
