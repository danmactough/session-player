use std::path::Path;

use serde_json::Value;

use super::{content_text, for_each_line, parse_ts, pretty, str_of};
use crate::model::{Event, EventKind, Parsed, Source};

/// User-role messages that Codex injects on its own (environment, instructions).
const INJECTED_PREFIXES: &[&str] = &[
    "<environment_context>",
    "<user_instructions>",
    "<permissions",
    "<INSTRUCTIONS>",
    "# AGENTS.md",
    "<skill>",
    "<turn_aborted>",
    "<collaboration_mode>",
    "<apps_instructions>",
    "<plugins_instructions>",
    "<skills_instructions>",
];

pub fn parse(path: &Path) -> anyhow::Result<Parsed> {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    // rollout-<timestamp>-<uuid>: the uuid is the last 36 characters.
    let fallback_id = if stem.len() >= 36 { &stem[stem.len() - 36..] } else { stem };
    let mut p = Parsed::new(Source::Codex, fallback_id.to_string());
    let mut model: Option<String> = None;
    let mut guardian = false;

    for_each_line(path, |line| {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            return;
        };
        let ts = parse_ts(v.get("timestamp"));
        let payload = v.get("payload").unwrap_or(&Value::Null);
        match str_of(&v, "type").unwrap_or("") {
            "session_meta" => {
                if let Some(id) = str_of(payload, "id") {
                    p.native_id = id.to_string();
                }
                p.cwd = str_of(payload, "cwd").map(str::to_string);
                p.git_branch = payload
                    .get("git")
                    .and_then(|g| str_of(g, "branch"))
                    .map(str::to_string);
                p.started_at = parse_ts(payload.get("timestamp")).or(ts);
                // The guardian subagent only approves or refuses the main
                // agent's riskier actions; its reviews aren't worth reading.
                guardian = find_key(payload.get("source").unwrap_or(&Value::Null), "other").as_deref() == Some("guardian")
                    || str_of(payload, "thread_source") == Some("guardian_review");
                if payload.get("source").is_some_and(Value::is_object) {
                    p.is_subagent = true;
                    p.parent_id = find_key(payload.get("source").unwrap(), "parent_thread_id");
                }
                if let Some(fork) = str_of(payload, "forked_from_id") {
                    p.parent_id = Some(fork.to_string());
                }
            }
            "turn_context" => {
                if let Some(m) = str_of(payload, "model") {
                    model = Some(m.to_string());
                    p.add_model(m);
                }
                if p.cwd.is_none() {
                    p.cwd = str_of(payload, "cwd").map(str::to_string);
                }
            }
            "compacted" => {
                let text = str_of(payload, "message").unwrap_or("").to_string();
                p.events.push(Event::new(EventKind::Compaction, ts, text));
            }
            "event_msg" => match str_of(payload, "type") {
                Some("token_count") => {
                    if let Some(u) = payload.get("info").and_then(|i| i.get("total_token_usage")) {
                        let n = |k: &str| u.get(k).and_then(Value::as_i64).unwrap_or(0);
                        p.tokens.cache_read = n("cached_input_tokens");
                        p.tokens.input = n("input_tokens") - p.tokens.cache_read;
                        p.tokens.output = n("output_tokens");
                    }
                }
                Some("turn_aborted") => {
                    p.events.push(Event::new(EventKind::System, ts, "Turn aborted"));
                }
                Some("item_completed") => {
                    if let Some(item) = payload.get("item") {
                        for mut e in completed_item(item, ts) {
                            if e.kind == EventKind::ToolCall {
                                e.model = model.clone();
                            }
                            p.events.push(e);
                        }
                    }
                }
                _ => {}
            },
            "response_item" => handle_item(&mut p, payload, ts, model.as_deref()),
            _ => {}
        }
    })?;
    if guardian {
        // A session with no events isn't listed.
        p.events.clear();
    }
    drop_code_mode_wrappers(&mut p.events);
    super::mark_failed_exits(&mut p.events);
    Ok(p)
}

