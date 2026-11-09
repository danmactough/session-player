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
    let report = reports::generate(&conn, &picks, None, &mut ask, &mut |_, _| {}).unwrap();

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
    let report = reports::generate_in_chunks(&conn, &picks, None, 300, &mut ask, &mut |done, total| steps.push((done, total))).unwrap();

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
    let err = reports::generate(&conn, &[pick("local:claude:none", "Nothing")], None, &mut ask, &mut |_, _| {}).unwrap_err();
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
    let first = reports::generate(&conn, &[pick(key, "A")], None, &mut ask, &mut |_, _| {}).unwrap();
    reports::save_report(&conn, &first).unwrap();
    let second = reports::generate(&conn, &[pick(key, "A")], None, &mut ask, &mut |_, _| {}).unwrap();
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

#[test]
fn a_report_shows_annotations_as_it_was_given_them() {
    let (_, conn) = temp_db("cited");
    let key = "local:claude:a";
    let kept = annotate(&conn, key, notes::Kind::ToolError, "as reported");
    let gone = annotate(&conn, key, notes::Kind::Other, "deleted later");
    let mut ask = |_: &str, _: &str, _: &Value| Ok(out(json!({ "title": "t", "summary": "s", "takeaways": [{ "text": "x", "refs": ["a1"] }], "actions": [] })));
    let report = reports::generate(&conn, &[pick(key, "A")], None, &mut ask, &mut |_, _| {}).unwrap();
    reports::save_report(&conn, &report).unwrap();
    notes::save_annotation(&conn, notes::Annotation { note: "edited".into(), ..kept.clone() }).unwrap();
    notes::delete_annotation(&conn, &gone.id).unwrap();

    let cited = reports::cited_annotations(&conn, &report).unwrap();
    let find = |id: &str| cited.iter().find(|c| c.annotation.id == id).unwrap();
    assert_eq!((find(&kept.id).annotation.note.as_str(), find(&kept.id).current_version), ("as reported", Some(2)));
    assert_eq!((find(&gone.id).annotation.note.as_str(), find(&gone.id).current_version), ("deleted later", None));
}

#[test]
fn a_report_sends_and_records_the_users_own_focus() {
    let (_, conn) = temp_db("focus");
    let key = "local:claude:a";
    annotate(&conn, key, notes::Kind::ToolError, "npm test failed");
    let systems = RefCell::new(Vec::new());
    let mut ask = |system: &str, _: &str, _: &Value| {
        systems.borrow_mut().push(system.to_string());
        Ok(out(json!({ "title": "t", "summary": "s", "takeaways": [{ "text": "x", "refs": ["a1"] }], "actions": [] })))
    };
    let default = reports::generate(&conn, &[pick(key, "A")], None, &mut ask, &mut |_, _| {}).unwrap();
    let same = reports::generate(&conn, &[pick(key, "A")], Some(reports::DEFAULT_FOCUS), &mut ask, &mut |_, _| {}).unwrap();
    let custom = reports::generate(&conn, &[pick(key, "A")], Some(" Focus on test flakiness. "), &mut ask, &mut |_, _| {}).unwrap();
    let systems = systems.into_inner();
    assert!(systems[0].ends_with(reports::DEFAULT_FOCUS));
    assert!(systems[2].ends_with("\nFocus on test flakiness."));
    assert_eq!(default.focus, None);
    assert_eq!(same.focus, None);
    assert_eq!(custom.focus.as_deref(), Some("Focus on test flakiness."));
    reports::save_report(&conn, &custom).unwrap();
    assert_eq!(reports::report(&conn, &custom.id).unwrap().unwrap().focus.as_deref(), Some("Focus on test flakiness."));
}

// Responses to recommended actions, and how later reports use them.

fn index_session(conn: &rusqlite::Connection, native_id: &str, project: &str, started_at: i64) {
    conn.execute(
        "INSERT INTO sessions(path, root, source, native_id, project, title, title_source, started_at, user_turns, assistant_msgs,
            tool_calls, input_tokens, output_tokens, cache_read_tokens, models, is_subagent, size)
         VALUES(?1, '/', 'claude', ?2, ?3, 't', 'auto', ?4, 0, 0, 0, 0, 0, 0, '[]', 0, 0)",
        rusqlite::params![format!("/{native_id}.jsonl"), native_id, project, started_at],
    )
    .unwrap();
}

