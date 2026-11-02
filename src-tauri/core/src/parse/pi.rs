use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde_json::Value;

use super::{between, content_text, for_each_line, parse_ts, pretty, str_of};
use crate::model::{Branch, ContextKind, Event, EventKind, Parsed, Source};

pub fn parse(path: &Path) -> anyhow::Result<Parsed> {
    parse_branch(path, None)
}

/// Parses the session, rendering the branch that ends at `leaf`, or the
/// active branch (the one ending at the last entry) when `leaf` is None.
pub fn parse_branch(path: &Path, leaf: Option<&str>) -> anyhow::Result<Parsed> {
    parse_tree(path, leaf, true)
}

/// Like `parse_branch`, but always renders the whole root-to-leaf chain,
/// including the history a branch or fork shares with its source.
pub fn parse_chain(path: &Path, leaf: Option<&str>) -> anyhow::Result<Parsed> {
    parse_tree(path, leaf, false)
}

fn parse_tree(path: &Path, leaf: Option<&str>, from_split: bool) -> anyhow::Result<Parsed> {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let fallback_id = stem.rsplit('_').next().unwrap_or(stem);
    let mut p = Parsed::new(Source::Pi, fallback_id.to_string());

    // pi stores a session as a tree (entries point at parentId) so the user can
    // rewind and branch. Collect everything, then render one root-to-leaf path.
    // Every other leaf is reported in `branches` so the UI can open it.
    let mut entries: Vec<Value> = Vec::new();
    let mut name: Option<String> = None;
    let mut parent_file: Option<String> = None;
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
                    parent_file = Some(parent.to_string());
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

    let active = path_to(&entries, None);
    p.branches = branches(&entries, &active);
    let shown = match leaf {
        Some(id) => path_to(&entries, Some(id)),
        None => active.clone(),
    };
    // A branch or fork is shown from the point where it split off; the UI
    // links back to the split point in its source.
    let start = match leaf {
        _ if !from_split => 0,
        Some(id) => p
            .branches
            .iter()
            .find(|b| b.leaf == id)
            .and_then(|b| b.fork_entry.as_deref())
            .and_then(|f| shown.iter().position(|v| str_of(v, "id") == Some(f)))
            .map_or(0, |i| i + 1),
        None => match parent_file.as_deref().and_then(|f| shared_prefix(&shown, Path::new(f))) {
            Some(n) => {
                p.fork_entry = last_shown_entry(&shown[..n]).map(str::to_string);
                p.fork_turn = Some(count_prompts(&shown[..n]));
                n
            }
            None => 0,
        },
    };
    // Every entry of the chain is rendered so anchors ("e<n>") count from the
    // session root; the part before the split is then dropped from the view.
    let mut hidden = 0;
    for (i, v) in shown.iter().enumerate() {
        if i == start {
            hidden = p.events.len();
            // Models used before the split belong to the source session.
            p.models.clear();
        }
        let before = p.events.len();
        handle_entry(&mut p, v);
        // Entry ids let the UI place fork markers next to the right message.
        let id = str_of(v, "id").map(str::to_string);
        for e in &mut p.events[before..] {
            e.entry = id.clone();
        }
    }
    if start >= shown.len() {
        hidden = p.events.len();
    }
    super::assign_anchors(&mut p.events);
    p.events.drain(..hidden);
    // Token usage counts every model call, including abandoned branches.
    for v in &entries {
        if let Some(u) = v.get("message").filter(|m| str_of(m, "role") == Some("assistant")).and_then(|m| m.get("usage")) {
            let n = |k: &str| u.get(k).and_then(Value::as_i64).unwrap_or(0);
            p.tokens.input += n("input") + n("cacheWrite");
            p.tokens.output += n("output");
            p.tokens.cache_read += n("cacheRead");
        }
    }
    super::mark_failed_exits(&mut p.events);
    super::fill_context_scopes(&mut p.events, p.cwd.as_deref());
    super::mark_repeated_context(&mut p.events);
    Ok(p)
}

/// The id of every entry in the file, on any branch.
pub fn entry_ids(path: &Path) -> anyhow::Result<HashSet<String>> {
    let mut ids = HashSet::new();
    for_each_line(path, |line| {
        if let Ok(v) = serde_json::from_str::<Value>(line)
            && let Some(id) = str_of(&v, "id")
            && str_of(&v, "type") != Some("session")
        {
            ids.insert(id.to_string());
        }
    })?;
    Ok(ids)
}

