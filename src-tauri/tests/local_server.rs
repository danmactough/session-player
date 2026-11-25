// The local server's state comes from its lock and token files (probe_local) and from what
// `sp service status` prints. A stand-in `sp` (a shell script) plays the bundled one: it logs its
// arguments and prints the status a test gives it, so no test touches the real launchd or systemd.
#[path = "../testutil/tempdir.rs"]
mod tempdir;
#[path = "../server/tests/common/mod.rs"]
mod common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use common::{CtxParams, loopback_config, open_ctx, start, wait_until};
use session_player_client::{LocalProbe, probe_local};
use session_player_lib::local::{self, LocalServerState, ServiceFacts, StartParams, StateParams, local_connection, local_state, parse_service_status};
use tempdir::TempDir;

/// A stand-in `sp` in `dir`: appends its arguments to `dir/sp.log`, prints `dir/status` for any
/// `service` action, and touches `dir/{action}.done`.
fn fake_sp(dir: &Path) -> PathBuf {
    let sp = dir.join("sp");
    let script = format!(
        "#!/bin/sh\necho \"$@\" >> '{d}/sp.log'\ntouch '{d}/'\"$2\".done\ncat '{d}/status'\n",
        d = dir.display()
    );
    // A shell writes the script so this process never holds it open for writing: a parallel test's
    // fork would inherit that handle, and Linux then refuses to run the script ("Text file busy").
    let written = std::process::Command::new("/bin/sh")
        .args(["-c", r#"printf '%s' "$1" > "$2" && chmod 755 "$2""#, "sh", &script])
        .arg(&sp)
        .status()
        .unwrap();
    assert!(written.success(), "write the fake sp");
    sp
}

fn set_status(dir: &Path, unit: &Path, installed: bool, running: bool) {
    let first = if installed { "Installed" } else { "Not installed" };
    let second = if running { "Running" } else { "Not running" };
    std::fs::write(dir.join("status"), format!("{first}: {}\n{second}\n", unit.display())).unwrap();
}

fn sp_log(dir: &Path) -> Vec<String> {
    std::fs::read_to_string(dir.join("sp.log")).unwrap_or_default().lines().map(str::to_string).collect()
}

#[test]
fn parse_service_status_reads_what_sp_service_prints() {
    let parsed = parse_service_status("Installed: /home/u/.config/systemd/user/session-player.service\nRunning\n");
    let stopped = parse_service_status("Not installed: /Users/u/Library/LaunchAgents/x.plist\nNot running\n");

    assert_eq!(parsed, Some(ServiceFacts { installed: true, running: true, unit_file: PathBuf::from("/home/u/.config/systemd/user/session-player.service") }));
    assert_eq!(stopped, Some(ServiceFacts { installed: false, running: false, unit_file: PathBuf::from("/Users/u/Library/LaunchAgents/x.plist") }));
    assert_eq!(parse_service_status("something else entirely"), None);
    assert_eq!(parse_service_status("Installed: /x\nMaybe\n"), None);
    assert_eq!(parse_service_status(""), None);
}

#[test]
fn local_state_is_running_for_a_live_lock_and_otherwise_follows_the_service() {
    let live = LocalProbe::Live { port: 47800, token: Some("t".to_string()) };
    let installed = ServiceFacts { installed: true, running: false, unit_file: PathBuf::from("/u") };
    let missing = ServiceFacts { installed: false, running: false, unit_file: PathBuf::from("/u") };

    assert_eq!(local_state(&live, None), LocalServerState::Running { port: 47800 });
    assert_eq!(local_state(&live, Some(&missing)), LocalServerState::Running { port: 47800 });
    assert_eq!(local_state(&LocalProbe::NotLive, Some(&installed)), LocalServerState::Installed);
    assert_eq!(local_state(&LocalProbe::NotLive, Some(&missing)), LocalServerState::NotInstalled);
    assert_eq!(local_state(&LocalProbe::NotLive, None), LocalServerState::NotInstalled);
    assert_eq!(serde_json::to_value(LocalServerState::Running { port: 1 }).unwrap(), serde_json::json!({ "state": "running", "port": 1 }));
    assert_eq!(serde_json::to_value(LocalServerState::NotInstalled).unwrap(), serde_json::json!({ "state": "not_installed" }));
}

#[test]
fn a_server_started_here_is_found_through_its_lock_and_token_files() {
    let root = TempDir::new("shell-local-probe");
    let ctx = open_ctx(CtxParams { dir: &root, clock: None, event_capacity: None, fixtures: false });
    let data = ctx.paths().data.clone();
    let handle = start(&ctx, loopback_config());
    let port = handle.addrs().loopback.port();

    let state = local::server_state(StateParams { data: &data, sp: None, data_override: None });
    let connection = local_connection(&probe_local(&data)).expect("attach to the running server");

    assert_eq!(state, LocalServerState::Running { port });
    assert_eq!(connection.url, format!("http://127.0.0.1:{port}"));
    assert_eq!(connection.token, std::fs::read_to_string(data.join("server.token")).unwrap().trim());
    handle.shutdown();
    // A process forked by a parallel test can hold the lock file for a moment after shutdown.
    wait_until("the lock is released", Duration::from_secs(5), || probe_local(&data) == LocalProbe::NotLive);
    assert_eq!(local::server_state(StateParams { data: &data, sp: None, data_override: None }), LocalServerState::NotInstalled);
    assert!(local_connection(&probe_local(&data)).is_err());
}

#[test]
fn an_installed_stopped_service_is_reported_from_sp_service_status() {
    let root = TempDir::new("shell-local-installed");
    let data = root.join("data");
    std::fs::create_dir_all(&data).unwrap();
    let sp = fake_sp(&root);
    set_status(&root, &root.join("unit"), true, false);

    let state = local::server_state(StateParams { data: &data, sp: Some(&sp), data_override: None });

    assert_eq!(state, LocalServerState::Installed);
    assert_eq!(sp_log(&root), vec!["service status"]);
}

#[test]
fn start_runs_sp_service_start_and_waits_for_the_server_to_be_live() {
    let root = TempDir::new("shell-local-start");
    let ctx = open_ctx(CtxParams { dir: &root, clock: None, event_capacity: None, fixtures: false });
    let data = ctx.paths().data.clone();
    let sp = fake_sp(&root);
    set_status(&root, &root.join("unit"), true, true);
    // Plays the service manager: starts the server once sp was asked to.
    let marker = root.join("start.done");
    let manager = std::thread::spawn(move || {
        wait_until("sp service start runs", Duration::from_secs(10), || marker.exists());
        start(&ctx, loopback_config())
    });

    let state = local::start(StartParams { data: &data, sp: &sp, data_override: None, wait: Duration::from_secs(10) }).expect("start");

    let handle = manager.join().unwrap();
    assert_eq!(state, LocalServerState::Running { port: handle.addrs().loopback.port() });
    assert_eq!(sp_log(&root), vec!["service start"]);
    handle.shutdown();
}

#[test]
fn start_fails_when_the_server_never_comes_up() {
    let root = TempDir::new("shell-local-start-fails");
    let data = root.join("data");
    std::fs::create_dir_all(&data).unwrap();
    let sp = fake_sp(&root);
    set_status(&root, &root.join("unit"), true, false);

    let e = local::start(StartParams { data: &data, sp: &sp, data_override: None, wait: Duration::from_millis(300) }).unwrap_err();

    assert!(e.contains("didn't start"), "{e}");
}

#[test]
fn install_runs_sp_service_install() {
    let root = TempDir::new("shell-local-install");
    let data = root.join("data");
    std::fs::create_dir_all(&data).unwrap();
    let sp = fake_sp(&root);
    set_status(&root, &root.join("unit"), true, true);

    // Nothing serves in this test, so it times out after sp ran.
    let _ = local::install(StartParams { data: &data, sp: &sp, data_override: None, wait: Duration::from_millis(100) });

    assert_eq!(sp_log(&root), vec!["service install"]);
}

#[test]
fn after_an_update_the_service_is_reinstalled_only_when_installed_and_naming_another_sp() {
    let root = TempDir::new("shell-local-refresh");
    let sp = fake_sp(&root);
    let unit = root.join("unit");

    set_status(&root, &unit, false, false);
    assert!(!local::refresh_after_update(&sp, None).unwrap(), "not installed");

    set_status(&root, &unit, true, true);
    std::fs::write(&unit, format!("ExecStart=\"{}\" serve\n", sp.display())).unwrap();
    assert!(!local::refresh_after_update(&sp, None).unwrap(), "already names this sp");

    std::fs::write(&unit, "ExecStart=\"/Applications/Old Session Player.app/Contents/MacOS/sp\" serve\n").unwrap();
    assert!(local::refresh_after_update(&sp, None).unwrap(), "names another sp");

    assert_eq!(sp_log(&root), vec!["service status", "service status", "service status", "service install"]);
}
