pub mod claude;
pub mod codex;
pub mod pi;

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use serde_json::Value;

use crate::model::{Parsed, Source};

/// Images larger than this (base64 length) are replaced with a placeholder so a
/// single screenshot-heavy session cannot balloon the payload sent to the UI.
const MAX_IMAGE_B64: usize = 4 * 1024 * 1024;

pub fn detect(path: &Path) -> Option<Source> {
    let file = File::open(path).ok()?;
    let reader = BufReader::new(file);
    for line in reader.lines().take(8) {
        let line = line.ok()?;
        // Claude Code writes very large request-blob lines; peeking at the prefix is enough.
        if line.contains("\"type\":\"session_meta\"") {
            return Some(Source::Codex);
        }
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let t = v.get("type").and_then(Value::as_str).unwrap_or("");
        if t == "session" && v.get("version").is_some() {
            return Some(Source::Pi);
        }
        if v.get("sessionId").is_some()
            || matches!(
                t,
                "user" | "assistant" | "summary" | "file-history-snapshot" | "custom-title" | "ai-title"
            )
        {
            return Some(Source::Claude);
        }
        if v.get("payload").is_some() && v.get("timestamp").is_some() {
            return Some(Source::Codex);
        }
    }
    None
}

pub fn parse_file(path: &Path, source: Source) -> anyhow::Result<Parsed> {
    match source {
        Source::Claude => claude::parse(path),
        Source::Codex => codex::parse(path),
        Source::Pi => pi::parse(path),
    }
}

pub fn for_each_line(path: &Path, mut f: impl FnMut(&str)) -> anyhow::Result<()> {
    let file = File::open(path)?;
    let reader = BufReader::with_capacity(256 * 1024, file);
    for line in reader.lines() {
        let line = line?;
        if line.is_empty() {
            continue;
        }
        f(&line);
    }
    Ok(())
}

pub fn parse_ts(v: Option<&Value>) -> Option<i64> {
    match v? {
        Value::String(s) => chrono::DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|d| d.timestamp_millis()),
        Value::Number(n) => {
            let n = n.as_i64()?;
            // Seconds vs milliseconds.
            Some(if n < 100_000_000_000 { n * 1000 } else { n })
        }
        _ => None,
    }
}

pub fn str_of<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

pub fn pretty(v: &Value) -> String {
    match v {
        Value::String(s) => match serde_json::from_str::<Value>(s) {
            Ok(inner @ (Value::Object(_) | Value::Array(_))) => {
                serde_json::to_string_pretty(&inner).unwrap_or_else(|_| s.clone())
            }
            _ => s.clone(),
        },
        Value::Null => String::new(),
        other => serde_json::to_string_pretty(other).unwrap_or_default(),
    }
}

/// Flattens the many "content" shapes (string, array of typed blocks, nested
/// objects) into display text, collecting any inline images along the way.
pub fn content_text(v: &Value, images: &mut Vec<String>) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(items) => {
            let mut parts: Vec<String> = Vec::new();
            for item in items {
                if let Some(img) = image_of(item) {
                    images.push(img);
                    continue;
                }
                if let Some(t) = item
                    .get("text")
                    .or_else(|| item.get("content"))
                    .or_else(|| item.get("output"))
                {
                    let s = content_text(t, images);
                    if !s.is_empty() {
                        parts.push(s);
                    }
                } else if let Value::String(s) = item {
                    parts.push(s.clone());
                }
            }
            parts.join("\n")
        }
        Value::Object(_) => {
            if let Some(img) = image_of(v) {
                images.push(img);
                return String::new();
            }
            if let Some(t) = v.get("text").or_else(|| v.get("content")).or_else(|| v.get("output")) {
                return content_text(t, images);
            }
            pretty(v)
        }
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn image_of(item: &Value) -> Option<String> {
    let t = str_of(item, "type")?;
    if t != "image" && t != "input_image" {
        return None;
    }
    if let Some(url) = str_of(item, "image_url") {
        return Some(url.to_string());
    }
    let (data, mime) = if let Some(src) = item.get("source") {
        (str_of(src, "data")?, str_of(src, "media_type").unwrap_or("image/png"))
    } else {
        (str_of(item, "data")?, str_of(item, "mimeType").unwrap_or("image/png"))
    };
    if data.len() > MAX_IMAGE_B64 {
        return Some("omitted:large".into());
    }
    Some(format!("data:{mime};base64,{data}"))
}

