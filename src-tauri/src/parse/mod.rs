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
