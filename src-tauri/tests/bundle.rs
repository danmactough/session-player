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
    let report = bundle::import(&recipient.library, &file, None).unwrap();
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
    bundle::import(&recipient.library, &file, None).unwrap();
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
    bundle::import(&recipient.library, &file, None).unwrap();
    let rows = recipient.reindex();
    assert_eq!(rows[0].title, "Retry: stress test my caching plan");
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
    bundle::import(&recipient.library, &file, None).unwrap();
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
    bundle::import(&recipient.library, &file, None).map_err(|e| format!("{e:#}"))
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
        ("duplicate", Box::new(|v| v["sessions"][1]["id"] = "claude:abc".into()), "claude:abc appears more than once"),
        ("stranger", Box::new(|v| v["sessions"][1]["parent_id"] = "zzz".into()), "isn't a subagent of"),
        ("dangling", Box::new(|v| v["attachments"] = serde_json::json!([])), "missing attachment a0"),
        ("orphan", Box::new(|v| v["attachments"][0]["session"] = "claude:nope".into()), "attachment a0 belongs to session claude:nope"),
        ("no-data", Box::new(|v| v["attachments"][0]["data"] = serde_json::Value::Null), "attachment a0 is marked included but has no data"),
        ("bad-data", Box::new(|v| v["attachments"][0]["data"] = "not base64!".into()), "attachment a0 isn't valid base64"),
        ("anchors", Box::new(|v| v["sessions"][0]["events"][1]["anchor"] = "e0".into()), "anchor e0 appears more than once"),
    ];
    for (name, edit, expected) in cases {
        let err = import_edited(&recipient, name, edit).expect_err(name);
        assert!(err.contains(expected), "{name}: {err}");
    }
    let garbage = recipient.base.join("garbage.spsession");
    std::fs::write(&garbage, "{ not json").unwrap();
    let err = format!("{:#}", bundle::import(&recipient.library, &garbage, None).unwrap_err());
    assert!(err.contains("isn't a Session Player session bundle"), "{err}");
    assert!(!recipient.library.exists() || std::fs::read_dir(&recipient.library).unwrap().next().is_none());
}

#[test]
fn session_ids_cannot_place_files_outside_the_library() {
    let recipient = Recipient::new("paths");
    let report = import_edited(&recipient, "escape", |v| {
        v["sessions"] = serde_json::json!([v["sessions"][0].clone()]);
        v["sessions"][0]["id"] = "../../escaped".into();
        v["attachments"] = serde_json::json!([]);
    })
    .unwrap();
    assert_eq!(report.imported, vec!["../../escaped".to_string()]);
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
    bundle::import(&recipient.library, &first, None).unwrap();

    // The same content again is not a conflict.
    let again = bundle::import(&recipient.library, &first, None).unwrap();
    assert!(again.imported.is_empty());
    assert_eq!(again.unchanged, vec!["claude:abc".to_string(), "claude:agent-a1".to_string()]);

    // A different version of a session already in the library waits for a decision, and nothing is written.
    let asked = bundle::import(&recipient.library, &second, None).unwrap();
    assert!(asked.imported.is_empty());
    assert_eq!(asked.conflicts.len(), 1);
    assert_eq!((asked.conflicts[0].id.as_str(), asked.conflicts[0].existing_title.as_str(), asked.conflicts[0].incoming_title.as_str()), ("claude:abc", "Flaky login test", "Renamed by the sender"));
    assert!(recipient.reindex().iter().any(|r| r.title == "Flaky login test"));

    let kept = bundle::import(&recipient.library, &second, Some(OnConflict::Keep)).unwrap();
    assert_eq!(kept.kept, vec!["claude:abc".to_string()]);
    assert!(recipient.reindex().iter().any(|r| r.title == "Flaky login test"));

    let replaced = bundle::import(&recipient.library, &second, Some(OnConflict::Replace)).unwrap();
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
    bundle::import(&recipient.library, &file, None).unwrap();
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
