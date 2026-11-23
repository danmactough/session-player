// `load_session` rewrites each data: image to `/api/v1/image/{generation}/{event}/{image}`, and a
// GET of that path (bearer or cookie) answers the image's exact bytes with the data URL's content
// type, private caching and a stable ETag. A matching If-None-Match is a 304. Out of range or
// stale ids are a JSON 404 that names no paths; a non-numeric id is a 400; without a token it is
// the same 401 as any other refusal.
#[path = "../../testutil/tempdir.rs"]
mod tempdir;
mod common;

use std::net::SocketAddr;
use std::sync::Arc;

use base64::Engine;
use common::{CtxParams, RawRequest, RawResponse, SessionFile, bearer, loopback_config, local_client, open_ctx, post_json, raw, reindex, row, session_file, start};
use session_player_client::{Client, PairingString};
use session_player_core::ops::{Ctx, LOAD_SESSION, LoadSessionRequest};
use session_player_server::ServerHandle;
use session_player_server::pairing::{PairRequest, PairResponse};
use tempdir::TempDir;

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n not really a png";
const JPEG: &[u8] = b"\xff\xd8\xff jpeg bytes";

struct Setup {
    ctx: Arc<Ctx>,
    server: ServerHandle,
    client: Client,
    path: String,
}

fn setup(dir: &TempDir) -> Setup {
    let ctx = open_ctx(CtxParams { dir, clock: None, event_capacity: None, fixtures: true });
    let home = dir.join("home");
    let b64 = |bytes: &[u8]| base64::engine::general_purpose::STANDARD.encode(bytes);
    let (png, jpeg) = (b64(PNG), b64(JPEG));
    session_file(SessionFile { home: &home, cwd: &home.join("code/app"), id: "i1", prompt: "Look at these", images: &[("image/png", &png), ("image/jpeg", &jpeg)] });
    reindex(&ctx);
    let path = row(&ctx, "i1").path;
    let server = start(&ctx, loopback_config());
    let client = local_client(&server, &ctx.paths().data);
    Setup { ctx, server, client, path }
}

/// The image URLs `load_session` returned, in order.
fn image_urls(s: &Setup) -> Vec<String> {
    let events = s.client.call(&LOAD_SESSION, &LoadSessionRequest { path: s.path.clone(), branch: None }).expect("load_session");
    let urls: Vec<String> = events.into_iter().flat_map(|e| e.images).filter(|u| u.starts_with("/api/v1/image/")).collect();
    assert_eq!(urls.len(), 2, "{urls:?}");
    urls
}

fn get(addr: SocketAddr, path: &str, headers: &[(&str, &str)]) -> RawResponse {
    raw(addr, RawRequest { method: "GET", path, host: None, headers, body: b"" })
}

/// (generation, event, image) of an image URL.
fn ids(url: &str) -> (u64, usize, usize) {
    let parts: Vec<&str> = url.trim_start_matches("/api/v1/image/").split('/').collect();
    (parts[0].parse().unwrap(), parts[1].parse().unwrap(), parts[2].parse().unwrap())
}

#[test]
fn an_image_url_from_load_session_serves_the_exact_bytes_with_its_type_and_private_caching() {
    let dir = TempDir::new("server-image-ok");
    let s = setup(&dir);
    let urls = image_urls(&s);
    let addr = s.server.addrs().loopback;
    let authorization = bearer(&s.ctx.paths().data);

    let png = get(addr, &urls[0], &[("Authorization", &authorization)]);
    let jpeg = get(addr, &urls[1], &[("Authorization", &authorization)]);
    let png_again = get(addr, &urls[0], &[("Authorization", &authorization)]);

    assert_eq!(png.status, 200, "{}", String::from_utf8_lossy(&png.body));
    assert_eq!(png.header("content-type"), Some("image/png"));
    assert_eq!(png.body, PNG);
    assert_eq!(png.header("cache-control"), Some("private, max-age=3600"));
    let etag = png.header("etag").expect("an ETag");
    assert!(etag.starts_with('"') && etag.ends_with('"') && etag.len() > 2, "{etag}");
    assert_eq!(png_again.header("etag"), Some(etag), "the ETag is stable");
    assert_eq!(jpeg.status, 200);
    assert_eq!(jpeg.header("content-type"), Some("image/jpeg"));
    assert_eq!(jpeg.body, JPEG);
    assert_ne!(jpeg.header("etag"), Some(etag), "another image has another ETag");
    s.server.shutdown();
}

