// Exporting a session: each device previews into its own slot and downloads exactly the bundle it
// previewed, once.
#[path = "../../testutil/tempdir.rs"]
mod tempdir;
#[path = "common/ops.rs"]
mod common;

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Value, json};
use session_player_core::ops::{self, Ctx, Device, DeviceId, DispatchParams, Input, OpError, OpenParams, Output, RawData};
use session_player_core::test_support::bundle;
use tempdir::TempDir;

fn run(ctx: &Arc<Ctx>, device: &str, name: &str, body: Value) -> Result<Output, OpError> {
    ops::dispatch(ctx, DispatchParams { name: name.to_string(), device: DeviceId(device.to_string()), input: Input::Json(body) })
}

fn preview(ctx: &Arc<Ctx>, device: &str, native_id: &str, request: u64) -> Value {
    let path = common::row(ctx, native_id).path;
    match run(ctx, device, "export_preview", json!({ "path": path, "request": request })).expect("export_preview") {
        Output::Json(value) => value,
        Output::Raw(_) => panic!("export_preview answered with a raw body"),
    }
}

/// The downloaded bundle, read back as a bundle file.
fn download(ctx: &Arc<Ctx>, dir: &Path, device: &str, token: u64, name: &str) -> Result<(bundle::Bundle, Option<String>), OpError> {
    match run(ctx, device, "export_session", json!({ "token": token, "exported_by": name }))? {
        Output::Raw(body) => {
            let RawData::Bytes(bytes) = body.data else { panic!("export_session streamed its body") };
            let file = dir.join(format!("download-{device}-{token}.{}", bundle::EXTENSION));
            std::fs::write(&file, bytes).unwrap();
            Ok((bundle::read(&file).expect("a bundle file"), body.filename))
        }
        Output::Json(value) => panic!("export_session answered with JSON: {value}"),
    }
}

fn setup(dir: &TempDir) -> Arc<Ctx> {
    let ctx = common::open_simple(dir, None);
    common::session(dir, "s1", "Add a health check endpoint");
    common::session(dir, "s2", "Rename the config loader");
    common::reindex(&ctx);
    ctx
}

#[test]
fn a_preview_describes_the_bundle_and_downloading_gives_that_bundle_once() {
    let dir = TempDir::new("export-once");
    let ctx = setup(&dir);

    let shown = preview(&ctx, "local", "s1", 1);
    assert_eq!(shown["token"], 1);
    assert_eq!(shown["summary"]["sessions"][0]["id"], "claude:s1");
    let (bundle, filename) = download(&ctx, &dir, "local", 1, "Dana").expect("export_session");

    assert_eq!(bundle.sessions[0].id, "claude:s1");
    assert_eq!(bundle.exported_by.as_deref(), Some("Dana"));
    assert_eq!(bundle.exporter, "Session Player 9.9.9");
    assert_eq!(filename.as_deref(), shown["summary"]["file_name"].as_str());
    let again = download(&ctx, &dir, "local", 1, "Dana");
    assert!(matches!(again, Err(OpError::NotFound(_))), "{again:?}");
}

#[test]
fn a_token_reads_only_its_own_devices_slot() {
    let dir = TempDir::new("export-per-device");
    let ctx = setup(&dir);

    preview(&ctx, "aaaa", "s1", 1);
    preview(&ctx, "bbbb", "s2", 1);

    let stranger = download(&ctx, &dir, "cccc", 1, "Eve");
    assert!(matches!(stranger, Err(OpError::NotFound(_))), "{stranger:?}");
    assert_eq!(download(&ctx, &dir, "bbbb", 1, "Bo").expect("b's bundle").0.sessions[0].id, "claude:s2");
    assert_eq!(download(&ctx, &dir, "aaaa", 1, "Al").expect("a's bundle").0.sessions[0].id, "claude:s1");
}