/// The agent named by an `<active_agent name="…"/>` tag in `text`, and the
/// text without the tag.
fn agent_tag(text: &str) -> Option<(&str, String)> {
    let start = text.find("<active_agent")?;
    let end = start + text[start..].find('>')? + 1;
    let name = between(&text[start..end], "name=\"", "\"")?;
    Some((name, format!("{}{}", &text[..start], &text[end..])))
}

/// The root-to-leaf chain ending at `leaf`, or at the last entry in the file.
fn path_to<'a>(entries: &'a [Value], leaf: Option<&str>) -> Vec<&'a Value> {
    let by_id: HashMap<&str, &Value> = entries
        .iter()
        .filter_map(|v| str_of(v, "id").map(|id| (id, v)))
        .collect();
    let end = match leaf {
        Some(id) => by_id.get(id).copied(),
        None => entries.iter().rev().find(|v| str_of(v, "id").is_some()),
    };
    let Some(end) = end else {
        return entries.iter().collect();
    };
    let mut chain = vec![end];
    let mut seen = HashSet::new();
    let mut cur = end;
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
    // Without parent links there is no tree; show the file in order.
    if chain.len() == 1 && entries.len() > 1 {
        return entries.iter().collect();
    }
    chain.reverse();
    chain
}

/// Every leaf other than the active one whose own part holds at least one
/// message (leaves that only change a setting are not worth showing),
/// arranged as a tree: a branch's source is whichever earlier branch (or the
/// active one) it shares the longest history with.
fn branches(entries: &[Value], active: &[&Value]) -> Vec<Branch> {
    let parents: HashSet<&str> = entries.iter().filter_map(|v| str_of(v, "parentId")).collect();
    let active_ids: HashSet<&str> = active.iter().filter_map(|v| str_of(v, "id")).collect();

    struct Candidate<'a> {
        leaf: &'a str,
        chain: Vec<&'a Value>,
        started_at: Option<i64>,
    }
    let mut candidates: Vec<Candidate> = entries
        .iter()
        .filter_map(|v| str_of(v, "id"))
        .filter(|id| !parents.contains(id) && !active_ids.contains(id))
        .filter_map(|leaf| {
            let chain = path_to(entries, Some(leaf));
            let split = common_prefix(&chain, active);
            let own = &chain[split..];
            if !own.iter().any(|v| is_message(v)) {
                return None;
            }
            let started_at = own.iter().find_map(|v| parse_ts(v.get("timestamp")));
            Some(Candidate { leaf, started_at, chain })
        })
        .collect();
    candidates.sort_by_key(|c| c.started_at);

    let mut out: Vec<Branch> = Vec::new();
    for (i, c) in candidates.iter().enumerate() {
        let mut source: Option<&str> = None;
        let mut split = common_prefix(&c.chain, active);
        for earlier in &candidates[..i] {
            let n = common_prefix(&c.chain, &earlier.chain);
            if n > split && out.iter().any(|b| b.leaf == earlier.leaf) {
                split = n;
                source = Some(earlier.leaf);
            }
        }
        let own = &c.chain[split..];
        let messages: Vec<&Value> = own.iter().filter(|v| is_message(v)).filter_map(|v| v.get("message")).collect();
        if messages.is_empty() {
            continue;
        }
        let is_user = |m: &Value| str_of(m, "role") == Some("user");
        let label = messages
            .iter()
            .find(|m| is_user(m))
            .and_then(|m| super::label_from_prompt(&prompt_text(m)))
            .or_else(|| {
                // No new prompt: the model was re-run from an earlier prompt.
                c.chain[..split]
                    .iter()
                    .rev()
                    .filter_map(|v| v.get("message"))
                    .find(|m| is_user(m))
                    .and_then(|m| super::label_from_prompt(&prompt_text(m)))
                    .map(|l| format!("Retry: {l}"))
            })
            .unwrap_or_else(|| "Alternate branch".to_string());
        out.push(Branch {
            leaf: c.leaf.to_string(),
            label,
            started_at: c.started_at,
            updated_at: own.iter().rev().find_map(|v| parse_ts(v.get("timestamp"))),
            user_turns: messages.iter().filter(|m| is_user(m)).count() as i64,
            fork_turn: count_prompts(&c.chain[..split]),
            fork_entry: last_shown_entry(&c.chain[..split]).map(str::to_string),
            parent_leaf: source.map(str::to_string),
        });
    }
    out
}

fn common_prefix(a: &[&Value], b: &[&Value]) -> usize {
    a.iter()
        .zip(b)
        .take_while(|(x, y)| str_of(x, "id").is_some() && str_of(x, "id") == str_of(y, "id"))
        .count()
}

fn is_message(v: &Value) -> bool {
    str_of(v, "type") == Some("message")
}

