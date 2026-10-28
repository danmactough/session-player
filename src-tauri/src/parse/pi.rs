use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde_json::Value;

use super::{between, content_text, for_each_line, parse_ts, pretty, str_of};
use crate::model::{Event, EventKind, Parsed, Source};

pub fn parse(path: &Path) -> anyhow::Result<Parsed> {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let fallback_id = stem.rsplit('_').next().unwrap_or(stem);
    let mut p = Parsed::new(Source::Pi, fallback_id.to_string());

    // pi stores a session as a tree (entries point at parentId) so the user can
    // rewind and branch. Collect everything, then render only the branch that
    // ends at the last entry, which is what the user last saw.
    let mut entries: Vec<Value> = Vec::new();
    let mut name: Option<String> = None;
    for_each_line(path, |line| {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            return;
        };
        match str_of(&v, "type").unwrap_or("") {
            "session" => {
                if let Some(id) = str_of(&v, "id") {
                    p.native_id = id.to_string();
                }
                p.cwd = str_of(&v, "cwd").map(str::to_string);
                p.started_at = parse_ts(v.get("timestamp"));
                if let Some(parent) = str_of(&v, "parentSession") {
                    p.parent_id = Path::new(parent)
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .and_then(|s| s.rsplit('_').next())
                        .map(str::to_string);
                }
            }
            "session_info" => name = str_of(&v, "name").map(str::to_string),
            _ => entries.push(v),
        }
    })?;

    // Nested files (<session-dir>/<run-id>/<file>.jsonl) are subagent runs.
    let in_project_dir = path
        .parent()
        .and_then(|d| d.file_name())
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with("--"));
    let in_forks = path.components().any(|c| c.as_os_str() == "forks");
    if !in_project_dir && !in_forks {
        p.is_subagent = true;
        if p.parent_id.is_none() {
            p.parent_id = path
                .ancestors()
                .filter_map(|a| a.file_name().and_then(|n| n.to_str()))
                .find(|n| n.contains('_') && n.len() > 36)
                .and_then(|n| n.rsplit('_').next())
                .map(str::to_string);
        }
    }
    if let Some(n) = &name {
        if n.starts_with("subagent-") {
            p.is_subagent = true;
        } else {
            p.title = Some((n.clone(), "name"));
        }
    }

    for v in active_branch(&entries) {
        handle_entry(&mut p, v);
    }
    // Token usage counts every model call, including abandoned branches.
    for v in &entries {
        if let Some(u) = v.get("message").filter(|m| str_of(m, "role") == Some("assistant")).and_then(|m| m.get("usage")) {
            let n = |k: &str| u.get(k).and_then(Value::as_i64).unwrap_or(0);
            p.tokens.input += n("input") + n("cacheWrite");
            p.tokens.output += n("output");
            p.tokens.cache_read += n("cacheRead");
        }
    }
    Ok(p)
}

fn active_branch(entries: &[Value]) -> Vec<&Value> {
    let by_id: HashMap<&str, &Value> = entries
        .iter()
        .filter_map(|v| str_of(v, "id").map(|id| (id, v)))
        .collect();
    let Some(leaf) = entries.iter().rev().find(|v| str_of(v, "id").is_some()) else {
        return entries.iter().collect();
    };
    let mut chain = vec![leaf];
    let mut seen = HashSet::new();
    let mut cur = leaf;
    while let Some(pid) = str_of(cur, "parentId") {
        if !seen.insert(pid) {
            break;
        }
        match by_id.get(pid) {
            Some(parent) => {
                chain.push(parent);
                cur = parent;
            }
            None => break,
        }
    }
    // If the chain is implausibly short the file is probably linear without
    // parent links; fall back to file order.
    if chain.len() * 4 < by_id.len() {
        return entries.iter().collect();
    }
    chain.reverse();
    chain
}

