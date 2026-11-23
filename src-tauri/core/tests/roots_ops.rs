// The session folders: listed, suggested, added and removed, with their sessions following, and
// full-text search over what is indexed.
#[path = "../../testutil/tempdir.rs"]
mod tempdir;
#[path = "common/ops.rs"]
mod common;

use std::path::Path;
use std::sync::Arc;

use session_player_core::ops::{
    self, ADD_ROOT, AddRootRequest, Ctx, GET_ROOTS, GetRootsRequest, REMOVE_ROOT, RemoveRootRequest, RootInfo, SEARCH, SUGGESTED_ROOTS, SearchRequest,
    SuggestedRootsRequest,
};
use session_player_core::test_support::index;
use tempdir::TempDir;

fn roots(ctx: &Arc<Ctx>) -> Vec<RootInfo> {
    ops::call(ctx, &GET_ROOTS, &GetRootsRequest {}).expect("get_roots")
}

fn search(ctx: &Arc<Ctx>, query: &str) -> Vec<String> {
    ops::call(ctx, &SEARCH, &SearchRequest { query: query.to_string() }).expect("search").into_iter().map(|h| h.path).collect()
}

fn listed(ctx: &Arc<Ctx>) -> Vec<String> {
    let mut ids: Vec<String> = common::rows(ctx).into_iter().map(|r| r.native_id).collect();
    ids.sort();
    ids
}

/// A session folder at `base/.claude/projects` holding one session.
fn folder_with_session(base: &Path, id: &str, prompt: &str) -> String {
    let cwd = base.join("code/app");
    common::session_file(common::SessionFile { home: base, cwd: &cwd, id, prompt, images: &[], linked_images: &[] });
    base.join(".claude/projects").to_string_lossy().into_owned()
}

#[test]
fn registered_roots_can_be_listed_added_and_removed_and_their_sessions_follow_and_can_be_searched() {
    let dir = TempDir::new("roots-ops");
    let ctx = common::open(common::Open { dir: &dir, claude: None, upload_cap: None, register_claude_root: false });
    let first = folder_with_session(&dir.join("first"), "alpha", "Teach the zebra parser new tricks");
    let second = folder_with_session(&dir.join("second"), "beta", "Feed the giraffe service");
    assert_eq!(roots(&ctx), Vec::<RootInfo>::new());

    let added = ops::call(&ctx, &ADD_ROOT, &AddRootRequest { path: first.clone() }).expect("add_root");

    assert_eq!((added.path.as_str(), added.exists), (first.as_str(), true));
    // Adding a folder starts an index pass that reads it.
    common::wait_until("alpha is listed", || listed(&ctx) == ["alpha"]);
    assert_eq!(roots(&ctx), [RootInfo { path: first.clone(), exists: true, sessions: 1 }]);
    let alpha_path = common::row(&ctx, "alpha").path;
    assert_eq!(search(&ctx, "zebra"), [alpha_path]);

    ops::call(&ctx, &ADD_ROOT, &AddRootRequest { path: second.clone() }).expect("add_root");
    common::wait_until("beta is listed", || listed(&ctx) == ["alpha", "beta"]);
    assert_eq!(roots(&ctx).iter().map(|r| r.path.clone()).collect::<Vec<_>>(), [first.clone(), second.clone()]);

    ops::call(&ctx, &REMOVE_ROOT, &RemoveRootRequest { path: first }).expect("remove_root");

    assert_eq!(listed(&ctx), ["beta"]);
    assert_eq!(search(&ctx, "zebra"), Vec::<String>::new());
    assert_eq!(search(&ctx, "giraffe").len(), 1);
    assert_eq!(roots(&ctx), [RootInfo { path: second, exists: true, sessions: 1 }]);
}

#[test]
fn a_registered_folder_that_no_longer_exists_is_listed_as_missing() {
    let dir = TempDir::new("roots-missing");
    let ctx = common::open(common::Open { dir: &dir, claude: None, upload_cap: None, register_claude_root: false });
    let gone = dir.join("gone").to_string_lossy().into_owned();
    let conn = index::open(&ctx.paths().db).unwrap();
    index::add_root(&conn, &gone).unwrap();

    assert_eq!(roots(&ctx), [RootInfo { path: gone, exists: false, sessions: 0 }]);
}

#[test]
fn suggested_roots_are_the_agent_folders_under_the_servers_home_not_yet_registered() {
    let dir = TempDir::new("roots-suggested");
    let ctx = common::open(common::Open { dir: &dir, claude: None, upload_cap: None, register_claude_root: true });
    let home = common::home(&dir);
    for folder in [".codex/sessions", ".pi/agent/sessions"] {
        std::fs::create_dir_all(home.join(folder)).unwrap();
    }

    let suggested = ops::call(&ctx, &SUGGESTED_ROOTS, &SuggestedRootsRequest {}).expect("suggested_roots");

    let expected: Vec<String> = [".codex/sessions", ".pi/agent/sessions"].iter().map(|f| home.join(f).to_string_lossy().into_owned()).collect();
    assert_eq!(suggested, expected, "the registered Claude Code folder and the missing archive folder are left out");
}