fn handle_item(p: &mut Parsed, item: &Value, ts: Option<i64>, model: Option<&str>) {
    let mut e = match str_of(item, "type").unwrap_or("") {
        "message" => {
            let role = str_of(item, "role").unwrap_or("");
            if role != "user" && role != "assistant" {
                return;
            }
            let mut images = Vec::new();
            let text = content_text(item.get("content").unwrap_or(&Value::Null), &mut images);
            let trimmed = text.trim();
            if role == "user" {
                if INJECTED_PREFIXES.iter().any(|pre| trimmed.starts_with(pre)) {
                    return;
                }
                if let Some((reference, request)) = chatgpt_handoff(trimmed, ts) {
                    p.events.push(reference);
                    let mut e = Event::new(EventKind::User, ts, request);
                    e.images = images;
                    p.events.push(e);
                    return;
                }
                if trimmed.starts_with("<user_action>") || trimmed.starts_with("<user_shell_command>") {
                    let mut e = Event::new(EventKind::System, ts, trimmed);
                    if trimmed.starts_with("<user_shell_command>") {
                        e.kind = EventKind::Command;
                    }
                    p.events.push(e);
                    return;
                }
            }
            if trimmed.is_empty() && images.is_empty() {
                return;
            }
            if role == "user" {
                let mut e = match super::claude::user_text_event(trimmed, ts) {
                    Some(e) => e,
                    None if !images.is_empty() => Event::new(EventKind::User, ts, ""),
                    None => return,
                };
                e.images = images;
                e
            } else if trimmed.contains("[external_agent_tool_") {
                for mut e in imported_events(trimmed, ts, &p.events) {
                    e.model = model.map(str::to_string);
                    p.events.push(e);
                }
                return;
            } else {
                Event::new(EventKind::Assistant, ts, trimmed)
            }
        }
        "reasoning" => {
            let mut imgs = Vec::new();
            let text = content_text(item.get("summary").unwrap_or(&Value::Null), &mut imgs);
            if text.trim().is_empty() {
                return;
            }
            Event::new(EventKind::Thinking, ts, text)
        }
        "function_call" | "custom_tool_call" | "local_shell_call" => {
            let input = item.get("arguments").or_else(|| item.get("input")).or_else(|| item.get("action"));
            let mut e = Event::new(EventKind::ToolCall, ts, pretty(input.unwrap_or(&Value::Null)));
            e.tool_name = str_of(item, "name").map(str::to_string).or(Some("shell".into()));
            e.call_id = str_of(item, "call_id").map(str::to_string);
            e
        }
        "web_search_call" | "tool_search_call" => {
            let name = str_of(item, "type").unwrap().trim_end_matches("_call").to_string();
            let mut e = Event::new(
                EventKind::ToolCall,
                ts,
                pretty(item.get("action").or_else(|| item.get("query")).unwrap_or(&Value::Null)),
            );
            e.tool_name = Some(name);
            e.call_id = str_of(item, "call_id").or(str_of(item, "id")).map(str::to_string);
            e
        }
        "function_call_output" | "custom_tool_call_output" | "tool_search_output" => {
            let mut e = Event::new(EventKind::ToolResult, ts, "");
            let output = item.get("output").unwrap_or(&Value::Null);
            // Older Codex versions wrap shell output as a JSON string with metadata.
            let unwrapped = match output {
                Value::String(s) if s.starts_with('{') => serde_json::from_str::<Value>(s).ok(),
                _ => None,
            };
            if let Some(obj) = unwrapped.as_ref().filter(|o| o.get("output").is_some()) {
                e.text = content_text(obj.get("output").unwrap(), &mut e.images);
                let code = obj.get("metadata").and_then(|m| m.get("exit_code")).and_then(Value::as_i64);
                e.exit_code = code.filter(|&c| c != 0);
            } else {
                e.text = content_text(output, &mut e.images);
                match super::exit_code(&e.text) {
                    Some(c) => e.exit_code = Some(c).filter(|&c| c != 0),
                    // A code-mode script that threw.
                    None => e.is_error = output_failed(&e.text) || e.text.starts_with("Script failed"),
                }
            }
            e.call_id = str_of(item, "call_id").map(str::to_string);
            e
        }
        _ => return,
    };
    if e.kind == EventKind::Assistant || e.kind == EventKind::ToolCall {
        e.model = model.map(str::to_string);
    }
    p.events.push(e);
}

