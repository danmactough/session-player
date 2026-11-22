// While `serve` runs, a file watcher on the session roots (raw notify, Access events ignored,
// a 400 ms debounce) starts an index pass when a session file appears, so it is listed within
// about two seconds. An idle server runs no index passes, including after a triggered one.
#[path = "../../testutil/tempdir.rs"]
mod tempdir;
mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{CtxParams, SessionFile, loopback_config, open_ctx, rows, session_file, start, wait_until};
use session_player_core::ops::{Ctx, EventsSince, PushedEvent};
use tempdir::TempDir;

/// The newest event id, to read later events after.
fn cursor(ctx: &Ctx) -> u64 {
    ctx.events_since(EventsSince { after: Some(u64::MAX), wait: Duration::ZERO })[0].id
}

fn index_events(ctx: &Ctx, after: u64) -> Vec<PushedEvent> {
    ctx.events_since(EventsSince { after: Some(after), wait: Duration::ZERO })
        .into_iter()
        .map(|e| e.event)
        .filter(|e| matches!(e, PushedEvent::IndexStarted | PushedEvent::IndexProgress(_) | PushedEvent::IndexDone(_) | PushedEvent::IndexError(_)))
        .collect()
}

fn listed(ctx: &Arc<Ctx>, native_id: &str) -> bool {
    rows(ctx).iter().any(|r| r.native_id == native_id)
}

fn drop_session(dir: &TempDir, id: &str) {
    let home = dir.join("home");
    session_file(SessionFile { home: &home, cwd: &home.join("code/watched"), id, prompt: "A session that appeared while serving", images: &[] });
}

#[test]
fn a_new_session_file_in_a_root_is_listed_within_two_seconds_with_index_events() {
    let dir = TempDir::new("server-watcher-new");
    let ctx = open_ctx(CtxParams { dir: &dir, clock: None, event_capacity: None, fixtures: true });
    let before = cursor(&ctx);
    let server = start(&ctx, loopback_config());
    std::thread::sleep(Duration::from_millis(200));

    drop_session(&dir, "watched1");

    wait_until("the new session is listed", Duration::from_secs(2), || listed(&ctx, "watched1"));
    let events = index_events(&ctx, before);
    assert!(events.iter().any(|e| matches!(e, PushedEvent::IndexStarted)), "{events:?}");
    wait_until("the pass ends", Duration::from_secs(5), || index_events(&ctx, before).iter().any(|e| matches!(e, PushedEvent::IndexDone(_))));
    server.shutdown();
}

#[test]
fn an_idle_server_runs_no_index_passes() {
    let dir = TempDir::new("server-watcher-idle");
    let ctx = open_ctx(CtxParams { dir: &dir, clock: None, event_capacity: None, fixtures: true });
    let before = cursor(&ctx);
    let server = start(&ctx, loopback_config());

    std::thread::sleep(Duration::from_secs(2));

    let events = index_events(&ctx, before);
    assert!(events.is_empty(), "{events:?}");
    server.shutdown();
}

#[test]
fn after_a_triggered_pass_the_server_stays_idle() {
    let dir = TempDir::new("server-watcher-settle");
    let ctx = open_ctx(CtxParams { dir: &dir, clock: None, event_capacity: None, fixtures: true });
    let before = cursor(&ctx);
    let server = start(&ctx, loopback_config());
    std::thread::sleep(Duration::from_millis(200));
    drop_session(&dir, "watched2");
    wait_until("the new session is listed", Duration::from_secs(2), || listed(&ctx, "watched2"));
    wait_until("the pass ends", Duration::from_secs(5), || index_events(&ctx, before).iter().any(|e| matches!(e, PushedEvent::IndexDone(_))));
    // Past the debounce, so a change the pass itself caused would have started another one.
    std::thread::sleep(Duration::from_millis(600));
    let settled = cursor(&ctx);

    std::thread::sleep(Duration::from_secs(2));

    let events = index_events(&ctx, settled);
    assert!(events.is_empty(), "the index pass fed the watcher: {events:?}");
    server.shutdown();
}
