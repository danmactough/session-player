// `sp service`: tested only where nothing changes on this machine. `systemctl` is a stub on PATH.

#[path = "../../testutil/tempdir.rs"]
mod tempdir;

mod common;

use common::{Machine, ok, stderr};
use tempdir::TempDir;

#[cfg(target_os = "linux")]
#[test]
fn sp_service_status_in_a_fresh_home_reports_not_installed_and_changes_nothing() {
    let dir = TempDir::new("cli-service-status");
    let alpha = Machine::new(&dir, "alpha");
    alpha.stub("systemctl", "inactive\n", 3);

    let printed = ok(&alpha.sp(&["service", "status"]), "sp service status").to_lowercase();
    assert!(printed.contains("not installed"), "{printed}");
    assert!(printed.contains("session-player.service"), "the unit file is named: {printed}");
    assert!(!alpha.home.join(".config/systemd").exists(), "status writes nothing");
    for call in alpha.stub_calls("systemctl") {
        assert!(call.starts_with("--user is-active"), "status only asks: {call}");
    }
}

#[test]
fn sp_service_with_a_paired_server_is_refused_because_it_only_manages_this_machine() {
    let dir = TempDir::new("cli-service-remote");
    let alpha = Machine::new(&dir, "alpha");
    let beta = Machine::new(&dir, "beta");
    alpha.stub("systemctl", "inactive\n", 3);
    beta.stub("systemctl", "inactive\n", 3);
    let _server = alpha.serve_external();
    common::pair(&alpha, &beta, "lab");

    let output = beta.sp(&["--server", "lab", "service", "status"]);
    assert_ne!(output.status.code(), Some(0));
    assert!(stderr(&output).contains("this machine") && !stderr(&output).contains("panicked"), "{}", stderr(&output));
    assert!(alpha.stub_calls("systemctl").is_empty(), "the server's systemctl isn't run");
}
