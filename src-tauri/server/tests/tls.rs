// The external listener serves TLS with a pinned self-signed certificate: the pairing string
// carries the port it bound and the certificate's real fingerprint, a client pinned to another
// fingerprint is refused before anything is sent, and the identity lasts across restarts until the
// advertised host changes.
#[path = "../../testutil/tempdir.rs"]
mod tempdir;
mod common;

use std::sync::Arc;

use common::{CtxParams, client_for, external_config, free_port, local_client, open_ctx, start};
use session_player_client::{Client, ConnectParams, PairedServer, PairingString, connect};
use session_player_core::ops::{Ctx, HELLO, HelloRequest};
use session_player_server::pairing::{PairRequest, PairResponse};
use session_player_server::{ServerConfig, ServerHandle};
use tempdir::TempDir;

fn pair(client: &Client) -> PairingString {
    let offer: PairResponse = client.call_named("pair", &PairRequest {}).expect("pair");
    PairingString::parse(&offer.pairing).expect("parse the pairing string")
}

/// Fixed ports, so a restarted server is at the address a paired client remembers.
fn fixed_ports(advertise: &str) -> ServerConfig {
    ServerConfig { port: free_port(), external_port: free_port(), advertise: Some(advertise.to_string()), ..external_config() }
}

fn restart(dir: &TempDir, config: ServerConfig) -> (Arc<Ctx>, ServerHandle, Client) {
    let ctx = open_ctx(CtxParams { dir, clock: None, event_capacity: None, fixtures: false });
    let server = start(&ctx, config);
    let client = local_client(&server, &ctx.paths().data);
    (ctx, server, client)
}

fn flip_last_hex_digit(fingerprint: &str) -> String {
    let mut chars: Vec<char> = fingerprint.chars().collect();
    let last = chars.last_mut().unwrap();
    *last = if *last == '0' { '1' } else { '0' };
    chars.into_iter().collect()
}

#[test]
fn the_pairing_string_carries_the_external_listeners_bound_port_and_real_fingerprint() {
    let dir = TempDir::new("server-tls-pin");
    let ctx = open_ctx(CtxParams { dir: &dir, clock: None, event_capacity: None, fixtures: false });
    let server = start(&ctx, external_config());
    let client = local_client(&server, &ctx.paths().data);

    let pairing = pair(&client);

    let external = server.addrs().external.expect("pair opens the external listener");
    assert_ne!(external.port(), 0);
    assert_ne!(external.port(), server.addrs().loopback.port());
    assert_eq!(pairing.port, external.port());
    assert_eq!(pairing.address, "127.0.0.1");
    let fingerprint = pairing.fingerprint.clone().expect("a TLS listener's pairing string carries its fingerprint");
    assert_eq!(fingerprint.len(), 64, "lowercase hex SHA-256: {fingerprint}");
    assert!(fingerprint.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()), "{fingerprint}");

    let paired = connect(ConnectParams { pairing: &pairing.encode(), name: "laptop" }).expect("connect over pinned TLS");
    assert_eq!(paired.fingerprint.as_deref(), Some(fingerprint.as_str()));
    assert_eq!(paired.port, external.port());
    Client::paired(&paired).unwrap().call(&HELLO, &HelloRequest {}).expect("hello over pinned TLS");
    server.shutdown();
}

#[test]
fn a_client_pinned_to_another_fingerprint_is_refused_before_the_secret_is_sent() {
    let dir = TempDir::new("server-tls-wrong-pin");
    let ctx = open_ctx(CtxParams { dir: &dir, clock: None, event_capacity: None, fixtures: false });
    let server = start(&ctx, external_config());
    let client = local_client(&server, &ctx.paths().data);
    let pairing = pair(&client);
    let wrong = PairingString { fingerprint: Some(flip_last_hex_digit(pairing.fingerprint.as_deref().unwrap())), ..pairing.clone() };

    let refused = connect(ConnectParams { pairing: &wrong.encode(), name: "laptop" });
    assert!(refused.is_err(), "connect succeeded with the wrong pin");

    // The handshake failed, so the offer was never presented and is still good.
    let paired = connect(ConnectParams { pairing: &pairing.encode(), name: "laptop" }).expect("connect with the right pin");
    let pinned_wrong = client_for(&wrong, Some(paired.token.clone()));
    assert!(pinned_wrong.call(&HELLO, &HelloRequest {}).is_err(), "a call succeeded with the wrong pin");
    server.shutdown();
}

#[test]
fn the_certificate_and_paired_devices_survive_a_restart() {
    let dir = TempDir::new("server-tls-restart");
    let config = fixed_ports("127.0.0.1");
    let (_ctx, server, client) = restart(&dir, config.clone());
    let first = pair(&client);
    let paired: PairedServer = connect(ConnectParams { pairing: &first.encode(), name: "laptop" }).expect("connect");
    server.shutdown();

    let (_ctx, server, client) = restart(&dir, config);
    let second = pair(&client);

    assert_eq!(second.fingerprint, first.fingerprint, "the certificate changed across a restart");
    Client::paired(&paired).unwrap().call(&HELLO, &HelloRequest {}).expect("the paired client after a restart");
    server.shutdown();
}

#[test]
fn changing_the_advertised_host_makes_a_new_certificate_that_old_pins_refuse() {
    let dir = TempDir::new("server-tls-advertise");
    let config = fixed_ports("127.0.0.1");
    let (_ctx, server, client) = restart(&dir, config.clone());
    let first = pair(&client);
    let paired = connect(ConnectParams { pairing: &first.encode(), name: "laptop" }).expect("connect");
    server.shutdown();

    let (_ctx, server, client) = restart(&dir, ServerConfig { advertise: Some("localhost".to_string()), ..config });
    let second = pair(&client);

    assert_eq!(second.address, "localhost");
    assert_ne!(second.fingerprint, first.fingerprint, "a new advertised host must regenerate the certificate");
    assert!(Client::paired(&paired).unwrap().call(&HELLO, &HelloRequest {}).is_err(), "the old pin still matched");
    server.shutdown();
}
