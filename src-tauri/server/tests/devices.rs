// devices.json keeps paired devices across restarts: owner-only, replaced atomically, and holding
// SHA-256 hashes of the tokens, never the tokens.
#[path = "../../testutil/tempdir.rs"]
mod tempdir;
mod common;

use std::path::Path;

use common::{CtxParams, client_for, loopback_config, local_client, open_ctx, start};
use session_player_client::{Client, ConnectParams, PairedServer, PairingString, connect};
use session_player_core::ops::{HELLO, HelloRequest};
use session_player_server::pairing::{DeviceInfo, DevicesRequest, PairRequest, PairResponse, UnpairRequest};
use tempdir::TempDir;

fn pair_device(client: &Client, name: &str) -> (PairingString, PairedServer) {
    let offer: PairResponse = client.call_named("pair", &PairRequest {}).expect("pair");
    let paired = connect(ConnectParams { pairing: &offer.pairing, name }).expect("connect");
    (PairingString::parse(&offer.pairing).unwrap(), paired)
}

fn sha256_hex(text: &str) -> String {
    ring::digest::digest(&ring::digest::SHA256, text.as_bytes()).as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

fn leftovers(data: &Path) -> Vec<String> {
    std::fs::read_dir(data)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.starts_with("devices") && n != "devices.json")
        .collect()
}

#[test]
fn devices_json_is_owner_only_holds_token_hashes_only_and_is_replaced_atomically() {
    let dir = TempDir::new("server-devices-file");
    let ctx = open_ctx(CtxParams { dir: &dir, clock: None, event_capacity: None, fixtures: false });
    let data = ctx.paths().data.clone();
    let server = start(&ctx, loopback_config());
    let client = local_client(&server, &data);

    let (_, laptop) = pair_device(&client, "laptop");
    let file = data.join("devices.json");
    let text = std::fs::read_to_string(&file).expect("devices.json after a connect");
    assert!(!text.contains(&laptop.token), "devices.json holds a token");
    assert!(text.contains(&sha256_hex(&laptop.token)), "devices.json lacks the token's SHA-256: {text}");
    assert!(text.contains(&laptop.device.0));
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let before = std::fs::metadata(&file).unwrap();
        assert_eq!(before.permissions().mode() & 0o777, 0o600);

        let (_, phone) = pair_device(&client, "phone");

        let after = std::fs::metadata(&file).unwrap();
        assert_ne!(after.ino(), before.ino(), "devices.json was rewritten in place instead of replaced by a rename");
        assert_eq!(after.permissions().mode() & 0o777, 0o600);
        assert!(!std::fs::read_to_string(&file).unwrap().contains(&phone.token));
    }
    assert!(leftovers(&data).is_empty(), "temp files left: {:?}", leftovers(&data));
    server.shutdown();
}

#[test]
fn paired_devices_survive_a_restart_and_an_unpair_survives_one_too() {
    let dir = TempDir::new("server-devices-restart");
    let ctx = open_ctx(CtxParams { dir: &dir, clock: None, event_capacity: None, fixtures: false });
    let data = ctx.paths().data.clone();
    let server = start(&ctx, loopback_config());
    let client = local_client(&server, &data);
    let (_, laptop) = pair_device(&client, "laptop");
    let (_, phone) = pair_device(&client, "phone");
    server.shutdown();

    let ctx = open_ctx(CtxParams { dir: &dir, clock: None, event_capacity: None, fixtures: false });
    let server = start(&ctx, loopback_config());
    let client = local_client(&server, &data);
    let listed: Vec<DeviceInfo> = client.call_named("devices", &DevicesRequest {}).expect("devices");
    assert_eq!(listed.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(), vec!["laptop", "phone"]);
    let loopback = PairingString { address: "127.0.0.1".to_string(), port: server.addrs().loopback.port(), fingerprint: None, secret: String::new() };
    client_for(&loopback, Some(laptop.token.clone())).call(&HELLO, &HelloRequest {}).expect("the laptop's token after a restart");
    client.call_named::<_, ()>("unpair", &UnpairRequest { id: phone.device.clone() }).expect("unpair");
    server.shutdown();

    let ctx = open_ctx(CtxParams { dir: &dir, clock: None, event_capacity: None, fixtures: false });
    let server = start(&ctx, loopback_config());
    let loopback = PairingString { port: server.addrs().loopback.port(), ..loopback };
    let refused = client_for(&loopback, Some(phone.token.clone())).call(&HELLO, &HelloRequest {});
    assert!(refused.is_err(), "an unpaired device authenticated after a restart");
    client_for(&loopback, Some(laptop.token.clone())).call(&HELLO, &HelloRequest {}).expect("the laptop still works");
    server.shutdown();
}
