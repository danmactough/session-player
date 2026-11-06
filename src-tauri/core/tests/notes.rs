// Session metadata and annotations: stored by the app, never in the agent's
// files, and kept through index rebuilds.
use std::path::{Path, PathBuf};

use session_player_core::test_support::{annotations as notes, index, parse};

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
        outcome: notes::Outcome::Partial,
        outcome_note: Some("Fixed locally, CI still flaky".into()),
        components: vec!["packages/auth".into(), " ".into()],
        ..Default::default()
    };
    notes::save_meta(&conn, &meta).unwrap();
    meta.components = vec!["packages/auth".into(), "apps/web".into()];
    meta.outcome = notes::Outcome::Failed;
    notes::save_meta(&conn, &meta).unwrap();
    let saved = notes::meta(&conn, KEY).unwrap().unwrap();
    assert_eq!(saved.components, ["packages/auth", "apps/web"]);
    assert_eq!(saved.outcome, notes::Outcome::Failed);
    assert!(saved.updated_at > 0);

    // Rebuild index, then a parser change that forces a rebuild on open.
    index::run(&mut conn, true, None, &|_, _| {}).unwrap();
    conn.execute("UPDATE settings SET value='0' WHERE key='index_version'", []).unwrap();
    drop(conn);
    let conn = index::open(&path).unwrap();
    assert_eq!(notes::meta(&conn, KEY).unwrap().unwrap().components, ["packages/auth", "apps/web"]);

    // Clearing every field (blank components count as empty) keeps only
    // which fields were cleared, so generating won't fill them again.
    notes::save_meta(&conn, &notes::SessionMeta { key: KEY.into(), components: vec![" ".into()], ..Default::default() }).unwrap();
    let cleared = notes::meta(&conn, KEY).unwrap().unwrap();
    assert_eq!((cleared.outcome, cleared.components.len()), (notes::Outcome::Unknown, 0));
    assert_eq!(cleared.cleared, ["outcome", "components"]);

    // A session that never had any details stores nothing.
    let other = "local:claude:never";
    notes::save_meta(&conn, &notes::SessionMeta { key: other.into(), ..Default::default() }).unwrap();
    assert_eq!(notes::meta(&conn, other).unwrap(), None);
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
    let second = notes::save_annotation(&conn, notes::Annotation::new(KEY, notes::Kind::Automatable, "", vec![event_ref("e1", "user", None)], person())).unwrap();

    let mut edited = created.clone();
    edited.note = "npm test failed: login spec".into();
    edited.kind = notes::Kind::AgentStuck;
    let edited = notes::save_annotation(&conn, edited).unwrap();
    assert_eq!((edited.id.as_str(), edited.created_at), (created.id.as_str(), created.created_at));
    assert!(edited.updated_at >= edited.created_at);

    let listed = notes::annotations(&conn, KEY).unwrap();
    assert_eq!(listed.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), [created.id.as_str(), second.id.as_str()]);
    assert_eq!(listed[0].note, "npm test failed: login spec");

    notes::delete_annotation(&conn, &created.id).unwrap();
    assert_eq!(notes::annotations(&conn, KEY).unwrap().len(), 1);
    assert!(notes::annotations(&conn, "local:claude:other").unwrap().is_empty());
}

