// One server per data folder: a second start is refused without disturbing the first, and starts
// once the first has stopped.
#[path = "../../testutil/tempdir.rs"]
mod tempdir;
mod common;

use std::collections::HashSet;

use common::{CtxParams, loopback_config, open_ctx, read_token, start};
use session_player_client::Client;
use session_player_core::ops::{HELLO, HelloRequest};
use session_player_server::{ServeParams, serve};
use tempdir::TempDir;

/// The socket check below sees every listener in this process, so the other test's servers must
/// not start or stop while it runs.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Inodes of the TCP sockets in LISTEN state this process holds open.
fn listening_sockets_of_this_process() -> HashSet<String> {
    let mut listening = HashSet::new();
    for table in ["/proc/net/tcp", "/proc/net/tcp6"] {
        let text = std::fs::read_to_string(table).unwrap_or_default();
        for line in text.lines().skip(1) {
            let columns: Vec<&str> = line.split_whitespace().collect();
            if columns.get(3) == Some(&"0A") {
                listening.insert(columns[9].to_string());
            }
        }
    }
    let mut owned = HashSet::new();
    for entry in std::fs::read_dir("/proc/self/fd").expect("list open files").flatten() {
        if let Ok(target) = std::fs::read_link(entry.path()) {
            let target = target.to_string_lossy().to_string();
            if let Some(inode) = target.strip_prefix("socket:[").and_then(|t| t.strip_suffix(']'))
                && listening.contains(inode)
            {
                owned.insert(inode.to_string());
            }
        }
    }
    owned
}

#[test]
fn a_second_server_on_the_same_data_folder_is_refused_without_disturbing_the_first_and_can_start_after_it_stops() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let dir = TempDir::new("server-lock");
    let ctx = open_ctx(CtxParams { dir: &dir, clock: None, event_capacity: None, fixtures: false });
    let data = ctx.paths().data.clone();
    let first = start(&ctx, loopback_config());
    let token = read_token(&data);
    let lock_before = std::fs::read_to_string(data.join("server.lock")).expect("server.lock");
    let listeners_before = listening_sockets_of_this_process();

    let second = serve(ServeParams { ctx: ctx.clone(), data: data.clone(), config: loopback_config() });

    assert!(second.is_err(), "a second server started on the same data folder");
    assert_eq!(read_token(&data), token, "the refused start rewrote server.token");
    assert_eq!(std::fs::read_to_string(data.join("server.lock")).unwrap(), lock_before, "the refused start rewrote server.lock");
    if cfg!(target_os = "linux") {
        assert_eq!(listening_sockets_of_this_process(), listeners_before, "the refused start left a socket listening");
    }
    let client = Client::local(first.addrs().loopback.port(), &token).unwrap();
    client.call(&HELLO, &HelloRequest {}).expect("the first server still answers");

    first.shutdown();
    let third = serve(ServeParams { ctx: ctx.clone(), data: data.clone(), config: loopback_config() }).expect("start after the first stopped");
    let client = Client::local(third.addrs().loopback.port(), &read_token(&data)).unwrap();
    client.call(&HELLO, &HelloRequest {}).expect("the new server answers");
    third.shutdown();
}

#[test]
fn server_token_is_owner_only_and_new_on_each_start() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let dir = TempDir::new("server-lock-token");
    let ctx = open_ctx(CtxParams { dir: &dir, clock: None, event_capacity: None, fixtures: false });
    let data = ctx.paths().data.clone();

    let first = start(&ctx, loopback_config());
    let token = read_token(&data);
    first.shutdown();
    let second = start(&ctx, loopback_config());

    assert_ne!(read_token(&data), token);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(data.join("server.token")).unwrap().permissions().mode() & 0o777, 0o600);
    }
    second.shutdown();
}
