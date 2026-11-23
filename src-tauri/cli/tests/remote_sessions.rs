// `sp --server <name> sessions` lists the paired server's sessions of this checkout's repository.

#[path = "../../testutil/tempdir.rs"]
mod tempdir;

mod common;

use common::{Machine, ok};
use tempdir::TempDir;

#[test]
fn sp_sessions_with_a_paired_server_lists_the_servers_sessions_of_the_same_repository() {
    let dir = TempDir::new("cli-remote-sessions");
    let alpha = Machine::new(&dir, "alpha");
    let beta = Machine::new(&dir, "beta");
    // The same repository, checked out at different paths on the two machines.
    let alpha_widgets = alpha.checkout("widgets");
    let alpha_gadgets = alpha.checkout("gadgets");
    let beta_widgets = beta.checkout("widgets");
    alpha.session(&alpha_widgets, "aaaa1111-server-widgets", "2026-03-10");
    alpha.session(&alpha_gadgets, "cccc3333-server-gadgets", "2026-03-11");
    beta.session(&beta_widgets, "bbbb2222-local-widgets", "2026-03-12");
    ok(&alpha.sp(&["index"]), "sp index on alpha");
    ok(&beta.sp(&["index"]), "sp index on beta");
    let _server = alpha.serve_external();
    common::pair(&alpha, &beta, "lab");

    let listed = ok(&beta.sp(&["--server", "lab", "sessions", "--project", beta_widgets.to_str().unwrap()]), "sp --server lab sessions");
    assert!(listed.contains("aaaa1111"), "the server's widgets session is listed: {listed}");
    assert!(!listed.contains("bbbb2222"), "beta's own session isn't: {listed}");
    assert!(!listed.contains("cccc3333"), "another repository's session isn't: {listed}");
}
