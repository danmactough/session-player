// server.token is the local token: the loopback listener accepts it and the external one refuses it
// like any wrong token. A device token works on both.
#[path = "../../testutil/tempdir.rs"]
mod tempdir;
mod common;

use common::{CtxParams, client_for, external_config, local_client, open_ctx, read_token, start};
use session_player_client::{ConnectParams, PairingString, connect};
use session_player_core::ops::{ErrorCode, HELLO, HelloRequest};
use session_player_server::pairing::{PairRequest, PairResponse};
use tempdir::TempDir;

#[test]
fn the_local_token_is_accepted_on_loopback_and_refused_on_the_external_listener() {
    let dir = TempDir::new("server-listener-auth");
    let ctx = open_ctx(CtxParams { dir: &dir, clock: None, event_capacity: None, fixtures: false });
    let data = ctx.paths().data.clone();
    let server = start(&ctx, external_config());
    let local = local_client(&server, &data);
    let offer: PairResponse = local.call_named("pair", &PairRequest {}).expect("pair");
    let pairing = PairingString::parse(&offer.pairing).unwrap();
    let paired = connect(ConnectParams { pairing: &offer.pairing, name: "laptop" }).expect("connect");

    let external_with_local_token = client_for(&pairing, Some(read_token(&data)));
    let refused = external_with_local_token.call(&HELLO, &HelloRequest {}).expect_err("the local token on the external listener");
    assert_eq!(refused.code, ErrorCode::Unauthenticated);

    local.call(&HELLO, &HelloRequest {}).expect("the local token on loopback");
    client_for(&pairing, Some(paired.token.clone())).call(&HELLO, &HelloRequest {}).expect("a device token on the external listener");
    let loopback = PairingString { address: "127.0.0.1".to_string(), port: server.addrs().loopback.port(), fingerprint: None, secret: String::new() };
    client_for(&loopback, Some(paired.token.clone())).call(&HELLO, &HelloRequest {}).expect("a device token on loopback");
    server.shutdown();
}
