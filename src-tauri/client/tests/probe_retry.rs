// probe_local while a server is starting: the lock is held with port 0 until the server has bound.
// The probe rereads for up to 500 ms and never reports Live with port 0. These tests hold the lock
// themselves, as a starting server would.
#[path = "../../testutil/tempdir.rs"]
mod tempdir;

use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};

use session_player_client::{LocalProbe, probe_local};
use tempdir::TempDir;

fn hold_lock(data: &Path, port: u16) -> File {
    let mut file = File::create(data.join("server.lock")).unwrap();
    file.lock().expect("take the lock");
    write!(file, r#"{{"pid":{},"port":{port}}}"#, std::process::id()).unwrap();
    file
}

fn rewrite(data: &Path, port: u16) {
    // Truncates, then writes: a probe may read the file empty in between and must survive that.
    std::fs::write(data.join("server.lock"), format!(r#"{{"pid":{},"port":{port}}}"#, std::process::id())).unwrap();
}

#[test]
fn a_probe_with_no_lock_file_is_not_live() {
    let dir = TempDir::new("client-probe-none");
    assert_eq!(probe_local(&dir), LocalProbe::NotLive);
}

#[test]
fn a_lock_file_nobody_holds_is_not_live_even_with_a_port_in_it() {
    let dir = TempDir::new("client-probe-stale");
    std::fs::write(dir.join("server.lock"), r#"{"pid":1,"port":4321}"#).unwrap();
    std::fs::write(dir.join("server.token"), "stale").unwrap();
    assert_eq!(probe_local(&dir), LocalProbe::NotLive);
}

#[test]
fn a_probe_during_a_start_waits_for_the_bound_port_and_reads_the_token() {
    let dir = TempDir::new("client-probe-starting");
    let lock = hold_lock(&dir, 0);
    let writer = {
        let data = dir.to_path_buf();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            std::fs::write(data.join("server.token"), "the-token\n").unwrap();
            rewrite(&data, 4321);
        })
    };

    let probe = probe_local(&dir);

    writer.join().unwrap();
    assert_eq!(probe, LocalProbe::Live { port: 4321, token: Some("the-token".to_string()) });
    drop(lock);
}

#[test]
fn a_probe_never_reports_port_zero_and_gives_up_after_about_500_ms() {
    let dir = TempDir::new("client-probe-stuck");
    let lock = hold_lock(&dir, 0);

    let started = Instant::now();
    let probe = probe_local(&dir);
    let took = started.elapsed();

    assert_eq!(probe, LocalProbe::NotLive);
    assert!(took >= Duration::from_millis(400), "gave up after {took:?}, before retrying for 500 ms");
    assert!(took < Duration::from_secs(2), "took {took:?}");
    drop(lock);
}

#[test]
fn a_live_server_whose_token_cannot_be_read_is_live_with_no_token() {
    let dir = TempDir::new("client-probe-no-token");
    let lock = hold_lock(&dir, 4321);
    assert_eq!(probe_local(&dir), LocalProbe::Live { port: 4321, token: None });
    drop(lock);
}