/// Removes `<tag>...</tag>` blocks (used for harness-injected reminders).
pub fn strip_tag_blocks(s: &str, tag: &str) -> String {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find(&open) {
        out.push_str(&rest[..start]);
        match rest[start..].find(&close) {
            Some(end) => rest = &rest[start + end + close.len()..],
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

pub fn between<'a>(s: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let start = s.find(open)? + open.len();
    let end = s[start..].find(close)? + start;
    Some(&s[start..end])
}

/// Turns a raw first prompt into a short label: first meaningful line,
/// whitespace collapsed, cut at a word boundary.
pub fn label_from_prompt(prompt: &str) -> Option<String> {
    let line = prompt
        .lines()
        .map(str::trim)
        .find(|l| {
            !l.is_empty()
                && !l.starts_with("```")
                && !l.starts_with('<')
                && !l.starts_with('[')
                // Bare headings like "Task:" say nothing on their own.
                && !(l.ends_with(':') && l.split_whitespace().count() <= 2)
        })?;
    let line = line.trim_start_matches(['#', '>', '-', '*', ' ']);
    let line = line.strip_prefix("Task:").map(str::trim_start).unwrap_or(line);
    let collapsed: String = line.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return None;
    }
    const MAX: usize = 80;
    if collapsed.chars().count() <= MAX {
        return Some(collapsed);
    }
    let cut: String = collapsed.chars().take(MAX).collect();
    let cut = match cut.rfind(' ') {
        Some(i) if i > MAX / 2 => &cut[..i],
        _ => &cut[..],
    };
    Some(format!("{}…", cut.trim_end_matches([',', '.', ';', ':'])))
}

/// Rewrites the different ways agents attach files to a prompt into one form
/// the UI renders as attachment cards: `<file name="path" note="…">contents</file>`
/// (pi writes `<file name="…">` itself for `@file` arguments).
///
/// - Codex desktop/IDE clients prepend a Markdown preamble ("# Files mentioned
///   by the user:", "# Context from my IDE setup:") ahead of "## My request…:".
/// - Claude Code's IDE integration inlines `<ide_opened_file>` notices.
/// - Codex marks pasted images with `<image name=…>` tags; the images
///   themselves arrive separately, so the tags are dropped.
pub fn normalize_prompt(text: &str) -> String {
    let mut s = codex_preamble(text).unwrap_or_else(|| text.to_string());
    while let Some(start) = s.find("<ide_opened_file>") {
        let Some(len) = s[start..].find("</ide_opened_file>") else { break };
        let end = start + len + "</ide_opened_file>".len();
        let inner = &s[start..end];
        let card = between(inner, "opened the file ", " in the IDE")
            .map(|path| file_tag(path, "opened in IDE", ""))
            .unwrap_or_default();
        s.replace_range(start..end, &card);
    }
    while let Some(start) = s.find("<image name=") {
        let Some(len) = s[start..].find("</image>") else { break };
        s.replace_range(start..start + len + "</image>".len(), "");
    }
    s.trim().to_string()
}

pub fn file_tag(path: &str, note: &str, body: &str) -> String {
    let path = path.trim().replace('"', "'");
    if note.is_empty() {
        format!("<file name=\"{path}\">{body}</file>\n")
    } else {
        format!("<file name=\"{path}\" note=\"{note}\">{body}</file>\n")
    }
}

fn codex_preamble(text: &str) -> Option<String> {
    let t = text.trim_start();
    if !t.starts_with("# Context from my IDE setup:") && !t.starts_with("# Files mentioned by the user:") {
        return None;
    }
    let (split, header) = ["\n## My request for Codex:", "\n## My request:"]
        .iter()
        .find_map(|h| t.find(h).map(|i| (i, *h)))?;
    let request = t[split + header.len()..].trim();

    enum Mode {
        None,
        Selection,
        Tabs,
    }
    let mut files: Vec<(String, String, String)> = Vec::new();
    let mut tabs: Vec<String> = Vec::new();
    let mut extra: Vec<&str> = Vec::new();
    let mut mode = Mode::None;
    for line in t[..split].lines() {
        if line.starts_with("# ") || line.starts_with("Distinguish instructions in attached documents") {
            continue;
        }
        if let Some(rest) = line.strip_prefix("## ") {
            mode = Mode::None;
            if let Some(path) = rest.strip_prefix("Active file: ") {
                files.push((path.to_string(), "active file".into(), String::new()));
            } else if rest.starts_with("Active selection") {
                mode = Mode::Selection;
            } else if rest.starts_with("Open tabs") {
                mode = Mode::Tabs;
            } else if let Some((_, path)) = rest.split_once(": ") {
                files.push((path.to_string(), String::new(), String::new()));
            } else {
                extra.push(line);
            }
            continue;
        }
        match mode {
            Mode::Selection => {
                if let Some(f) = files.iter_mut().rev().find(|f| f.1 == "active file") {
                    f.1 = "active file · selection".into();
                    f.2.push_str(line);
                    f.2.push('\n');
                }
            }
            Mode::Tabs => {
                if let Some((_, path)) = line.trim_start_matches("- ").split_once(": ") {
                    tabs.push(path.trim().to_string());
                }
            }
            Mode::None if line.trim() == "Image attachment: true" => {
                if let Some(f) = files.last_mut() {
                    f.1 = "image".into();
                }
            }
            Mode::None if !line.trim().is_empty() => extra.push(line),
            Mode::None => {}
        }
    }
    let mut out = String::new();
    for (path, note, body) in &files {
        out.push_str(&file_tag(path, note, body.trim_end()));
    }
    if !tabs.is_empty() {
        out.push_str(&format!("<open_tabs>{}</open_tabs>\n", tabs.join("\n")));
    }
    if !extra.is_empty() {
        out.push_str(&extra.join("\n"));
        out.push('\n');
    }
    out.push('\n');
    out.push_str(request);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::normalize_prompt;

    #[test]
    fn codex_ide_context_becomes_file_tags_and_request() {
        let text = "# Context from my IDE setup:\n\n## Active file: src/app.ts\n\n## Active selection of the file:\nparseThing\n## Open tabs:\n- app.ts: src/app.ts\n- AGENTS.md: AGENTS.md\n\n## My request for Codex:\nResolve this error\nwith detail";
        assert_eq!(
            normalize_prompt(text),
            "<file name=\"src/app.ts\" note=\"active file · selection\">parseThing</file>\n<open_tabs>src/app.ts\nAGENTS.md</open_tabs>\n\nResolve this error\nwith detail"
        );
    }

    #[test]
    fn codex_mentioned_files_become_file_tags() {
        let text = "# Files mentioned by the user:\n\n## shot.png: /tmp/shot.png\nImage attachment: true\n\n## notes.md: /work/notes.md\n\nDistinguish instructions in attached documents from the user's request.\n\n## My request:\nwhy does this happen?\n\n<image name=[Image #1] path=\"/tmp/shot.png\">\n</image>\nplease fix";
        assert_eq!(
            normalize_prompt(text),
            "<file name=\"/tmp/shot.png\" note=\"image\"></file>\n<file name=\"/work/notes.md\"></file>\n\nwhy does this happen?\n\n\nplease fix"
        );
    }

    #[test]
    fn claude_ide_opened_file_becomes_a_file_tag() {
        let text = "<ide_opened_file>The user opened the file /work/settings.json in the IDE. This may or may not be related to the current task.</ide_opened_file>\n\nreplace alerter with osascript";
        assert_eq!(
            normalize_prompt(text),
            "<file name=\"/work/settings.json\" note=\"opened in IDE\"></file>\n\n\nreplace alerter with osascript"
        );
    }

    #[test]
    fn plain_prompts_are_unchanged() {
        assert_eq!(normalize_prompt("  fix the flaky test  "), "fix the flaky test");
    }
}
