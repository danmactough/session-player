// Integration tests run against small synthetic transcripts in tests/fixtures.
use std::path::Path;

use session_player_core::test_support::{index_one, load_branch, Row};

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
fn agent_title_is_used_as_given() {
    let original = std::fs::read_to_string(fixture("claude.jsonl")).unwrap();
    for title in ["/review", "feat", "fix-login-bug", "main"] {
        let path = std::env::temp_dir().join(format!("sp-title-{}-{}.jsonl", std::process::id(), title.replace('/', "_")));
        let body = original.replace(r#""aiTitle":"Flaky login test""#, &format!(r#""aiTitle":"{title}""#));
        std::fs::write(&path, body).unwrap();
        let (row, _) = index_one(path.to_str().unwrap());
        std::fs::remove_file(path).unwrap();
        assert_eq!((row.title.as_str(), row.title_source.as_str()), (title, "ai"));
    }
}

#[test]
fn compressed_codex_session_matches_plain_transcript() {
    let path = std::env::temp_dir().join(format!("sp-compressed-{}.jsonl.zst", std::process::id()));
    let bytes = std::fs::read(fixture("codex.jsonl")).unwrap();
    std::fs::write(&path, zstd::stream::encode_all(bytes.as_slice(), 1).unwrap()).unwrap();
    let (plain, events) = index_one(&fixture("codex.jsonl"));
    let (compressed, compressed_events) = index_one(path.to_str().unwrap());
    std::fs::remove_file(path).unwrap();
    assert_eq!(compressed_events, events);
    assert_eq!(compressed.native_id, plain.native_id);
    assert_eq!(compressed.title, plain.title);
    assert_eq!(compressed.user_turns, plain.user_turns);
    assert_eq!(compressed.input_tokens, plain.input_tokens);
}

#[test]
fn index_discovers_compressed_sessions_and_tracks_updates() {
    use session_player_core::test_support::index;
    let root = std::env::temp_dir().join(format!("sp-compressed-index-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("rollout.jsonl.zst");
    let mut bytes = std::fs::read(fixture("codex.jsonl")).unwrap();
    std::fs::write(&path, zstd::stream::encode_all(bytes.as_slice(), 1).unwrap()).unwrap();
    let mut conn = index::open(&root.join("index.sqlite")).unwrap();
    index::add_root(&conn, root.to_str().unwrap()).unwrap();
    let stats = index::run(&mut conn, false, None, &|_, _| {}).unwrap();
    assert_eq!((stats.scanned, stats.updated, stats.failed), (1, 1, 0));
    assert_eq!(index::list(&conn).unwrap()[0].native_id, "019d-codex");
    assert_eq!(index::run(&mut conn, false, None, &|_, _| {}).unwrap().updated, 0);
    bytes.extend_from_slice(b"\n{\"timestamp\":\"2026-11-02T12:00:00Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Another request added later\"}]}}\n");
    std::fs::write(&path, zstd::stream::encode_all(bytes.as_slice(), 1).unwrap()).unwrap();
    assert_eq!(index::run(&mut conn, false, None, &|_, _| {}).unwrap().updated, 1);
    assert_eq!(index::list(&conn).unwrap()[0].user_turns, 3);
    std::fs::remove_file(&path).unwrap();
    assert_eq!(index::run(&mut conn, false, None, &|_, _| {}).unwrap().removed, 1);
    assert!(index::list(&conn).unwrap().is_empty());
    drop(conn);
    std::fs::remove_dir_all(root).unwrap();
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
    // The skill's SKILL.md stays in the transcript as context.
    assert_eq!(events, vec!["command:/skill:grill-me", "context:long skill body", "user:stress test my caching plan", "assistant:kept branch"]);
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
    assert!(!session_player_core::test_support::is_listed(&fixture("codex-guardian.jsonl")));
}

/// Context events as "kind:name:text", repeats marked with "(again)".
fn context_of(events: &[session_player_core::test_support::model::Event]) -> Vec<String> {
    events
        .iter()
        .filter_map(|e| {
            let c = e.context.as_ref()?;
            let kind = serde_json::to_value(c.kind).unwrap();
            Some(format!("{}:{}:{}{}", kind.as_str().unwrap(), c.name, e.text.trim(), if c.repeat { " (again)" } else { "" }))
        })
        .collect()
}

fn parsed(path: &str) -> Vec<session_player_core::test_support::model::Event> {
    use session_player_core::test_support::parse;
    let p = Path::new(path);
    parse::parse_file(p, parse::detect(p).unwrap()).unwrap().events
}

#[test]
fn claude_instruction_files_skills_and_mcp_instructions_are_context() {
    let events = parsed(&fixture("claude-context.jsonl"));
    assert_eq!(
        context_of(&events),
        [
            "instructions:/home/u/.claude/CLAUDE.md:# Rules\nUse tabs.",
            "instructions:/work/repo/CLAUDE.local.md:Local note.",
            "mcp:docs:## docs\nUse the docs tools for docs.",
            "skill:/home/u/.claude/skills/deploy:# deploy\n\nRun the deploy script.",
            "instructions:/home/u/.claude/CLAUDE.md:# Rules\nUse tabs. (again)",
        ]
    );
    // Other harness messages stay out, and context isn't a prompt.
    assert!(!events.iter().any(|e| e.text.contains("Caveat")));
    let (row, _) = index_one(&fixture("claude-context.jsonl"));
    assert_eq!(row.user_turns, 1);
}

#[test]
fn a_claude_subagent_has_its_agent_definition_as_context() {
    let events = parsed(&fixture("context/claude/abc/subagents/agent-x1.jsonl"));
    assert_eq!(context_of(&events), ["agent:senior-code-reviewer:You are a careful reviewer.\nFind bugs.\n\nNotes: the cwd resets."]);
}

#[test]
fn codex_agents_md_and_skills_are_context() {
    let events = parsed(&fixture("codex-context.jsonl"));
    assert_eq!(
        context_of(&events),
        ["instructions:/work/repo/AGENTS.md:Run tests before committing.", "skill:/home/u/.codex/skills/release/SKILL.md:Tag and push."]
    );
    assert!(!events.iter().any(|e| e.text.contains("environment_context")));
}

#[test]
fn pi_project_instructions_agents_and_skills_are_context() {
    let events = parsed(&fixture("pi-context.jsonl"));
    assert_eq!(
        context_of(&events),
        [
            "agent:worker:You are worker: the implementer.",
            "instructions:/home/u/.pi/agent/AGENTS.md:Be terse.",
            "instructions:/work/repo/AGENTS.md:Use pnpm.",
            "skill:/home/u/.pi/skills/grill-me/SKILL.md:Ask hard questions.",
        ]
    );
    let texts: Vec<_> = events.iter().filter(|e| e.context.is_none()).map(|e| e.text.as_str()).collect();
    assert_eq!(texts, ["/skill:grill-me", "stress test my plan", "Here goes."]);
}

#[test]
fn context_events_are_numbered_apart_from_the_conversation() {
    let events = parsed(&fixture("claude-context.jsonl"));
    let anchors: Vec<_> = events.iter().map(|e| e.anchor.clone().unwrap()).collect();
    // Adding context to a log doesn't renumber the conversation's events.
    assert_eq!(anchors, ["c0", "c1", "c2", "e0", "c3", "e1", "c4"]);
}

#[test]
fn pi_addendum_and_delegate_agents_are_context_and_malformed_tags_are_skipped() {
    let dir = std::env::temp_dir().join(format!("sp-pi-addendum-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("session.jsonl");
    let system = serde_json::json!({
        "type": "message", "id": "s1", "parentId": null, "timestamp": "2026-04-03T10:00:00.000Z",
        "message": { "role": "system", "content": "", "sections": {
            "preamble": "You are a helpful agent. See <br/> here.",
            "addendum": "<addendum>\n<active_agent name=\"delegate\"/>\nHand work to others.\n</addendum>",
            "project_context": "<project_context><project_instructions path=\"/a\"</project_instructions><project_instructions path=\"/b\">Fine.</project_instructions></project_context>"
        } }
    });
    let user = serde_json::json!({ "type": "message", "id": "u1", "parentId": "s1", "timestamp": "2026-04-03T10:00:01.000Z",
        "message": { "role": "user", "content": [{ "type": "text", "text": "<skill name=\"plain\">\nBody mentions location=\"/nope\" in passing.\n</skill>\n\ndo it" }] } });
    std::fs::write(&file, format!("{}\n{}\n{}\n", r#"{"type":"session","version":3,"id":"pi-add","timestamp":"2026-04-03T10:00:00.000Z","cwd":"/w"}"#, system, user)).unwrap();
    let events = parsed(&file.to_string_lossy());
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        context_of(&events),
        [
            "agent:delegate:Hand work to others.",
            "instructions:/b:Fine.",
            "skill:plain:Body mentions location=\"/nope\" in passing.",
        ]
    );
}

/// Each context document's name and scope, as "name (scope)".
fn scopes_of(events: &[session_player_core::test_support::model::Event]) -> Vec<String> {
    events
        .iter()
        .filter_map(|e| e.context.as_ref())
        .filter(|c| !c.repeat)
        .map(|c| format!("{} ({})", c.name, c.scope.as_deref().unwrap_or("-")))
        .collect()
}

#[test]
fn context_documents_say_whose_they_are() {
    assert_eq!(
        scopes_of(&parsed(&fixture("claude-context.jsonl"))),
        ["/home/u/.claude/CLAUDE.md (user)", "/work/repo/CLAUDE.local.md (local)", "docs (-)", "/home/u/.claude/skills/deploy (user)"]
    );
    assert_eq!(
        scopes_of(&parsed(&fixture("codex-context.jsonl"))),
        ["/work/repo/AGENTS.md (repo)", "/home/u/.codex/skills/release/SKILL.md (user)"]
    );
    assert_eq!(
        scopes_of(&parsed(&fixture("pi-context.jsonl"))),
        ["worker (-)", "/home/u/.pi/agent/AGENTS.md (user)", "/work/repo/AGENTS.md (repo)", "/home/u/.pi/skills/grill-me/SKILL.md (user)"]
    );
}

#[test]
fn a_claude_subagent_is_titled_by_its_agent_type_and_description() {
    let (row, _) = index_one(&fixture("context/claude/abc/subagents/agent-x1.jsonl"));
    assert_eq!(row.title, "senior-code-reviewer: Review");
    assert_eq!(row.title_source, "agent");
    assert_eq!(row.user_turns, 1);
    assert_eq!(row.first_prompt.as_deref(), Some("Review the diff"));
}

#[test]
fn a_claude_subagent_without_a_description_is_titled_by_its_prompt() {
    let (row, _) = index_one(&fixture("context/claude/abc/subagents/agent-x2.jsonl"));
    assert_eq!(row.title, "Review the diff");
    assert_eq!(row.title_source, "prompt");
}

#[test]
fn a_claude_subagent_without_a_description_or_prompt_is_titled_by_its_agent_type() {
    let (row, _) = index_one(&fixture("context/claude/abc/subagents/agent-x3.jsonl"));
    assert_eq!(row.title, "senior-code-reviewer");
    assert_eq!(row.title_source, "agent");
}

#[test]
fn a_claude_subagent_with_nothing_to_name_it_is_untitled() {
    let (row, _) = index_one(&fixture("context/claude/abc/subagents/agent-x4.jsonl"));
    assert_eq!(row.title, "Untitled session");
    assert_eq!(row.title_source, "none");
}

#[test]
fn a_claude_subagent_that_only_replied_in_text_is_listed() {
    let (row, _) = index_one(&fixture("context/claude/abc/subagents/agent-x5.jsonl"));
    assert_eq!(row.title, "Review the login change");
    assert_eq!((row.user_turns, row.assistant_msgs, row.tool_calls), (1, 1, 0));
}

const PI_RUN: &str = "10b4d1d8-7eb7-4e35-948c-60b250e4ba99";

fn pi_subagent_log(name: Option<&str>, prompt: &str) -> String {
    let info = name.map_or(String::new(), |n| format!("{{\"type\":\"session_info\",\"id\":\"s1\",\"parentId\":null,\"timestamp\":\"2026-01-03T10:00:00.000Z\",\"name\":\"{n}\"}}\n"));
    let prompt = serde_json::to_string(prompt).unwrap();
    format!(
        "{{\"type\":\"session\",\"version\":3,\"id\":\"child\",\"timestamp\":\"2026-01-03T10:00:00.000Z\",\"cwd\":\"/work/pi\"}}\n{info}{{\"type\":\"message\",\"id\":\"m1\",\"parentId\":\"s1\",\"timestamp\":\"2026-01-03T10:00:01.000Z\",\"message\":{{\"role\":\"user\",\"content\":[{{\"type\":\"text\",\"text\":{prompt}}}]}}}}\n"
    )
}

fn pi_child_notice(key: &str, run: &str) -> String {
    let content = serde_json::to_string(&format!("Workflow child completed: **{key}**\nWorkflow run: 38b3e1b9\nChild run: {run}\nStatus: workflow still running")).unwrap();
    format!("{{\"type\":\"custom_message\",\"customType\":\"subagent-incremental-child-notify\",\"id\":\"c1\",\"parentId\":\"a1\",\"timestamp\":\"2026-01-03T10:00:09.000Z\",\"content\":{content}}}\n")
}

/// Lays out a parent log and one subagent log the way pi writes them, and returns
/// the parent's path, the subagent's path, and the session folder's temp root.
fn pi_subagent_files(test: &str, parent_extra: &str, child: &str) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
    let root = std::env::temp_dir().join(format!("sp-pi-subagent-{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let project = root.join("--work-pi--");
    let stem = "2026-01-03T10-00-00-000Z_01a0cdf2-badc-7337-8fd2-81ab00fb91b7";
    let run_dir = project.join(stem).join("eecbe780-f75c-46aa-80cf-d68570eddea1/run-0");
    std::fs::create_dir_all(&run_dir).unwrap();
    let parent = project.join(format!("{stem}.jsonl"));
    let mut log = std::fs::read_to_string(fixture("pi.jsonl")).unwrap();
    log.push_str(parent_extra);
    std::fs::write(&parent, log).unwrap();
    let path = run_dir.join("session.jsonl");
    std::fs::write(&path, child).unwrap();
    (parent, path, root)
}

fn pi_subagent_title(test: &str, parent_extra: &str, child: &str) -> (String, String) {
    let (_, path, root) = pi_subagent_files(test, parent_extra, child);
    let (row, _) = index_one(path.to_str().unwrap());
    std::fs::remove_dir_all(&root).unwrap();
    (row.title, row.title_source)
}

#[test]
fn a_pi_subagent_is_titled_by_its_agent_and_workflow_key() {
    let child = pi_subagent_log(Some(&format!("subagent-senior-code-reviewer-{PI_RUN}-1")), "You are resolving a ticket");
    let notices = pi_child_notice("other-child", "3af4ce52-bfec-473a-9c17-2b9b08edfc18") + &pi_child_notice("perry-local", PI_RUN);
    assert_eq!(pi_subagent_title("key", &notices, &child), ("senior-code-reviewer: perry-local".to_string(), "agent".to_string()));
}

#[test]
fn a_pi_subagent_without_a_workflow_key_is_titled_by_its_agent_and_prompt() {
    let child = pi_subagent_log(Some(&format!("subagent-worker-{PI_RUN}-1")), "Task: \nYou are resolving a wayfinder ticket");
    assert_eq!(pi_subagent_title("no-key", "", &child), ("worker: resolving a wayfinder ticket".to_string(), "prompt".to_string()));
}

#[test]
fn an_automatic_workflow_key_says_nothing_so_the_prompt_is_used() {
    let child = pi_subagent_log(Some(&format!("subagent-delegate-{PI_RUN}-1")), "Research strand D");
    assert_eq!(pi_subagent_title("auto-key", &pi_child_notice("task-3", PI_RUN), &child), ("delegate: Research strand D".to_string(), "prompt".to_string()));
}

#[test]
fn a_forked_pi_subagents_title_skips_the_fork_instructions() {
    let prompt = "Task: You are a delegated subagent running from a fork of the parent session. Treat it as reference only.\n\nTask:\n\nYou are resolving ticket 01.";
    let child = pi_subagent_log(Some(&format!("subagent-worker-{PI_RUN}-1")), prompt);
    assert_eq!(pi_subagent_title("fork", "", &child), ("worker: resolving ticket 01.".to_string(), "prompt".to_string()));
}

#[test]
fn a_pi_subagent_with_no_recorded_agent_is_titled_by_its_prompt_alone() {
    let child = pi_subagent_log(None, "You are resolving ticket 02");
    assert_eq!(pi_subagent_title("no-agent", "", &child), ("resolving ticket 02".to_string(), "prompt".to_string()));
}

#[test]
fn a_pi_subagent_picks_up_its_key_when_the_parent_is_notified_later() {
    use session_player_core::test_support::index;
    let child = pi_subagent_log(Some(&format!("subagent-worker-{PI_RUN}-1")), "You are resolving ticket 03");
    let (parent, _, root) = pi_subagent_files("later", "", &child);
    let mut conn = index::open(&root.join("index.sqlite")).unwrap();
    index::add_root(&conn, root.to_str().unwrap()).unwrap();
    index::run(&mut conn, false, None, &|_, _| {}).unwrap();
    let title = |conn: &rusqlite::Connection| index::list(conn).unwrap().into_iter().find(|r| r.is_subagent).unwrap().title;
    assert_eq!(title(&conn), "worker: resolving ticket 03");

    let mut log = std::fs::read_to_string(&parent).unwrap();
    log.push_str(&pi_child_notice("perry-local", PI_RUN));
    std::fs::write(&parent, log).unwrap();
    index::run(&mut conn, false, None, &|_, _| {}).unwrap();
    assert_eq!(title(&conn), "worker: perry-local");
    std::fs::remove_dir_all(&root).unwrap();
}

fn codex_subagent_title(test: &str, source: &str, fork: bool) -> (String, String) {
    let fork = if fork { r#""forked_from_id":"parent","# } else { "" };
    let log = format!(
        "{{\"timestamp\":\"2026-11-04T01:56:33.164Z\",\"type\":\"session_meta\",\"payload\":{{\"id\":\"child\",{fork}\"cwd\":\"/work/repo\",\"timestamp\":\"2026-11-04T01:56:33.164Z\",\"source\":{source}}}}}\n{{\"timestamp\":\"2026-11-04T01:56:34.000Z\",\"type\":\"response_item\",\"payload\":{{\"type\":\"message\",\"role\":\"user\",\"content\":[{{\"type\":\"input_text\",\"text\":\"You are checking the login flow\"}}]}}}}\n"
    );
    let path = std::env::temp_dir().join(format!("sp-codex-subagent-{test}-{}.jsonl", std::process::id()));
    std::fs::write(&path, log).unwrap();
    let (row, _) = index_one(path.to_str().unwrap());
    std::fs::remove_file(&path).unwrap();
    assert!(row.is_subagent);
    (row.title, row.title_source)
}

fn thread_spawn(role: &str, path: &str) -> String {
    format!(r#"{{"subagent":{{"thread_spawn":{{"parent_thread_id":"parent","depth":1,{path}"agent_nickname":"Parfit","agent_role":{role}}}}}}}"#)
}

#[test]
fn a_codex_subagent_is_titled_by_its_role_and_task_name() {
    let source = thread_spawn(r#""reviewer""#, r#""agent_path":"/root/check/login-flow","#);
    assert_eq!(codex_subagent_title("role", &source, true), ("reviewer: login-flow".to_string(), "agent".to_string()));
}

#[test]
fn a_codex_subagent_with_no_role_is_titled_by_its_nickname() {
    let source = thread_spawn("null", r#""agent_path":"/root/explorer","#);
    assert_eq!(codex_subagent_title("nickname", &source, true), ("Parfit: explorer".to_string(), "agent".to_string()));
}

#[test]
fn a_codex_subagent_without_a_task_name_is_titled_by_its_agent_and_prompt() {
    let source = thread_spawn(r#""reviewer""#, "");
    assert_eq!(codex_subagent_title("prompt", &source, false), ("reviewer: checking the login flow".to_string(), "prompt".to_string()));
}

#[test]
fn a_forked_codex_subagent_does_not_take_its_parents_prompt_as_a_title() {
    let source = thread_spawn(r#""reviewer""#, "");
    assert_eq!(codex_subagent_title("fork", &source, true), ("reviewer".to_string(), "agent".to_string()));
}

#[test]
fn a_codex_review_subagent_is_titled_by_its_kind_and_prompt() {
    assert_eq!(codex_subagent_title("review", r#"{"subagent":"review"}"#, false), ("review: checking the login flow".to_string(), "prompt".to_string()));
}

#[test]
fn a_forked_codex_session_keeps_its_own_id_after_its_parents_copied_metadata() {
    let meta = |id: &str, source: &str| format!("{{\"timestamp\":\"2026-11-04T01:56:33.164Z\",\"type\":\"session_meta\",\"payload\":{{\"id\":\"{id}\",\"forked_from_id\":\"parent\",\"cwd\":\"/work/{id}\",\"source\":{source}}}}}\n");
    let log = meta("child", &thread_spawn("null", r#""agent_path":"/root/explorer","#))
        + &meta("parent", r#""exec""#)
        + "{\"timestamp\":\"2026-11-04T01:56:34.000Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"The parent's prompt\"}]}}\n";
    let path = std::env::temp_dir().join(format!("sp-codex-second-meta-{}.jsonl", std::process::id()));
    std::fs::write(&path, log).unwrap();
    let (row, _) = index_one(path.to_str().unwrap());
    std::fs::remove_file(&path).unwrap();
    assert_eq!(row.native_id, "child");
    assert_eq!(row.cwd.as_deref(), Some("/work/child"));
    assert_eq!(row.parent_id.as_deref(), Some("parent"));
    assert_eq!(row.title, "Parfit: explorer");
}
