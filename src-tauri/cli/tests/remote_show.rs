// `sp --server <name> show` prints a report saved on the paired server.

#[path = "../../testutil/tempdir.rs"]
mod tempdir;

mod common;

use common::{Machine, ok, stderr};
use tempdir::TempDir;

#[test]
fn sp_show_with_a_paired_server_prints_the_servers_report_as_markdown() {
    let dir = TempDir::new("cli-remote-show");
    let alpha = Machine::new(&dir, "alpha");
    let beta = Machine::new(&dir, "beta");
    alpha.save_report("r-alpha-1", "Alpha's own report", 1);
    beta.save_report("r-beta-1", "Beta's own report", 1);
    let _server = alpha.serve_external();
    common::pair(&alpha, &beta, "lab");

    let shown = ok(&beta.sp(&["--server", "lab", "show", "r-alpha"]), "sp --server lab show");
    assert!(shown.contains("Alpha's own report") && shown.contains("Alpha's own report summary"), "{shown}");
    assert!(shown.contains("Alpha's own report action 1"), "{shown}");

    let missing = beta.sp(&["--server", "lab", "show", "r-beta"]);
    assert_ne!(missing.status.code(), Some(0), "beta's report isn't on the server");
    assert!(stderr(&missing).contains("r-beta") && !stderr(&missing).contains("panicked"), "{}", stderr(&missing));
}
