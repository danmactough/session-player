// A bundle import is an upload: the bundle file is the request body (application/octet-stream).
// Over the server's cap it is a 413 too_large and nothing permanent is written. The client streams
// an upload from a reader instead of reading it all into memory first.
#[path = "../../testutil/tempdir.rs"]
mod tempdir;
mod common;

use std::io::Read;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use common::{CtxParams, OpenWith, RawRequest, SessionFile, bearer, files_under, loopback_config, local_client, open_ctx, open_ctx_with, raw, reindex, row, rows, session_file, start};
use session_player_client::RawCall;
use session_player_core::ops::{ErrorCode, RawData};
use session_player_core::test_support::{bundle, index};
use session_player_server::SERVER_EXTENSIONS;
use tempdir::TempDir;

/// A bundle file of one session, made in a library of its own under `dir/name`.
fn bundle_bytes(dir: &Path, name: &str, id: &str, prompt: &str) -> Vec<u8> {
    let exporter = dir.join(name);
    let ctx = open_ctx(CtxParams { dir: &exporter, clock: None, event_capacity: None, fixtures: true });
    let home = exporter.join("home");
    session_file(SessionFile { home: &home, cwd: &home.join("code/app"), id, prompt, images: &[] });
    reindex(&ctx);
    let path = row(&ctx, id).path;
    let conn = index::open(&ctx.paths().db).unwrap();
    let built = bundle::build(&conn, bundle::ExportRequest { path: &path, branch: None, subagents: None }, "test").unwrap();
    let file = exporter.join(format!("{id}.{}", bundle::EXTENSION));
    bundle::write(&built, &file).unwrap();
    std::fs::read(&file).unwrap()
}

fn upload(data: RawData) -> RawCall {
    RawCall { name: "import_bundle".to_string(), body: data, content_type: "application/octet-stream".to_string(), query: Vec::new() }
}

#[test]
fn an_uploaded_bundle_is_imported_and_its_session_listed() {
    let dir = TempDir::new("server-import-upload");
    let bytes = bundle_bytes(&dir, "exporter", "u1", "Add a health check endpoint");
    let ctx = open_ctx(CtxParams { dir: &dir.join("importer"), clock: None, event_capacity: None, fixtures: false });
    let server = start(&ctx, loopback_config());
    let client = local_client(&server, &ctx.paths().data);

    let answer = client.call_raw(upload(RawData::Bytes(bytes))).expect("import_bundle");

    let RawData::Stream(mut body) = answer.data else { panic!("call_raw returned bytes") };
    let mut text = String::new();
    body.read_to_string(&mut text).unwrap();
    let report: serde_json::Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("not JSON ({e}): {text}"));
    assert_eq!(report["imported"], serde_json::json!(["claude:u1"]));
    let imported = row(&ctx, "u1");
    assert!(imported.from_bundle);
    assert!(Path::new(&imported.path).starts_with(&ctx.paths().library), "{}", imported.path);
    server.shutdown();
}

#[test]
fn an_upload_over_the_cap_is_413_too_large_and_writes_nothing_permanent() {
    let dir = TempDir::new("server-import-cap");
    let bytes = bundle_bytes(&dir, "exporter", "u1", "Add a health check endpoint");
    assert!(bytes.len() > 256, "the bundle must be over the cap: {}", bytes.len());
    let importer = dir.join("importer");
    let ctx = open_ctx_with(CtxParams { dir: &importer, clock: None, event_capacity: None, fixtures: false }, OpenWith {
        upload_cap: Some(256),
        extensions: SERVER_EXTENSIONS.to_vec(),
    });
    let server = start(&ctx, loopback_config());
    let before = files_under(&importer);

    let answer = raw(server.addrs().loopback, RawRequest {
        method: "POST",
        path: "/api/v1/import_bundle",
        host: None,
        headers: &[("Content-Type", "application/octet-stream"), ("Authorization", &bearer(&ctx.paths().data))],
        body: &bytes,
    });
    let through_client = local_client(&server, &ctx.paths().data).call_raw(upload(RawData::Bytes(bytes.clone())));

    assert_eq!(answer.status, 413, "{}", String::from_utf8_lossy(&answer.body));
    assert_eq!(answer.code(), "too_large");
    match through_client {
        Err(e) => assert_eq!(e.code, ErrorCode::TooLarge, "{e:?}"),
        Ok(_) => panic!("the client's upload over the cap was accepted"),
    }
    assert_eq!(files_under(&importer), before);
    assert!(rows(&ctx).is_empty());
    server.shutdown();
}

/// Hands out `len` bytes a chunk at a time, counting what it has handed out.
struct CountingReader {
    left: u64,
    handed_out: Arc<AtomicU64>,
}

impl Read for CountingReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = (buf.len() as u64).min(self.left).min(64 * 1024) as usize;
        buf[..n].fill(b'x');
        self.left -= n as u64;
        self.handed_out.fetch_add(n as u64, Ordering::SeqCst);
        Ok(n)
    }
}

#[test]
fn the_client_streams_an_upload_from_a_reader_instead_of_reading_it_all_first() {
    const CAP: u64 = 1024 * 1024;
    const UPLOAD: u64 = 64 * 1024 * 1024;
    let dir = TempDir::new("server-import-stream");
    let ctx = open_ctx_with(CtxParams { dir: &dir, clock: None, event_capacity: None, fixtures: false }, OpenWith {
        upload_cap: Some(CAP),
        extensions: SERVER_EXTENSIONS.to_vec(),
    });
    let server = start(&ctx, loopback_config());
    let client = local_client(&server, &ctx.paths().data);
    let handed_out = Arc::new(AtomicU64::new(0));
    let reader = CountingReader { left: UPLOAD, handed_out: handed_out.clone() };

    let answer = client.call_raw(upload(RawData::Stream(Box::new(reader))));

    match answer {
        Err(e) => assert_eq!(e.code, ErrorCode::TooLarge, "{e:?}"),
        Ok(_) => panic!("an upload over the cap was accepted"),
    }
    let read = handed_out.load(Ordering::SeqCst);
    assert!(read < UPLOAD / 4, "the client read {read} of {UPLOAD} bytes for a server that stopped after {CAP}");
    server.shutdown();
}
