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
                _ => {}
            },
            "response_item" => handle_item(&mut p, payload, ts, model.as_deref()),
            _ => {}
        }
    })?;
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
                e.is_error = code.is_some_and(|c| c != 0);
            } else {
                e.text = content_text(output, &mut e.images);
                e.is_error = exit_code_from_text(&e.text).is_some_and(|c| c != 0);
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

fn exit_code_from_text(text: &str) -> Option<i64> {
    let head: String = text.chars().take(400).collect();
    let idx = head.find("exited with code ")?;
    head[idx + 17..]
        .split(|c: char| !c.is_ascii_digit() && c != '-')
        .next()?
        .parse()
        .ok()
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