#[test]
fn a_request_with_the_images_etag_in_if_none_match_gets_304_without_a_body() {
    let dir = TempDir::new("server-image-304");
    let s = setup(&dir);
    let urls = image_urls(&s);
    let addr = s.server.addrs().loopback;
    let authorization = bearer(&s.ctx.paths().data);
    let first = get(addr, &urls[0], &[("Authorization", &authorization)]);
    let etag = first.header("etag").expect("an ETag").to_string();

    let again = get(addr, &urls[0], &[("Authorization", &authorization), ("If-None-Match", &etag)]);

    assert_eq!(again.status, 304);
    assert!(again.body.is_empty());
    let other = get(addr, &urls[0], &[("Authorization", &authorization), ("If-None-Match", "\"something-else\"")]);
    assert_eq!(other.status, 200);
    s.server.shutdown();
}

#[test]
fn an_out_of_range_or_stale_image_is_a_json_404_that_names_no_paths() {
    let dir = TempDir::new("server-image-404");
    let s = setup(&dir);
    let urls = image_urls(&s);
    let (generation, event, image) = ids(&urls[0]);
    let addr = s.server.addrs().loopback;
    let authorization = bearer(&s.ctx.paths().data);
    // Four more loads push the first generation out of the cache.
    for _ in 0..4 {
        image_urls(&s);
    }

    for path in [
        format!("/api/v1/image/{generation}/{}/{image}", event + 1000),
        format!("/api/v1/image/{generation}/{event}/{}", image + 1000),
        format!("/api/v1/image/{}/{event}/{image}", generation + 1000),
        urls[0].clone(),
    ] {
        let answer = get(addr, &path, &[("Authorization", &authorization)]);
        assert_eq!(answer.status, 404, "{path}");
        assert_eq!(answer.code(), "not_found", "{path}");
        let body = String::from_utf8_lossy(&answer.body);
        assert!(!body.contains(&*dir.to_string_lossy()) && !body.contains(".jsonl") && !body.contains(&s.path), "{path}: {body}");
    }
    s.server.shutdown();
}

#[test]
fn a_non_numeric_image_path_is_a_400() {
    let dir = TempDir::new("server-image-400");
    let s = setup(&dir);
    let addr = s.server.addrs().loopback;
    let authorization = bearer(&s.ctx.paths().data);

    for path in ["/api/v1/image/x/0/0", "/api/v1/image/1/y/0", "/api/v1/image/1/0/-1", "/api/v1/image/1/0/0.5"] {
        let answer = get(addr, path, &[("Authorization", &authorization)]);
        assert_eq!(answer.status, 400, "{path}: {}", String::from_utf8_lossy(&answer.body));
    }
    s.server.shutdown();
}

#[test]
fn an_image_request_without_a_token_gets_the_same_401_as_any_other() {
    let dir = TempDir::new("server-image-401");
    let s = setup(&dir);
    let urls = image_urls(&s);
    let addr = s.server.addrs().loopback;
    let refused = post_json(addr, "/api/v1/hello", &[], "{}");

    for path in [urls[0].as_str(), "/api/v1/image/x/0/0", "/api/v1/image/999/0/0"] {
        let answer = get(addr, path, &[]);
        assert_eq!(answer.status, 401, "{path}");
        assert_eq!(answer.body, refused.body, "{path}");
    }
    let wrong = get(addr, &urls[0], &[("Authorization", "Bearer wrong")]);
    assert_eq!(wrong.status, 401);
    assert_eq!(wrong.body, refused.body);
    s.server.shutdown();
}

#[test]
fn a_browser_with_the_session_cookie_gets_the_image() {
    let dir = TempDir::new("server-image-cookie");
    let s = setup(&dir);
    let urls = image_urls(&s);
    let addr = s.server.addrs().loopback;
    let offer: PairResponse = s.client.call_named("pair", &PairRequest {}).expect("pair");
    let secret = PairingString::parse(&offer.pairing).unwrap().secret;
    let origin = format!("http://{addr}");
    let body = format!(r#"{{"secret":"{secret}","name":"browser","browser":true}}"#);
    let connected = raw(addr, RawRequest {
        method: "POST",
        path: "/api/v1/connect",
        host: None,
        headers: &[("Content-Type", "application/json"), ("Origin", &origin)],
        body: body.as_bytes(),
    });
    let cookie = connected.header("set-cookie").expect("a cookie").split(';').next().unwrap().trim().to_string();

    let answer = get(addr, &urls[0], &[("Cookie", &cookie)]);

    assert_eq!(answer.status, 200, "{}", String::from_utf8_lossy(&answer.body));
    assert_eq!(answer.body, PNG);
    s.server.shutdown();
}
