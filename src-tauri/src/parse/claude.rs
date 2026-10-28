use std::collections::HashMap;
use std::path::Path;

use serde_json::Value;

use super::{between, content_text, for_each_line, parse_ts, pretty, str_of, strip_tag_blocks};
use crate::model::{Event, EventKind, Parsed, Source};

pub fn parse(path: &Path) -> anyhow::Result<Parsed> {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
    let mut p = Parsed::new(Source::Claude, stem.clone());

    // Subagent transcripts live at <session-id>/subagents/<agent>.jsonl.
    let parent = path.parent();
    if parent.and_then(|d| d.file_name()).and_then(|n| n.to_str()) == Some("subagents") {
        p.is_subagent = true;
        p.parent_id = parent
            .and_then(|d| d.parent())
            .and_then(|d| d.file_name())
            .and_then(|n| n.to_str())
            .map(str::to_string);
    }

    let mut custom_title: Option<String> = None;
    let mut ai_title: Option<String> = None;
    let mut agent_name: Option<String> = None;
    let mut summary: Option<String> = None;
    // Claude Code writes one line per content block, repeating the message's
    // usage on each, so usage is keyed by message id and summed at the end.
    let mut usage: HashMap<String, (i64, i64, i64)> = HashMap::new();

    for_each_line(path, |line| {
        // Request blobs and file snapshots are large and never displayed.
        if line.contains("\"type\":\"api-request")
            || line.contains("\"type\":\"file-history-snapshot\"")
        {
            return;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            return;
        };
        let t = str_of(&v, "type").unwrap_or("");
        match t {
            "custom-title" => custom_title = str_of(&v, "customTitle").map(str::to_string),
            "ai-title" => ai_title = str_of(&v, "aiTitle").map(str::to_string),
            "agent-name" => agent_name = str_of(&v, "agentName").map(str::to_string),
            "summary" => summary = str_of(&v, "summary").map(str::to_string),
            "user" | "assistant" => handle_message(&mut p, &v, t, &mut usage),
            "attachment" => handle_attachment(&mut p, &v),
            "system" => {
                let ts = parse_ts(v.get("timestamp"));
                match str_of(&v, "subtype") {
                    Some("compact_boundary") => {
                        p.events.push(Event::new(EventKind::Compaction, ts, ""));
                    }
                    Some("local_command") => {
                        if let Some(c) = str_of(&v, "content") {
                            push_user_text(&mut p, c, ts, false);
                        }
                    }
                    Some("api_error") => {
                        let mut e = Event::new(EventKind::System, ts, "API error");
                        e.is_error = true;
                        p.events.push(e);
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    })?;

    for (input, output, cache) in usage.values() {
        p.tokens.input += input;
        p.tokens.output += output;
        p.tokens.cache_read += cache;
    }

    p.title = custom_title
        .map(|t| (t, "custom"))
        .or(ai_title.map(|t| (t, "ai")))
        .or(agent_name.map(|t| (t, "name")))
        .or(summary.map(|t| (t, "summary")));

    Ok(p)
}

fn handle_message(
    p: &mut Parsed,
    v: &Value,
    t: &str,
    usage: &mut HashMap<String, (i64, i64, i64)>,
) {
    if p.cwd.is_none() {
        p.cwd = str_of(v, "cwd").map(str::to_string);
    }
    if let Some(b) = str_of(v, "gitBranch").filter(|b| !b.is_empty() && *b != "HEAD") {
        p.git_branch = Some(b.to_string());
    }
    if v.get("isMeta").and_then(Value::as_bool) == Some(true) {
        return;
    }
    let ts = parse_ts(v.get("timestamp"));
    if p.started_at.is_none() {
        p.started_at = ts;
    }
    let sidechain = v.get("isSidechain").and_then(Value::as_bool) == Some(true);
    let Some(msg) = v.get("message") else { return };
    let content = msg.get("content").unwrap_or(&Value::Null);

    if t == "user" {
        if v.get("isCompactSummary").and_then(Value::as_bool) == Some(true) {
            let mut imgs = Vec::new();
            p.events
                .push(Event::new(EventKind::Compaction, ts, content_text(content, &mut imgs)));
            return;
        }
        match content {
            Value::String(s) => push_user_text(p, s, ts, sidechain),
            Value::Array(blocks) => {
                for b in blocks {
                    match str_of(b, "type") {
                        Some("text") => {
                            if let Some(s) = str_of(b, "text") {
                                push_user_text(p, s, ts, sidechain);
                            }
                        }
                        Some("image") => {
                            let mut e = Event::new(EventKind::User, ts, "");
                            content_text(&Value::Array(vec![b.clone()]), &mut e.images);
                            e.sidechain = sidechain;
                            p.events.push(e);
                        }
                        Some("tool_result") => {
                            let mut e = Event::new(EventKind::ToolResult, ts, "");
                            e.text = content_text(b.get("content").unwrap_or(&Value::Null), &mut e.images);
                            e.call_id = str_of(b, "tool_use_id").map(str::to_string);
                            e.is_error = b.get("is_error").and_then(Value::as_bool) == Some(true);
                            e.sidechain = sidechain;
                            p.events.push(e);
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        return;
    }

    let model = str_of(msg, "model").map(str::to_string);
    if let Some(m) = &model {
        p.add_model(m);
    }
    if let (Some(id), Some(u)) = (str_of(msg, "id"), msg.get("usage")) {
        let n = |k: &str| u.get(k).and_then(Value::as_i64).unwrap_or(0);
        usage.insert(
            id.to_string(),
            (
                n("input_tokens") + n("cache_creation_input_tokens"),
                n("output_tokens"),
                n("cache_read_input_tokens"),
            ),
        );
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
            Some("tool_use") | Some("server_tool_use") => {
                let mut e = Event::new(
                    EventKind::ToolCall,
                    ts,
                    pretty(b.get("input").unwrap_or(&Value::Null)),
                );
                e.tool_name = str_of(b, "name").map(str::to_string);
                e.call_id = str_of(b, "id").map(str::to_string);
                e
            }
            _ => continue,
        };
        e.model = model.clone();
        e.sidechain = sidechain;
        p.events.push(e);
    }
}

/// An `@file` mention: Claude Code records the file's contents as an
/// attachment right after the prompt. Show it inside that prompt.
fn handle_attachment(p: &mut Parsed, v: &Value) {
    let Some(a) = v.get("attachment").filter(|a| str_of(a, "type") == Some("file")) else {
        return;
    };
    let Some(path) = str_of(a, "filename") else { return };
    let content = a.get("content");
    let body = content
        .and_then(|c| c.get("file"))
        .and_then(|f| str_of(f, "content"))
        .or_else(|| content.and_then(Value::as_str))
        .unwrap_or("");
    let is_image = content.and_then(|c| str_of(c, "type")) == Some("image");
    let tag = super::file_tag(path, if is_image { "image" } else { "" }, if is_image { "" } else { body });
    match p.events.iter_mut().rev().take(4).find(|e| e.kind == EventKind::User) {
        Some(e) => {
            e.text.push('\n');
            e.text.push_str(&tag);
        }
        None => p.events.push(Event::new(EventKind::User, parse_ts(v.get("timestamp")), tag)),
    }
}

fn push_user_text(p: &mut Parsed, raw: &str, ts: Option<i64>, sidechain: bool) {
    if let Some(mut e) = user_text_event(raw, ts) {
        e.sidechain = sidechain;
        p.events.push(e);
    }
}

/// Classifies a user-role text block. Claude Code wraps slash commands, shell
/// escapes and notifications in pseudo-XML; Codex sessions imported from
/// Claude Code carry the same wrappers.
pub fn user_text_event(raw: &str, ts: Option<i64>) -> Option<Event> {
    let s = raw.trim();
    if s.starts_with("<local-command-caveat>") || s.starts_with("Caveat:") {
        return None;
    }
    let e = if s.contains("<command-name>") {
        Event::new(EventKind::Command, ts, command_label(s))
    } else if s.starts_with("<local-command-stdout>") || s.starts_with("<local-command-stderr>") {
        let inner = between(s, ">", "</local-command-").unwrap_or("").trim();
        if inner.is_empty() {
            return None;
        }
        Event::new(EventKind::System, ts, inner)
    } else if s.starts_with("[Request interrupted") {
        Event::new(EventKind::System, ts, "Interrupted by user")
    } else if s.starts_with("<task-notification>") || s.starts_with("<bash-notification>") {
        let summary = between(s, "<summary>", "</summary>").unwrap_or(s).trim();
        Event::new(EventKind::System, ts, summary)
    } else if s.starts_with("<bash-input>") {
        let cmd = between(s, "<bash-input>", "</bash-input>").unwrap_or("").trim();
        Event::new(EventKind::Command, ts, format!("! {cmd}"))
    } else if s.starts_with("<bash-stdout>") {
        let out = between(s, "<bash-stdout>", "</bash-stdout>").unwrap_or("").trim();
        let err = between(s, "<bash-stderr>", "</bash-stderr>").unwrap_or("").trim();
        let text = [out, err].iter().filter(|x| !x.is_empty()).copied().collect::<Vec<_>>().join("\n");
        if text.is_empty() {
            return None;
        }
        Event::new(EventKind::System, ts, text)
    } else {
        let cleaned = super::normalize_prompt(&strip_tag_blocks(s, "system-reminder"));
        if cleaned.is_empty() {
            return None;
        }
        Event::new(EventKind::User, ts, cleaned)
    };
    Some(e)
}

/// "<command-name>/model</command-name><command-args>haiku</command-args>" -> "/model haiku"
pub fn command_label(s: &str) -> String {
    let name = between(s, "<command-name>", "</command-name>").unwrap_or("").trim();
    let args = between(s, "<command-args>", "</command-args>").unwrap_or("").trim();
    if args.is_empty() { name.to_string() } else { format!("{name} {args}") }
}