#[test]
fn a_newer_preview_replaces_an_older_one_from_the_same_device() {
    let dir = TempDir::new("export-newer");
    let ctx = setup(&dir);

    preview(&ctx, "local", "s1", 1);
    preview(&ctx, "local", "s2", 2);

    let stale = download(&ctx, &dir, "local", 1, "Dana");
    assert!(matches!(stale, Err(OpError::NotFound(_))), "{stale:?}");
    assert_eq!(download(&ctx, &dir, "local", 2, "Dana").expect("the newer bundle").0.sessions[0].id, "claude:s2");
}

#[test]
fn a_download_without_a_name_is_a_bad_request_and_keeps_the_preview() {
    let dir = TempDir::new("export-no-name");
    let ctx = setup(&dir);
    preview(&ctx, "local", "s1", 1);

    let refused = download(&ctx, &dir, "local", 1, "   ");

    assert!(matches!(refused, Err(OpError::BadRequest(_))), "{refused:?}");
    assert_eq!(download(&ctx, &dir, "local", 1, "Dana").expect("still there").0.sessions[0].id, "claude:s1");
}

#[test]
fn export_cancel_drops_only_the_callers_preview() {
    let dir = TempDir::new("export-cancel");
    let ctx = setup(&dir);
    preview(&ctx, "aaaa", "s1", 1);
    preview(&ctx, "bbbb", "s2", 1);

    run(&ctx, "aaaa", "export_cancel", json!({})).expect("export_cancel");

    let dropped = download(&ctx, &dir, "aaaa", 1, "Al");
    assert!(matches!(dropped, Err(OpError::NotFound(_))), "{dropped:?}");
    assert_eq!(download(&ctx, &dir, "bbbb", 1, "Bo").expect("b's bundle").0.sessions[0].id, "claude:s2");
}

#[test]
fn unpairing_a_device_drops_its_preview() {
    let dir = TempDir::new("export-unpair");
    let ctx = setup(&dir);
    let device = Device { id: DeviceId("aaaa".to_string()), name: "laptop".to_string(), token_sha256: "0".repeat(64), paired_at: 1 };
    ctx.add_device(device.clone()).unwrap();
    preview(&ctx, "aaaa", "s1", 1);

    assert!(ctx.remove_device(&device.id).unwrap());

    let dropped = download(&ctx, &dir, "aaaa", 1, "Al");
    assert!(matches!(dropped, Err(OpError::NotFound(_))), "{dropped:?}");
}

#[test]
fn a_preview_older_than_30_minutes_by_the_ctx_clock_is_dropped() {
    let dir = TempDir::new("export-expiry");
    let now = Arc::new(AtomicU64::new(common::CLOCK));
    let clock = now.clone();
    let home = common::home(&dir);
    std::fs::create_dir_all(home.join(".claude/projects")).unwrap();
    let paths = common::paths(&dir);
    let conn = session_player_core::test_support::index::open(&paths.db).unwrap();
    session_player_core::test_support::index::add_root(&conn, &home.join(".claude/projects").to_string_lossy()).unwrap();
    drop(conn);
    let ctx = Ctx::open(OpenParams {
        paths,
        home,
        version: "9.9.9".to_string(),
        claude: None,
        clock: Some(Arc::new(move || clock.load(Ordering::SeqCst))),
        event_capacity: None,
        upload_cap: None,
        extensions: vec![],
    })
    .unwrap();
    common::session(&dir, "s1", "Add a health check endpoint");
    common::reindex(&ctx);
    let minute = 60_000;

    preview(&ctx, "aaaa", "s1", 1);
    preview(&ctx, "bbbb", "s1", 1);
    now.fetch_add(29 * minute, Ordering::SeqCst);
    assert_eq!(download(&ctx, &dir, "aaaa", 1, "Al").expect("a 29-minute-old preview").0.sessions[0].id, "claude:s1");
    now.fetch_add(2 * minute, Ordering::SeqCst);

    let expired = download(&ctx, &dir, "bbbb", 1, "Bo");
    assert!(matches!(expired, Err(OpError::NotFound(_))), "{expired:?}");
}