fn earlier_report(conn: &rusqlite::Connection, native_id: &str, project: &str) -> reports::Report {
    index_session(conn, native_id, project, 1_000);
    let key = format!("local:claude:{native_id}");
    annotate(conn, &key, notes::Kind::MissingContext, "Didn't know about the fixtures folder");
    let mut ask = |_: &str, _: &str, _: &Value| {
        Ok(out(json!({
            "title": "Earlier", "summary": "s", "takeaways": [], "impact": [],
            "actions": [{ "title": "Document fixtures", "category": "instructions", "priority": "high", "change": "Add a Fixtures section",
                          "success_check": "Notes about fixtures stop appearing", "refs": ["s1"] }]
        })))
    };
    let report = reports::generate(conn, &[pick(&key, "Earlier")], None, &mut ask, &mut |_, _| {}).unwrap();
    reports::save_report(conn, &report).unwrap();
    report
}

fn respond(conn: &rusqlite::Connection, report: &reports::Report, body: Value) -> Option<reports::ActionResponse> {
    let mut input = json!({ "report_id": report.id, "action_index": 0, "status": null, "note": null, "user_impact": null });
    input.as_object_mut().unwrap().extend(body.as_object().unwrap().clone());
    reports::set_action_response(conn, &serde_json::from_value(input).unwrap()).unwrap()
}

/// Generates a report on `natives` with a canned reply, returning it and the input the model got.
fn next_report(conn: &rusqlite::Connection, natives: &[&str], reply: Value) -> (reports::Report, String) {
    let picks: Vec<_> = natives
        .iter()
        .map(|n| {
            let key = format!("local:claude:{n}");
            annotate(conn, &key, notes::Kind::ToolError, "npm test failed");
            pick(&key, n)
        })
        .collect();
    let input = RefCell::new(String::new());
    let mut ask = |_: &str, i: &str, _: &Value| {
        *input.borrow_mut() = i.to_string();
        Ok(out(reply.clone()))
    };
    let report = reports::generate(conn, &picks, None, &mut ask, &mut |_, _| {}).unwrap();
    (report, input.into_inner())
}

fn empty_reply() -> Value {
    json!({ "title": "t", "summary": "s", "takeaways": [{ "text": "x", "refs": ["s1"] }], "actions": [], "impact": [] })
}

#[test]
fn a_response_saves_updates_and_goes_with_its_report() {
    let (_, conn) = temp_db("response");
    let report = earlier_report(&conn, "a", "/p");
    let done = respond(&conn, &report, json!({ "status": "done", "note": " added it " })).unwrap();
    assert_eq!(done.status, Some(reports::ActionStatus::Done));
    assert_eq!(done.note.as_deref(), Some("added it"));
    let done_at = done.done_at.expect("done_at is set");

    let again = respond(&conn, &report, json!({ "status": "done", "note": "added it", "user_impact": "did_not_help" })).unwrap();
    assert_eq!(again.done_at, Some(done_at), "staying Done keeps the date");
    assert_eq!(again.user_impact, Some(reports::UserImpact::DidNotHelp));

    let moved = respond(&conn, &report, json!({ "status": "in_progress", "note": "redoing it", "user_impact": "did_not_help" })).unwrap();
    assert_eq!((moved.done_at, moved.user_impact), (None, None), "only a Done action has a date and an assessment");
    assert_eq!(reports::action_responses(&conn, &report.id).unwrap(), [moved]);

    assert_eq!(respond(&conn, &report, json!({})), None, "no status and no note is no response");
    assert!(reports::action_responses(&conn, &report.id).unwrap().is_empty());

    respond(&conn, &report, json!({ "status": "declined" }));
    reports::delete_report(&conn, &report.id).unwrap();
    assert!(reports::action_responses(&conn, &report.id).unwrap().is_empty());
}

#[test]
fn a_response_needs_an_action_that_exists() {
    let (_, conn) = temp_db("response-missing");
    let report = earlier_report(&conn, "a", "/p");
    let input = serde_json::from_value(json!({ "report_id": report.id, "action_index": 5, "status": "done", "note": null, "user_impact": null })).unwrap();
    assert!(reports::set_action_response(&conn, &input).is_err());
}

