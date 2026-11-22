// `list_sessions`: the indexed sessions, optionally narrowed to one repository and a start time.
#[path = "../../testutil/tempdir.rs"]
mod tempdir;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::json;
use session_player_core::index;
use session_player_core::model::SessionRow;
use session_player_core::ops::{self, Ctx, LIST_SESSIONS, ListSessionsRequest, OpError, OpenParams, Paths};
use tempdir::TempDir;

fn paths(dir: &Path) -> Paths {
    let data = dir.join("data");
    std::fs::create_dir_all(&data).unwrap();
    Paths { db: data.join("index.sqlite"), library: data.join("library"), logs: data.join("logs"), data }
}

/// A checkout of a repository: a folder with a `.git` folder, named like the repository.
fn checkout(dir: &Path, name: &str) -> PathBuf {
    let project = dir.join("code").join(name);
    std::fs::create_dir_all(project.join(".git")).unwrap();
    project
}

/// Writes a one-turn Claude Code session held in `cwd` on `day`.
fn claude_session(home: &Path, cwd: &Path, id: &str, day: &str) {
    let folder: String = cwd.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    let folder = home.join(".claude/projects").join(folder);
    std::fs::create_dir_all(&folder).unwrap();
    let user = json!({ "type": "user", "sessionId": id, "cwd": cwd, "timestamp": format!("{day}T10:00:00Z"),
                       "message": { "role": "user", "content": format!("Work on {id}") } });
    let assistant = json!({ "type": "assistant", "sessionId": id, "timestamp": format!("{day}T10:00:30Z"),
                            "message": { "id": "m1", "model": "claude-opus-5-5", "role": "assistant",
                                         "content": [{ "type": "text", "text": "Done." }] } });
    std::fs::write(folder.join(format!("{id}.jsonl")), format!("{user}\n{assistant}\n")).unwrap();
}

/// Sessions in two repositories on two days, indexed, and a Ctx on the same data folder.
fn open(dir: &Path) -> Arc<Ctx> {
    let home = dir.join("home");
    let widgets = checkout(dir, "widgets");
    let gadgets = checkout(dir, "gadgets");
    claude_session(&home, &widgets, "widgets-early", "2026-03-01");
    claude_session(&home, &widgets, "widgets-late", "2026-03-10");
    claude_session(&home, &gadgets, "gadgets-late", "2026-03-10");
    let paths = paths(dir);
    let mut conn = index::open(&paths.db).unwrap();
    index::add_root(&conn, &home.join(".claude/projects").to_string_lossy()).unwrap();
    index::run(&mut conn, false, None, &|_, _| {}).unwrap();
    drop(conn);
    Ctx::open(OpenParams {
        paths,
        home,
        version: "9.9.9".to_string(),
        claude: None,
        clock: Some(Arc::new(|| 7_000_000)),
        event_capacity: None,
        extensions: vec![],
    })
    .expect("open the Ctx")
}

fn list(ctx: &Arc<Ctx>, repo: Option<&str>, since: Option<&str>) -> Result<Vec<SessionRow>, OpError> {
    ops::call(ctx, &LIST_SESSIONS, &ListSessionsRequest { repo: repo.map(String::from), since: since.map(String::from) })
}

fn ids(rows: &[SessionRow]) -> BTreeSet<&str> {
    rows.iter().map(|r| r.native_id.as_str()).collect()
}

#[test]
fn list_sessions_with_no_filter_returns_what_the_index_lists() {
    let dir = TempDir::new("list-all");
    let ctx = open(&dir);

    let rows = list(&ctx, None, None).expect("list_sessions");

    let conn = index::open(&ctx.paths().db).unwrap();
    let indexed = index::list(&conn).unwrap();
    assert_eq!(indexed.len(), 3);
    assert_eq!(serde_json::to_value(&rows).unwrap(), serde_json::to_value(&indexed).unwrap());
}

#[test]
fn the_repo_filter_keeps_only_that_repositorys_sessions() {
    let dir = TempDir::new("list-repo");
    let ctx = open(&dir);

    let rows = list(&ctx, Some("widgets"), None).expect("list_sessions");

    assert_eq!(ids(&rows), BTreeSet::from(["widgets-early", "widgets-late"]));
}

#[test]
fn the_since_filter_keeps_sessions_started_on_or_after_it() {
    let dir = TempDir::new("list-since");
    let ctx = open(&dir);

    let rows = list(&ctx, None, Some("2026-03-05")).expect("list_sessions");

    assert_eq!(ids(&rows), BTreeSet::from(["widgets-late", "gadgets-late"]));
}

#[test]
fn the_repo_and_since_filters_combine() {
    let dir = TempDir::new("list-both");
    let ctx = open(&dir);

    let rows = list(&ctx, Some("widgets"), Some("2026-03-05")).expect("list_sessions");

    assert_eq!(ids(&rows), BTreeSet::from(["widgets-late"]));
}

#[test]
fn a_since_that_is_not_a_time_is_a_bad_request() {
    let dir = TempDir::new("list-bad-since");
    let ctx = open(&dir);

    let refused = list(&ctx, None, Some("whenever"));

    assert!(matches!(refused, Err(OpError::BadRequest(_))), "{refused:?}");
}
