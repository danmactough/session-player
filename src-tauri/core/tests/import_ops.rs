// Moving sessions in and out: a bundle upload with a size cap, a whole project streamed out with
// `export_project`, and `ops::import_stream` reading such a stream into another library.
#[path = "../../testutil/tempdir.rs"]
mod tempdir;
#[path = "common/ops.rs"]
mod common;

use std::collections::BTreeSet;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Value, json};
use session_player_core::ops::{self, Ctx, ImportStreamParams, Input, LIST_SESSIONS, ListSessionsRequest, OpError, Output, RawBody, RawData};
use session_player_core::test_support::{bundle, index};
use tempdir::TempDir;

/// A bundle file of one session, made in a library of its own under `dir/name`.
fn bundle_bytes(dir: &Path, name: &str, id: &str, prompt: &str) -> Vec<u8> {
    let exporter = dir.join(name);
    let ctx = common::open_simple(&exporter, None);
    common::session(&exporter, id, prompt);
    common::reindex(&ctx);
    let row = common::row(&ctx, id);
    let conn = index::open(&ctx.paths().db).unwrap();
    let built = bundle::build(&conn, bundle::ExportRequest { path: &row.path, branch: None, subagents: None }, "test").unwrap();
    let file = exporter.join(format!("{id}.{}", bundle::EXTENSION));
    bundle::write(&built, &file).unwrap();
    std::fs::read(&file).unwrap()
}

fn upload(ctx: &Arc<Ctx>, bytes: Vec<u8>, query: &[(&str, &str)]) -> Result<Value, OpError> {
    let input = Input::Raw {
        body: Box::new(Cursor::new(bytes)),
        content_type: Some("application/octet-stream".to_string()),
        query: query.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
    };
    match ops::call_raw(ctx, "import_bundle", input)? {
        Output::Json(value) => Ok(value),
        Output::Raw(_) => panic!("import_bundle answered with a raw body"),
    }
}

/// Every file under `dir`, except SQLite's own journal files.
fn files_under(dir: &Path) -> BTreeSet<PathBuf> {
    let mut found = BTreeSet::new();
    let mut todo = vec![dir.to_path_buf()];
    while let Some(folder) = todo.pop() {
        let Ok(entries) = std::fs::read_dir(&folder) else { continue };
        for entry in entries {
            let path = entry.unwrap().path();
            if path.is_dir() {
                todo.push(path);
            } else if !path.to_string_lossy().ends_with("-wal") && !path.to_string_lossy().ends_with("-shm") {
                found.insert(path);
            }
        }
    }
    found
}

#[test]
fn an_uploaded_bundle_is_imported_into_the_library_and_its_session_listed() {
    let dir = TempDir::new("import-upload");
    let bytes = bundle_bytes(&dir, "exporter", "s1", "Add a health check endpoint");
    let ctx = common::open_simple(&dir.join("importer"), None);

    let report = upload(&ctx, bytes, &[]).expect("import_bundle");

    assert_eq!(report["imported"], json!(["claude:s1"]));
    common::reindex(&ctx);
    let row = common::row(&ctx, "s1");
    assert!(row.from_bundle);
    assert!(Path::new(&row.path).starts_with(&ctx.paths().library), "{}", row.path);
}

#[test]
fn a_conflicting_upload_writes_nothing_until_on_conflict_is_given_in_the_query() {
    let dir = TempDir::new("import-conflict");
    let first = bundle_bytes(&dir, "first", "s1", "Add a health check endpoint");
    let changed = bundle_bytes(&dir, "changed", "s1", "Add a readiness endpoint instead");
    let ctx = common::open_simple(&dir.join("importer"), None);
    upload(&ctx, first, &[]).expect("first import");

    let asked = upload(&ctx, changed.clone(), &[]).expect("import_bundle");
    assert_eq!(asked["imported"], json!([]));
    assert_eq!(asked["conflicts"][0]["id"], "claude:s1");

    let replaced = upload(&ctx, changed, &[("on_conflict", "replace")]).expect("import_bundle replacing");
    assert_eq!(replaced["imported"], json!(["claude:s1"]));
}

