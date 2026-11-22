// The client crate's `pull` operation, registered in an in-process Ctx, pulls a project from a
// server paired over pinned HTTPS (export_project, then ops::import_stream) into this data folder.
// A dry run reports without writing; a second pull reports the sessions unchanged; a wrong pin is
// refused before anything is read; an unknown server name says to run `sp connect`.
#[path = "../../testutil/tempdir.rs"]
mod tempdir;
mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use common::{CtxParams, OpenWith, SessionFile, checkout, external_config, local_client, open_ctx, open_ctx_with, reindex, rows, session_file, start};
use session_player_client::{ConnectParams, PULL, PairedServer, PullRequest, PullResponse, ServersStore, connect};
use session_player_core::ops::{self, Ctx, Input, OpError, Output};
use session_player_server::ServerHandle;
use session_player_server::pairing::{PairRequest, PairResponse};
use tempdir::TempDir;

struct Setup {
    server: ServerHandle,
    puller: Arc<Ctx>,
    /// The puller's checkout of "widget".
    project: PathBuf,
    paired: PairedServer,
}

/// An exporting server with two sessions of "widget" and an external TLS listener, and a puller
/// whose servers.json holds it as "lab".
fn setup(dir: &Path) -> Setup {
    let exporter_dir = dir.join("exporter");
    let exporter = open_ctx(CtxParams { dir: &exporter_dir, clock: None, event_capacity: None, fixtures: true });
    let home = exporter_dir.join("home");
    let widget = checkout(&home, "widget");
    session_file(SessionFile { home: &home, cwd: &widget, id: "p1", prompt: "Add a health check endpoint", images: &[] });
    session_file(SessionFile { home: &home, cwd: &widget, id: "p2", prompt: "Make the health check report the database", images: &[] });
    reindex(&exporter);
    let server = start(&exporter, external_config());
    let offer: PairResponse = local_client(&server, &exporter.paths().data).call_named("pair", &PairRequest {}).expect("pair");
    let paired = connect(ConnectParams { pairing: &offer.pairing, name: "lab" }).expect("connect");
    assert!(paired.fingerprint.is_some(), "the pairing is pinned TLS");

    let puller_dir = dir.join("puller");
    let puller = open_ctx_with(CtxParams { dir: &puller_dir, clock: None, event_capacity: None, fixtures: false }, OpenWith {
        upload_cap: None,
        extensions: vec![PULL],
    });
    ServersStore::new(&puller.paths().data).save(paired.clone()).expect("save the server");
    let project = checkout(&puller_dir.join("home"), "widget");
    Setup { server, puller, project, paired }
}

fn pull(ctx: &Arc<Ctx>, req: PullRequest) -> Result<PullResponse, OpError> {
    match ops::call_raw(ctx, "pull", Input::Json(serde_json::to_value(req).unwrap()))? {
        Output::Json(value) => Ok(serde_json::from_value(value).expect("a PullResponse")),
        Output::Raw(_) => panic!("pull answered with a raw body"),
    }
}

fn request(server: &str, project: &Path, dry_run: bool) -> PullRequest {
    PullRequest { server: server.to_string(), project: project.to_string_lossy().into_owned(), since: None, dry_run }
}

fn widget_ids(ctx: &Arc<Ctx>) -> Vec<String> {
    let mut ids: Vec<String> = rows(ctx).into_iter().map(|r| r.native_id).collect();
    ids.sort();
    ids
}

#[test]
fn a_pull_over_paired_https_imports_the_projects_sessions_and_a_second_pull_finds_them_unchanged() {
    let dir = TempDir::new("server-pull");
    let s = setup(&dir);

    let first = pull(&s.puller, request("lab", &s.project, false)).expect("the first pull");

    assert!(!first.dry_run);
    assert_eq!(first.sessions.added, 2, "{first:?}");
    assert_eq!(widget_ids(&s.puller), vec!["p1".to_string(), "p2".to_string()], "listed when pull returns");

    let second = pull(&s.puller, request("lab", &s.project, false)).expect("the second pull");
    assert_eq!(second.sessions.unchanged, 2, "{second:?}");
    assert_eq!(second.sessions.added, 0, "{second:?}");
    s.server.shutdown();
}

#[test]
fn a_dry_run_pull_reports_what_it_would_add_and_writes_nothing() {
    let dir = TempDir::new("server-pull-dry");
    let s = setup(&dir);
    let home = dir.join("puller/home");
    let before = common::files_under(&home);

    let dry = pull(&s.puller, request("lab", &s.project, true)).expect("a dry run");

    assert!(dry.dry_run);
    assert_eq!(dry.sessions.added, 2, "{dry:?}");
    assert!(dry.files.iter().filter(|f| f.is_session).count() >= 2, "{dry:?}");
    assert_eq!(common::files_under(&home), before, "a dry run writes no session files");
    assert!(rows(&s.puller).is_empty(), "a dry run indexes nothing");
    s.server.shutdown();
}

#[test]
fn a_pull_from_a_server_whose_certificate_does_not_match_the_pin_is_refused() {
    let dir = TempDir::new("server-pull-pin");
    let s = setup(&dir);
    let wrong = PairedServer { name: "impostor".to_string(), fingerprint: Some("00".repeat(32)), ..s.paired.clone() };
    ServersStore::new(&s.puller.paths().data).save(wrong).expect("save the server");

    let refused = pull(&s.puller, request("impostor", &s.project, false));

    assert!(refused.is_err(), "{refused:?}");
    assert!(rows(&s.puller).is_empty(), "nothing was imported");
    s.server.shutdown();
}

#[test]
fn a_pull_from_an_unknown_server_name_says_to_run_sp_connect() {
    let dir = TempDir::new("server-pull-unknown");
    let s = setup(&dir);

    let refused = pull(&s.puller, request("nowhere", &s.project, false));

    let message = format!("{:?}", refused.expect_err("an unknown server name is an error"));
    assert!(message.contains("nowhere"), "{message}");
    assert!(message.contains("sp connect"), "{message}");
    s.server.shutdown();
}