#[test]
fn editing_an_annotation_that_was_deleted_fails() {
    let (_, conn) = temp_db("invalid");
    let mut unknown = notes::Annotation::new(KEY, notes::Kind::Other, "x", vec![event_ref("e1", "user", None)], person());
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
    notes::resolve(&mut list, &events, None);
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

    let meta = notes::SessionMeta { key: KEY.into(), outcome_note: Some("stabilized the zeppelin harness".into()), components: vec!["packages/quokka".into()], ..Default::default() };
    notes::save_meta(&conn, &meta).unwrap();
    // A pi session's notes belong to the file, whichever branch they were made on.
    notes::save_annotation(&conn, notes::Annotation::new("local:pi:01a-pi", notes::Kind::AgentStuck, "retried the marmalade cache three times", vec![event_ref("e2", "assistant", None)], person())).unwrap();

    let hit = |q: &str| index::search(&conn, q).unwrap().into_iter().map(|h| h.path).collect::<Vec<_>>();
    let claude = root.join("claude/-work-repo/abc.jsonl").to_string_lossy().into_owned();
    let pi = root.join("pi/--work-pi--/2026_01a-pi.jsonl").to_string_lossy().into_owned();
    assert_eq!(hit("zeppelin"), [claude.clone()]);
    assert_eq!(hit("quokka"), [claude]);
    assert_eq!(hit("marmalade"), [pi]);

    let summaries = notes::summaries(&conn).unwrap();
    let pi_summary = summaries.iter().find(|s| s.key == "local:pi:01a-pi").unwrap();
    assert_eq!(pi_summary.kinds, [notes::Kind::AgentStuck]);
    let claude_summary = summaries.iter().find(|s| s.key == KEY).unwrap();
    assert_eq!((claude_summary.components.as_slice(), claude_summary.outcome), (["packages/quokka".to_string()].as_slice(), notes::Outcome::Unknown));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn an_annotation_must_point_at_an_event() {
    let (_, conn) = temp_db("anchorless");
    let err = notes::save_annotation(&conn, notes::Annotation::new(KEY, notes::Kind::Other, "about everything", vec![], person())).unwrap_err().to_string();
    assert!(err.contains("needs an event"), "{err}");
}

fn pi_ref(anchor: &str, kind: &str, ts: Option<i64>, entry: &str) -> notes::EventRef {
    serde_json::from_value(serde_json::json!({ "anchor": anchor, "kind": kind, "ts": ts, "entry": entry })).unwrap()
}

#[test]
fn pi_anchors_follow_their_entry_and_other_branches_are_kept_apart() {
    let tree = fixture("pi-tree.jsonl");
    let main = parse::pi::parse_branch(&tree, None).unwrap().events;
    let entries = parse::pi::entry_ids(&tree).unwrap();
    let reply = main.iter().find(|e| e.text == "main reply").unwrap();
    let mut list = vec![
        // Numbered when the transcript had a different shape; the entry still finds it.
        notes::Annotation::new("local:pi:01b-tree", notes::Kind::KeyDecision, "", vec![pi_ref("e9", "assistant", reply.ts, "m4")], person()),
        // Made on the b1 branch: not in this view, but still in the file.
        notes::Annotation::new("local:pi:01b-tree", notes::Kind::Other, "", vec![pi_ref("e3", "assistant", Some(1), "b1a")], person()),
        notes::Annotation::new("local:pi:01b-tree", notes::Kind::Other, "", vec![pi_ref("e3", "assistant", Some(1), "gone")], person()),
    ];
    notes::resolve(&mut list, &main, Some(&entries));
    let state: Vec<_> = list.iter().map(|a| (a.anchors[0].anchor.as_str(), a.anchors[0].found, a.anchors[0].elsewhere)).collect();
    assert_eq!(state, [(reply.anchor.as_deref().unwrap(), true, false), ("e3", false, true), ("e3", false, false)]);
}

#[test]
fn notes_never_change_the_agent_files() {
    let (path, mut conn) = temp_db("untouched");
    let root = path.with_extension("agents");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("claude/-work-repo")).unwrap();
    let file = root.join("claude/-work-repo/abc.jsonl");
    std::fs::copy(fixture("claude.jsonl"), &file).unwrap();
    let before = (std::fs::read(&file).unwrap(), std::fs::metadata(&file).unwrap().modified().unwrap());
    index::add_root(&conn, &root.to_string_lossy()).unwrap();
    index::run(&mut conn, false, None, &|_, _| {}).unwrap();
    notes::save_meta(&conn, &notes::SessionMeta { key: KEY.into(), components: vec!["x".into()], ..Default::default() }).unwrap();
    notes::save_annotation(&conn, notes::Annotation::new(KEY, notes::Kind::ToolError, "y", vec![event_ref("e4", "tool_result", None)], person())).unwrap();
    index::run(&mut conn, true, None, &|_, _| {}).unwrap();
    assert_eq!((std::fs::read(&file).unwrap(), std::fs::metadata(&file).unwrap().modified().unwrap()), before);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn an_annotation_on_one_of_several_context_documents_stays_on_it() {
    let events = parse::parse_file(&fixture("pi-context.jsonl"), parse::detect(&fixture("pi-context.jsonl")).unwrap()).unwrap().events;
    let repo = events.iter().find(|e| e.context.as_ref().is_some_and(|c| c.name == "/work/repo/AGENTS.md")).unwrap();
    let r: notes::EventRef = serde_json::from_value(serde_json::json!({
        "anchor": "c9", "kind": "context", "ts": repo.ts, "entry": "s1", "name": "/work/repo/AGENTS.md"
    }))
    .unwrap();
    let mut list = vec![notes::Annotation::new("local:pi:pi-ctx", notes::Kind::IgnoredInstruction, "", vec![r], person())];
    notes::resolve(&mut list, &events, None);
    assert_eq!((list[0].anchors[0].anchor.as_str(), list[0].anchors[0].found), (repo.anchor.as_deref().unwrap(), true));
}

fn cite(conn: &rusqlite::Connection, report: &str, a: &notes::Annotation) {
    conn.execute("INSERT INTO report_citations(report_id, annotation_id, version) VALUES(?1, ?2, ?3)", rusqlite::params![report, a.id, a.version]).unwrap();
}

fn versions(conn: &rusqlite::Connection, id: &str) -> Vec<u32> {
    let mut stmt = conn.prepare("SELECT version FROM annotation_versions WHERE id=?1 ORDER BY version").unwrap();
    stmt.query_map([id], |r| r.get(0)).unwrap().collect::<Result<_, _>>().unwrap()
}

#[test]
fn each_change_saves_a_new_version_and_only_cited_old_versions_are_kept() {
    let (_, conn) = temp_db("versions");
    let first = notes::save_annotation(&conn, notes::Annotation::new(KEY, notes::Kind::ToolError, "one", vec![event_ref("e1", "user", None)], person())).unwrap();
    assert_eq!(first.version, 1);

    // Saving it unchanged, as the UI does with resolved anchors, adds no version.
    let mut same = first.clone();
    same.anchors[0].found = true;
    assert_eq!(notes::save_annotation(&conn, same).unwrap().version, 1);

    // An uncited version is dropped once it is replaced.
    let second = notes::save_annotation(&conn, notes::Annotation { note: "two".into(), ..first.clone() }).unwrap();
    assert_eq!((second.version, versions(&conn, &first.id)), (2, vec![2]));

    // A cited version is kept, unchanged, after later edits; a version sent in is ignored.
    cite(&conn, "r1", &second);
    let third = notes::save_annotation(&conn, notes::Annotation { note: "three".into(), version: 9, ..second.clone() }).unwrap();
    assert_eq!((third.version, versions(&conn, &first.id)), (3, vec![2, 3]));
    assert_eq!(notes::annotation_version(&conn, &first.id, 2).unwrap().unwrap().note, "two");
    assert_eq!(notes::annotation(&conn, &first.id).unwrap().unwrap().note, "three");
}

#[test]
fn deleting_keeps_only_the_versions_a_report_cites() {
    let (_, conn) = temp_db("delete-versions");
    let new = |note: &str| notes::save_annotation(&conn, notes::Annotation::new(KEY, notes::Kind::Other, note, vec![event_ref("e1", "user", None)], person())).unwrap();

    let uncited = new("never cited");
    notes::delete_annotation(&conn, &uncited.id).unwrap();
    assert!(versions(&conn, &uncited.id).is_empty());

    let cited = new("cited");
    cite(&conn, "r1", &cited);
    let later = notes::save_annotation(&conn, notes::Annotation { note: "edited after the report".into(), ..cited.clone() }).unwrap();
    notes::delete_annotation(&conn, &cited.id).unwrap();
    assert_eq!(versions(&conn, &cited.id), [1]);
    assert_eq!(notes::annotation(&conn, &cited.id).unwrap(), None);
    assert!(notes::annotations(&conn, KEY).unwrap().is_empty());
    assert_eq!(notes::annotation_version(&conn, &cited.id, later.version).unwrap(), None);
    assert_eq!(notes::annotation_version(&conn, &cited.id, 1).unwrap().unwrap().note, "cited");
    assert!(notes::search(&conn, "cited").unwrap().is_empty());

    // Arriving again in a bundle, it continues after the kept version instead of reusing its number.
    let mut incoming = cited.clone();
    incoming.version = 0;
    incoming.note = "back from a bundle".into();
    notes::put_annotation(&conn, &incoming).unwrap();
    assert_eq!(notes::annotation(&conn, &cited.id).unwrap().unwrap().version, 2);
    assert_eq!(versions(&conn, &cited.id), [1, 2]);
}

#[test]
fn an_imported_annotation_adds_a_version_only_when_it_differs() {
    let (_, conn) = temp_db("import-versions");
    let saved = notes::save_annotation(&conn, notes::Annotation::new(KEY, notes::Kind::Other, "mine", vec![event_ref("e1", "user", None)], person())).unwrap();
    let unnumbered = notes::Annotation { version: 0, ..saved.clone() };
    notes::put_annotation(&conn, &unnumbered).unwrap();
    assert_eq!(versions(&conn, &saved.id), [1]);

    notes::put_annotation(&conn, &notes::Annotation { note: "theirs".into(), ..unnumbered.clone() }).unwrap();
    let now = notes::annotation(&conn, &saved.id).unwrap().unwrap();
    assert_eq!((now.version, now.note.as_str()), (2, "theirs"));

    // Never moves an annotation to another session.
    notes::put_annotation(&conn, &notes::Annotation { key: "local:claude:other".into(), note: "elsewhere".into(), ..unnumbered }).unwrap();
    assert_eq!(notes::annotation(&conn, &saved.id).unwrap().unwrap().note, "theirs");
}

#[test]
fn annotations_saved_before_versions_existed_become_version_1() {
    let path = std::env::temp_dir().join(format!("sp-notes-migrate-{}.sqlite", std::process::id()));
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }
    let old = notes::Annotation { id: "old-1".into(), created_at: 1, updated_at: 1, ..notes::Annotation::new(KEY, notes::Kind::Other, "from 0.13", vec![event_ref("e1", "user", None)], person()) };
    let json = serde_json::to_string(&old).unwrap();
    assert!(!json.contains("\"version\""));
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch("CREATE TABLE annotations (id TEXT PRIMARY KEY, key TEXT NOT NULL, json TEXT NOT NULL);").unwrap();
        conn.execute("INSERT INTO annotations(id, key, json) VALUES('old-1', ?1, ?2)", [KEY, json.as_str()]).unwrap();
    }
    let conn = index::open(&path).unwrap();
    assert_eq!(notes::annotation(&conn, "old-1").unwrap().unwrap().version, 1);
    assert_eq!(versions(&conn, "old-1"), [1]);
    let edited = notes::save_annotation(&conn, notes::Annotation { note: "edited".into(), ..old }).unwrap();
    assert_eq!((edited.version, versions(&conn, "old-1")), (2, vec![2]));
}

