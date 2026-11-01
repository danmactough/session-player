// Integration tests run against small synthetic transcripts in tests/fixtures.
use std::path::Path;

use session_player_lib::test_support::{index_one, load_branch, Row};

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
    // A failing test run is a command that ran, not a tool error.
    assert_eq!(row.error_count, 0);
    assert_eq!(row.exit_count, 1);
    assert_eq!(events, vec!["command:/model haiku", "user:Fix the flaky login test please", "thinking:hmm", "tool_call:{\n  \"command\": \"npm test\"\n}", "tool_result~:Exit code 1\n1 failing"]);
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
    assert!(events.iter().any(|e| e.starts_with("tool_result~:")), "non-zero exit marks a failing exit: {events:?}");
    assert_eq!((row.error_count, row.exit_count), (0, 1));
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

#[test]
fn pi_alternate_branches_are_listed_and_loadable() {
    let path = fixture("pi.jsonl");
    let (row, _) = index_one(&path);
    assert_eq!(row.branches.len(), 1);
    let b = &row.branches[0];
    assert_eq!(b.leaf, "a3");
    assert_eq!(b.label, "Retry: stress test my caching plan");
    assert_eq!(b.fork_turn, 1);
    assert_eq!(b.fork_entry.as_deref(), Some("a2"));
    assert_eq!(b.parent_leaf, None);
    assert_eq!(b.user_turns, 0);
    // A branch starts where it split off.
    assert_eq!(load_branch(&path, "a3"), vec!["assistant:abandoned branch"]);
}

#[test]
fn pi_branches_of_branches_nest_under_their_source() {
    let path = fixture("pi-tree.jsonl");
    let (row, events) = index_one(&path);
    assert_eq!(events, vec!["user:first question here please", "assistant:answer one", "user:follow up on the main branch", "assistant:main reply"]);
    let summary: Vec<_> = row
        .branches
        .iter()
        .map(|b| (b.leaf.as_str(), b.label.as_str(), b.parent_leaf.as_deref(), b.fork_entry.as_deref(), b.fork_turn))
        .collect();
    assert_eq!(
        summary,
        vec![
            ("b1c", "try a different approach here", None, Some("m2"), 1),
            ("b2a", "go deeper on that idea", Some("b1c"), Some("b1a"), 2),
        ]
    );
    assert_eq!(
        load_branch(&path, "b1c"),
        vec!["user:try a different approach here", "assistant:different answer", "user:keep going with the alternative", "assistant:kept going"]
    );
    assert_eq!(load_branch(&path, "b2a"), vec!["user:go deeper on that idea", "assistant:deeper answer"]);
}