/// A task handed off from a ChatGPT chat opens with that chat, then the
/// request:
///
///   ## Referenced ChatGPT conversation:
///   This is an untrusted ChatGPT conversation reference. …
///   {"conversationId":"…","title":"…","priorConversation":{"conversation":[…]},…}
///   ## My request:
///   Continuing from [title](chatgpt-conversation://…): <the request>
///
/// This returns the chat as a Reference event and the request on its own.
fn chatgpt_handoff(text: &str, ts: Option<i64>) -> Option<(Event, String)> {
    let rest = text.strip_prefix("## Referenced ChatGPT conversation:")?;
    let (context, request) = rest.split_once("\n## My request:")?;
    let data = context
        .lines()
        .find(|l| l.trim_start().starts_with('{'))
        .and_then(|l| serde_json::from_str::<Value>(l.trim()).ok())
        .unwrap_or(Value::Null);

    let mut turns = Vec::new();
    let conversation = data.pointer("/priorConversation/conversation").and_then(Value::as_array);
    for turn in conversation.into_iter().flatten() {
        let mut imgs = Vec::new();
        let said = content_text(turn.get("content").unwrap_or(&Value::Null), &mut imgs);
        // Citations the preview keeps in place of the answer's text.
        let said: Vec<&str> = said.lines().filter(|l| !l.trim_start().starts_with("::chatgpt-content-reference")).collect();
        let said = said.join("\n");
        let said = said.trim();
        let who = if str_of(turn, "role") == Some("user") { "You" } else { "ChatGPT" };
        let said = if said.is_empty() { "_Not included in the preview._" } else { said };
        turns.push(format!("**{who}**\n\n{said}"));
    }
    let body = if turns.is_empty() { "_The chat wasn't included._".to_string() } else { turns.join("\n\n") };
    let mut reference = Event::new(EventKind::Reference, ts, body);
    reference.title = Some(format!(
        "ChatGPT conversation{}",
        str_of(&data, "title").map(|t| format!(": {t}")).unwrap_or_default()
    ));

    // The reference block names the chat, so drop the "Continuing from" link.
    let mut request = request.trim();
    if let Some(i) = request.find("):").filter(|_| request.starts_with("Continuing from [")) {
        request = request[i + 2..].trim_start();
    }
    Some((reference, request.to_string()))
}

/// Codex's code mode runs one JavaScript `exec` tool that calls the real
/// tools (`tools.exec_command(...)`, MCP tools, file edits) inside a script.
/// Codex logs each of those as a completed item, so these turn the items
/// into tool calls and results, and `drop_code_mode_wrappers` removes the
/// scripts they came from.
fn completed_item(item: &Value, ts: Option<i64>) -> Vec<Event> {
    let id = format!("item-{}", str_of(item, "id").unwrap_or(""));
    let started = item.get("started_at_ms").and_then(Value::as_i64).or(ts);
    let failed = matches!(str_of(item, "status"), Some("failed" | "declined"));
    let (name, input, mut result) = match str_of(item, "type").unwrap_or("") {
        "CommandExecution" => {
            let command = match item.get("command") {
                // ["/bin/zsh", "-lc", "<script>"]: show the script.
                Some(Value::Array(a)) => {
                    let parts: Vec<&str> = a.iter().filter_map(Value::as_str).collect();
                    match parts.as_slice() {
                        [_, flag, script] if flag.starts_with('-') && flag.ends_with('c') => script.to_string(),
                        _ => parts.join(" "),
                    }
                }
                Some(Value::String(s)) => s.clone(),
                _ => String::new(),
            };
            let mut input = serde_json::json!({ "command": command });
            if let Some(cwd) = str_of(item, "cwd") {
                input["workdir"] = cwd.trim_start_matches("file://").into();
            }
            // The desktop app's sandbox prints this before every command's output.
            let mut output = str_of(item, "aggregated_output").unwrap_or("");
            while let Some(rest) = output.strip_prefix("Not privileged to set domain environment.\n") {
                output = rest;
            }
            let mut r = Event::new(EventKind::ToolResult, ts, output);
            match item.get("exit_code").and_then(Value::as_i64) {
                Some(code) => r.exit_code = Some(code).filter(|&c| c != 0),
                None => r.is_error = failed,
            }
            ("shell".to_string(), input, r)
        }
        "FileChange" => {
            let changes = item.get("changes").and_then(Value::as_object);
            let paths: Vec<&str> = changes.into_iter().flatten().map(|(path, _)| path.as_str()).collect();
            let summary: Vec<String> = changes
                .into_iter()
                .flatten()
                .map(|(path, c)| {
                    let kind = str_of(c, "type").unwrap_or("update");
                    let body = str_of(c, "unified_diff").or_else(|| str_of(c, "content")).unwrap_or("");
                    format!("{kind} {path}\n{body}")
                })
                .collect();
            let mut r = Event::new(EventKind::ToolResult, ts, summary.join("\n"));
            if failed {
                r.is_error = true;
                r.text = str_of(item, "stderr").filter(|s| !s.is_empty()).unwrap_or("The edit failed").to_string();
            }
            ("apply_patch".to_string(), serde_json::json!({ "path": paths.join(", ") }), r)
        }
        "McpToolCall" => {
            let name = format!("mcp__{}__{}", str_of(item, "server").unwrap_or("mcp"), str_of(item, "tool").unwrap_or(""));
            let mut imgs = Vec::new();
            let content = item.get("result").and_then(|r| r.get("content")).unwrap_or(&Value::Null);
            let mut r = Event::new(EventKind::ToolResult, ts, content_text(content, &mut imgs));
            r.images = imgs;
            r.is_error = failed;
            let args = match item.get("arguments") {
                Some(Value::String(s)) => serde_json::from_str(s).unwrap_or(Value::String(s.clone())),
                Some(v) => v.clone(),
                None => Value::Null,
            };
            (name, args, r)
        }
        _ => return Vec::new(),
    };
    let mut call = Event::new(EventKind::ToolCall, started, pretty(&input));
    call.tool_name = Some(name);
    call.call_id = Some(id.clone());
    result.call_id = Some(id);
    vec![call, result]
}

