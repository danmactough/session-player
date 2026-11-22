// The external listener is open iff external_bind is set and a device or an unexpired offer exists.
// It opens before `pair` answers and closes within a short grace once nothing needs it.
#[path = "../../testutil/tempdir.rs"]
mod tempdir;
mod common;

use std::net::SocketAddr;
use std::time::Duration;

use common::{CtxParams, TestClock, accepts_connections, external_config, free_port, local_client, loopback_config, open_ctx, start, wait_until};
use session_player_client::{ConnectParams, PairingString, connect};
use session_player_server::pairing::{PairRequest, PairResponse, UnpairRequest};
use session_player_server::{ServerConfig, ServerHandle};
use tempdir::TempDir;

const CLOSES_WITHIN: Duration = Duration::from_secs(3);

fn wait_closed(server: &ServerHandle, addr: SocketAddr) {
    wait_until("the external listener closes", CLOSES_WITHIN, || server.addrs().external.is_none() && !accepts_connections(addr));
}

#[test]
fn with_no_external_bind_the_external_listener_never_opens_even_while_an_offer_is_open() {
    let dir = TempDir::new("server-listener-none");
    let ctx = open_ctx(CtxParams { dir: &dir, clock: None, event_capacity: None, fixtures: false });
    let server = start(&ctx, loopback_config());
    let client = local_client(&server, &ctx.paths().data);

    let offer: PairResponse = client.call_named("pair", &PairRequest {}).expect("pair");
    connect(ConnectParams { pairing: &offer.pairing, name: "laptop" }).expect("connect");
    std::thread::sleep(Duration::from_millis(300));

    assert_eq!(server.addrs().external, None);
    server.shutdown();
}

#[test]
fn the_external_listener_opens_for_an_offer_stays_for_a_device_and_closes_after_unpair_or_expiry() {
    let dir = TempDir::new("server-listener-life");
    let clock = TestClock::new();
    let ctx = open_ctx(CtxParams { dir: &dir, clock: Some(clock.clone()), event_capacity: None, fixtures: false });
    let config = ServerConfig { external_port: free_port(), ..external_config() };
    let external = SocketAddr::new("127.0.0.1".parse().unwrap(), config.external_port);
    let server = start(&ctx, config);
    let client = local_client(&server, &ctx.paths().data);

    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(server.addrs().external, None, "open with no device and no offer");
    assert!(!accepts_connections(external));

    // An offer opens it before pair answers; expiry closes it.
    client.call_named::<_, PairResponse>("pair", &PairRequest {}).expect("pair");
    assert_eq!(server.addrs().external, Some(external));
    assert!(accepts_connections(external));
    clock.advance(300_001);
    wait_closed(&server, external);

    // A device keeps it open after its offer is used; unpairing the last device closes it.
    let offer: PairResponse = client.call_named("pair", &PairRequest {}).expect("pair");
    let paired = connect(ConnectParams { pairing: &offer.pairing, name: "laptop" }).expect("connect");
    assert_eq!(PairingString::parse(&offer.pairing).unwrap().port, external.port());
    clock.advance(300_001);
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(server.addrs().external, Some(external), "closed while a device is paired");
    client.call_named::<_, ()>("unpair", &UnpairRequest { id: paired.device }).expect("unpair");
    wait_closed(&server, external);
    server.shutdown();
}

#[test]
fn a_server_restarted_with_a_paired_device_opens_the_external_listener_without_an_offer() {
    let dir = TempDir::new("server-listener-restart");
    let config = ServerConfig { external_port: free_port(), ..external_config() };
    let ctx = open_ctx(CtxParams { dir: &dir, clock: None, event_capacity: None, fixtures: false });
    let server = start(&ctx, config.clone());
    let offer: PairResponse = local_client(&server, &ctx.paths().data).call_named("pair", &PairRequest {}).expect("pair");
    connect(ConnectParams { pairing: &offer.pairing, name: "laptop" }).expect("connect");
    server.shutdown();

    let ctx = open_ctx(CtxParams { dir: &dir, clock: None, event_capacity: None, fixtures: false });
    let server = start(&ctx, config.clone());

    wait_until("the external listener opens", Duration::from_secs(2), || server.addrs().external.is_some());
    assert_eq!(server.addrs().external.unwrap().port(), config.external_port);
    server.shutdown();
}

#[test]
fn the_external_listener_opens_and_closes_twenty_times_on_one_port() {
    let dir = TempDir::new("server-listener-cycles");
    let clock = TestClock::new();
    let ctx = open_ctx(CtxParams { dir: &dir, clock: Some(clock.clone()), event_capacity: None, fixtures: false });
    let config = ServerConfig { external_port: free_port(), ..external_config() };
    let external = SocketAddr::new("127.0.0.1".parse().unwrap(), config.external_port);
    let server = start(&ctx, config);
    let client = local_client(&server, &ctx.paths().data);

    for cycle in 0..20 {
        let offer: PairResponse = client.call_named("pair", &PairRequest {}).unwrap_or_else(|e| panic!("pair in cycle {cycle}: {e:?}"));
        assert_eq!(PairingString::parse(&offer.pairing).unwrap().port, external.port(), "cycle {cycle}");
        assert!(accepts_connections(external), "cycle {cycle}: not open after pair");
        clock.advance(300_001);
        wait_closed(&server, external);
    }
    server.shutdown();
}
