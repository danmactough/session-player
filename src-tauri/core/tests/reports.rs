// Reports on a set of sessions: what the model is given, how its refs map
// back to sessions and annotation versions, splitting long input, and how
// saving and deleting a report keeps or releases annotation versions.
use std::cell::RefCell;
use std::path::PathBuf;

use serde_json::{Value, json};
use session_player_core::test_support::{annotations as notes, generate, index, reports};

fn temp_db(name: &str) -> (PathBuf, rusqlite::Connection) {
    let path = std::env::temp_dir().join(format!("sp-reports-{name}-{}.sqlite", std::process::id()));
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }
    let conn = index::open(&path).unwrap();
    (path, conn)
}

fn at() -> Vec<notes::EventRef> {
    vec![serde_json::from_value(json!({ "anchor": "e1", "kind": "user", "ts": null })).unwrap()]
}

fn annotate(conn: &rusqlite::Connection, key: &str, kind: notes::Kind, note: &str) -> notes::Annotation {
    notes::save_annotation(conn, notes::Annotation::new(key, kind, note, at(), notes::Author::Person { name: None })).unwrap()
}

fn analyzed(conn: &rusqlite::Connection, key: &str, outcome: notes::Outcome, note: &str) {
    notes::save_meta(conn, &notes::SessionMeta { key: key.into(), outcome, outcome_note: Some(note.into()), ..Default::default() }).unwrap();
}

fn pick(key: &str, title: &str) -> reports::SessionPick {
    serde_json::from_value(json!({ "key": key, "title": title })).unwrap()
}

fn out(structured: Value) -> generate::ClaudeOutput {
    generate::ClaudeOutput { structured, model: "claude-sonnet-5-5".into() }
}

#[test]
fn refs_map_back_to_sessions_and_annotation_versions() {
    let (_, conn) = temp_db("refs");
    analyzed(&conn, "local:claude:a", notes::Outcome::Failed, "Never got the tests passing");
    let stuck = annotate(&conn, "local:claude:a", notes::Kind::AgentStuck, "Retried the same fix four times");
    analyzed(&conn, "local:claude:b", notes::Outcome::Succeeded, "Done");
    let missing = annotate(&conn, "local:claude:b", notes::Kind::MissingContext, "Didn't know about the fixtures folder");

    let inputs = RefCell::new(Vec::new());
    let mut ask = |system: &str, input: &str, _schema: &Value| {
        inputs.borrow_mut().push((system.to_string(), input.to_string()));
        Ok(out(json!({
            "title": "Agents lack test setup knowledge",
            "summary": "Two sessions; one failed.",
            "takeaways": [
                { "text": "Agents get stuck without fixture docs", "refs": ["a1", "[a2]", "s1", "a1"] },
                { "text": "Made up", "refs": ["a99"] }
            ],
            "actions": [
                { "title": "Document fixtures", "category": "instructions", "priority": "high", "change": "Add a Fixtures section to AGENTS.md", "refs": ["a2"] },
                { "title": "No evidence", "category": "codebase", "priority": "low", "change": "x", "refs": ["s9"] }
            ]
        })))
    };
    // The same key twice (two branches of one pi session) counts once.
    let picks = [pick("local:claude:a", "Fix the login test"), pick("local:claude:b", "Add fixtures"), pick("local:claude:a", "Fix the login test")];
    let report = reports::generate(&conn, &picks, &mut ask, &mut |_, _| {}).unwrap();

    let inputs = inputs.into_inner();
    assert_eq!(inputs.len(), 1);
    let input = &inputs[0].1;
    assert!(input.contains("[s1] Session\n"));
    assert!(!input.contains("Fix the login test"), "titles aren't sent to the model");
    assert!(input.contains("outcome: failed. Never got the tests passing"));
    assert!(input.contains("[a1] agent_stuck (by a person): Retried the same fix four times"));
    assert!(input.contains("[a2] missing_context"));

    assert_eq!(report.title, "Agents lack test setup knowledge");
    assert_eq!(report.model, "claude-sonnet-5-5");
    assert_eq!(report.sessions.len(), 2);
    assert_eq!(report.takeaways.len(), 1);
    assert_eq!(
        report.takeaways[0].evidence,
        [
            reports::Evidence::Annotation { id: stuck.id.clone(), version: 1, key: "local:claude:a".into() },
            reports::Evidence::Annotation { id: missing.id.clone(), version: 1, key: "local:claude:b".into() },
            reports::Evidence::Session { key: "local:claude:a".into() },
        ]
    );
    assert_eq!(report.actions.len(), 1);
    assert_eq!((report.actions[0].category, report.actions[0].priority), (reports::ActionCategory::Instructions, reports::Priority::High));
    assert_eq!(report.rejected, 2, "the takeaway and the action citing refs that don't exist");
    assert_eq!((report.stats.sessions, report.stats.annotations), (2, 2));
    assert_eq!(report.stats.outcomes[&notes::Outcome::Failed], 1);
    assert_eq!(report.stats.kinds[&notes::Kind::MissingContext], 1);
}

