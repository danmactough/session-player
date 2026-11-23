// The UI's settings (stored under `ui.`), the frontend's log line, and library maintenance.
#[path = "../../testutil/tempdir.rs"]
mod tempdir;
#[path = "common/ops.rs"]
mod common;

use std::sync::Arc;

use rusqlite::OptionalExtension;
use session_player_core::ops::{
    self, Ctx, GET_SETTING, GetSettingRequest, LIBRARY_ERRORS, LOG_FRONTEND, LOG_FRONTEND_MAX, LibraryErrorsRequest, LogFrontendRequest, OpError,
    REMOVE_IMPORTED, RemoveImportedRequest, SERVER_LOG, SET_SETTING, SETTING_KEY_MAX, SetSettingRequest,
};
use session_player_core::test_support::{bundle, index};
use tempdir::TempDir;

fn get(ctx: &Arc<Ctx>, key: &str) -> Result<Option<String>, OpError> {
    ops::call(ctx, &GET_SETTING, &GetSettingRequest { key: key.to_string() })
}

fn set(ctx: &Arc<Ctx>, key: &str, value: &str) -> Result<(), OpError> {
    ops::call(ctx, &SET_SETTING, &SetSettingRequest { key: key.to_string(), value: value.to_string() })
}

/// A setting as the database holds it.
fn stored(ctx: &Ctx, key: &str) -> Option<String> {
    let conn = index::open(&ctx.paths().db).unwrap();
    conn.query_row("SELECT value FROM settings WHERE key=?1", [key], |r| r.get(0)).optional().unwrap()
}

#[test]
fn a_setting_is_stored_under_the_ui_prefix_and_read_back_by_its_bare_key() {
    let dir = TempDir::new("settings-round-trip");
    let ctx = common::open_simple(&dir, None);

    set(&ctx, "theme", "dark").expect("set_setting");

    assert_eq!(get(&ctx, "theme").expect("get_setting").as_deref(), Some("dark"));
    assert_eq!(stored(&ctx, "ui.theme").as_deref(), Some("dark"));
    assert_eq!(stored(&ctx, "theme"), None);
    assert_eq!(get(&ctx, "never-set").expect("get_setting"), None);
}

#[test]
fn the_index_settings_can_be_neither_read_nor_written_through_the_setting_operations() {
    let dir = TempDir::new("settings-internal");
    let ctx = common::open_simple(&dir, None);
    let conn = index::open(&ctx.paths().db).unwrap();
    index::seed_roots(&conn).unwrap();
    drop(conn);
    assert!(stored(&ctx, "seeded").is_some() && stored(&ctx, "index_version").is_some());

    assert_eq!(get(&ctx, "seeded").expect("get_setting"), None);
    assert_eq!(get(&ctx, "index_version").expect("get_setting"), None);
    set(&ctx, "seeded", "x").expect("set_setting");

    assert_eq!(stored(&ctx, "seeded").as_deref(), Some("1"));
    assert_eq!(stored(&ctx, "ui.seeded").as_deref(), Some("x"));
}

#[test]
fn a_setting_key_that_is_empty_too_long_or_holds_control_characters_is_a_bad_request_naming_the_key_but_not_the_value() {
    let dir = TempDir::new("settings-bad-key");
    let ctx = common::open_simple(&dir, None);
    let longest = "k".repeat(SETTING_KEY_MAX);
    assert_eq!(SETTING_KEY_MAX, 200);
    set(&ctx, &longest, "ok").expect("a key of the longest length is fine");

    for key in [String::new(), "k".repeat(SETTING_KEY_MAX + 1), "line\nbreak".to_string(), "bell\u{7}".to_string()] {
        let refused = set(&ctx, &key, "SECRET-VALUE");
        let Err(OpError::BadRequest(message)) = refused else { panic!("{key:?}: {refused:?}") };
        assert!(!message.contains("SECRET-VALUE"), "{message}");
        if !key.is_empty() {
            assert!(message.contains(&format!("{key:?}")), "the key, escaped, is named: {message}");
        }
        let refused = get(&ctx, &key);
        assert!(matches!(refused, Err(OpError::BadRequest(_))), "get {key:?}: {refused:?}");
    }
}

#[test]
fn log_frontend_writes_one_line_cut_to_the_limit_with_line_breaks_escaped() {
    let dir = TempDir::new("settings-log");
    let ctx = common::open_simple(&dir, None);
    let message = format!("first\nsecond\r\n[server] forged line{}", "x".repeat(10_000));

    ops::call(&ctx, &LOG_FRONTEND, &LogFrontendRequest { level: "error\n[server] level".to_string(), message }).expect("log_frontend");

    let log = std::fs::read_to_string(ctx.paths().logs.join(SERVER_LOG)).expect("the server log");
    let lines: Vec<&str> = log.lines().collect();
    assert_eq!(lines.len(), 1, "{log}");
    assert!(lines[0].contains(r"first\nsecond\r\n[server] forged line"), "{}", &lines[0][..200]);
    assert!(lines[0].chars().count() <= LOG_FRONTEND_MAX + 100, "{} characters", lines[0].chars().count());
    assert!(lines[0].chars().count() > LOG_FRONTEND_MAX, "cut at the limit, not shorter");
}

#[test]
fn library_errors_lists_unreadable_library_files_and_remove_imported_deletes_an_imported_session() {
    let dir = TempDir::new("settings-library");
    let ctx = common::open_simple(&dir, None);
    // An imported session: a bundle made from a session of another library.
    let exporter = common::open_simple(&dir.join("exporter"), None);
    common::session(&dir.join("exporter"), "s1", "Add a health check endpoint");
    common::reindex(&exporter);
    let file = dir.join(format!("s1.{}", bundle::EXTENSION));
    let conn = index::open(&exporter.paths().db).unwrap();
    let path = common::row(&exporter, "s1").path;
    bundle::write(&bundle::build(&conn, bundle::ExportRequest { path: &path, branch: None, subagents: None }, "test").unwrap(), &file).unwrap();
    let conn = index::open(&ctx.paths().db).unwrap();
    bundle::import(&conn, &ctx.paths().library, &file, None).unwrap();
    drop(conn);
    let broken = ctx.paths().library.join(format!("broken.{}", bundle::EXTENSION));
    std::fs::write(&broken, "not a bundle").unwrap();
    common::reindex(&ctx);

    let errors = ops::call(&ctx, &LIBRARY_ERRORS, &LibraryErrorsRequest {}).expect("library_errors");
    assert_eq!(errors.iter().map(|e| e.path.clone()).collect::<Vec<_>>(), [broken.to_string_lossy().into_owned()]);

    let imported = common::row(&ctx, "s1");
    assert!(imported.from_bundle);
    ops::call(&ctx, &REMOVE_IMPORTED, &RemoveImportedRequest { path: imported.path.clone() }).expect("remove_imported");

    assert!(!std::path::Path::new(&imported.path).exists());
    common::wait_until("the removed session leaves the list", || common::rows(&ctx).iter().all(|r| r.native_id != "s1"));
}
