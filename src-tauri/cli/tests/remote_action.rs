// `sp --server <name> action` records a response to an action on the paired server.

#[path = "../../testutil/tempdir.rs"]
mod tempdir;

mod common;

use common::{Machine, ok};
use session_player_core::reports::ActionStatus;
use tempdir::TempDir;

#[test]
fn sp_action_with_a_paired_server_records_the_response_on_the_server_only() {
    let dir = TempDir::new("cli-remote-action");
    let alpha = Machine::new(&dir, "alpha");
    let beta = Machine::new(&dir, "beta");
    alpha.save_report("r-shared", "Alpha's own report", 2);
    // The same id on both machines, as after a pull, so a local write would succeed too.
    beta.save_report("r-shared", "Beta's copy", 2);
    let server = alpha.serve_external();
    common::pair(&alpha, &beta, "lab");

    let printed = ok(&beta.sp(&["--server", "lab", "action", "r-shared", "2", "--status", "done", "--note", "Shipped"]), "sp --server lab action");
    assert!(printed.contains("Alpha's own report action 2"), "the title comes from the server's report: {printed}");

    drop(server);
    let on_alpha = alpha.action_responses("r-shared");
    assert_eq!(on_alpha.len(), 1, "{on_alpha:?}");
    assert_eq!((on_alpha[0].action_index, on_alpha[0].status, on_alpha[0].note.as_deref()), (1, Some(ActionStatus::Done), Some("Shipped")));
    assert!(beta.action_responses("r-shared").is_empty(), "nothing is recorded on beta");
}