#[test]
fn long_input_is_sent_in_groups_of_whole_sessions_then_merged() {
    let (_, conn) = temp_db("chunks");
    for (key, note) in [("local:pi:one", "first"), ("local:pi:two", "second"), ("local:pi:three", "third")] {
        annotate(&conn, key, notes::Kind::ToolError, &format!("{note} {}", "x".repeat(200)));
    }
    let calls = RefCell::new(Vec::new());
    let mut ask = |system: &str, input: &str, _schema: &Value| {
        calls.borrow_mut().push((system.to_string(), input.to_string()));
        let n = calls.borrow().len();
        Ok(out(json!({
            "title": format!("part {n}"), "summary": "s",
            "takeaways": [{ "text": format!("takeaway {n}"), "refs": [format!("a{}", n.min(3))] }],
            "actions": []
        })))
    };
    let picks = [pick("local:pi:one", "One"), pick("local:pi:two", "Two"), pick("local:pi:three", "Three")];
    let mut steps = Vec::new();
    let report = reports::generate_in_chunks(&conn, &picks, 300, &mut ask, &mut |done, total| steps.push((done, total))).unwrap();

    let calls = calls.into_inner();
    assert_eq!(calls.len(), 4, "three groups and a merge");
    assert!(calls[1].1.starts_with("Group 2 of 3"));
    // Refs are numbered across every group, so the merge can keep them.
    assert!(calls[2].1.contains("[s3] Session") && calls[2].1.contains("[a3] tool_error"));
    assert!(calls[3].0.starts_with("You combine reports"));
    assert_eq!(calls[3].1.parse::<Value>().unwrap().as_array().unwrap().len(), 3);
    assert_eq!(report.title, "part 4");
    assert_eq!(steps, [(0, 4), (1, 4), (2, 4), (3, 4), (4, 4)]);
}

#[test]
fn sessions_with_nothing_to_report_on_are_refused() {
    let (_, conn) = temp_db("empty");
    let mut ask = |_: &str, _: &str, _: &Value| -> anyhow::Result<generate::ClaudeOutput> { panic!("no call expected") };
    let err = reports::generate(&conn, &[pick("local:claude:none", "Nothing")], &mut ask, &mut |_, _| {}).unwrap_err();
    assert!(err.to_string().contains("Analyze them first"));
}

fn versions(conn: &rusqlite::Connection, id: &str) -> Vec<u32> {
    let mut stmt = conn.prepare("SELECT version FROM annotation_versions WHERE id=?1 ORDER BY version").unwrap();
    stmt.query_map([id], |r| r.get(0)).unwrap().collect::<Result<_, _>>().unwrap()
}

#[test]
fn a_saved_report_keeps_the_versions_it_was_given_until_it_is_deleted() {
    let (_, conn) = temp_db("keep");
    let key = "local:claude:a";
    let edited = annotate(&conn, key, notes::Kind::ToolError, "as reported");
    let deleted = annotate(&conn, key, notes::Kind::Other, "deleted later");
    let mut ask = |_: &str, _: &str, _: &Value| {
        Ok(out(json!({ "title": "t", "summary": "s", "takeaways": [{ "text": "x", "refs": ["a1"] }], "actions": [] })))
    };
    let first = reports::generate(&conn, &[pick(key, "A")], &mut ask, &mut |_, _| {}).unwrap();
    reports::save_report(&conn, &first).unwrap();
    let second = reports::generate(&conn, &[pick(key, "A")], &mut ask, &mut |_, _| {}).unwrap();
    reports::save_report(&conn, &second).unwrap();
    assert_eq!(reports::reports(&conn).unwrap().iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), [second.id.as_str(), first.id.as_str()]);
    assert_eq!(reports::report(&conn, &first.id).unwrap().unwrap(), first);

    notes::save_annotation(&conn, notes::Annotation { note: "edited after the reports".into(), ..edited.clone() }).unwrap();
    notes::delete_annotation(&conn, &deleted.id).unwrap();
    assert_eq!(versions(&conn, &edited.id), [1, 2]);
    assert_eq!(versions(&conn, &deleted.id), [1]);

    // Another report still cites them, so they stay.
    reports::delete_report(&conn, &first.id).unwrap();
    assert_eq!(versions(&conn, &edited.id), [1, 2]);
    assert_eq!(versions(&conn, &deleted.id), [1]);

    reports::delete_report(&conn, &second.id).unwrap();
    assert_eq!(versions(&conn, &edited.id), [2], "only the current version is left");
    assert!(versions(&conn, &deleted.id).is_empty(), "nothing is left of the deleted annotation");
    assert!(reports::reports(&conn).unwrap().is_empty());
}