#[test]
fn only_responses_from_reports_sharing_a_project_reach_the_prompt() {
    let (_, conn) = temp_db("projects");
    let same = earlier_report(&conn, "same", "/shared");
    let other = earlier_report(&conn, "other", "/elsewhere");
    let unknown = earlier_report(&conn, "unknown", "(unknown)");
    respond(&conn, &same, json!({ "status": "declined", "note": "we use a monorepo tool for this" }));
    respond(&conn, &other, json!({ "status": "declined", "note": "not this project" }));
    respond(&conn, &unknown, json!({ "status": "declined", "note": "no project at all" }));

    index_session(&conn, "new1", "/shared", 5_000);
    index_session(&conn, "new2", "/third", 5_000);
    index_session(&conn, "new3", "(unknown)", 5_000);
    let (_, input) = next_report(&conn, &["new1", "new2", "new3"], empty_reply());
    assert!(input.contains("[p1] Earlier action: \"Document fixtures\" (instructions)"));
    assert!(input.contains("change: Add a Fixtures section"));
    assert!(input.contains("success check: Notes about fixtures stop appearing"));
    assert!(input.contains("response: declined. Note: we use a monorepo tool for this"));
    assert!(!input.contains("not this project"));
    assert!(!input.contains("no project at all"));
    assert!(!input.contains("[p2]"));
}

#[test]
fn a_report_with_no_responses_sends_only_the_sessions() {
    let (_, conn) = temp_db("no-responses");
    earlier_report(&conn, "a", "/p");
    index_session(&conn, "b", "/p", 5_000);
    let (_, input) = next_report(&conn, &["b"], empty_reply());
    assert!(input.starts_with("[s1] Session"));
}

#[test]
fn the_prompt_lists_sessions_started_after_an_action_was_done() {
    let (_, conn) = temp_db("later");
    let report = earlier_report(&conn, "a", "/p");
    let done_at = respond(&conn, &report, json!({ "status": "done", "note": "added", "user_impact": "did_not_help" })).unwrap().done_at.unwrap();
    index_session(&conn, "before", "/p", done_at - 1_000);
    index_session(&conn, "after", "/p", done_at + 1_000);
    let (_, input) = next_report(&conn, &["before", "after"], empty_reply());
    assert!(input.contains("sessions in this report started after it was done: s2\n"), "{input}");
    assert!(input.contains("developer's assessment: did not help"));
}

fn verdict_reply(verdict: &str, refs: Value) -> Value {
    let mut reply = empty_reply();
    reply["impact"] = json!([{ "action": "p1", "verdict": verdict, "reasoning": "The notes say so", "refs": refs }]);
    reply
}

/// An earlier action marked `status`, and sessions "before" and "after" it.
fn before_and_after(conn: &rusqlite::Connection, status: &str) -> reports::Report {
    let report = earlier_report(conn, "a", "/p");
    let at = respond(conn, &report, json!({ "status": status })).unwrap().done_at.unwrap_or(10_000);
    index_session(conn, "before", "/p", at - 1_000);
    index_session(conn, "after", "/p", at + 1_000);
    report
}

#[test]
fn did_not_work_citing_later_sessions_is_kept_and_shown_on_the_earlier_action() {
    let (_, conn) = temp_db("verdict");
    let earlier = before_and_after(&conn, "done");
    let (report, _) = next_report(&conn, &["before", "after"], verdict_reply("did_not_work", json!(["s2", "s1"])));
    assert_eq!(report.rejected, 0);
    assert_eq!(report.impact.len(), 1);
    let v = &report.impact[0];
    assert_eq!(v.verdict, reports::Verdict::DidNotWork);
    assert_eq!((v.action.report_id.as_str(), v.action.action_index), (earlier.id.as_str(), 0));
    assert_eq!(v.action_title, "Document fixtures");
    assert_eq!(v.evidence.len(), 2, "earlier sessions may be cited alongside later ones");

    reports::save_report(&conn, &report).unwrap();
    let latest = reports::action_verdicts(&conn, &earlier.id).unwrap();
    assert_eq!(latest.len(), 1);
    assert_eq!((latest[0].action_index, latest[0].verdict, latest[0].report_id.as_str()), (0, reports::Verdict::DidNotWork, report.id.as_str()));

    // The next report is told what the last one concluded.
    index_session(&conn, "third", "/p", i64::MAX / 2);
    let (_, input) = next_report(&conn, &["third"], empty_reply());
    assert!(input.contains("latest verdict: did_not_work"), "{input}");
}

