// Session metadata and annotations: stored by the app, never in the agent's
// files, and kept through index rebuilds.
use std::path::{Path, PathBuf};

use session_player_lib::test_support::{annotations as notes, index, parse};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

#[test]
fn every_event_has_an_anchor_and_a_pi_branch_keeps_its_chain_anchors() {
    let claude = parse::parse_file(&fixture("claude.jsonl"), parse::detect(&fixture("claude.jsonl")).unwrap()).unwrap();
    let anchors: Vec<_> = claude.events.iter().map(|e| e.anchor.clone().unwrap()).collect();
    assert_eq!(anchors, ["e0", "e1", "e2", "e3", "e4"]);

    let tree = fixture("pi-tree.jsonl");
    let chain = parse::pi::parse_chain(&tree, Some("b2a")).unwrap();
    let view = parse::pi::parse_branch(&tree, Some("b2a")).unwrap();
    let view_anchors: Vec<_> = view.events.iter().map(|e| e.anchor.clone().unwrap()).collect();
    assert_eq!(view_anchors, ["e4", "e5"]);
    let chain_tail: Vec<_> = chain.events[4..].iter().map(|e| (e.anchor.clone(), e.text.clone())).collect();
    let view_events: Vec<_> = view.events.iter().map(|e| (e.anchor.clone(), e.text.clone())).collect();
    assert_eq!(chain_tail, view_events);
}

fn temp_db(name: &str) -> (PathBuf, rusqlite::Connection) {
    let path = std::env::temp_dir().join(format!("sp-notes-{name}-{}.sqlite", std::process::id()));
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }
    let conn = index::open(&path).unwrap();
    (path, conn)
}

const KEY: &str = "local:claude:abc";

#[test]
fn metadata_is_saved_edited_and_kept_through_an_index_rebuild() {
    let (path, mut conn) = temp_db("meta");
    assert_eq!(notes::meta(&conn, KEY).unwrap(), None);
    let mut meta = notes::SessionMeta {
        key: KEY.into(),
        title: Some("Fix the flaky login test".into()),
        goal: Some("CI fails one run in five".into()),
        outcome: notes::Outcome::Partial,
        outcome_note: Some("Fixed locally, CI still flaky".into()),
        tags: vec!["ci".into(), "auth".into()],
        links: vec![notes::Link { url: "https://github.com/o/r/pull/7".into(), label: Some("PR 7".into()) }],
        ..Default::default()
    };
    notes::save_meta(&conn, &meta).unwrap();
    meta.tags.push("flaky".into());
    notes::save_meta(&conn, &meta).unwrap();
    let saved = notes::meta(&conn, KEY).unwrap().unwrap();
    assert_eq!(saved.tags, ["ci", "auth", "flaky"]);
    assert!(saved.updated_at > 0);

    // Rebuild index, then a parser change that forces a rebuild on open.
    index::run(&mut conn, true, None, &|_, _| {}).unwrap();
    conn.execute("UPDATE settings SET value='0' WHERE key='index_version'", []).unwrap();
    drop(conn);
    let conn = index::open(&path).unwrap();
    assert_eq!(notes::meta(&conn, KEY).unwrap().unwrap().title.as_deref(), Some("Fix the flaky login test"));

    // Clearing every field removes the record.
    notes::save_meta(&conn, &notes::SessionMeta { key: KEY.into(), ..Default::default() }).unwrap();
    assert_eq!(notes::meta(&conn, KEY).unwrap(), None);
}

fn event_ref(anchor: &str, kind: &str, ts: Option<i64>) -> notes::EventRef {
    serde_json::from_value(serde_json::json!({ "anchor": anchor, "kind": kind, "ts": ts })).unwrap()
}

fn person() -> notes::Author {
    notes::Author::Person { name: Some("Dan".into()) }
}

#[test]
fn annotations_are_created_edited_and_deleted() {
    let (_, conn) = temp_db("annotations");
    let created = notes::save_annotation(
        &conn,
        notes::Annotation::new(KEY, notes::Kind::ToolError, "npm test failed", vec![event_ref("e4", "tool_result", Some(1767261605000))], person()),
    )
    .unwrap();
    assert!(!created.id.is_empty());
    assert_eq!(created.created_at, created.updated_at);
    let session_wide = notes::save_annotation(&conn, notes::Annotation::new(KEY, notes::Kind::Automatable, "same fix as last week", vec![], person())).unwrap();

    let mut edited = created.clone();
    edited.note = "npm test failed: login spec".into();
    edited.kind = notes::Kind::AgentStuck;
    let edited = notes::save_annotation(&conn, edited).unwrap();
    assert_eq!((edited.id.as_str(), edited.created_at), (created.id.as_str(), created.created_at));
    assert!(edited.updated_at >= edited.created_at);

    let listed = notes::annotations(&conn, KEY).unwrap();
    assert_eq!(listed.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), [created.id.as_str(), session_wide.id.as_str()]);
    assert_eq!(listed[0].note, "npm test failed: login spec");

    notes::delete_annotation(&conn, &created.id).unwrap();
    assert_eq!(notes::annotations(&conn, KEY).unwrap().len(), 1);
    assert!(notes::annotations(&conn, "local:claude:other").unwrap().is_empty());
}

