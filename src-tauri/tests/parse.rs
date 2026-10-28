// Integration tests run against small synthetic transcripts in tests/fixtures.
use std::path::Path;

use session_player_lib::test_support::{index_one, Row};

fn fixture(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name).to_string_lossy().into_owned()
}

#[test]
fn claude_session() {
    let (row, events): (Row, Vec<String>) = index_one(&fixture("claude.jsonl"));
    assert_eq!(row.source, "claude");
    assert_eq!(row.title, "Flaky login test");
    assert_eq!(row.title_source, "ai");
    // Worktrees fold back into the repository they came from.
    assert_eq!(row.project, "/work/repo");
    assert_eq!(row.user_turns, 1);
    assert_eq!(row.tool_calls, 1);
    // Usage repeated across content-block lines of one message counts once.
    assert_eq!(row.output_tokens, 5);
    assert_eq!(row.first_prompt.as_deref(), Some("Fix the flaky login test please"));
    assert_eq!(events, vec!["command:/model haiku", "user:Fix the flaky login test please", "thinking:hmm", "tool_call:{\n  \"command\": \"npm test\"\n}", "tool_result!:1 failing"]);
}

#[test]
fn codex_session() {
    let (row, events) = index_one(&fixture("codex.jsonl"));
    assert_eq!(row.source, "codex");
    assert_eq!(row.native_id, "019d-codex");
    // A one-word opener is skipped in favor of the next descriptive prompt.
    assert_eq!(row.title, "add retry support to the uploader");
    assert_eq!(row.input_tokens, 200);
    assert_eq!(row.cache_read_tokens, 800);
    assert!(events.iter().any(|e| e.starts_with("tool_result!:")), "non-zero exit marks an error: {events:?}");
    assert!(!events.iter().any(|e| e.contains("environment_context")));
    assert_eq!(row.imported_from, None);
}

#[test]
fn codex_session_imported_from_claude_code() {
    // The import record sits next to the sessions folder, as in ~/.codex.
    let home = std::env::temp_dir().join(format!("sp-codex-import-{}", std::process::id()));
    let day = home.join("sessions/2026/01/02");
    std::fs::create_dir_all(&day).unwrap();
    let rollout = day.join("rollout-2026-01-02T10-00-00-019d-codex.jsonl");
    std::fs::copy(fixture("codex.jsonl"), &rollout).unwrap();
    std::fs::write(
        home.join("external_agent_session_imports.json"),
        r#"{"records":[{"source_path":"/Users/x/.claude/projects/-work/abc.jsonl","imported_thread_id":"019d-codex"}]}"#,
    )
    .unwrap();
    let (row, _) = index_one(&rollout.to_string_lossy());
    std::fs::remove_dir_all(&home).unwrap();
    assert_eq!(row.imported_from.as_deref(), Some("Claude Code"));
}

#[test]
fn pi_session() {
    let (row, events) = index_one(&fixture("pi.jsonl"));
    assert_eq!(row.source, "pi");
    assert_eq!(row.title, "Caching plan review");
    assert_eq!(row.models, vec!["glm-5".to_string()]);
    assert_eq!(events, vec!["command:/skill:grill-me", "user:stress test my caching plan", "assistant:kept branch"]);
    // Tokens include every model call, abandoned branches too.
    assert_eq!(row.output_tokens, 3);
}
