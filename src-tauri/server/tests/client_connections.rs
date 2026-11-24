// The client crate's `Client` shares one tokio runtime across the process and keeps its
// connections to a server alive between calls.
#[path = "../../testutil/tempdir.rs"]
mod tempdir;
mod common;

use common::{CtxParams, local_client, loopback_config, open_ctx, start};
use session_player_core::ops::{HELLO, HelloRequest};
use tempdir::TempDir;

/// Threads of this process named like the client's runtime workers.
// Reads /proc, which only Linux has.
#[cfg(target_os = "linux")]
fn client_threads() -> usize {
    std::fs::read_dir("/proc/self/task")
        .unwrap()
        .flatten()
        .filter(|t| std::fs::read_to_string(t.path().join("comm")).is_ok_and(|c| c.trim() == "sp-client"))
        .count()
}

/// TCP sockets of this machine whose far end is `port` on 127.0.0.1, in any state (TIME_WAIT
/// included), so each connection a client opened shows up.
// Reads /proc, which only Linux has.
#[cfg(target_os = "linux")]
fn sockets_to(port: u16) -> usize {
    let wanted = format!("0100007F:{port:04X}");
    std::fs::read_to_string("/proc/net/tcp").unwrap().lines().skip(1).filter(|l| l.split_whitespace().nth(2) == Some(wanted.as_str())).count()
}

#[test]
#[cfg(target_os = "linux")]
fn many_clients_share_one_runtime() {
    let dir = TempDir::new("client-shared-runtime");
    let ctx = open_ctx(CtxParams { dir: &dir, clock: None, event_capacity: None, fixtures: false });
    let server = start(&ctx, loopback_config());

    let clients: Vec<_> = (0..8).map(|_| local_client(&server, &ctx.paths().data)).collect();
    for client in &clients {
        client.call(&HELLO, &HelloRequest {}).expect("hello");
    }

    assert!(client_threads() <= 4, "{} client runtime threads for 8 clients", client_threads());
    drop(clients);
    server.shutdown();
}

#[test]
#[cfg(target_os = "linux")]
fn one_client_reuses_its_connection_for_call_after_call() {
    let dir = TempDir::new("client-keepalive");
    let ctx = open_ctx(CtxParams { dir: &dir, clock: None, event_capacity: None, fixtures: false });
    let server = start(&ctx, loopback_config());
    let port = server.addrs().loopback.port();
    let client = local_client(&server, &ctx.paths().data);

    for _ in 0..20 {
        client.call(&HELLO, &HelloRequest {}).expect("hello");
    }

    assert!(sockets_to(port) <= 2, "{} connections for 20 calls", sockets_to(port));
    drop(client);
    server.shutdown();
}

#[test]
fn a_client_whose_kept_connection_was_closed_by_a_restart_still_gets_through() {
    let dir = TempDir::new("client-keepalive-restart");
    let ctx = open_ctx(CtxParams { dir: &dir, clock: None, event_capacity: None, fixtures: false });
    let config = session_player_server::ServerConfig { port: common::free_port(), ..loopback_config() };
    let server = start(&ctx, config.clone());
    let client = local_client(&server, &ctx.paths().data);
    client.call(&HELLO, &HelloRequest {}).expect("hello");
    server.shutdown();

    let server = start(&ctx, config);
    let fresh = local_client(&server, &ctx.paths().data);
    fresh.call(&HELLO, &HelloRequest {}).expect("hello from a new client");
    let answer = client.call(&HELLO, &HelloRequest {});

    // The old token is gone after a restart, so the old client is refused, but it reaches the
    // server instead of failing on the closed connection.
    let error = answer.expect_err("the old token is refused");
    assert_eq!(error.code, session_player_core::ops::ErrorCode::Unauthenticated, "{error:?}");
    server.shutdown();
}