#[test]
fn an_annotation_needs_a_note_or_an_event() {
    let (_, conn) = temp_db("invalid");
    let err = notes::save_annotation(&conn, notes::Annotation::new(KEY, notes::Kind::Other, "  ", vec![], person())).unwrap_err();
    assert!(err.to_string().contains("needs a note or an event"), "{err}");
    let mut unknown = notes::Annotation::new(KEY, notes::Kind::Other, "x", vec![], person());
    unknown.id = "missing".into();
    assert!(notes::save_annotation(&conn, unknown).unwrap_err().to_string().contains("no longer exists"));
}

#[test]
fn anchors_are_checked_against_the_transcript_and_refound_when_events_shift() {
    let path = fixture("claude.jsonl");
    let events = parse::parse_file(&path, parse::detect(&path).unwrap()).unwrap().events;
    let prompt_ts = events[1].ts;
    let mut list = vec![
        notes::Annotation::new(KEY, notes::Kind::HumanIntervention, "", vec![event_ref("e1", "user", prompt_ts)], person()),
        // Written against an older parse where the prompt was e0.
        notes::Annotation::new(KEY, notes::Kind::HumanIntervention, "", vec![event_ref("e0", "user", prompt_ts)], person()),
        notes::Annotation::new(KEY, notes::Kind::ToolError, "gone", vec![event_ref("e9", "tool_result", Some(1))], person()),
    ];
    notes::resolve(&mut list, &events);
    let refs: Vec<_> = list.iter().map(|a| (a.anchors[0].anchor.as_str(), a.anchors[0].found)).collect();
    assert_eq!(refs, [("e1", true), ("e1", true), ("e9", false)]);
}

#[test]
fn search_finds_sessions_by_their_metadata_and_annotations() {
    let (path, mut conn) = temp_db("search");
    let root = path.with_extension("agents");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("claude/-work-repo")).unwrap();
    std::fs::create_dir_all(root.join("pi/--work-pi--")).unwrap();
    std::fs::copy(fixture("claude.jsonl"), root.join("claude/-work-repo/abc.jsonl")).unwrap();
    std::fs::copy(fixture("pi.jsonl"), root.join("pi/--work-pi--/2026_01a-pi.jsonl")).unwrap();
    index::add_root(&conn, &root.to_string_lossy()).unwrap();
    index::run(&mut conn, false, None, &|_, _| {}).unwrap();

    let meta = notes::SessionMeta { key: KEY.into(), goal: Some("stabilize the zeppelin harness".into()), tags: vec!["quokka".into()], ..Default::default() };
    notes::save_meta(&conn, &meta).unwrap();
    notes::save_annotation(&conn, notes::Annotation::new("local:pi:01a-pi@a3", notes::Kind::AgentStuck, "retried the marmalade cache three times", vec![], person())).unwrap();

    let hit = |q: &str| index::search(&conn, q).unwrap().into_iter().map(|h| h.path).collect::<Vec<_>>();
    let claude = root.join("claude/-work-repo/abc.jsonl").to_string_lossy().into_owned();
    let pi = root.join("pi/--work-pi--/2026_01a-pi.jsonl").to_string_lossy().into_owned();
    assert_eq!(hit("zeppelin"), [claude.clone()]);
    assert_eq!(hit("quokka"), [claude]);
    assert_eq!(hit("marmalade"), [format!("{pi}#a3")]);

    let summaries = notes::summaries(&conn).unwrap();
    let pi_summary = summaries.iter().find(|s| s.key == "local:pi:01a-pi@a3").unwrap();
    assert_eq!(pi_summary.kinds, [notes::Kind::AgentStuck]);
    let claude_summary = summaries.iter().find(|s| s.key == KEY).unwrap();
    assert_eq!((claude_summary.tags.as_slice(), claude_summary.outcome), (["quokka".to_string()].as_slice(), notes::Outcome::Unknown));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn links_must_be_web_addresses() {
    let (_, conn) = temp_db("links");
    let meta = notes::SessionMeta {
        key: KEY.into(),
        links: vec![notes::Link { url: "javascript:alert(1)".into(), label: None }],
        ..Default::default()
    };
    let err = notes::save_meta(&conn, &meta).unwrap_err().to_string();
    assert!(err.contains("isn't a web address"), "{err}");
    assert_eq!(notes::meta(&conn, KEY).unwrap(), None);
}