fn handle_entry(p: &mut Parsed, v: &Value) {
    let ts = parse_ts(v.get("timestamp"));
    match str_of(v, "type").unwrap_or("") {
        "model_change" => {
            if let Some(m) = str_of(v, "modelId") {
                p.add_model(m);
            }
        }
        "compaction" | "branch_summary" => {
            let text = str_of(v, "summary").unwrap_or("").to_string();
            p.events.push(Event::new(EventKind::Compaction, ts, text));
        }
        "custom_message" => {
            let mut imgs = Vec::new();
            let text = content_text(v.get("content").unwrap_or(&Value::Null), &mut imgs);
            if !text.trim().is_empty() {
                p.events.push(Event::new(EventKind::System, ts, text));
            }
        }
        "message" => {
            let Some(m) = v.get("message") else { return };
            let ts = ts.or_else(|| parse_ts(m.get("timestamp")));
            let content = m.get("content").unwrap_or(&Value::Null);
            match str_of(m, "role").unwrap_or("") {
                "user" => {
                    let mut e = Event::new(EventKind::User, ts, "");
                    e.text = content_text(content, &mut e.images).trim().to_string();
                    // Skill invocations inline the whole SKILL.md ahead of the user's text.
                    if e.text.starts_with("<skill name=\"") {
                        let name = between(&e.text, "<skill name=\"", "\"").unwrap_or("skill");
                        p.events.push(Event::new(EventKind::Command, ts, format!("/skill:{name}")));
                        e.text = match e.text.find("</skill>") {
                            Some(i) => e.text[i + "</skill>".len()..].trim().to_string(),
                            None => String::new(),
                        };
                    }
                    if !e.text.is_empty() || !e.images.is_empty() {
                        p.events.push(e);
                    }
                }
                "assistant" => {
                    let model = str_of(m, "model").map(str::to_string);
                    if let Some(md) = &model {
                        p.add_model(md);
                    }
                    let Value::Array(blocks) = content else { return };
                    for b in blocks {
                        let mut e = match str_of(b, "type") {
                            Some("text") => {
                                let s = str_of(b, "text").unwrap_or("");
                                if s.trim().is_empty() {
                                    continue;
                                }
                                Event::new(EventKind::Assistant, ts, s)
                            }
                            Some("thinking") => {
                                let s = str_of(b, "thinking").unwrap_or("");
                                if s.trim().is_empty() {
                                    continue;
                                }
                                Event::new(EventKind::Thinking, ts, s)
                            }
                            Some("toolCall") => {
                                let mut e = Event::new(
                                    EventKind::ToolCall,
                                    ts,
                                    pretty(b.get("arguments").unwrap_or(&Value::Null)),
                                );
                                e.tool_name = str_of(b, "name").map(str::to_string);
                                e.call_id = str_of(b, "id").map(str::to_string);
                                e
                            }
                            _ => continue,
                        };
                        e.model = model.clone();
                        p.events.push(e);
                    }
                    if str_of(m, "stopReason") == Some("error") {
                        let msg = str_of(m, "errorMessage").unwrap_or("Model error");
                        let mut e = Event::new(EventKind::System, ts, msg);
                        e.is_error = true;
                        p.events.push(e);
                    }
                }
                "toolResult" => {
                    let mut e = Event::new(EventKind::ToolResult, ts, "");
                    e.text = content_text(content, &mut e.images);
                    e.call_id = str_of(m, "toolCallId").map(str::to_string);
                    e.tool_name = str_of(m, "toolName").map(str::to_string);
                    e.is_error = m.get("isError").and_then(Value::as_bool) == Some(true);
                    p.events.push(e);
                }
                "bashExecution" => {
                    let id = format!("bash-{}", str_of(v, "id").unwrap_or(""));
                    let cmd = str_of(m, "command").unwrap_or("");
                    let mut call = Event::new(EventKind::Command, ts, format!("! {cmd}"));
                    call.call_id = Some(id.clone());
                    p.events.push(call);
                    let out = str_of(m, "output").unwrap_or("");
                    if !out.trim().is_empty() {
                        let mut e = Event::new(EventKind::System, ts, out);
                        e.is_error = m.get("exitCode").and_then(Value::as_i64).is_some_and(|c| c != 0);
                        p.events.push(e);
                    }
                }
                _ => {}
            }
        }
        _ => {}
    }
}
