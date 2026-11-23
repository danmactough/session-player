// `export_project` streams a repository's sessions over HTTP: the first bytes reach the client
// while the producer is still writing (the server never buffers the whole export), and a client
// that drops its reader early stops the producer.
#[path = "../../testutil/tempdir.rs"]
mod tempdir;
mod common;

use std::collections::BTreeSet;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use common::{CtxParams, SessionFile, checkout, loopback_config, local_client, noise, open_ctx, reindex, session_file, start, wait_until};
use session_player_client::{Client, RawCall};
use session_player_core::ops::{self, Ctx, ImportStreamParams, LIST_SESSIONS, ListSessionsRequest, RawData};
use tempdir::TempDir;

/// The large session's prompt, big enough that its export can't sit in the pipe, the socket
/// buffers and the response channel at once.
const LARGE: usize = 30 * 1024 * 1024;

/// A library under `dir` holding two sessions of the repository "widget", one of them large.
fn exporter(dir: &Path, large: usize) -> Arc<Ctx> {
    let ctx = open_ctx(CtxParams { dir, clock: None, event_capacity: None, fixtures: true });
    let home = dir.join("home");
    let project = checkout(&home, "widget");
    session_file(SessionFile { home: &home, cwd: &project, id: "w1", prompt: "Add a health check endpoint", images: &[] });
    session_file(SessionFile { home: &home, cwd: &project, id: "w2", prompt: &noise(large), images: &[] });
    reindex(&ctx);
    ctx
}

fn export_project(client: &Client) -> Box<dyn Read + Send> {
    let call = RawCall {
        name: "export_project".to_string(),
        body: RawData::Bytes(br#"{"repo":"widget"}"#.to_vec()),
        content_type: "application/json".to_string(),
        query: Vec::new(),
    };
    match client.call_raw(call).expect("export_project").data {
        RawData::Stream(reader) => reader,
        RawData::Bytes(_) => panic!("call_raw returned bytes"),
    }
}

/// SQLite removes the write-ahead log when the last connection to the database closes, so the
/// log is there exactly while the producer holds its connection.
fn wal(ctx: &Ctx) -> PathBuf {
    PathBuf::from(format!("{}-wal", ctx.paths().db.display()))
}

#[test]
fn export_project_over_http_streams_a_project_that_import_stream_reads_into_another_library() {
    let dir = TempDir::new("server-export-project");
    let ctx = exporter(&dir.join("exporter"), 10_000);
    let server = start(&ctx, loopback_config());
    let importer_dir = dir.join("importer");
    let importer = open_ctx(CtxParams { dir: &importer_dir, clock: None, event_capacity: None, fixtures: false });
    let project = checkout(&importer_dir.join("home"), "widget");

    let mut exported = Vec::new();
    export_project(&local_client(&server, &ctx.paths().data)).read_to_end(&mut exported).expect("read the stream");

    let pulled = ops::import_stream(&importer, ImportStreamParams { project, dry_run: false, input: &mut Cursor::new(exported) }).expect("import_stream");
    assert_eq!(pulled.sessions.added, 2);
    let listed = ops::call(&importer, &LIST_SESSIONS, &ListSessionsRequest { repo: Some("widget".to_string()), since: None }).expect("list_sessions");
    let ids: BTreeSet<String> = listed.into_iter().map(|r| r.native_id).collect();
    assert_eq!(ids, BTreeSet::from(["w1".to_string(), "w2".to_string()]));
    server.shutdown();
}

#[test]
fn a_large_export_reaches_the_client_while_the_producer_is_still_writing() {
    let dir = TempDir::new("server-export-project-large");
    let ctx = exporter(&dir, LARGE);
    let server = start(&ctx, loopback_config());
    wait_until("no connection is left open by indexing", Duration::from_secs(10), || !wal(&ctx).exists());

    let mut stream = export_project(&local_client(&server, &ctx.paths().data));
    let mut start_bytes = [0u8; 64];
    stream.read_exact(&mut start_bytes).expect("the first bytes");
    std::thread::sleep(Duration::from_millis(300));

    assert!(wal(&ctx).exists(), "the first bytes arrived only after the producer had finished");
    let mut rest = Vec::new();
    stream.read_to_end(&mut rest).expect("the rest of the stream");
    assert!(rest.len() > 1024 * 1024, "the export is large: {} bytes", rest.len());
    server.shutdown();
}

#[test]
fn dropping_the_clients_reader_early_stops_the_producer() {
    let dir = TempDir::new("server-export-project-drop");
    let ctx = exporter(&dir, LARGE);
    let server = start(&ctx, loopback_config());
    wait_until("no connection is left open by indexing", Duration::from_secs(10), || !wal(&ctx).exists());

    let mut stream = export_project(&local_client(&server, &ctx.paths().data));
    let mut start_bytes = [0u8; 64];
    stream.read_exact(&mut start_bytes).expect("the first bytes");
    std::thread::sleep(Duration::from_millis(300));
    assert!(wal(&ctx).exists(), "the producer finished without waiting for the reader");

    drop(stream);

    wait_until("the producer stops and closes its connection", Duration::from_secs(10), || !wal(&ctx).exists());
    server.shutdown();
}
