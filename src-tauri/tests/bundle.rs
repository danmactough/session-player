// Export/import round trips through a real index, with the original session
// files deleted before the recipient replays them.
use std::path::{Path, PathBuf};

use session_player_lib::test_support::bundle::{self, ExportRequest};
use session_player_lib::test_support::index;

/// A sender's machine: agent folders laid out the way each agent writes them,
/// indexed into their own database.
struct Sender {
    base: PathBuf,
    root: PathBuf,
    conn: rusqlite::Connection,
}

impl Sender {
    fn new(name: &str) -> Sender {
        let base = std::env::temp_dir().join(format!("sp-bundle-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("agents");
        let fx = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let claude = root.join("claude/-work-repo");
        std::fs::create_dir_all(claude.join("abc/subagents")).unwrap();
        std::fs::copy(fx.join("claude.jsonl"), claude.join("abc.jsonl")).unwrap();
        std::fs::copy(fx.join("claude-subagent.jsonl"), claude.join("abc/subagents/agent-a1.jsonl")).unwrap();
        std::fs::copy(fx.join("claude-attach.jsonl"), claude.join("att.jsonl")).unwrap();
        std::fs::copy(fx.join("claude-ide.jsonl"), claude.join("ide.jsonl")).unwrap();
        let pi = root.join("pi/--work-pi--");
        std::fs::create_dir_all(&pi).unwrap();
        std::fs::copy(fx.join("pi.jsonl"), pi.join("2026-01-01_01a-pi.jsonl")).unwrap();
        std::fs::create_dir_all(root.join("codex")).unwrap();
        std::fs::copy(fx.join("codex.jsonl"), root.join("codex/rollout-2026-01-02-019d-codex.jsonl")).unwrap();
        let mut conn = index::open(&base.join("index.sqlite")).unwrap();
        index::add_root(&conn, &root.to_string_lossy()).unwrap();
        index::run(&mut conn, false, None, &|_, _| {}).unwrap();
        Sender { base, root, conn }
    }

    fn path(&self, rel: &str) -> String {
        self.root.join(rel).to_string_lossy().into_owned()
    }

    fn export(&self, rel: &str, branch: Option<&str>, subagents: Option<Vec<String>>) -> bundle::Bundle {
        let path = self.path(rel);
        bundle::build(&self.conn, ExportRequest { path: &path, branch, subagents }, "Session Player test").unwrap()
    }

    fn save(&self, b: &bundle::Bundle, name: &str) -> PathBuf {
        let out = self.base.join(name);
        bundle::write(b, &out).unwrap();
        out
    }
}

impl Drop for Sender {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

/// A recipient with an empty library and index of its own.
struct Recipient {
    base: PathBuf,
    library: PathBuf,
    conn: rusqlite::Connection,
}

impl Recipient {
    fn new(name: &str) -> Recipient {
        let base = std::env::temp_dir().join(format!("sp-recipient-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let conn = index::open(&base.join("index.sqlite")).unwrap();
        Recipient { library: base.join("library"), base, conn }
    }

    fn reindex(&mut self) -> Vec<session_player_lib::test_support::Row> {
        index::run(&mut self.conn, false, Some(&self.library), &|_, _| {}).unwrap();
        index::list(&self.conn).unwrap()
    }
}

impl Drop for Recipient {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

#[test]
fn claude_session_replays_from_the_library_without_its_source_files() {
    let sender = Sender::new("tracer");
    let exported = sender.export("claude/-work-repo/abc.jsonl", None, Some(vec![]));
    let file = sender.save(&exported, "abc.spsession");
    std::fs::remove_dir_all(&sender.root).unwrap();

    let mut recipient = Recipient::new("tracer");
    let report = bundle::import(&recipient.conn, &recipient.library, &file, None).unwrap();
    assert_eq!(report.imported, vec!["claude:abc".to_string()]);
    let rows = recipient.reindex();
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert!(row.from_bundle);
    assert_eq!(row.title, "Flaky login test");
    assert_eq!(row.project, "/work/repo");
    assert_eq!((row.user_turns, row.tool_calls, row.exit_count), (1, 1, 1));

    let events = bundle::load_events(Path::new(&row.path)).unwrap();
    assert_eq!(events, exported.sessions[0].events);
    let anchors: Vec<_> = events.iter().map(|e| e.anchor.clone().unwrap()).collect();
    assert_eq!(anchors, ["e0", "e1", "e2", "e3", "e4"]);
    assert_eq!(events[1].text, "Fix the flaky login test please");
    assert_eq!(events[1].ts, Some(1767261602000));
}

#[test]
fn subagents_and_their_screenshots_travel_with_the_session() {
    let sender = Sender::new("subagents");
    let exported = sender.export("claude/-work-repo/abc.jsonl", None, None);
    let ids: Vec<_> = exported.sessions.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, ["claude:abc", "claude:agent-a1"]);
    let shot = &exported.attachments[0];
    assert_eq!((shot.session.as_str(), shot.kind), ("claude:agent-a1", bundle::AttachmentKind::Image));
    assert_eq!(shot.status, bundle::AttachmentStatus::Included);
    assert_eq!(shot.mime.as_deref(), Some("image/png"));
    assert_eq!(shot.bytes, Some(5));
    // The transcript points at the attachment instead of carrying the image.
    let anchored = exported.sessions[1].events.iter().find(|e| e.anchor.as_deref() == Some(shot.anchor.as_str())).unwrap();
    assert_eq!(anchored.images, vec![format!("attachment:{}", shot.id)]);

    let file = sender.save(&exported, "abc.spsession");
    std::fs::remove_dir_all(&sender.root).unwrap();
    let mut recipient = Recipient::new("subagents");
    bundle::import(&recipient.conn, &recipient.library, &file, None).unwrap();
    let rows = recipient.reindex();
    let sub = rows.iter().find(|r| r.is_subagent).expect("subagent row");
    assert_eq!(sub.parent_id.as_deref(), Some("abc"));
    let events = bundle::load_events(Path::new(&sub.path)).unwrap();
    let with_image = events.iter().find(|e| e.anchor.as_deref() == Some(shot.anchor.as_str())).unwrap();
    assert_eq!(with_image.images, vec!["data:image/png;base64,aGVsbG8=".to_string()]);
    assert_eq!(with_image.ts, Some(1767261606000));
}

#[test]
fn excluded_subagents_are_listed_not_included() {
    let sender = Sender::new("exclude");
    let exported = sender.export("claude/-work-repo/abc.jsonl", None, Some(vec![]));
    assert_eq!(exported.sessions.len(), 1);
    assert!(exported.attachments.is_empty());
    let omitted = &exported.sessions[0].omitted;
    assert_eq!(omitted.len(), 1);
    assert_eq!((omitted[0].relation, omitted[0].id.as_str()), (bundle::Relation::Subagent, "claude:agent-a1"));
}

#[test]
fn exporting_a_subagent_leaves_its_parent_out() {
    let sender = Sender::new("child");
    let exported = sender.export("claude/-work-repo/abc/subagents/agent-a1.jsonl", None, None);
    assert_eq!(exported.sessions.len(), 1);
    let omitted = &exported.sessions[0].omitted;
    assert_eq!((omitted[0].relation, omitted[0].id.as_str()), (bundle::Relation::Parent, "claude:abc"));
    assert_eq!(omitted[0].title, "Flaky login test");
}

#[test]
fn a_pi_branch_exports_its_whole_path_and_lists_the_others() {
    let sender = Sender::new("pi");
    let branch = sender.export("pi/--work-pi--/2026-01-01_01a-pi.jsonl", Some("a3"), None);
    let s = &branch.sessions[0];
    assert_eq!(s.id, "pi:01a-pi@a3");
    assert_eq!(s.title, "Retry: stress test my caching plan");
    // The shared history before the split comes along so the branch reads on its own.
    let texts: Vec<_> = s.events.iter().map(|e| e.text.as_str()).collect();
    assert_eq!(texts, ["/skill:grill-me", "stress test my caching plan", "abandoned branch"]);
    let omitted: Vec<_> = s.omitted.iter().map(|r| (r.relation, r.id.as_str(), r.title.as_str())).collect();
    assert_eq!(omitted, [(bundle::Relation::Branch, "pi:01a-pi", "Caching plan review")]);

    let active = sender.export("pi/--work-pi--/2026-01-01_01a-pi.jsonl", None, None);
    let omitted: Vec<_> = active.sessions[0].omitted.iter().map(|r| (r.relation, r.id.as_str())).collect();
    assert_eq!(omitted, [(bundle::Relation::Branch, "pi:01a-pi@a3")]);

    let file = sender.save(&branch, "branch.spsession");
    std::fs::remove_dir_all(&sender.root).unwrap();
    let mut recipient = Recipient::new("pi");
    bundle::import(&recipient.conn, &recipient.library, &file, None).unwrap();
    let rows = recipient.reindex();
    assert_eq!(rows[0].title, "Retry: stress test my caching plan");
    // Its notes are kept apart from those of an imported copy of the active branch.
    assert_eq!(session_player_lib::test_support::annotations::session_key(&rows[0]), "bundle:pi:01a-pi@a3");
    // Other branches stay with the sender.
    assert!(rows[0].branches.is_empty());
    assert_eq!(bundle::load_events(Path::new(&rows[0].path)).unwrap(), s.events);
}

#[test]
fn codex_session_round_trips() {
    let sender = Sender::new("codex");
    let exported = sender.export("codex/rollout-2026-01-02-019d-codex.jsonl", None, None);
    assert_eq!(exported.sessions[0].id, "codex:019d-codex");
    assert_eq!(exported.sessions[0].source_file, "rollout-2026-01-02-019d-codex.jsonl");
    let file = sender.save(&exported, "codex.spsession");
    std::fs::remove_dir_all(&sender.root).unwrap();
    let mut recipient = Recipient::new("codex");
    bundle::import(&recipient.conn, &recipient.library, &file, None).unwrap();
    let rows = recipient.reindex();
    let row = &rows[0];
    assert_eq!((row.source.as_str(), row.native_id.as_str()), ("codex", "019d-codex"));
    assert_eq!(row.title, "add retry support to the uploader");
    assert_eq!((row.input_tokens, row.cache_read_tokens, row.exit_count), (200, 800, 1));
    assert_eq!(bundle::load_events(Path::new(&row.path)).unwrap(), exported.sessions[0].events);
}

#[test]
fn file_cards_are_listed_with_whether_their_contents_came_along() {
    let sender = Sender::new("files");
    let attached = sender.export("claude/-work-repo/att.jsonl", None, None);
    let a = &attached.attachments[0];
    assert_eq!((a.kind, a.status, a.name.as_deref()), (bundle::AttachmentKind::File, bundle::AttachmentStatus::Included, Some("/tmp/handoff.md")));
    assert_eq!(a.bytes, Some("# Handoff\n- step one".len() as u64));
    assert_eq!(a.data, None, "file contents stay in the prompt text");
    let anchored = attached.sessions[0].events.iter().find(|e| e.anchor.as_deref() == Some(a.anchor.as_str())).unwrap();
    assert!(anchored.text.contains("# Handoff"));

    let ide = sender.export("claude/-work-repo/ide.jsonl", None, None);
    let a = &ide.attachments[0];
    assert_eq!((a.kind, a.status, a.name.as_deref()), (bundle::AttachmentKind::File, bundle::AttachmentStatus::Omitted, Some("/work/settings.json")));
    assert!(a.reason.as_deref().unwrap().contains("only the path"), "{:?}", a.reason);
}

/// Writes `edit` applied to a valid two-session bundle (with a screenshot)
/// and returns what importing it says.
fn import_edited(recipient: &Recipient, name: &str, edit: impl FnOnce(&mut serde_json::Value)) -> Result<bundle::ImportReport, String> {
    let sender = Sender::new(&format!("edit-{name}"));
    let mut v = serde_json::to_value(sender.export("claude/-work-repo/abc.jsonl", None, None)).unwrap();
    edit(&mut v);
    let file = recipient.base.join(format!("{name}.spsession"));
    std::fs::write(&file, serde_json::to_vec(&v).unwrap()).unwrap();
    bundle::import(&recipient.conn, &recipient.library, &file, None).map_err(|e| format!("{e:#}"))
}

#[test]
fn invalid_bundles_are_rejected_with_a_reason_and_nothing_is_imported() {
    let recipient = Recipient::new("invalid");
    let cases: Vec<(&str, Box<dyn FnOnce(&mut serde_json::Value)>, &str)> = vec![
        ("format", Box::new(|v| v["format"] = "something-else".into()), "isn't a Session Player session bundle"),
        ("newer", Box::new(|v| v["version"] = 2.into()), "newer version of Session Player"),
        ("unversioned", Box::new(|v| v.as_object_mut().unwrap().remove("version").map(|_| ()).unwrap()), "no version"),
        ("empty", Box::new(|v| v["sessions"] = serde_json::json!([])), "no sessions"),
        ("missing-field", Box::new(|v| v["sessions"][0].as_object_mut().unwrap().remove("events").map(|_| ()).unwrap()), "missing field `events`"),
        ("duplicate", Box::new(|v| {
            v["sessions"][1]["id"] = "claude:abc".into();
            v["sessions"][1]["native_id"] = "abc".into();
        }), "claude:abc appears more than once"),
        ("stranger", Box::new(|v| v["sessions"][1]["parent_id"] = "zzz".into()), "isn't a subagent of"),
        ("dangling", Box::new(|v| v["attachments"] = serde_json::json!([])), "missing attachment a0"),
        ("orphan", Box::new(|v| v["attachments"][0]["session"] = "claude:nope".into()), "attachment a0 belongs to session claude:nope"),
        ("no-data", Box::new(|v| v["attachments"][0]["data"] = serde_json::Value::Null), "attachment a0 is marked included but has no data"),
        ("bad-data", Box::new(|v| v["attachments"][0]["data"] = "not base64!".into()), "attachment a0 isn't valid base64"),
        ("anchors", Box::new(|v| v["sessions"][0]["events"][1]["anchor"] = "e0".into()), "anchor e0 appears more than once"),
        ("mime", Box::new(|v| v["attachments"][0]["mime"] = "image/png\nX-Evil: 1".into()), "attachment a0 has unsupported image type"),
        ("id", Box::new(|v| v["sessions"][0]["id"] = "".into()), "session id \"\" doesn't match claude:abc"),
    ];
    for (name, edit, expected) in cases {
        let err = import_edited(&recipient, name, edit).expect_err(name);
        assert!(err.contains(expected), "{name}: {err}");
    }
    let garbage = recipient.base.join("garbage.spsession");
    std::fs::write(&garbage, "{ not json").unwrap();
    let err = format!("{:#}", bundle::import(&recipient.conn, &recipient.library, &garbage, None).unwrap_err());
    assert!(err.contains("isn't a Session Player session bundle"), "{err}");
    assert!(!recipient.library.exists() || std::fs::read_dir(&recipient.library).unwrap().next().is_none());
}

#[test]
fn session_ids_cannot_place_files_outside_the_library() {
    let recipient = Recipient::new("paths");
    let report = import_edited(&recipient, "escape", |v| {
        v["sessions"] = serde_json::json!([v["sessions"][0].clone()]);
        v["sessions"][0]["id"] = "claude:../../escaped".into();
        v["sessions"][0]["native_id"] = "../../escaped".into();
        v["attachments"] = serde_json::json!([]);
    })
    .unwrap();
    assert_eq!(report.imported, vec!["claude:../../escaped".to_string()]);
    let files: Vec<_> = std::fs::read_dir(&recipient.library).unwrap().map(|e| e.unwrap().path()).collect();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].parent().unwrap(), recipient.library);
}

#[test]
fn re_importing_resolves_duplicates_by_the_chosen_policy() {
    use bundle::OnConflict;
    let sender = Sender::new("conflict");
    let first = sender.save(&sender.export("claude/-work-repo/abc.jsonl", None, None), "first.spsession");
    let mut changed = sender.export("claude/-work-repo/abc.jsonl", None, None);
    changed.sessions[0].title = "Renamed by the sender".into();
    let second = sender.save(&changed, "second.spsession");

    let mut recipient = Recipient::new("conflict");
    bundle::import(&recipient.conn, &recipient.library, &first, None).unwrap();

    // The same content again is not a conflict.
    let again = bundle::import(&recipient.conn, &recipient.library, &first, None).unwrap();
    assert!(again.imported.is_empty());
    assert_eq!(again.unchanged, vec!["claude:abc".to_string(), "claude:agent-a1".to_string()]);

    // A different version of a session already in the library waits for a decision, and nothing is written.
    let asked = bundle::import(&recipient.conn, &recipient.library, &second, None).unwrap();
    assert!(asked.imported.is_empty());
    assert_eq!(asked.conflicts.len(), 1);
    assert_eq!((asked.conflicts[0].id.as_str(), asked.conflicts[0].existing_title.as_str(), asked.conflicts[0].incoming_title.as_str()), ("claude:abc", "Flaky login test", "Renamed by the sender"));
    assert!(recipient.reindex().iter().any(|r| r.title == "Flaky login test"));

    let kept = bundle::import(&recipient.conn, &recipient.library, &second, Some(OnConflict::Keep)).unwrap();
    assert_eq!(kept.kept, vec!["claude:abc".to_string()]);
    assert!(recipient.reindex().iter().any(|r| r.title == "Flaky login test"));

    let replaced = bundle::import(&recipient.conn, &recipient.library, &second, Some(OnConflict::Replace)).unwrap();
    assert_eq!(replaced.imported, vec!["claude:abc".to_string()]);
    assert_eq!(replaced.unchanged, vec!["claude:agent-a1".to_string()]);
    let rows = recipient.reindex();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().any(|r| r.title == "Renamed by the sender"));
}

#[test]
fn an_imported_session_can_be_exported_again_unchanged() {
    let sender = Sender::new("forward");
    let original = sender.export("claude/-work-repo/abc.jsonl", None, None);
    let file = sender.save(&original, "abc.spsession");
    let mut recipient = Recipient::new("forward");
    bundle::import(&recipient.conn, &recipient.library, &file, None).unwrap();
    let rows = recipient.reindex();
    let primary = rows.iter().find(|r| !r.is_subagent).unwrap();
    let request = bundle::ExportRequest { path: &primary.path, branch: None, subagents: None };
    let forwarded = bundle::build(&recipient.conn, request, "Session Player test").unwrap();
    assert_eq!(forwarded.sessions, original.sessions);
    assert_eq!(forwarded.attachments, original.attachments);
}

#[test]
fn the_preview_describes_exactly_what_gets_saved() {
    let sender = Sender::new("preview");
    let exported = sender.export("claude/-work-repo/abc.jsonl", None, None);
    let preview = bundle::summarize(&exported);
    let file = sender.save(&exported, "abc.spsession");
    assert_eq!(preview.bytes, std::fs::metadata(&file).unwrap().len());
    let sessions: Vec<_> = preview.sessions.iter().map(|s| (s.id.as_str(), s.prompts, s.events)).collect();
    assert_eq!(sessions, [("claude:abc", 1, 5), ("claude:agent-a1", 1, 5)]);
    assert_eq!(preview.attachments.len(), 1);
    assert_eq!(preview.attachments[0].data, None, "the preview lists attachments without their contents");
    assert_eq!(preview.file_name, "Flaky login test.spsession");
}

#[test]
fn a_subagent_imported_alone_matches_the_same_subagent_in_its_parents_bundle() {
    let mut sender = Sender::new("renumber");
    // Give the parent a screenshot of its own, ahead of the subagent's.
    let parent = sender.root.join("claude/-work-repo/abc.jsonl");
    let mut log = std::fs::read_to_string(&parent).unwrap();
    log.push_str(r#"{"type":"user","sessionId":"abc","timestamp":"2026-01-01T10:00:05Z","message":{"role":"user","content":[{"type":"text","text":"and this one"},{"type":"image","source":{"type":"base64","media_type":"image/png","data":"d29ybGQ="}}]}}"#);
    log.push('\n');
    std::fs::write(&parent, log).unwrap();
    index::run(&mut sender.conn, false, None, &|_, _| {}).unwrap();
    let alone = sender.save(&sender.export("claude/-work-repo/abc/subagents/agent-a1.jsonl", None, None), "alone.spsession");
    let family = sender.save(&sender.export("claude/-work-repo/abc.jsonl", None, None), "family.spsession");
    let recipient = Recipient::new("renumber");
    bundle::import(&recipient.conn, &recipient.library, &alone, None).unwrap();
    let report = bundle::import(&recipient.conn, &recipient.library, &family, None).unwrap();
    assert!(report.conflicts.is_empty(), "{:?}", report.conflicts);
    assert_eq!(report.unchanged, vec!["claude:agent-a1".to_string()]);
}

#[test]
fn a_pi_fork_file_exports_with_the_history_it_copied() {
    let mut sender = Sender::new("pifork");
    let dir = sender.root.join("pi/--work-tree--");
    std::fs::create_dir_all(&dir).unwrap();
    let parent = dir.join("2026-02-01T10-00-00-000Z_01b-tree.jsonl");
    std::fs::copy(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pi-tree.jsonl"), &parent).unwrap();
    let lines: Vec<String> = std::fs::read_to_string(&parent).unwrap().lines().map(str::to_string).collect();
    std::fs::write(
        dir.join("2026-02-02T10-00-00-000Z_01c-fork.jsonl"),
        [
            format!(r#"{{"type":"session","version":3,"id":"01c-fork","timestamp":"2026-02-02T10:00:00.000Z","cwd":"/work/tree","parentSession":"{}"}}"#, parent.display()),
            lines[1].clone(),
            lines[2].clone(),
            r#"{"type":"message","id":"f1","parentId":"m2","timestamp":"2026-02-02T10:00:01.000Z","message":{"role":"user","content":"forked question about caching"}}"#.into(),
        ]
        .join("\n"),
    )
    .unwrap();
    index::run(&mut sender.conn, false, None, &|_, _| {}).unwrap();
    let exported = sender.export("pi/--work-tree--/2026-02-02T10-00-00-000Z_01c-fork.jsonl", None, None);
    let texts: Vec<_> = exported.sessions[0].events.iter().map(|e| e.text.as_str()).collect();
    assert_eq!(texts, ["first question here please", "answer one", "forked question about caching"]);
    assert_eq!(exported.sessions[0].omitted[0].relation, bundle::Relation::Parent);
}

#[test]
fn sessions_with_no_conversation_are_not_imported() {
    let recipient = Recipient::new("empty");
    let report = import_edited(&recipient, "empty", |v| {
        // Keep only the slash command: no prompt, reply or tool call.
        let events = v["sessions"][1]["events"].as_array_mut().unwrap();
        events.retain(|e| e["kind"] == "system");
        v["attachments"] = serde_json::json!([]);
    })
    .unwrap();
    assert_eq!(report.imported, vec!["claude:abc".to_string()]);
    assert_eq!(report.empty, vec!["claude:agent-a1".to_string()]);
    assert_eq!(std::fs::read_dir(&recipient.library).unwrap().count(), 1);
}

/// Serves `/shot.png` as a PNG, `/logo.svg` as SVG and everything else as 404, on a local port.
fn image_server() -> String {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buf = [0u8; 2048];
            let n = stream.read(&mut buf).unwrap_or(0);
            let request = String::from_utf8_lossy(&buf[..n]);
            let response: Vec<u8> = if request.starts_with("GET /shot.png ") {
                [b"HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello".as_slice()].concat()
            } else if request.starts_with("GET /logo.svg ") {
                b"HTTP/1.1 200 OK\r\nContent-Type: image/svg+xml\r\nContent-Length: 11\r\nConnection: close\r\n\r\n<svg></svg>".to_vec()
            } else {
                b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
            };
            let _ = stream.write_all(&response);
        }
    });
    format!("http://{addr}")
}

#[test]
fn remote_images_in_the_transcript_are_downloaded_into_the_bundle() {
    let host = image_server();
    let mut sender = Sender::new("remote");
    let text = format!("Here it is: ![login]({host}/shot.png) and <img alt=\\\"x\\\" src=\\\"{host}/missing.png\\\"> ![logo]({host}/logo.svg)");
    std::fs::write(
        sender.root.join("claude/-work-repo/remote.jsonl"),
        format!(
            "{}\n{}\n",
            r#"{"type":"user","sessionId":"remote","cwd":"/work/repo","timestamp":"2026-03-03T10:00:00Z","message":{"role":"user","content":"show me the login page"}}"#,
            format!(r#"{{"type":"assistant","sessionId":"remote","timestamp":"2026-03-03T10:00:01Z","message":{{"id":"m1","model":"claude-opus-5-5","role":"assistant","content":[{{"type":"text","text":"{text}"}}]}}}}"#)
        ),
    )
    .unwrap();
    index::run(&mut sender.conn, false, None, &|_, _| {}).unwrap();
    let exported = sender.export("claude/-work-repo/remote.jsonl", None, None);

    let shot = exported.attachments.iter().find(|a| a.name.as_deref() == Some(&format!("{host}/shot.png"))).unwrap();
    assert_eq!((shot.kind, shot.status, shot.mime.as_deref(), shot.bytes), (bundle::AttachmentKind::Image, bundle::AttachmentStatus::Included, Some("image/png"), Some(5)));
    let logo = exported.attachments.iter().find(|a| a.name.as_deref() == Some(&format!("{host}/logo.svg"))).unwrap();
    assert_eq!((logo.status, logo.mime.as_deref()), (bundle::AttachmentStatus::Included, Some("image/svg+xml")));
    let missing = exported.attachments.iter().find(|a| a.name.as_deref() == Some(&format!("{host}/missing.png"))).unwrap();
    assert_eq!(missing.status, bundle::AttachmentStatus::Omitted);
    assert!(missing.reason.as_deref().unwrap().contains("couldn't download"), "{:?}", missing.reason);
    let reply = &exported.sessions[0].events[1].text;
    assert!(!reply.contains(&host), "remote addresses are replaced: {reply}");
    assert!(reply.contains(&format!("![login](attachment:{})", shot.id)), "{reply}");

    let file = sender.save(&exported, "remote.spsession");
    let mut recipient = Recipient::new("remote");
    bundle::import(&recipient.conn, &recipient.library, &file, None).unwrap();
    let rows = recipient.reindex();
    let events = bundle::load_events(Path::new(&rows[0].path)).unwrap();
    assert!(events[1].text.contains("![login](data:image/png;base64,aGVsbG8=)"), "{}", events[1].text);
}

#[test]
fn removing_an_imported_session_removes_the_subagents_imported_with_it() {
    let sender = Sender::new("remove");
    let file = sender.save(&sender.export("claude/-work-repo/abc.jsonl", None, None), "abc.spsession");
    let mut recipient = Recipient::new("remove");
    bundle::import(&recipient.conn, &recipient.library, &file, None).unwrap();
    let rows = recipient.reindex();
    let primary = rows.iter().find(|r| !r.is_subagent).unwrap();
    let removed = bundle::remove(&recipient.conn, &recipient.library, Path::new(&primary.path)).unwrap();
    assert_eq!(removed.len(), 2);
    assert!(recipient.reindex().is_empty());
    assert_eq!(std::fs::read_dir(&recipient.library).unwrap().count(), 0);
}

#[test]
fn only_files_in_the_library_can_be_removed() {
    let sender = Sender::new("remove-outside");
    let recipient = Recipient::new("remove-outside");
    let local = sender.root.join("claude/-work-repo/abc.jsonl");
    let err = bundle::remove(&recipient.conn, &recipient.library, &local).unwrap_err().to_string();
    assert!(err.contains("isn't an imported session"), "{err}");
    assert!(local.exists());
}

#[test]
fn a_damaged_library_file_is_reported_and_can_be_removed() {
    let sender = Sender::new("damaged");
    let file = sender.save(&sender.export("claude/-work-repo/abc.jsonl", None, Some(vec![])), "abc.spsession");
    let mut recipient = Recipient::new("damaged");
    bundle::import(&recipient.conn, &recipient.library, &file, None).unwrap();
    let copy = std::fs::read_dir(&recipient.library).unwrap().next().unwrap().unwrap().path();
    std::fs::write(&copy, "{ damaged").unwrap();
    assert!(recipient.reindex().is_empty());

    let problems = index::library_errors(&recipient.conn, &recipient.library).unwrap();
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].path, copy.to_string_lossy());
    let err = format!("{:#}", bundle::import(&recipient.conn, &recipient.library, &file, None).unwrap_err());
    assert!(err.contains("Your library's copy of claude:abc is damaged"), "{err}");

    bundle::remove(&recipient.conn, &recipient.library, &copy).unwrap();
    assert!(bundle::import(&recipient.conn, &recipient.library, &file, None).unwrap().imported == vec!["claude:abc".to_string()]);
    assert_eq!(recipient.reindex().len(), 1);
    assert!(index::library_errors(&recipient.conn, &recipient.library).unwrap().is_empty());
}

#[test]
fn nothing_is_imported_when_the_exported_session_is_empty() {
    let recipient = Recipient::new("empty-primary");
    let report = import_edited(&recipient, "empty-primary", |v| {
        v["sessions"][0]["events"].as_array_mut().unwrap().retain(|e| e["kind"] == "system");
    })
    .unwrap();
    assert!(report.imported.is_empty());
    assert_eq!(report.empty, vec!["claude:abc".to_string()]);
    assert!(!recipient.library.exists() || std::fs::read_dir(&recipient.library).unwrap().count() == 0);
}

#[test]
fn details_and_annotations_travel_with_the_session() {
    use session_player_lib::test_support::annotations as notes;
    let sender = Sender::new("notes");
    let meta = notes::SessionMeta { key: "local:claude:abc".into(), goal: Some("Fix the login test".into()), tags: vec!["ci".into()], ..Default::default() };
    notes::save_meta(&sender.conn, &meta).unwrap();
    let event = |anchor: &str, kind: &str| -> notes::EventRef { serde_json::from_value(serde_json::json!({ "anchor": anchor, "kind": kind, "ts": 1767261605000i64 })).unwrap() };
    let theirs = notes::save_annotation(
        &sender.conn,
        notes::Annotation::new("local:claude:abc", notes::Kind::ToolError, "npm test failed", vec![event("e4", "tool_result")], notes::Author::Person { name: Some("Sam".into()) }),
    )
    .unwrap();
    notes::save_annotation(&sender.conn, notes::Annotation::new("local:claude:agent-a1", notes::Kind::KeyDecision, "read the login code", vec![], notes::Author::Model { model: "claude-haiku-5-5".into() })).unwrap();

    let exported = sender.export("claude/-work-repo/abc.jsonl", None, None);
    assert_eq!(exported.sessions[0].meta.as_ref().unwrap().goal.as_deref(), Some("Fix the login test"));
    assert_eq!(exported.sessions[0].annotations.len(), 1);
    assert_eq!(exported.sessions[1].annotations[0].author, notes::Author::Model { model: "claude-haiku-5-5".into() });
    let preview = bundle::summarize(&exported);
    assert_eq!((preview.sessions[0].annotations, preview.sessions[0].has_details), (1, true));
    let file = sender.save(&exported, "abc.spsession");

    let recipient = Recipient::new("notes");
    bundle::import(&recipient.conn, &recipient.library, &file, None).unwrap();
    let key = "bundle:claude:abc";
    assert_eq!(notes::meta(&recipient.conn, key).unwrap().unwrap().tags, ["ci"]);
    let imported = notes::annotations(&recipient.conn, key).unwrap();
    assert_eq!((imported[0].id.as_str(), imported[0].author.clone()), (theirs.id.as_str(), notes::Author::Person { name: Some("Sam".into()) }));
    assert_eq!(notes::annotations(&recipient.conn, "bundle:claude:agent-a1").unwrap().len(), 1);

    // The recipient adds a note of their own, then the sender sends an updated bundle.
    notes::save_annotation(&recipient.conn, notes::Annotation::new(key, notes::Kind::Other, "mine", vec![], notes::Author::Person { name: None })).unwrap();
    let again = bundle::import(&recipient.conn, &recipient.library, &file, None).unwrap();
    assert!(again.imported.is_empty());
    assert_eq!(notes::annotations(&recipient.conn, key).unwrap().len(), 2, "nothing duplicated");

    let mut updated = exported.clone();
    updated.sessions[0].annotations[0].note = "npm test failed on login.spec".into();
    let updated_file = sender.save(&updated, "abc-2.spsession");
    let asked = bundle::import(&recipient.conn, &recipient.library, &updated_file, None).unwrap();
    assert_eq!(asked.conflicts.len(), 1);
    assert_eq!(notes::annotations(&recipient.conn, key).unwrap()[0].note, "npm test failed", "nothing changes before a decision");
    bundle::import(&recipient.conn, &recipient.library, &updated_file, Some(bundle::OnConflict::Replace)).unwrap();
    let after: Vec<_> = notes::annotations(&recipient.conn, key).unwrap().into_iter().map(|a| a.note).collect();
    assert_eq!(after, ["npm test failed on login.spec", "mine"]);
}
