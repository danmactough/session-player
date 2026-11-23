// A session's notes through the operations: saved, listed, counted and deleted.
#[path = "../../testutil/tempdir.rs"]
mod tempdir;
#[path = "common/ops.rs"]
mod common;

use std::path::Path;

use session_player_core::ops::{
    self, DELETE_ANNOTATION, DeleteAnnotationRequest, NOTE_SUMMARIES, NoteSummariesRequest, SAVE_ANNOTATION, SAVE_META, SESSION_NOTES,
    SaveAnnotationRequest, SaveMetaRequest, SessionNotesRequest,
};
use session_player_core::test_support::{annotations, parse};
use tempdir::TempDir;

#[test]
fn notes_on_a_session_are_saved_listed_counted_and_deleted_and_anchors_carry_the_matching_event_ts() {
    let dir = TempDir::new("notes-ops");
    let ctx = common::open_simple(&dir, None);
    common::session(&dir, "s1", "Add a health check endpoint");
    common::reindex(&ctx);
    let row = common::row(&ctx, "s1");
    let key = common::key(&row);
    let events = parse::parse_file(Path::new(&row.path), parse::detect(Path::new(&row.path)).unwrap()).unwrap().events;
    let first = events.iter().find(|e| e.anchor.as_deref() == Some("e1")).expect("an e1 event");
    let anchor: annotations::EventRef =
        serde_json::from_value(serde_json::json!({ "anchor": "e1", "kind": first.kind, "ts": first.ts })).unwrap();

    let meta = annotations::SessionMeta {
        key: key.clone(),
        outcome: annotations::Outcome::Failed,
        outcome_note: Some("Never got the endpoint wired".to_string()),
        components: vec!["api".to_string()],
        ..Default::default()
    };
    let saved_meta = ops::call(&ctx, &SAVE_META, &SaveMetaRequest { meta }).expect("save_meta").expect("the stored metadata");
    assert_eq!(saved_meta.outcome, annotations::Outcome::Failed);
    assert_eq!(saved_meta.components, ["api"]);

    let note = annotations::Annotation::new(&key, annotations::Kind::ToolError, "The build broke", vec![anchor], annotations::Author::Person { name: None });
    let saved = ops::call(&ctx, &SAVE_ANNOTATION, &SaveAnnotationRequest { annotation: note }).expect("save_annotation");
    assert!(!saved.id.is_empty());
    assert_eq!(saved.version, 1);

    let notes = ops::call(&ctx, &SESSION_NOTES, &SessionNotesRequest { key: key.clone(), path: row.path.clone(), branch: None }).expect("session_notes");
    assert_eq!(notes.meta.as_ref().map(|m| m.outcome), Some(annotations::Outcome::Failed));
    assert_eq!(notes.annotations.iter().map(|a| a.id.clone()).collect::<Vec<_>>(), [saved.id.clone()]);
    assert!(notes.annotations[0].anchors[0].found, "the anchor is found in the transcript");
    assert_eq!(notes.revision, None, "nothing was generated");

    let summaries = ops::call(&ctx, &NOTE_SUMMARIES, &NoteSummariesRequest {}).expect("note_summaries");
    let summary = summaries.iter().find(|s| s.key == key).expect("a summary for the session");
    assert_eq!((summary.outcome, summary.person_annotations, summary.analyzed), (annotations::Outcome::Failed, 1, false));

    ops::call(&ctx, &DELETE_ANNOTATION, &DeleteAnnotationRequest { id: saved.id }).expect("delete_annotation");

    let notes = ops::call(&ctx, &SESSION_NOTES, &SessionNotesRequest { key: key.clone(), path: row.path, branch: None }).expect("session_notes");
    assert!(notes.annotations.is_empty());
    let summaries = ops::call(&ctx, &NOTE_SUMMARIES, &NoteSummariesRequest {}).expect("note_summaries");
    assert_eq!(summaries.iter().find(|s| s.key == key).map(|s| s.person_annotations), Some(0));
}