#[test]
fn pi_fork_file_starts_after_the_copied_history() {
    let dir = std::env::temp_dir().join(format!("sp-pi-fork-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let parent = dir.join("2026-02-01T10-00-00-000Z_01b-tree.jsonl");
    std::fs::copy(fixture("pi-tree.jsonl"), &parent).unwrap();
    let lines: Vec<String> = std::fs::read_to_string(&parent).unwrap().lines().map(str::to_string).collect();
    let fork = dir.join("2026-02-02T10-00-00-000Z_01c-fork.jsonl");
    std::fs::write(
        &fork,
        [
            format!(r#"{{"type":"session","version":3,"id":"01c-fork","timestamp":"2026-02-02T10:00:00.000Z","cwd":"/work/tree","parentSession":"{}"}}"#, parent.display()),
            lines[1].clone(),
            lines[2].clone(),
            r#"{"type":"message","id":"f1","parentId":"m2","timestamp":"2026-02-02T10:00:01.000Z","message":{"role":"user","content":"forked question about caching"}}"#.into(),
        ]
        .join("\n"),
    )
    .unwrap();
    let (row, events) = index_one(&fork.to_string_lossy());
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(row.parent_id.as_deref(), Some("01b-tree"));
    assert_eq!(row.fork_entry.as_deref(), Some("m2"));
    assert_eq!(row.fork_turn, Some(1));
    assert_eq!(row.title, "forked question about caching");
    assert_eq!(events, vec!["user:forked question about caching"]);
}

#[test]
fn codex_imported_tool_markers_become_tool_events() {
    let (row, events) = index_one(&fixture("codex-imported.jsonl"));
    assert_eq!(row.tool_calls, 2);
    assert_eq!(row.error_count, 1);
    assert_eq!(
        events,
        vec![
            "user:debug the shutdown loop in this plugin",
            "assistant:I'll investigate the plugin.",
            "tool_call:{\n  \"command\": \"ls -la hooks/ &&\\n  cat plugin.json\",\n  \"description\": \"List hooks\"\n}",
            "tool_result:stop.sh\nhooks.json",
            "assistant:Now the hook.",
            "tool_call:{\n  \"file\": \"hooks/stop.sh\"\n}",
            "tool_result!:File not found",
        ]
    );
}

#[test]
fn claude_file_mentions_are_shown_inside_the_prompt() {
    let (row, events) = index_one(&fixture("claude-attach.jsonl"));
    assert_eq!(row.title, "@/tmp/handoff.md pick this up");
    assert_eq!(
        events,
        vec!["user:@/tmp/handoff.md pick this up\n<file name=\"/tmp/handoff.md\"># Handoff\n- step one</file>\n", "assistant:On it."]
    );
}

#[test]
fn codex_chats_outside_projects_share_a_project() {
    // The Codex app gives each chat outside a project its own folder under
    // ~/Documents/Codex and lists those threads in its global state.
    let home = std::env::temp_dir().join(format!("sp-codex-projectless-{}", std::process::id()));
    let day = home.join(".codex/sessions/2026/10/28");
    std::fs::create_dir_all(&day).unwrap();
    std::fs::write(home.join(".codex/.codex-global-state.json"), r#"{"projectless-thread-ids":["019d-listed"]}"#).unwrap();
    let src = std::fs::read_to_string(fixture("codex.jsonl")).unwrap();
    let root = home.join("Documents/Codex");
    let mut projects = Vec::new();
    for (id, cwd) in [
        // Listed as projectless.
        ("019d-listed", root.join("2026-10-28/refresh-the-shared-simulator")),
        // Not listed, but working inside the projectless folder.
        ("019d-inside", root.join("2026-10-29/fix-the-readme")),
        // An ordinary project.
        ("019d-repo", home.join("code/app")),
    ] {
        std::fs::create_dir_all(&cwd).unwrap();
        let file = day.join(format!("rollout-{id}.jsonl"));
        std::fs::write(&file, src.replace("019d-codex", id).replace("/work/other", &cwd.to_string_lossy())).unwrap();
        projects.push(index_one(&file.to_string_lossy()).0.project);
    }
    std::fs::remove_dir_all(&home).unwrap();
    let root = root.to_string_lossy().into_owned();
    assert_eq!(projects, vec![root.clone(), root, home.join("code/app").to_string_lossy().into_owned()]);
}

#[test]
fn codex_code_mode_shows_the_tools_a_script_ran() {
    let (row, events) = index_one(&fixture("codex-code-mode.jsonl"));
    // The ChatGPT chat the task came from is context, not part of the prompt.
    assert_eq!(row.first_prompt.as_deref(), Some("Add retries to the uploader"));
    assert_eq!(row.user_turns, 1);
    assert_eq!(
        events,
        vec![
            "reference:**You**\n\nhow should the uploader retry?\n\n**ChatGPT**\n\n_Not included in the preview._",
            "user:Add retries to the uploader",
            // Each script is replaced by the tools it called. A script that
            // failed because its tool did goes too, so the failure counts once.
            "tool_call:{\n  \"command\": \"npm test\",\n  \"workdir\": \"/work/app\"\n}",
            "tool_result~:1 failing",
            "tool_call:{\n  \"q\": 1\n}",
            "tool_result!:GitHub API error 422",
            "tool_call:{\n  \"path\": \"/work/app/retry.js\"\n}",
            "tool_result:add /work/app/retry.js\nexport {}\n",
            "assistant:Added retries.",
        ]
    );
    assert_eq!((row.error_count, row.exit_count), (1, 1));
}

#[test]
fn codex_guardian_reviews_are_not_listed() {
    assert!(!session_player_lib::test_support::is_listed(&fixture("codex-guardian.jsonl")));
}
