// `sp --server <name> reports` lists the paired server's reports, not this machine's.

#[path = "../../testutil/tempdir.rs"]
mod tempdir;

mod common;

use common::{Machine, ok};
use tempdir::TempDir;

#[test]
fn sp_reports_with_a_paired_server_lists_the_servers_reports_and_not_the_local_ones() {
    let dir = TempDir::new("cli-remote-reports");
    let alpha = Machine::new(&dir, "alpha");
    let beta = Machine::new(&dir, "beta");
    alpha.save_report("r-alpha-1", "Alpha's own report", 2);
    beta.save_report("r-beta-1", "Beta's own report", 1);
    let _server = alpha.serve_external();
    common::pair(&alpha, &beta, "lab");

    let listed = ok(&beta.sp(&["--server", "lab", "reports"]), "sp --server lab reports");
    assert!(listed.contains("r-alpha-1") && listed.contains("Alpha's own report"), "{listed}");
    assert!(listed.contains("2 actions"), "{listed}");
    assert!(!listed.contains("r-beta-1"), "beta's report isn't listed: {listed}");
}