/// Removes code-mode `exec` scripts whose tool calls were logged as items
/// (each item follows the script that started it), and the `wait` calls
/// that only poll a running script. A script stays when it called no tools
/// (plain JavaScript), or when it failed and none of its tools did, so each
/// failure shows (and counts) once.
fn drop_code_mode_wrappers(events: &mut Vec<Event>) {
    use std::collections::HashSet;
    let is_exec = |e: &Event| e.kind == EventKind::ToolCall && e.tool_name.as_deref() == Some("exec");
    if !events.iter().any(is_exec) {
        return;
    }
    let failed: HashSet<String> = events
        .iter()
        .filter(|e| e.kind == EventKind::ToolResult && e.is_error)
        .filter_map(|e| e.call_id.clone())
        .collect();
    let mut drop: HashSet<String> = HashSet::new();
    // Scripts with an item whose tool failed.
    let mut failed_inside: HashSet<String> = HashSet::new();
    let mut script: Option<String> = None;
    for e in events.iter() {
        let Some(id) = e.call_id.clone() else { continue };
        if is_exec(e) {
            script = Some(id);
        } else if id.starts_with("item-") {
            if let Some(s) = &script {
                drop.insert(s.clone());
                if failed.contains(&id) {
                    failed_inside.insert(s.clone());
                }
            }
        } else if e.kind == EventKind::ToolCall && e.tool_name.as_deref() == Some("wait") {
            drop.insert(id);
        }
    }
    drop.retain(|id| !failed.contains(id) || failed_inside.contains(id));
    events.retain(|e| !e.call_id.as_ref().is_some_and(|id| drop.contains(id)));
}

