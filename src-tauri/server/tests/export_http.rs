// A session export is a download: `export_preview` builds the bundle into the caller's slot, and
// `export_session` sends exactly that bundle once, as an attachment with a filename that is safe
// in a Content-Disposition header, and only to the device that previewed it.
#[path = "../../testutil/tempdir.rs"]
mod tempdir;
mod common;

use std::sync::Arc;

use common::{CtxParams, RawRequest, RawResponse, SessionFile, bearer, loopback_config, local_client, open_ctx, raw, reindex, row, session_file, start};
use session_player_client::{Client, ConnectParams, connect};
use session_player_core::ops::{Ctx, EXPORT_PREVIEW, ErrorCode, ExportPreviewRequest};
use session_player_server::ServerHandle;
use session_player_server::pairing::{PairRequest, PairResponse};
use tempdir::TempDir;

/// A Ctx with one indexed session whose prompt (and so its title) holds quotes, a backslash,
/// slashes, a colon and non-ASCII letters, and a server for it.
fn setup(dir: &TempDir) -> (Arc<Ctx>, ServerHandle, String) {
    let ctx = open_ctx(CtxParams { dir, clock: None, event_capacity: None, fixtures: true });
    let home = dir.join("home");
    let cwd = home.join("code/app");
    session_file(SessionFile { home: &home, cwd: &cwd, id: "e1", prompt: r#"Fix the "café" bug \ split 50/50: naïve"#, images: &[] });
    reindex(&ctx);
    let path = row(&ctx, "e1").path;
    let server = start(&ctx, loopback_config());
    (ctx, server, path)
}

fn preview(client: &Client, path: &str, request: u64) -> u64 {
    let req = ExportPreviewRequest { path: path.to_string(), branch: None, subagents: None, request };
    // ExportPreview is Serialize only, so read its token from the JSON.
    let answer: serde_json::Value = client.call_named(EXPORT_PREVIEW.spec.name, &req).expect("export_preview");
    answer["token"].as_u64().expect("a token")
}

fn download(addr: std::net::SocketAddr, authorization: &str, token: u64) -> RawResponse {
    let body = format!(r#"{{"token":{token},"exported_by":"Tester"}}"#);
    raw(addr, RawRequest {
        method: "POST",
        path: "/api/v1/export_session",
        host: None,
        headers: &[("Content-Type", "application/json"), ("Authorization", authorization)],
        body: body.as_bytes(),
    })
}

/// The filename in `attachment; filename="..."`.
fn attachment_filename(answer: &RawResponse) -> String {
    let disposition = answer.header("content-disposition").expect("a Content-Disposition header");
    let rest = disposition.strip_prefix("attachment; filename=\"").unwrap_or_else(|| panic!("not an attachment with a quoted filename: {disposition}"));
    rest.strip_suffix('"').unwrap_or_else(|| panic!("the filename doesn't end the header: {disposition}")).to_string()
}

#[test]
fn a_previewed_session_downloads_as_an_attachment_with_a_safe_filename() {
    let dir = TempDir::new("server-export-download");
    let (ctx, server, path) = setup(&dir);
    let client = local_client(&server, &ctx.paths().data);
    let token = preview(&client, &path, 1);

    let answer = download(server.addrs().loopback, &bearer(&ctx.paths().data), token);

    assert_eq!(answer.status, 200, "{}", String::from_utf8_lossy(&answer.body));
    assert_eq!(answer.header("content-type"), Some("application/octet-stream"));
    let filename = attachment_filename(&answer);
    assert!(filename.ends_with(".spsession"), "{filename}");
    assert!(filename.len() > ".spsession".len(), "the title is kept in the name: {filename}");
    for c in filename.chars() {
        assert!(c.is_ascii() && !c.is_ascii_control() && !matches!(c, '"' | '\\' | '/'), "{c:?} in {filename}");
    }
    let bundle: serde_json::Value = serde_json::from_slice(&answer.body).expect("the bundle is JSON");
    assert_eq!(bundle["exported_by"], "Tester");
    assert_eq!(bundle["sessions"].as_array().map(Vec::len), Some(1));
    server.shutdown();
}

#[test]
fn a_previewed_bundle_downloads_once() {
    let dir = TempDir::new("server-export-once");
    let (ctx, server, path) = setup(&dir);
    let client = local_client(&server, &ctx.paths().data);
    let token = preview(&client, &path, 1);
    let addr = server.addrs().loopback;
    let authorization = bearer(&ctx.paths().data);

    let first = download(addr, &authorization, token);
    let second = download(addr, &authorization, token);

    assert_eq!(first.status, 200, "{}", String::from_utf8_lossy(&first.body));
    assert_eq!(second.status, 404);
    assert_eq!(second.code(), "not_found");
    server.shutdown();
}

#[test]
fn only_the_device_that_previewed_a_bundle_can_download_it() {
    let dir = TempDir::new("server-export-device");
    let (ctx, server, path) = setup(&dir);
    let local = local_client(&server, &ctx.paths().data);
    let offer: PairResponse = local.call_named("pair", &PairRequest {}).expect("pair");
    let laptop = connect(ConnectParams { pairing: &offer.pairing, name: "laptop" }).expect("connect");
    let token = preview(&local, &path, 1);
    let addr = server.addrs().loopback;

    let other = download(addr, &format!("Bearer {}", laptop.token), token);

    assert_eq!(other.status, 404, "{}", String::from_utf8_lossy(&other.body));
    assert_eq!(other.code(), "not_found");
    let mine = download(addr, &bearer(&ctx.paths().data), token);
    assert_eq!(mine.status, 200, "another device's attempt doesn't use up the preview: {}", String::from_utf8_lossy(&mine.body));

    let laptop_client = Client::paired(&laptop).expect("a paired client");
    let laptop_token = preview(&laptop_client, &path, 7);
    let theirs = download(addr, &format!("Bearer {}", laptop.token), laptop_token);
    assert_eq!(theirs.status, 200, "{}", String::from_utf8_lossy(&theirs.body));
    let again = download(addr, &bearer(&ctx.paths().data), laptop_token);
    assert_eq!(again.status, 404);
    let error: session_player_core::ops::ApiError = serde_json::from_slice(&again.body).expect("an ApiError");
    assert_eq!(error.code, ErrorCode::NotFound);
    server.shutdown();
}
