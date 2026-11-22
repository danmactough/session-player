// Pairing offers redeem once, expire after 300 s by the Ctx clock, and all die after 5 wrong
// secrets; every failure is the same 401. The device token a connect returns authenticates, is
// listed by `devices` and stops working after `unpair`.
//
// With no external_bind the pairing string points at the loopback listener over plain HTTP.
#[path = "../../testutil/tempdir.rs"]
mod tempdir;
mod common;

use common::{CtxParams, TestClock, client_for, loopback_config, local_client, open_ctx, post_json, start};
use session_player_client::{Client, ConnectParams, PairedServer, PairingString, connect};
use session_player_core::ops::{ApiError, ErrorCode, HELLO, HelloRequest};
use session_player_server::ServerHandle;
use session_player_server::pairing::{DeviceInfo, DevicesRequest, PairRequest, PairResponse, UnpairRequest};
use tempdir::TempDir;

fn pair(client: &Client) -> PairResponse {
    client.call_named("pair", &PairRequest {}).expect("pair")
}

fn redeem(pairing: &str, name: &str) -> Result<PairedServer, ApiError> {
    connect(ConnectParams { pairing, name })
}

fn with_secret(pairing: &str, secret: &str) -> String {
    PairingString { secret: secret.to_string(), ..PairingString::parse(pairing).unwrap() }.encode()
}

fn assert_refused(result: Result<PairedServer, ApiError>, what: &str) {
    match result {
        Err(e) => assert_eq!(e.code, ErrorCode::Unauthenticated, "{what}: {e:?}"),
        Ok(server) => panic!("{what}: connect succeeded as {}", server.device.0),
    }
}

fn devices(client: &Client) -> Vec<DeviceInfo> {
    client.call_named("devices", &DevicesRequest {}).expect("devices")
}

fn setup(dir: &TempDir, clock: TestClock) -> (ServerHandle, Client) {
    let ctx = open_ctx(CtxParams { dir, clock: Some(clock), event_capacity: None, fixtures: false });
    let server = start(&ctx, loopback_config());
    let client = local_client(&server, &ctx.paths().data);
    (server, client)
}

#[test]
fn pair_returns_an_sp1_string_for_the_loopback_listener_when_there_is_no_external_bind() {
    let dir = TempDir::new("server-pairing-string");
    let clock = TestClock::new();
    let (server, client) = setup(&dir, clock);

    let offer = pair(&client);
    let pairing = PairingString::parse(&offer.pairing).expect("parse the pairing string");

    assert!(offer.pairing.starts_with("sp1."), "{}", offer.pairing);
    assert_eq!(offer.expires_at, common::CLOCK + 300_000);
    assert_eq!(pairing.address, "127.0.0.1");
    assert_eq!(pairing.port, server.addrs().loopback.port());
    assert_eq!(pairing.fingerprint, None);
    assert!(pairing.secret.len() >= 43, "a 32-byte secret: {}", pairing.secret);
    assert_ne!(PairingString::parse(&pair(&client).pairing).unwrap().secret, pairing.secret, "every offer has its own secret");
    server.shutdown();
}