#[test]
fn a_verdict_citing_only_earlier_sessions_is_rejected() {
    let (_, conn) = temp_db("verdict-earlier");
    before_and_after(&conn, "done");
    let (report, _) = next_report(&conn, &["before", "after"], verdict_reply("did_not_work", json!(["s1"])));
    assert!(report.impact.is_empty());
    assert_eq!(report.rejected, 1);
}

#[test]
fn a_verdict_on_an_action_not_done_is_rejected() {
    let (_, conn) = temp_db("verdict-not-done");
    before_and_after(&conn, "in_progress");
    let (report, _) = next_report(&conn, &["before", "after"], verdict_reply("worked", json!(["s2"])));
    assert!(report.impact.is_empty());
    assert_eq!(report.rejected, 1);
}

#[test]
fn too_early_needs_no_citations_but_needs_later_sessions() {
    let (_, conn) = temp_db("too-early");
    before_and_after(&conn, "done");
    let (report, _) = next_report(&conn, &["before", "after"], verdict_reply("too_early", json!([])));
    assert_eq!(report.impact.len(), 1);
    let (report, _) = next_report(&conn, &["before"], verdict_reply("too_early", json!([])));
    assert!(report.impact.is_empty(), "no later sessions, so nothing to assess");
    assert_eq!(report.rejected, 1);
}

#[test]
fn an_action_can_follow_up_an_earlier_one_and_unknown_refs_are_cleared() {
    let (_, conn) = temp_db("follow-up");
    let earlier = earlier_report(&conn, "a", "/p");
    respond(&conn, &earlier, json!({ "status": "declined" }));
    index_session(&conn, "b", "/p", 5_000);
    let action = |title: &str, follows: &str| json!({ "title": title, "category": "codebase", "priority": "low", "change": "c", "success_check": "k", "follows_up": follows, "refs": ["s1"] });
    let (report, _) = next_report(&conn, &["b"], json!({ "title": "t", "summary": "s", "takeaways": [], "impact": [], "actions": [action("Revisit", "p1"), action("Unknown", "p9")] }));
    assert_eq!(report.actions.len(), 2);
    let follow = report.actions[0].follows_up.as_ref().unwrap();
    assert_eq!((follow.report_id.as_str(), follow.action_index, follow.title.as_str()), (earlier.id.as_str(), 0, "Document fixtures"));
    assert_eq!(report.actions[0].success_check.as_deref(), Some("k"));
    assert!(report.actions[1].follows_up.is_none());
}

#[test]
fn the_merge_call_gets_the_earlier_actions_too() {
    let (_, conn) = temp_db("merge-prior");
    let earlier = earlier_report(&conn, "a", "/p");
    respond(&conn, &earlier, json!({ "status": "declined", "note": "no thanks" }));
    for n in ["one", "two"] {
        index_session(&conn, n, "/p", 5_000);
        annotate(&conn, &format!("local:claude:{n}"), notes::Kind::ToolError, &"x".repeat(200));
    }
    let calls = RefCell::new(Vec::new());
    let mut ask = |system: &str, input: &str, _: &Value| {
        calls.borrow_mut().push((system.to_string(), input.to_string()));
        Ok(out(empty_reply()))
    };
    let picks = [pick("local:claude:one", "One"), pick("local:claude:two", "Two")];
    reports::generate_in_chunks(&conn, &picks, None, 100, &mut ask, &mut |_, _| {}).unwrap();
    let calls = calls.into_inner();
    assert_eq!(calls.len(), 3);
    assert!(calls.iter().all(|(_, input)| input.contains("[p1] Earlier action") && input.contains("no thanks")));
}

#[test]
fn a_report_written_before_responses_existed_still_loads() {
    let old = json!({
        "id": "r", "created_at": 1, "title": "t", "model": "m", "prompt_version": 2, "sessions": [],
        "stats": { "sessions": 0, "outcomes": {}, "kinds": {}, "annotations": 0 },
        "summary": "s", "takeaways": [], "rejected": 0,
        "actions": [{ "title": "a", "category": "codebase", "priority": "low", "change": "c", "evidence": [] }]
    });
    let report: reports::Report = serde_json::from_value(old).unwrap();
    assert!(report.impact.is_empty());
    assert_eq!((report.actions[0].success_check.clone(), report.actions[0].follows_up.clone()), (None, None));
}