#[test]
fn an_upload_over_the_cap_is_too_large_and_writes_nothing_permanent() {
    let dir = TempDir::new("import-cap");
    let bytes = bundle_bytes(&dir, "exporter", "s1", "Add a health check endpoint");
    assert!(bytes.len() > 256, "the bundle must be over the cap: {}", bytes.len());
    let importer = dir.join("importer");
    let ctx = common::open(common::Open { dir: &importer, claude: None, upload_cap: Some(256), register_claude_root: true });
    let before = files_under(&importer);

    let refused = upload(&ctx, bytes, &[]);

    assert!(matches!(refused, Err(OpError::TooLarge)), "{refused:?}");
    assert_eq!(files_under(&importer), before);
    common::reindex(&ctx);
    assert!(common::rows(&ctx).is_empty());
}

/// A library holding sessions of the repository "widget" and a large one, so its export is too
/// big to sit in a pipe.
fn exporter_with_project(dir: &Path) -> Arc<Ctx> {
    let ctx = common::open_simple(dir, None);
    let home = common::home(dir);
    let project = common::checkout(&home, "widget");
    for (id, prompt) in [("w1", "Add a health check endpoint".to_string()), ("w2", noise(3_000_000))] {
        common::session_file(common::SessionFile { home: &home, cwd: &project, id, prompt: &prompt, images: &[], linked_images: &[] });
    }
    common::reindex(&ctx);
    ctx
}

/// Text that compresses poorly.
fn noise(len: usize) -> String {
    let mut state: u64 = 0x2545_f491_4f6c_dd1d;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            char::from(b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789 "[(state % 63) as usize])
        })
        .collect()
}

fn export_project(ctx: &Arc<Ctx>, repo: &str) -> RawBody {
    match ops::call_raw(ctx, "export_project", Input::Json(json!({ "repo": repo }))).expect("export_project") {
        Output::Raw(body) => body,
        Output::Json(value) => panic!("export_project answered with JSON: {value}"),
    }
}

#[test]
fn export_project_streams_the_project_and_import_stream_reads_it_into_another_library() {
    let dir = TempDir::new("import-stream");
    let exporter = exporter_with_project(&dir.join("exporter"));
    let importer_dir = dir.join("importer");
    let importer = common::open_simple(&importer_dir, None);
    let project = common::checkout(&common::home(&importer_dir), "widget");

    let RawData::Stream(mut stream) = export_project(&exporter, "widget").data else { panic!("export_project sent bytes, not a stream") };
    let mut exported = Vec::new();
    stream.read_to_end(&mut exported).unwrap();

    let dry = ops::import_stream(&importer, ImportStreamParams { project: project.clone(), dry_run: true, input: &mut Cursor::new(exported.clone()) })
        .expect("a dry run");
    assert_eq!(dry.sessions.added, 2);
    assert!(common::rows(&importer).is_empty(), "a dry run stores nothing");

    let pulled = ops::import_stream(&importer, ImportStreamParams { project, dry_run: false, input: &mut Cursor::new(exported) }).expect("import_stream");

    assert_eq!(pulled.sessions.added, 2);
    let listed = ops::call(&importer, &LIST_SESSIONS, &ListSessionsRequest { repo: Some("widget".to_string()), since: None }).expect("list_sessions");
    let ids: BTreeSet<String> = listed.into_iter().map(|r| r.native_id).collect();
    assert_eq!(ids, BTreeSet::from(["w1".to_string(), "w2".to_string()]), "listed when import_stream returns");
}

#[test]
fn dropping_an_export_project_reader_stops_its_producer() {
    let dir = TempDir::new("import-stream-drop");
    let exporter_dir = dir.join("exporter");
    let exporter = exporter_with_project(&exporter_dir);
    // SQLite removes the write-ahead log when the last connection to the database closes, so the
    // log is there exactly while the producer holds its connection.
    let wal = PathBuf::from(format!("{}-wal", exporter.paths().db.display()));
    assert!(!wal.exists(), "a connection is still open before the export");

    let RawData::Stream(mut stream) = export_project(&exporter, "widget").data else { panic!("export_project sent bytes, not a stream") };
    let mut start = [0u8; 64];
    stream.read_exact(&mut start).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert!(wal.exists(), "the producer finished without waiting for the reader");

    drop(stream);

    common::wait_until("the producer stops and closes its connection", || !wal.exists());
}
