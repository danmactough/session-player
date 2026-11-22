// The server's first contract: a caller holding the local token can ask `hello` over HTTP what it
// is talking to.
#[path = "../../testutil/tempdir.rs"]
mod tempdir;
mod common;

use common::{CtxParams, loopback_config, local_client, open_ctx, start};
use session_player_core::ops::{API_MINOR, API_VERSION, HELLO, HelloRequest};
use tempdir::TempDir;

#[test]
fn a_request_with_the_local_token_to_hello_returns_the_api_and_server_versions() {
    let dir = TempDir::new("server-hello");
    let ctx = open_ctx(CtxParams { dir: &dir, clock: None, event_capacity: None, fixtures: false });
    let server = start(&ctx, loopback_config());
    let client = local_client(&server, &ctx.paths().data);

    let hello = client.call(&HELLO, &HelloRequest {}).expect("hello");

    assert_eq!(hello.api_version, API_VERSION);
    assert_eq!(hello.api_minor, API_MINOR);
    assert_eq!(hello.server_version, common::VERSION);
    assert!(server.addrs().loopback.ip().is_loopback());
    server.shutdown();
}