/// Sessions Codex imported from other agents flatten every tool call and
/// result into assistant text:
///
///   [external_agent_tool_call: Bash]
///   description: List hook files
///   command: ls -la hooks/
///   [/external_agent_tool_call]
///   [external_agent_tool_result: error]
///   ...output...
///   [/external_agent_tool_result]
///
/// This splits such text back into tool calls (with the `key: value` lines
/// turned into a JSON object) and results. Results are paired with the most
/// recent call, since the markers carry no ids.
fn imported_events(text: &str, ts: Option<i64>, prior: &[Event]) -> Vec<Event> {
    const CALL: &str = "[external_agent_tool_call:";
    const CALL_END: &str = "[/external_agent_tool_call]";
    const RESULT: &str = "[external_agent_tool_result";
    const RESULT_END: &str = "[/external_agent_tool_result]";

    let mut out: Vec<Event> = Vec::new();
    let mut calls = prior
        .iter()
        .filter(|e| e.kind == EventKind::ToolCall && e.call_id.as_deref().is_some_and(|c| c.starts_with("ext-")))
        .count();
    let mut rest = text;
    while !rest.is_empty() {
        let next_call = rest.find(CALL);
        let next_result = rest.find(RESULT);
        let start = match (next_call, next_result) {
            (Some(a), Some(b)) => a.min(b),
            (Some(a), None) | (None, Some(a)) => a,
            (None, None) => rest.len(),
        };
        let before = rest[..start].trim();
        if !before.is_empty() {
            out.push(Event::new(EventKind::Assistant, ts, before));
        }
        rest = &rest[start..];
        if rest.is_empty() {
            break;
        }
        let is_call = rest.starts_with(CALL);
        let (end_marker, open) = if is_call { (CALL_END, CALL) } else { (RESULT_END, RESULT) };
        let header_end = rest.find(']').unwrap_or(rest.len() - 1);
        let label = rest[open.len()..header_end].trim_start_matches(':').trim();
        let body_end = rest.find(end_marker).unwrap_or(rest.len());
        let body = rest[(header_end + 1).min(body_end)..body_end].trim_matches('\n');
        if is_call {
            calls += 1;
            let mut e = Event::new(EventKind::ToolCall, ts, key_values_json(body));
            e.tool_name = Some(label.to_string());
            e.call_id = Some(format!("ext-{calls}"));
            out.push(e);
        } else {
            let mut e = Event::new(EventKind::ToolResult, ts, body);
            e.is_error = label == "error";
            e.call_id = Some(format!("ext-{calls}"));
            out.push(e);
        }
        rest = rest.get(body_end + end_marker.len()..).unwrap_or("");
    }
    out
}

/// "key: value" lines (values may continue onto following lines) -> pretty JSON.
fn key_values_json(body: &str) -> String {
    let mut map = serde_json::Map::new();
    let mut current: Option<(String, String)> = None;
    for line in body.lines() {
        let key = line
            .split_once(": ")
            .map(|(k, v)| (k, v))
            .or_else(|| line.strip_suffix(':').map(|k| (k, "")))
            .filter(|(k, _)| !k.is_empty() && k.len() <= 30 && k.chars().all(|c| c.is_ascii_lowercase() || c == '_'));
        match key {
            Some((k, v)) => {
                if let Some((pk, pv)) = current.take() {
                    map.insert(pk, Value::String(pv));
                }
                current = Some((k.to_string(), v.to_string()));
            }
            None => match current.as_mut() {
                Some((_, v)) => {
                    v.push('\n');
                    v.push_str(line);
                }
                None => return body.to_string(),
            },
        }
    }
    if let Some((k, v)) = current {
        map.insert(k, Value::String(v));
    }
    serde_json::to_string_pretty(&Value::Object(map)).unwrap_or_else(|_| body.to_string())
}

/// Codex tool output carries no error flag. A call failed when the output
/// opens with "<tool> failed…" ("exec_command failed for …: SandboxDenied",
/// "apply_patch verification failed: …"). A command that ran and reported an
/// exit code ("Process exited with code 2") is handled separately.
fn output_failed(text: &str) -> bool {
    let head: String = text.chars().take(400).collect();
    let first = head.lines().next().unwrap_or("");
    // The prefix must look like a tool name (exec_command, write_stdin), so
    // ordinary prose that mentions a failure does not count.
    first.find(" failed").is_some_and(|i| {
        let words: Vec<&str> = first[..i].split(' ').collect();
        words.len() <= 2
            && words[0].contains('_')
            && words.iter().all(|w| w.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
    })
}

fn find_key(v: &Value, key: &str) -> Option<String> {
    match v {
        Value::Object(map) => {
            if let Some(s) = map.get(key).and_then(Value::as_str) {
                return Some(s.to_string());
            }
            map.values().find_map(|x| find_key(x, key))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::output_failed;

    #[test]
    fn detects_failed_tool_output() {
        assert!(output_failed("exec_command failed for `ls`: SandboxDenied { message: \"no\" }"));
        assert!(output_failed("apply_patch verification failed: Failed to find expected lines"));
        assert!(!output_failed("Success. Updated the following files:\nM README.md"));
        assert!(!output_failed("the test failed earlier but passes now"));
    }
}
