// Saved reports through the operations: listed, read, answered and deleted; and a report being
// written can be cancelled.
#[path = "../../testutil/tempdir.rs"]
mod tempdir;
#[path = "common/ops.rs"]
mod common;

use std::sync::Arc;

use common::FakeClaude;
use serde_json::json;
use session_player_core::ops::{
    self, ACTION_RESPONSES, ACTION_VERDICTS, ActionResponseChange, ActionResponsesRequest, ActionVerdictsRequest, CANCEL_REPORT, CancelReportRequest,
    Ctx, DELETE_REPORT, DeleteReportRequest, GENERATE_REPORT, GET_REPORT, GenerateReportRequest, GetReportRequest, JobState, LIST_REPORTS,
    ListReportsRequest, REPORT_ANNOTATIONS, ReportAnnotationsRequest, SET_ACTION_RESPONSE, SetActionResponseRequest,
};
use session_player_core::test_support::{index, reports};
use tempdir::TempDir;

/// A saved report with one action; `judges` adds a verdict on another report's first action.
fn save_report(ctx: &Ctx, id: &str, created_at: i64, judges: Option<&str>) -> reports::Report {
    let impact = match judges {
        Some(earlier) => json!([{
            "action": { "report_id": earlier, "action_index": 0 },
            "action_title": "Document the fixtures",
            "verdict": "worked",
            "reasoning": "Later sessions found the fixtures",
            "evidence": [{ "type": "session", "key": "local:claude:s1" }]
        }]),
        None => json!([]),
    };
    let report: reports::Report = serde_json::from_value(json!({
        "id": id,
        "created_at": created_at,
        "title": format!("Report {id}"),
        "model": "claude-sonnet-5-5",
        "prompt_version": reports::PROMPT_VERSION,
        "sessions": [{ "key": "local:claude:s1", "title": "s1", "outcome": "failed", "outcome_note": "x", "components": [], "annotations": [] }],
        "stats": { "sessions": 1, "outcomes": {}, "kinds": {}, "annotations": 0 },
        "summary": "One session.",
        "takeaways": [],
        "actions": [{ "title": "Document the fixtures", "category": "instructions", "priority": "high", "change": "Add a Fixtures section",
                      "evidence": [{ "type": "session", "key": "local:claude:s1" }] }],
        "impact": impact,
        "rejected": 0
    }))
    .unwrap();
    let conn = index::open(&ctx.paths().db).unwrap();
    reports::save_report(&conn, &report).unwrap();
    report
}

fn listed(ctx: &Arc<Ctx>) -> Vec<String> {
    ops::call(ctx, &LIST_REPORTS, &ListReportsRequest {}).expect("list_reports").into_iter().map(|r| r.id).collect()
}

#[test]
fn a_saved_report_is_listed_read_answered_and_deleted() {
    let dir = TempDir::new("report-ops");
    let ctx = common::open_simple(&dir, None);
    let first = save_report(&ctx, "r1", 1_000, None);
    save_report(&ctx, "r2", 2_000, Some("r1"));

    let mut ids = listed(&ctx);
    ids.sort();
    assert_eq!(ids, ["r1", "r2"]);
    assert_eq!(ops::call(&ctx, &GET_REPORT, &GetReportRequest { id: "r1".to_string() }).expect("get_report"), Some(first));
    assert_eq!(ops::call(&ctx, &REPORT_ANNOTATIONS, &ReportAnnotationsRequest { id: "r1".to_string() }).expect("report_annotations").len(), 0);

    let change = ActionResponseChange {
        report_id: "r1".to_string(),
        action_index: 0,
        status: Some(reports::ActionStatus::Done),
        note: Some("Added it".to_string()),
        user_impact: None,
    };
    let response = ops::call(&ctx, &SET_ACTION_RESPONSE, &SetActionResponseRequest { input: change }).expect("set_action_response").expect("a response");
    assert_eq!((response.status, response.note.as_deref()), (Some(reports::ActionStatus::Done), Some("Added it")));
    let responses = ops::call(&ctx, &ACTION_RESPONSES, &ActionResponsesRequest { id: "r1".to_string() }).expect("action_responses");
    assert_eq!(responses, [response]);

    let verdicts = ops::call(&ctx, &ACTION_VERDICTS, &ActionVerdictsRequest { id: "r1".to_string() }).expect("action_verdicts");
    assert_eq!(verdicts.iter().map(|v| (v.action_index, v.verdict, v.report_id.as_str())).collect::<Vec<_>>(), [(0, reports::Verdict::Worked, "r2")]);

    ops::call(&ctx, &DELETE_REPORT, &DeleteReportRequest { id: "r2".to_string() }).expect("delete_report");

    assert_eq!(listed(&ctx), ["r1"]);
    assert_eq!(ops::call(&ctx, &GET_REPORT, &GetReportRequest { id: "r2".to_string() }).expect("get_report"), None);
}

#[test]
fn a_report_being_written_can_be_cancelled_and_nothing_is_saved() {
    let dir = TempDir::new("report-ops-cancel");
    let fake = FakeClaude::new(&dir);
    let ctx = common::open_simple(&dir, Some(fake.claude()));
    common::session(&dir, "s1", "Work on s1");
    common::reindex(&ctx);
    common::mark_analyzed(&ctx, &common::key(&common::row(&ctx, "s1")));
    fake.hold();
    let paths = vec![common::row(&ctx, "s1").path];
    let id = ops::call(&ctx, &GENERATE_REPORT, &GenerateReportRequest { paths, model: None, focus: None, analyze_first: false }).expect("generate_report");
    fake.wait_for_calls(1);

    ops::call(&ctx, &CANCEL_REPORT, &CancelReportRequest { job: id.0.clone() }).expect("cancel_report");

    assert_eq!(common::wait_done(&ctx, &id).0.state, JobState::Cancelled);
    fake.release();
    assert!(listed(&ctx).is_empty());
}