#[test]
fn summaries_count_annotations_by_kind_author_and_confidence() {
    let (_, conn) = temp_db("summary-counts");
    let at = || vec![event_ref("e1", "user", None)];
    let model = || notes::Author::Model { model: "claude-haiku-5-5".into() };
    for (kind, author, confidence) in [
        (notes::Kind::ToolError, person(), None),
        (notes::Kind::ToolError, model(), Some("high")),
        (notes::Kind::MissingContext, model(), Some("low")),
    ] {
        let mut a = notes::Annotation::new(KEY, kind, "x", at(), author);
        a.confidence = confidence.map(String::from);
        notes::save_annotation(&conn, a).unwrap();
    }
    notes::save_meta(&conn, &notes::SessionMeta { key: "local:claude:by-hand".into(), outcome: notes::Outcome::Failed, ..Default::default() }).unwrap();

    let list = notes::summaries(&conn).unwrap();
    let s = list.iter().find(|s| s.key == KEY).unwrap();
    assert_eq!((s.kind_counts[&notes::Kind::ToolError], s.kind_counts[&notes::Kind::MissingContext]), (2, 1));
    assert_eq!((s.person_annotations, s.model_annotations), (1, 2));
    assert_eq!((s.confidence["high"], s.confidence["low"]), (1, 1));
    assert!(s.analyzed, "a model's annotations mean Analyze ran");
    let by_hand = list.iter().find(|s| s.key == "local:claude:by-hand").unwrap();
    assert!(!by_hand.analyzed);
    assert_eq!(by_hand.outcome, notes::Outcome::Failed);
}