fn count_prompts(chain: &[&Value]) -> i64 {
    chain
        .iter()
        .filter(|v| v.get("message").is_some_and(|m| str_of(m, "role") == Some("user")))
        .count() as i64
}

/// The last entry in `chain` that produces something in the transcript, so a
/// fork marker placed after it is visible.
fn last_shown_entry<'a>(chain: &[&'a Value]) -> Option<&'a str> {
    chain
        .iter()
        .rev()
        .find(|v| match str_of(v, "type") {
            Some("message") => v.get("message").is_some_and(|m| str_of(m, "role") != Some("system")),
            Some("compaction") | Some("branch_summary") | Some("custom_message") => true,
            _ => false,
        })
        .and_then(|v| str_of(v, "id"))
}

fn prompt_text(m: &Value) -> String {
    let mut imgs = Vec::new();
    let text = content_text(m.get("content").unwrap_or(&Value::Null), &mut imgs);
    // Skill invocations inline the SKILL.md ahead of the user's own words.
    match text.find("</skill>") {
        Some(i) if text.starts_with("<skill name=") => text[i + "</skill>".len()..].to_string(),
        _ => text,
    }
}

/// How many leading entries of a fork file's chain were copied from the
/// session it was forked from (pi keeps the entry ids when it copies them).
fn shared_prefix(chain: &[&Value], parent: &Path) -> Option<usize> {
    let ids = entry_ids(parent).ok()?;
    let n = chain.iter().take_while(|v| str_of(v, "id").is_some_and(|id| ids.contains(id))).count();
    (n > 0).then_some(n)
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
                        // Attributes are read from the opening tag only; the body can mention anything.
                        let open = e.text.find('>').map_or(e.text.as_str(), |i| &e.text[..i]);
                        let location = between(open, "location=\"", "\"").unwrap_or(name);
                        if let Some(body) = between(&e.text, ">", "</skill>") {
                            p.events.push(super::context_event(ContextKind::Skill, location, ts, body));
                        }
                        e.text = match e.text.find("</skill>") {
                            Some(i) => e.text[i + "</skill>".len()..].trim().to_string(),
                            None => String::new(),
                        };
                    }
                    if !e.text.is_empty() || !e.images.is_empty() {
                        p.events.push(e);
                    }
                }
                // A subagent run records its system prompt in sections: the
                // agent's definition (named by <active_agent>, in the preamble
                // or the addendum), more instructions in the addendum, and
                // the project's instruction files. A main session's log has
                // no system message.
                "system" => {
                    let sections = m.get("sections");
                    let section = |k: &str| sections.and_then(|s| str_of(s, k)).unwrap_or("");
                    for key in ["preamble", "addendum"] {
                        let text = section(key);
                        let text = text.trim().strip_prefix("<addendum>").map_or(text, |t| t.trim_end().trim_end_matches("</addendum>"));
                        match agent_tag(text) {
                            Some((agent, body)) => p.events.push(super::context_event(ContextKind::Agent, agent, ts, &body)),
                            None if key == "addendum" && !text.trim().is_empty() => {
                                p.events.push(super::context_event(ContextKind::Instructions, "system addendum", ts, text))
                            }
                            None => {}
                        }
                    }
                    let mut rest = section("project_context");
                    while let Some(start) = rest.find("<project_instructions path=\"") {
                        let tag = &rest[start + "<project_instructions path=\"".len()..];
                        let Some(close) = tag.find("</project_instructions>") else { break };
                        let (Some(path_end), Some(open_end)) = (tag.find('"'), tag[..close].find('>')) else {
                            // An opening tag that never closes: skip to the next block.
                            rest = &tag[close..];
                            continue;
                        };
                        p.events.push(super::context_event(ContextKind::Instructions, &tag[..path_end], ts, &tag[open_end + 1..close]));
                        rest = &tag[close..];
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
                    // pi's bash tool returns a non-zero exit as an error result
                    // ending "Command exited with code N"; timeouts and other
                    // failures end differently and stay errors.
                    if e.is_error && e.tool_name.as_deref() == Some("bash") {
                        let last = e.text.trim_end().lines().last().unwrap_or("");
                        let code = last.strip_prefix("Command exited with code ").and_then(super::leading_int);
                        if code.is_some() {
                            e.exit_code = code;
                            e.is_error = false;
                        }
                    }
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
                        let code = m.get("exitCode").and_then(Value::as_i64).filter(|&c| c != 0);
                        e.exit_code = code;
                        e.failed_exit = code.is_some_and(|c| !super::benign_exit(cmd, c));
                        p.events.push(e);
                    }
                }
                _ => {}
            }
        }
        _ => {}
    }
}