#[test]
fn an_offer_redeems_once_and_a_reused_string_gets_the_same_401_as_any_failure() {
    let dir = TempDir::new("server-pairing-once");
    let (server, client) = setup(&dir, TestClock::new());
    let offer = pair(&client);

    let paired = redeem(&offer.pairing, "laptop").expect("the first connect");
    assert_eq!(paired.name, "laptop");
    assert_refused(redeem(&offer.pairing, "laptop"), "a second connect with one string");

    let addr = server.addrs().loopback;
    let reused = post_json(addr, "/api/v1/connect", &[], &format!(r#"{{"secret":"{}","name":"x"}}"#, PairingString::parse(&offer.pairing).unwrap().secret));
    let unknown = post_json(addr, "/api/v1/connect", &[], r#"{"secret":"made-up","name":"x"}"#);
    let no_token = post_json(addr, "/api/v1/hello", &[], "{}");
    assert_eq!(reused.status, 401);
    assert_eq!(reused.body, no_token.body);
    assert_eq!(unknown.body, no_token.body);
    server.shutdown();
}

#[test]
fn an_offer_expires_300_seconds_after_it_was_made() {
    let dir = TempDir::new("server-pairing-expiry");
    let clock = TestClock::new();
    let (server, client) = setup(&dir, clock.clone());
    let early = pair(&client);
    let late = pair(&client);

    clock.advance(299_000);
    redeem(&early.pairing, "early").expect("an offer is good for 300 s");
    clock.advance(1_001);
    assert_refused(redeem(&late.pairing, "late"), "an expired offer");
    server.shutdown();
}

#[test]
fn four_wrong_secrets_leave_the_offers_open() {
    let dir = TempDir::new("server-pairing-four");
    let (server, client) = setup(&dir, TestClock::new());
    let a = pair(&client);

    for n in 0..4 {
        assert_refused(redeem(&with_secret(&a.pairing, &format!("wrong-{n}")), "guess"), "a wrong secret");
    }

    redeem(&a.pairing, "a").expect("4 wrong secrets leave the offers open");
    server.shutdown();
}

#[test]
fn five_wrong_secrets_wipe_every_open_offer() {
    let dir = TempDir::new("server-pairing-wipe");
    let (server, client) = setup(&dir, TestClock::new());
    let a = pair(&client);
    let b = pair(&client);

    for n in 0..5 {
        assert_refused(redeem(&with_secret(&a.pairing, &format!("wrong-{n}")), "guess"), "a wrong secret");
    }

    assert_refused(redeem(&a.pairing, "a"), "an offer after 5 wrong secrets");
    assert_refused(redeem(&b.pairing, "b"), "another offer after 5 wrong secrets");
    let fresh = pair(&client);
    redeem(&fresh.pairing, "fresh").expect("a new offer after the wipe");
    server.shutdown();
}

#[test]
fn at_most_five_offers_are_open_at_once() {
    let dir = TempDir::new("server-pairing-cap");
    let clock = TestClock::new();
    let (server, client) = setup(&dir, clock.clone());
    let offers: Vec<PairResponse> = (0..5).map(|_| pair(&client)).collect();

    let sixth = client.call_named::<_, PairResponse>("pair", &PairRequest {});
    assert_eq!(sixth.expect_err("a sixth open offer").code, ErrorCode::Conflict);

    redeem(&offers[0].pairing, "first").expect("redeem one");
    pair(&client);
    clock.advance(300_001);
    for _ in 0..5 {
        pair(&client);
    }
    server.shutdown();
}

#[test]
fn a_device_token_authenticates_is_listed_by_devices_and_stops_working_after_unpair() {
    let dir = TempDir::new("server-pairing-devices");
    let (server, client) = setup(&dir, TestClock::new());
    let offer = pair(&client);
    let pairing = PairingString::parse(&offer.pairing).unwrap();

    let laptop = redeem(&offer.pairing, "laptop").expect("connect");
    let device = client_for(&pairing, Some(laptop.token.clone()));
    device.call(&HELLO, &HelloRequest {}).expect("the device token authenticates");
    let phone = redeem(&pair(&client).pairing, "phone").expect("connect a second device");

    let listed = devices(&client);
    let names: Vec<(&str, &str)> = listed.iter().map(|d| (d.id.0.as_str(), d.name.as_str())).collect();
    assert_eq!(names, vec![(laptop.device.0.as_str(), "laptop"), (phone.device.0.as_str(), "phone")]);
    assert_ne!(laptop.device.0, "local");
    let as_json = serde_json::to_string(&listed).unwrap();
    assert!(!as_json.contains(&laptop.token) && !as_json.contains(&phone.token), "devices must not show tokens");
    assert_eq!(devices(&device).len(), 2, "a device has full access");

    client.call_named::<_, ()>("unpair", &UnpairRequest { id: laptop.device.clone() }).expect("unpair");

    let refused = device.call(&HELLO, &HelloRequest {}).expect_err("an unpaired device's token");
    assert_eq!(refused.code, ErrorCode::Unauthenticated);
    let still = client_for(&pairing, Some(phone.token.clone()));
    still.call(&HELLO, &HelloRequest {}).expect("the other device still works");
    assert_eq!(devices(&client).len(), 1);
    server.shutdown();
}
